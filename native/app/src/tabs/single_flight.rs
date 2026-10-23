//! At most one request of a kind in flight per poller. A poll stops by
//! dropping its GPUI task, but the request it was awaiting runs on in the
//! runtime (a blocking exec on the host's shared session, which can hold its
//! mutex for up to the 60 s timeout). Without this, every tab switch or panel
//! toggle on a stalled host sent another one, each in its own blocking
//! thread queued on that one mutex, delaying the tab's Docker, SFTP and
//! security commands behind them. A restarted poll now awaits the request
//! still running instead of sending a second.

use std::future::Future;

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};

pub(crate) struct SingleFlight<T: Clone> {
    inflight: Option<Shared<BoxFuture<'static, T>>>,
}

impl<T: Clone + Send + Sync + 'static> SingleFlight<T> {
    pub(crate) fn new() -> Self {
        Self { inflight: None }
    }

    /// The request still running, or a new one from `start`.
    pub(crate) fn get_or_start<F>(
        &mut self,
        start: impl FnOnce() -> F,
    ) -> Shared<BoxFuture<'static, T>>
    where
        F: Future<Output = T> + Send + 'static,
    {
        if let Some(running) = &self.inflight
            && running.peek().is_none()
        {
            return running.clone();
        }
        let next = start().boxed().shared();
        self.inflight = Some(next.clone());
        next
    }

    /// Drop the one in flight (the session it ran on is gone), so the next
    /// poll starts afresh.
    pub(crate) fn forget(&mut self) {
        self.inflight = None;
    }
}

/// A runtime task's result, with a join failure as the error text.
pub(crate) async fn joined<T>(
    handle: tokio::task::JoinHandle<Result<T, String>>,
    what: &str,
) -> Result<T, String> {
    handle
        .await
        .unwrap_or_else(|e| Err(format!("{what} task failed: {e}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::channel::oneshot;
    use futures::executor::block_on;

    use super::SingleFlight;

    #[test]
    fn a_restart_awaits_the_request_still_running() {
        let started = Arc::new(AtomicUsize::new(0));
        let mut flight = SingleFlight::<u32>::new();
        let (tx, rx) = oneshot::channel::<u32>();
        let mut rx = Some(rx);
        let mut start = || {
            started.fetch_add(1, Ordering::SeqCst);
            let rx = rx.take();
            async move {
                match rx {
                    Some(rx) => rx.await.unwrap_or(0),
                    None => 7,
                }
            }
        };
        let first = flight.get_or_start(&mut start);
        // The poll that awaited it was dropped; a restart reuses it.
        drop(first);
        let second = flight.get_or_start(&mut start);
        assert_eq!(started.load(Ordering::SeqCst), 1);
        tx.send(5).unwrap();
        assert_eq!(block_on(second), 5);
        // Finished: the next poll sends a new request.
        assert_eq!(block_on(flight.get_or_start(&mut start)), 7);
        assert_eq!(started.load(Ordering::SeqCst), 2);
        flight.forget();
        drop(flight.get_or_start(&mut start));
        assert_eq!(started.load(Ordering::SeqCst), 3);
    }
}
