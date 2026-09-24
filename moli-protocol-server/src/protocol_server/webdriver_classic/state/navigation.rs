use super::*;
use crate::cdp_scheduler::{
    CompletedDevToolsNavigationExecution, DevToolsNavigationCommandProgress,
    DevToolsNavigationCommandWait, PendingDevToolsNavigationLifecycle, ProtocolOutputSequence,
};

struct NavigationReply {
    response_tx: oneshot::Sender<ClassicSessionRuntimeCommandExecution>,
    page_residence: Option<DevToolsPageResidenceIdentity>,
    deadline: Option<tokio::time::Instant>,
}

enum NavigationWait {
    Network(DevToolsNavigationCommandWait),
    Lifecycle(Box<PendingDevToolsNavigationLifecycle>),
}

pub(super) struct ClassicPendingNavigation {
    reply: NavigationReply,
    wait: NavigationWait,
}

impl ClassicPendingNavigation {
    pub(super) fn deadline(&self) -> Option<tokio::time::Instant> {
        self.reply.deadline
    }

    pub(super) fn cancel(self, scheduler: &mut CdpScheduler, timed_out: bool) {
        match self.wait {
            NavigationWait::Network(wait) if timed_out => {
                scheduler.retain_detached_navigation(wait)
            }
            NavigationWait::Network(_) => {}
            NavigationWait::Lifecycle(wait) => {
                scheduler.cancel_devtools_navigation_lifecycle(*wait)
            }
        }
        let error = if timed_out {
            DevToolsError::new(DevToolsErrorKind::Timeout, "navigation wait timed out")
        } else {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchSession,
                "Classic session runtime stopped during navigation",
            )
        };
        let _ = self
            .reply
            .response_tx
            .send(ClassicSessionRuntimeCommandExecution {
                result: Err(error),
                page_residence: self.reply.page_residence,
            });
    }
}

pub(super) async fn start_navigation(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    command: DevToolsCommand,
    timeout: Option<Duration>,
    response_tx: oneshot::Sender<ClassicSessionRuntimeCommandExecution>,
    pending: &mut Option<ClassicPendingCommand>,
) -> ClassicSessionRuntimeRequestOutcome {
    let reply = NavigationReply {
        response_tx,
        page_residence: scheduler.page_residence_identity_for_devtools_context(command.context()),
        deadline: timeout.and_then(|timeout| tokio::time::Instant::now().checked_add(timeout)),
    };
    let progress = scheduler
        .start_devtools_navigation_command(receivers, command, None)
        .await;
    apply_progress(scheduler, reply, progress, pending)
}

pub(super) async fn recv_navigation_completion(
    pending: &mut Option<ClassicPendingCommand>,
) -> Result<CompletedDevToolsNavigationExecution, tokio::task::JoinError> {
    match pending.as_mut() {
        Some(ClassicPendingCommand::Navigation(ClassicPendingNavigation {
            wait: NavigationWait::Network(wait),
            ..
        })) => wait.await,
        _ => std::future::pending().await,
    }
}

pub(super) async fn wait_for_navigation_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

pub(super) async fn complete_navigation(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    completed: Result<CompletedDevToolsNavigationExecution, tokio::task::JoinError>,
    pending: &mut Option<ClassicPendingCommand>,
) -> ClassicSessionRuntimeRequestOutcome {
    let Some(ClassicPendingCommand::Navigation(command)) = pending.take() else {
        unreachable!("completed Classic navigation must retain its reply")
    };
    let progress = match completed {
        Ok(completed) => {
            scheduler
                .complete_devtools_navigation_command(receivers, completed)
                .await
        }
        Err(error) => {
            DevToolsNavigationCommandProgress::Complete(Box::new(DevToolsCommandExecution {
                result: Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    format!("Navigation waiter stopped: {error}"),
                )),
                protocol_output: ProtocolOutputSequence::empty(),
            }))
        }
    };
    apply_progress(scheduler, command.reply, progress, pending)
}

pub(super) fn poll_navigation_lifecycle(
    scheduler: &mut CdpScheduler,
    pending: &mut Option<ClassicPendingCommand>,
) -> ClassicSessionRuntimeRequestOutcome {
    if !pending.as_ref().is_some_and(|command| {
        matches!(
            command,
            ClassicPendingCommand::Navigation(ClassicPendingNavigation {
                wait: NavigationWait::Lifecycle(_),
                ..
            })
        )
    }) {
        return ClassicSessionRuntimeRequestOutcome::Continue;
    }
    let Some(ClassicPendingCommand::Navigation(command)) = pending.take() else {
        unreachable!()
    };
    let NavigationWait::Lifecycle(wait) = command.wait else {
        unreachable!()
    };
    let progress =
        scheduler.advance_devtools_navigation_lifecycle(wait, ProtocolOutputSequence::empty());
    apply_progress(scheduler, command.reply, progress, pending)
}

fn apply_progress(
    scheduler: &mut CdpScheduler,
    reply: NavigationReply,
    progress: DevToolsNavigationCommandProgress,
    pending: &mut Option<ClassicPendingCommand>,
) -> ClassicSessionRuntimeRequestOutcome {
    let (output, completed_reply) = match progress {
        DevToolsNavigationCommandProgress::Complete(execution) => {
            (execution.protocol_output, Some((reply, execution.result)))
        }
        DevToolsNavigationCommandProgress::Pending {
            pending: network,
            protocol_output,
        } => {
            *pending = Some(ClassicPendingCommand::Navigation(
                ClassicPendingNavigation {
                    reply,
                    wait: NavigationWait::Network(DevToolsNavigationCommandWait::new(*network)),
                },
            ));
            (protocol_output, None)
        }
        DevToolsNavigationCommandProgress::PendingLifecycle {
            pending: lifecycle,
            protocol_output,
        } => {
            *pending = Some(ClassicPendingCommand::Navigation(
                ClassicPendingNavigation {
                    reply,
                    wait: NavigationWait::Lifecycle(lifecycle),
                },
            ));
            (protocol_output, None)
        }
    };
    scheduler.publish_devtools_output(output);
    if let Some((reply, result)) = completed_reply {
        let _ = reply
            .response_tx
            .send(ClassicSessionRuntimeCommandExecution {
                result,
                page_residence: reply.page_residence,
            });
    }
    ClassicSessionRuntimeRequestOutcome::Continue
}

pub(super) async fn next_classic_request(
    rx: &mut mpsc::UnboundedReceiver<ClassicSessionRuntimeRequest>,
    queued: &mut std::collections::VecDeque<ClassicSessionRuntimeRequest>,
    navigation_pending: bool,
) -> Option<ClassicSessionRuntimeRequest> {
    if !navigation_pending && let Some(request) = queued.pop_front() {
        return Some(request);
    }
    rx.recv().await
}

pub(super) fn defer_classic_request(
    request: &ClassicSessionRuntimeRequest,
    navigation_pending: bool,
) -> bool {
    navigation_pending
        && !matches!(
            request,
            ClassicSessionRuntimeRequest::AttachBidiSocket { .. }
                | ClassicSessionRuntimeRequest::Shutdown { .. }
        )
}
