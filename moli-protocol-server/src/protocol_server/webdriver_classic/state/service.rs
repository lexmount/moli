use futures_util::{StreamExt, stream::FuturesUnordered};

use super::*;
use crate::cdp_scheduler::CompletedDevToolsNavigationExecution;
use crate::protocol_server::webdriver_bidi::BidiServiceFrontends;

pub(crate) struct ClassicSessionAttach {
    pub(super) session_id: String,
    pub(super) rx: mpsc::UnboundedReceiver<ClassicSessionRuntimeRequest>,
    pub(super) initial_cookie_snapshot: Vec<StoredCookie>,
    pub(super) response_tx: oneshot::Sender<Result<(), DevToolsError>>,
}

struct ClassicServiceSession {
    rx: mpsc::UnboundedReceiver<ClassicSessionRuntimeRequest>,
    initial_cookie_snapshot: Vec<StoredCookie>,
    pending_command: Option<ClassicPendingCommand>,
    queued_requests: std::collections::VecDeque<ClassicSessionRuntimeRequest>,
}

#[derive(Default)]
pub(crate) struct ClassicServiceSessions {
    sessions: BTreeMap<String, ClassicServiceSession>,
}

pub(crate) struct ClassicServiceInput(ClassicServiceEvent);

enum ClassicServiceEvent {
    Runtime(Box<moli_protocol::CompletedDevToolsRuntimeCommandDispatch>),
    RuntimeResponse(moli_protocol::conn::RuntimeInspectorResponseReady),
    Request(Option<ClassicSessionRuntimeRequest>),
    Navigation(Box<Result<CompletedDevToolsNavigationExecution, tokio::task::JoinError>>),
    Deadline,
}

pub(super) enum ClassicPendingCommand {
    Navigation(ClassicPendingNavigation),
    Runtime(Box<command::ClassicPendingRuntime>),
    Lifecycle {
        wait: crate::cdp_scheduler::DevToolsContextDocumentWait,
        deadline: Option<tokio::time::Instant>,
        response_tx: oneshot::Sender<Result<(), DevToolsError>>,
    },
}

impl ClassicPendingCommand {
    fn deadline(&self) -> Option<tokio::time::Instant> {
        match self {
            Self::Navigation(wait) => wait.deadline(),
            Self::Runtime(wait) => wait.deadline(),
            Self::Lifecycle { deadline, .. } => *deadline,
        }
    }

    fn cancel(self, scheduler: &mut CdpScheduler, timed_out: bool) {
        match self {
            Self::Navigation(wait) => wait.cancel(scheduler, timed_out),
            Self::Runtime(wait) => wait.cancel(scheduler),
            Self::Lifecycle {
                mut wait,
                response_tx,
                ..
            } => {
                scheduler.cancel_devtools_context_document_lifecycle(&mut wait);
                let error = if timed_out {
                    classic_pending_navigation_timeout_error()
                } else {
                    DevToolsError::new(
                        DevToolsErrorKind::NoSuchSession,
                        "Classic session ended during document wait",
                    )
                };
                let _ = response_tx.send(Err(error));
            }
        }
    }
}

async fn recv_pending_command(pending: &mut Option<ClassicPendingCommand>) -> ClassicServiceEvent {
    match pending {
        Some(ClassicPendingCommand::Runtime(wait)) => {
            ClassicServiceEvent::Runtime(Box::new(wait.wait().await))
        }
        _ => ClassicServiceEvent::Navigation(Box::new(
            navigation::recv_navigation_completion(pending).await,
        )),
    }
}

impl ClassicServiceSessions {
    pub(crate) fn attach(&mut self, scheduler: &mut CdpScheduler, attach: ClassicSessionAttach) {
        let result = scheduler.attach_webdriver_session(&attach.session_id);
        if result.is_ok() {
            self.sessions.insert(
                attach.session_id,
                ClassicServiceSession {
                    rx: attach.rx,
                    initial_cookie_snapshot: attach.initial_cookie_snapshot,
                    pending_command: None,
                    queued_requests: Default::default(),
                },
            );
        }
        let _ = attach.response_tx.send(result);
    }

    pub(crate) async fn recv(&mut self) -> (String, ClassicServiceInput) {
        let mut inputs = self.sessions.iter_mut().map(|(id, session)| async move {
            let deadline = session.pending_command.as_ref().and_then(ClassicPendingCommand::deadline);
            let pending = session.pending_command.is_some();
            let input = tokio::select! {
                biased;
                completed = recv_pending_command(&mut session.pending_command) => completed,
                _ = navigation::wait_for_navigation_deadline(deadline) => ClassicServiceEvent::Deadline,
                request = navigation::next_classic_request(&mut session.rx, &mut session.queued_requests, pending) => ClassicServiceEvent::Request(request),
            };
            (id.clone(), ClassicServiceInput(input))
        }).collect::<FuturesUnordered<_>>();
        match inputs.next().await {
            Some(input) => input,
            None => std::future::pending().await,
        }
    }

