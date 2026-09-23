//! Keep synchronous SQLite work off the async executor's worker threads.

use milim_core::{Error, Result};

/// Run blocking store work on tokio's blocking pool and await its result.
/// The closure must own what it touches, so no std lock guard can be held
/// across the `.await`.
pub(crate) async fn run<T, F>(work: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| Error::Other(format!("blocking store task failed: {error}")))?
}

/// Run blocking work from synchronous code that may be executing on an async
/// worker thread, such as timeline appends inside a streaming loop. On the
/// multi-threaded runtime the worker first hands its other tasks to another
/// thread; elsewhere (current-thread runtimes, plain threads) the work runs
/// inline.
pub(crate) fn in_place<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn in_place_runs_on_multi_thread_workers() {
        assert_eq!(in_place(|| 7), 7);
        assert_eq!(run(|| Ok(in_place(|| 8))).await.unwrap(), 8);
    }

    #[tokio::test]
    async fn in_place_runs_inline_on_current_thread_runtime() {
        assert_eq!(in_place(|| 9), 9);
        assert_eq!(run(|| Ok(10)).await.unwrap(), 10);
    }

    #[test]
    fn in_place_runs_outside_a_runtime() {
        assert_eq!(in_place(|| 11), 11);
    }
}
