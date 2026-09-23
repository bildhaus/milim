//! Per-key async locks whose map entries disappear once nobody uses them.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Mutex as AsyncMutex;

/// Map from key (command id, thread id) to the async lock serializing it.
pub(crate) type KeyedLocks = Mutex<HashMap<String, Arc<AsyncMutex<()>>>>;

/// A shared handle to one key's async lock. Dropping the last lease for a key
/// removes its map entry, so one-shot keys such as command ids do not
/// accumulate for the life of the process. A concurrent caller for the same
/// key keeps the entry alive, so retries still serialize on one lock.
pub(crate) struct KeyedLockLease<'a> {
    locks: &'a KeyedLocks,
    key: String,
    lock: Option<Arc<AsyncMutex<()>>>,
}

impl<'a> KeyedLockLease<'a> {
    pub(crate) fn acquire(locks: &'a KeyedLocks, key: &str) -> Self {
        let lock = locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.to_string())
            .or_default()
            .clone();
        Self {
            locks,
            key: key.to_string(),
            lock: Some(lock),
        }
    }
}

impl Deref for KeyedLockLease<'_> {
    type Target = AsyncMutex<()>;

    fn deref(&self) -> &Self::Target {
        self.lock
            .as_deref()
            .expect("keyed lock lease holds its lock until dropped")
    }
}

impl Drop for KeyedLockLease<'_> {
    fn drop(&mut self) {
        // Release this reference and inspect the entry under the map lock.
        // New leases clone only under the same lock, so a count of one (the
        // map's own reference) means no caller can still be using the entry.
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(lock) = self.lock.take() else {
            return;
        };
        let is_current = locks
            .get(&self.key)
            .is_some_and(|entry| Arc::ptr_eq(entry, &lock));
        drop(lock);
        if is_current
            && locks
                .get(&self.key)
                .is_some_and(|entry| Arc::strong_count(entry) == 1)
        {
            locks.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn last_lease_removes_the_entry() {
        let locks = KeyedLocks::default();
        {
            let lease = KeyedLockLease::acquire(&locks, "command-1");
            let _guard = lease.lock().await;
            assert_eq!(locks.lock().unwrap().len(), 1);
        }
        assert!(locks.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_same_key_leases_share_one_lock_until_both_finish() {
        let locks = KeyedLocks::default();
        let first = KeyedLockLease::acquire(&locks, "command-1");
        let second = KeyedLockLease::acquire(&locks, "command-1");
        let guard = first.lock().await;
        assert!(second.try_lock().is_err(), "same key must serialize");
        drop(guard);
        drop(first);
        assert_eq!(locks.lock().unwrap().len(), 1, "retry still holds the key");
        assert!(second.try_lock().is_ok());
        drop(second);
        assert!(locks.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn distinct_keys_do_not_interfere() {
        let locks = KeyedLocks::default();
        let first = KeyedLockLease::acquire(&locks, "command-1");
        let second = KeyedLockLease::acquire(&locks, "command-2");
        let _first_guard = first.lock().await;
        assert!(second.try_lock().is_ok());
        drop(second);
        assert_eq!(locks.lock().unwrap().len(), 1);
    }
}