    pub(crate) async fn poll(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) {
        if self
            .sessions
            .values()
            .any(|session| session.pending_command.is_some())
        {
            let mut output = scheduler.drain_browser_events().await;
            output.append(
                scheduler
                    .complete_ready_protocol_residences_for_external_load_wait()
                    .await,
            );
            scheduler.publish_devtools_output(output);
        }
        for session in self.sessions.values_mut() {
            match session.pending_command.take() {
                Some(ClassicPendingCommand::Runtime(wait)) => {
                    session.pending_command = wait
                        .poll(scheduler, receivers)
                        .await
                        .map(|wait| ClassicPendingCommand::Runtime(Box::new(wait)));
                }
                Some(ClassicPendingCommand::Lifecycle {
                    mut wait,
                    deadline,
                    response_tx,
                }) => match scheduler.poll_devtools_context_document_lifecycle(&mut wait) {
                    Some(result) => {
                        let _ = response_tx.send(result);
                    }
                    None => {
                        session.pending_command = Some(ClassicPendingCommand::Lifecycle {
                            wait,
                            deadline,
                            response_tx,
                        })
                    }
                },
                pending => {
                    session.pending_command = pending;
                    navigation::poll_navigation_lifecycle(scheduler, &mut session.pending_command);
                }
            }
        }
    }

    pub(crate) fn runtime_response_owner(
        &self,
        response: &moli_protocol::conn::RuntimeInspectorResponseReady,
    ) -> Option<String> {
        self.sessions
            .iter()
            .find_map(|(id, session)| match &session.pending_command {
                Some(ClassicPendingCommand::Runtime(wait))
                    if wait.command_id() == Some(response.command_id()) =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
    }

    pub(crate) async fn handle_runtime_response(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        bidi: &mut BidiServiceFrontends,
        id: String,
        response: moli_protocol::conn::RuntimeInspectorResponseReady,
    ) {
        self.handle_input(
            scheduler,
            receivers,
            bidi,
            id,
            ClassicServiceInput(ClassicServiceEvent::RuntimeResponse(response)),
        )
        .await;
    }

    pub(crate) async fn handle_input(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        bidi: &mut BidiServiceFrontends,
        id: String,
        input: ClassicServiceInput,
    ) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let outcome = match input.0 {
            ClassicServiceEvent::Runtime(completed) => {
                let Some(ClassicPendingCommand::Runtime(wait)) = session.pending_command.take()
                else {
                    unreachable!()
                };
                session.pending_command = wait
                    .complete(scheduler, receivers, *completed)
                    .await
                    .map(|wait| ClassicPendingCommand::Runtime(Box::new(wait)));
                ClassicSessionRuntimeRequestOutcome::Continue
            }
            ClassicServiceEvent::RuntimeResponse(response) => {
                let Some(ClassicPendingCommand::Runtime(wait)) = session.pending_command.take()
                else {
                    unreachable!()
                };
                session.pending_command = wait
                    .renderer_response(scheduler, receivers, response)
                    .await
                    .map(|wait| ClassicPendingCommand::Runtime(Box::new(wait)));
                ClassicSessionRuntimeRequestOutcome::Continue
            }
            ClassicServiceEvent::Request(None) => {
                if let Some(session) = self.sessions.remove(&id)
                    && let Some(pending) = session.pending_command
                {
                    pending.cancel(scheduler, false);
                }
                bidi.detach_session(scheduler, receivers, &id).await;
                if let Err(error) = scheduler.close_webdriver_session(&id) {
                    tracing::warn!(%error, "failed to retire abandoned Classic session contexts");
                }
                let output = scheduler.drain_browser_events().await;
                scheduler.publish_devtools_output(output);
                return;
            }
            ClassicServiceEvent::Request(Some(request)) => {
                if navigation::defer_classic_request(&request, session.pending_command.is_some()) {
                    session.queued_requests.push_back(request);
                    return;
                }
                handle_classic_session_runtime_request(
                    scheduler,
                    receivers,
                    &id,
                    request,
                    &mut session.pending_command,
                )
                .await
            }
            ClassicServiceEvent::Navigation(completed) => {
                navigation::complete_navigation(
                    scheduler,
                    receivers,
                    *completed,
                    &mut session.pending_command,
                )
                .await
            }
            ClassicServiceEvent::Deadline => {
                match session.pending_command.take() {
                    Some(ClassicPendingCommand::Runtime(wait)) => {
                        session.pending_command = wait
                            .expire(scheduler, receivers)
                            .await
                            .map(|wait| ClassicPendingCommand::Runtime(Box::new(wait)))
                    }
                    Some(wait) => wait.cancel(scheduler, true),
                    None => {}
                }
                ClassicSessionRuntimeRequestOutcome::Continue
            }
        };
        match outcome {
            ClassicSessionRuntimeRequestOutcome::Continue => {}
            ClassicSessionRuntimeRequestOutcome::AttachedBidi(attached) => {
                bidi.attach_existing_actor(scheduler, attached.actor, attached.session_registry);
            }
            ClassicSessionRuntimeRequestOutcome::Shutdown(response_tx) => {
                let session = self.sessions.remove(&id).unwrap();
                if let Some(pending) = session.pending_command {
                    pending.cancel(scheduler, false);
                }
                bidi.detach_session(scheduler, receivers, &id).await;
                let cookie_commit = CookieProfileCommit::from_optional_profile_backed_snapshot(
                    session.initial_cookie_snapshot,
                    scheduler.snapshot_profile_backed_cookies(),
                );
                if let Err(error) = scheduler.close_webdriver_session(&id) {
                    tracing::warn!(%error, "failed to retire Classic session contexts");
                }
                let output = scheduler.drain_browser_events().await;
                scheduler.publish_devtools_output(output);
                let _ = response_tx.send(cookie_commit);
            }
        }
    }

    pub(crate) fn shutdown(&mut self, scheduler: &mut CdpScheduler) {
        for (_, session) in std::mem::take(&mut self.sessions) {
            if let Some(pending) = session.pending_command {
                pending.cancel(scheduler, false);
            }
        }
    }
}
