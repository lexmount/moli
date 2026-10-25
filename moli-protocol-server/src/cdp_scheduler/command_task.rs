use std::{future::Future, marker::PhantomData, panic::AssertUnwindSafe};

use futures_util::FutureExt;
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CommandWaitIdentity {
    pub token: u64,
    pub turn: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommandWaitFailure {
    Panicked,
    Cancelled,
}

impl CommandWaitFailure {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Panicked => "CommandWaitPanicked",
            Self::Cancelled => "CommandWaitCancelled",
        }
    }
}

pub(super) struct CommandWaitCompletion<T> {
    pub identity: CommandWaitIdentity,
    pub result: Result<T, CommandWaitFailure>,
}

/// Owned by the existing in-flight entry. Dropping that entry revokes its
/// external wait even when the actor itself is cancelled before normal exit.
pub(super) struct CommandWaitTask(Option<JoinHandle<()>>);

impl CommandWaitTask {
    pub(super) fn abort(&self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }

    pub(super) async fn cancel_and_join(mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            // This joins cancellation/destruction, not the original external
            // operation: the wait is no longer polled for business progress.
            let _ = task.await;
        }
    }
}

impl Drop for CommandWaitTask {
    fn drop(&mut self) {
        self.abort();
    }
}

struct CompletionOnDrop<T, C: FnOnce(Result<T, CommandWaitFailure>)> {
    complete: Option<C>,
    result_type: PhantomData<fn(T)>,
}

impl<T, C: FnOnce(Result<T, CommandWaitFailure>)> CompletionOnDrop<T, C> {
    fn finish(mut self, result: Result<T, CommandWaitFailure>) {
        self.complete.take().expect("one wait completion authority")(result);
    }
}

impl<T, C: FnOnce(Result<T, CommandWaitFailure>)> Drop for CompletionOnDrop<T, C> {
    fn drop(&mut self) {
        if let Some(complete) = self.complete.take() {
            complete(Err(CommandWaitFailure::Cancelled));
        }
    }
}

pub(super) fn spawn_command_wait<F, T>(
    identity: CommandWaitIdentity,
    wait: F,
    sender: &mpsc::UnboundedSender<CommandWaitCompletion<T>>,
) -> CommandWaitTask
where
    F: Future<Output = T> + 'static,
    T: 'static,
{
    let sender = sender.clone();
    spawn_owned_wait(wait, move |result| {
        // A closed receiver means the actor has already discarded its in-flight
        // entries. Dropping the undeliverable result releases its owned work.
        let _ = sender.send(CommandWaitCompletion { identity, result });
    })
}

pub(super) fn spawn_owned_wait<F, T, C>(wait: F, complete: C) -> CommandWaitTask
where
    F: Future<Output = T> + 'static,
    T: 'static,
    C: FnOnce(Result<T, CommandWaitFailure>) + 'static,
{
    // Construct the cancellation authority before spawning: abort-before-first-
    // poll must notify the actor too, not only cancellation of a polled wait.
    let completion = CompletionOnDrop {
        complete: Some(complete),
        result_type: PhantomData,
    };
    CommandWaitTask(Some(tokio::task::spawn_local(async move {
        let result = AssertUnwindSafe(wait)
            .catch_unwind()
            .await
            .map_err(|_| CommandWaitFailure::Panicked);
        completion.finish(result);
    })))
}

#[cfg(test)]
mod tests {
    use tokio::sync::oneshot;

    use super::*;

