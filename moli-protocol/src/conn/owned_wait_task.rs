//! Supervision shared by command and browser-owner workflows.

use std::{future::Future, marker::PhantomData, panic::AssertUnwindSafe};

use futures_util::FutureExt;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnedProtocolWaitFailure {
    Panicked,
    Cancelled,
}

impl OwnedProtocolWaitFailure {
    pub fn message(self) -> &'static str {
        match self {
            Self::Panicked => "CommandWaitPanicked",
            Self::Cancelled => "CommandWaitCancelled",
        }
    }
}

/// Owned by the existing in-flight entry. Dropping that entry revokes its
/// external wait even when the actor itself is cancelled before normal exit.
#[derive(Debug)]
pub struct OwnedProtocolWaitTask(Option<JoinHandle<()>>);

impl OwnedProtocolWaitTask {
    pub fn abort(&self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }

    pub async fn cancel_and_join(mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            // This joins cancellation/destruction, not the original external
            // operation: the wait is no longer polled for business progress.
            let _ = task.await;
        }
    }
}

impl Drop for OwnedProtocolWaitTask {
    fn drop(&mut self) {
        self.abort();
    }
}

struct CompletionOnDrop<T, C: FnOnce(Result<T, OwnedProtocolWaitFailure>)> {
    complete: Option<C>,
    result_type: PhantomData<fn(T)>,
}

impl<T, C: FnOnce(Result<T, OwnedProtocolWaitFailure>)> CompletionOnDrop<T, C> {
    fn finish(mut self, result: Result<T, OwnedProtocolWaitFailure>) {
        self.complete.take().expect("one wait completion authority")(result);
    }
}

impl<T, C: FnOnce(Result<T, OwnedProtocolWaitFailure>)> Drop for CompletionOnDrop<T, C> {
    fn drop(&mut self) {
        if let Some(complete) = self.complete.take() {
            complete(Err(OwnedProtocolWaitFailure::Cancelled));
        }
    }
}

pub fn spawn_owned_protocol_wait<F, T, C>(wait: F, complete: C) -> OwnedProtocolWaitTask
where
    F: Future<Output = T> + 'static,
    T: 'static,
    C: FnOnce(Result<T, OwnedProtocolWaitFailure>) + 'static,
{
    // Construct the cancellation authority before spawning: abort-before-first-
    // poll must notify the actor too, not only cancellation of a polled wait.
    let completion = CompletionOnDrop {
        complete: Some(complete),
        result_type: PhantomData,
    };
    OwnedProtocolWaitTask(Some(tokio::task::spawn_local(async move {
        let result = AssertUnwindSafe(wait)
            .catch_unwind()
            .await
            .map_err(|_| OwnedProtocolWaitFailure::Panicked);
        completion.finish(result);
    })))
}
