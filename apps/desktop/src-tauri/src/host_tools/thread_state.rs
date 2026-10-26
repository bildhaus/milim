//! Host tool state kept per chat thread: the [`ToolState`] that the runs of
//! one thread share, so a later turn still knows which files were read, which
//! directory the shell is in, and which background processes it started.
//!
//! A thread's state is dropped, killing its background processes, when the
//! thread is deleted or archived ([`release`]), when a run of it starts in a
//! different workspace, when the app shuts down ([`release_all`]), and after
//! [`IDLE_RELEASE`] without a run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use super::ToolState;

/// How long a thread's state outlives its last run.
pub(super) const IDLE_RELEASE: Duration = Duration::from_secs(30 * 60);
/// How often idle thread state is looked for.
const REAP_INTERVAL: Duration = Duration::from_secs(60);

struct Entry {
    /// The workspace the thread's runs work in.
    workspace: PathBuf,
    state: Arc<ToolState>,
    /// Runs of the thread whose tools are still alive.
    active_runs: usize,
    /// When the last run of the thread ended.
    idle_since: Instant,
}

fn threads() -> &'static Mutex<HashMap<String, Entry>> {
    static THREADS: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    THREADS.get_or_init(Mutex::default)
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    threads()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One run's hold on its thread's state. The thread counts as idle once
/// every run's binding has dropped.
pub(super) struct ThreadBinding {
    thread_id: String,
    pub(super) state: Arc<ToolState>,
}

impl Drop for ThreadBinding {
    fn drop(&mut self) {
        if let Some(entry) = lock()
            .get_mut(&self.thread_id)
            .filter(|entry| Arc::ptr_eq(&entry.state, &self.state))
        {
            entry.active_runs = entry.active_runs.saturating_sub(1);
            entry.idle_since = Instant::now();
        }
    }
}

/// Bind a run to the state of `thread_id` in `workspace`. A thread that moved
/// to another workspace starts over; its old state goes once no run holds it.
pub(super) fn bind(thread_id: &str, workspace: &Path) -> ThreadBinding {
    start_reaper();
    let mut threads = lock();
    let replaced = match threads.get(thread_id) {
        Some(entry) if entry.workspace == workspace => None,
        _ => threads.insert(
            thread_id.to_string(),
            Entry {
                workspace: workspace.to_path_buf(),
                state: Arc::default(),
                active_runs: 0,
                idle_since: Instant::now(),
            },
        ),
    };
    let entry = threads.get_mut(thread_id).expect("just inserted");
    entry.active_runs += 1;
    let binding = ThreadBinding {
        thread_id: thread_id.to_string(),
        state: entry.state.clone(),
    };
    drop(threads);
    drop(replaced);
    binding
}

/// Drop the state of `thread_id`. Background processes die once no run of
/// the thread still holds the state.
pub(super) fn release(thread_id: &str) {
    let removed = lock().remove(thread_id);
    drop(removed);
}

/// Drop every thread's state, as the app shuts down.
pub(crate) fn release_all() {
    let removed = std::mem::take(&mut *lock());
    drop(removed);
}

/// Drop the state of threads with no live run for at least `idle`, and
/// return how many were dropped.
fn release_idle(now: Instant, idle: Duration) -> usize {
    release_idle_where(now, idle, |_| true)
}

fn release_idle_where(now: Instant, idle: Duration, considered: impl Fn(&str) -> bool) -> usize {
    let removed = {
        let mut threads = lock();
        let expired = threads
            .iter()
            .filter(|(thread_id, entry)| {
                considered(thread_id)
                    && entry.active_runs == 0
                    && now.saturating_duration_since(entry.idle_since) >= idle
            })
            .map(|(thread_id, _)| thread_id.clone())
            .collect::<Vec<_>>();
        expired
            .iter()
            .filter_map(|thread_id| threads.remove(thread_id))
            .collect::<Vec<_>>()
    };
    removed.len()
}

fn start_reaper() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("milim-host-tool-reaper".into())
            .spawn(|| loop {
                std::thread::sleep(REAP_INTERVAL);
                release_idle(Instant::now(), IDLE_RELEASE);
            });
    });
}

#[cfg(test)]
pub(super) fn is_tracked(thread_id: &str) -> bool {
    lock().contains_key(thread_id)
}

/// [`release_idle`] limited to one thread, so tests leave others alone.
#[cfg(test)]
pub(super) fn release_idle_thread(thread_id: &str, now: Instant) -> usize {
    release_idle_where(now, IDLE_RELEASE, |id| id == thread_id)
}