    struct ReleaseOnDrop(Option<oneshot::Sender<()>>);

    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(released) = self.0.take() {
                let _ = released.send(());
            }
        }
    }

    fn identity() -> CommandWaitIdentity {
        CommandWaitIdentity { token: 7, turn: 3 }
    }

    #[tokio::test]
    async fn normal_completion_keeps_identity_and_notifies_once() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let (release, wait) = oneshot::channel();
                let task = spawn_command_wait(identity(), async { wait.await.unwrap() }, &sender);
                assert!(receiver.try_recv().is_err());
                release.send(42).unwrap();
                let completion = receiver.recv().await.unwrap();
                assert_eq!(completion.identity, identity());
                assert_eq!(completion.result, Ok(42));
                drop(task);
                drop(sender);
                assert!(receiver.recv().await.is_none());
            })
            .await;
    }

    #[tokio::test]
    async fn domain_errors_are_business_results_not_task_failures() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let _task = spawn_command_wait(
                    identity(),
                    async { Err::<(), _>("network failure") },
                    &sender,
                );
                assert_eq!(
                    receiver.recv().await.unwrap().result,
                    Ok(Err("network failure"))
                );
            })
            .await;
    }

    #[tokio::test]
    async fn panicked_wait_notifies_actor_and_releases_owned_resource() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let (released, release_rx) = oneshot::channel();
                let resource = ReleaseOnDrop(Some(released));
                let _task = spawn_command_wait(
                    identity(),
                    async move {
                        let _resource = resource;
                        panic!("injected external wait failure");
                    },
                    &sender,
                );
                let completion: CommandWaitCompletion<()> = receiver.recv().await.unwrap();
                assert_eq!(completion.result, Err(CommandWaitFailure::Panicked));
                release_rx.await.unwrap();
                assert!(receiver.try_recv().is_err());
            })
            .await;
    }

    #[tokio::test]
    async fn abort_before_first_poll_still_terminates_the_wait() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let task = spawn_command_wait(identity(), std::future::pending::<()>(), &sender);
                task.abort();
                let completion = receiver.recv().await.unwrap();
                assert_eq!(completion.identity, identity());
                assert_eq!(completion.result, Err(CommandWaitFailure::Cancelled));
                drop(task);
                drop(sender);
                assert!(receiver.recv().await.is_none());
            })
            .await;
    }

    #[tokio::test]
    async fn owner_drop_cancels_a_polled_wait_and_releases_its_resource() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let (started, start_rx) = oneshot::channel();
                let (released, release_rx) = oneshot::channel();
                let resource = ReleaseOnDrop(Some(released));
                let task = spawn_command_wait(
                    identity(),
                    async move {
                        let _resource = resource;
                        let _ = started.send(());
                        std::future::pending::<()>().await;
                    },
                    &sender,
                );
                start_rx.await.unwrap();
                drop(task);
                assert_eq!(
                    receiver.recv().await.unwrap().result,
                    Err(CommandWaitFailure::Cancelled)
                );
                release_rx.await.unwrap();
                assert!(receiver.try_recv().is_err());
            })
            .await;
    }

    #[tokio::test]
    async fn failed_delivery_drops_the_completed_owned_result() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, receiver) = mpsc::unbounded_channel();
                let (released, release_rx) = oneshot::channel();
                let resource = ReleaseOnDrop(Some(released));
                drop(receiver);
                let _task = spawn_command_wait(identity(), async move { resource }, &sender);
                release_rx.await.unwrap();
            })
            .await;
    }

    #[tokio::test]
    async fn shutdown_join_finishes_owned_resource_destruction() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let (started, start_rx) = oneshot::channel();
                let (released, mut release_rx) = oneshot::channel();
                let resource = ReleaseOnDrop(Some(released));
                let task = spawn_command_wait(
                    identity(),
                    async move {
                        let _resource = resource;
                        let _ = started.send(());
                        std::future::pending::<()>().await;
                    },
                    &sender,
                );
                start_rx.await.unwrap();
                task.cancel_and_join().await;
                assert_eq!(release_rx.try_recv(), Ok(()));
                assert_eq!(
                    receiver.recv().await.unwrap().result,
                    Err(CommandWaitFailure::Cancelled)
                );
            })
            .await;
    }

    #[tokio::test]
    async fn cancelled_owner_does_not_retain_the_task_after_actor_exit() {
        let (released, release_rx) = oneshot::channel();
        let resource = ReleaseOnDrop(Some(released));
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let (started, start_rx) = oneshot::channel();
                let owner = tokio::task::spawn_local(async move {
                    let (sender, _completed) = mpsc::unbounded_channel();
                    let _owned_wait = spawn_command_wait(
                        identity(),
                        async move {
                            let _resource = resource;
                            std::future::pending::<()>().await;
                        },
                        &sender,
                    );
                    let _ = started.send(());
                    std::future::pending::<()>().await;
                });
                start_rx.await.unwrap();
                owner.abort();
                assert!(owner.await.unwrap_err().is_cancelled());
                release_rx.await.unwrap();
            })
            .await;
    }
}
