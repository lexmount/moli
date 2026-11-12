use std::time::Duration;

use crossbeam_channel::{Receiver, after, never, select_biased};

pub(crate) enum SynchronousLoadOutcome<T> {
    Completed(Result<T, String>),
    TimedOut,
    OwnerCancelled,
}

/// Suspends the caller, not the resource executor or protocol control lane.
/// This waiter deliberately has no PageVm, DOM, V8 or task-dispatch capability.
pub(crate) fn wait_for_synchronous_load<T>(
    completion: Receiver<Result<T, String>>,
    owner_cancel: &Receiver<()>,
    timeout: Option<Duration>,
) -> SynchronousLoadOutcome<T> {
    let deadline = timeout.map(after).unwrap_or_else(never);
    select_biased! {
        recv(owner_cancel) -> _ => SynchronousLoadOutcome::OwnerCancelled,
        recv(deadline) -> _ => SynchronousLoadOutcome::TimedOut,
        recv(completion) -> result => SynchronousLoadOutcome::Completed(
            result.unwrap_or_else(|_| Err("resource task dropped its completion channel".into()))
        ),
    }
}
