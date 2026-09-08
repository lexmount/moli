use super::*;
use crate::cdp_scheduler::{DevToolsRuntimeCommandProgress, PendingDevToolsRuntimeExecution};

pub(super) struct ClassicPendingRuntime {
    command: DevToolsCommand,
    timeout: Option<Duration>,
    navigation_timeout: Option<Duration>,
    navigation_deadline: Option<tokio::time::Instant>,
    deadline: Option<tokio::time::Instant>,
    expected_page: Option<DevToolsPageResidenceIdentity>,
    page_residence: Option<DevToolsPageResidenceIdentity>,
    terminate_on_timeout: bool,
    phase: RuntimePhase,
    response_tx: oneshot::Sender<ClassicSessionRuntimeCommandExecution>,
    wait: RuntimeWait,
}

enum RuntimePhase {
    Command,
    Terminating(DevToolsError),
}

enum RuntimeWait {
    Dispatch(Box<PendingDevToolsRuntimeExecution>),
    Navigation,
}

impl ClassicPendingRuntime {
    pub(super) async fn start(
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        request: ClassicSessionRuntimeRequest,
    ) -> Option<Self> {
        let ClassicSessionRuntimeRequest::Execute {
            command,
            timeout,
            pending_navigation_timeout,
            terminate_execution_on_timeout,
            expected_page,
            response_tx,
        } = request
        else {
            unreachable!("Runtime dispatch requires an execute request")
        };
        Self {
            command: *command,
            timeout,
            navigation_timeout: pending_navigation_timeout,
            navigation_deadline: None,
            deadline: None,
            expected_page,
            page_residence: None,
            terminate_on_timeout: terminate_execution_on_timeout,
            phase: RuntimePhase::Command,
            response_tx,
            wait: RuntimeWait::Navigation,
        }
        .dispatch(scheduler, receivers)
        .await
    }

    async fn dispatch(
        mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> Option<Self> {
        if matches!(self.phase, RuntimePhase::Command)
            && scheduler.devtools_context_has_pending_document_navigation(self.command.context())
        {
            if self.navigation_deadline.is_none() {
                self.navigation_deadline = self
                    .navigation_timeout
                    .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout));
            }
            self.deadline = self.navigation_deadline;
            self.wait = RuntimeWait::Navigation;
            return Some(self);
        }
        self.page_residence =
            scheduler.page_residence_identity_for_devtools_context(self.command.context());
        if self
            .expected_page
            .as_ref()
            .is_some_and(|expected| self.page_residence.as_ref() != Some(expected))
        {
            let error = match std::mem::replace(&mut self.phase, RuntimePhase::Command) {
                RuntimePhase::Terminating(original) => original,
                RuntimePhase::Command => DevToolsError::new(
                    DevToolsErrorKind::NoSuchNode,
                    "DOM reference belongs to a replaced Page",
                ),
            };
            self.reply(Err(error));
            return None;
        }
        self.deadline = self
            .timeout
            .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout));
        let progress = scheduler
            .start_devtools_runtime_command(receivers, self.command.clone())
            .await;
        self.apply(scheduler, progress)
    }

    fn apply(
        mut self,
        scheduler: &mut CdpScheduler,
        progress: DevToolsRuntimeCommandProgress,
    ) -> Option<Self> {
        match progress {
            DevToolsRuntimeCommandProgress::Pending {
                pending,
                protocol_output,
            } => {
                scheduler.publish_devtools_output(protocol_output);
                self.wait = RuntimeWait::Dispatch(pending);
                Some(self)
            }
            DevToolsRuntimeCommandProgress::Complete(execution) => {
                scheduler.publish_devtools_output(execution.protocol_output);
                match self.phase {
                    RuntimePhase::Terminating(error) => {
                        self.phase = RuntimePhase::Command;
                        if let Err(termination) = execution.result {
                            tracing::warn!(
                                ?termination,
                                "failed to terminate timed-out Classic script"
                            );
                        }
                        self.reply(Err(error));
                        None
                    }
                    RuntimePhase::Command
                        if classic_runtime_result_is_navigation_changing_document(
                            &execution.result,
                        ) && self.navigation_timeout.is_some() =>
                    {
                        if self.navigation_deadline.is_none() {
                            self.navigation_deadline =
                                self.navigation_timeout.and_then(|timeout| {
                                    tokio::time::Instant::now().checked_add(timeout)
                                });
                        }
                        self.deadline = self.navigation_deadline;
                        self.wait = RuntimeWait::Navigation;
                        Some(self)
                    }
                    RuntimePhase::Command => {
                        self.reply(execution.result);
                        None
                    }
                }
            }
        }
    }

    pub(super) fn deadline(&self) -> Option<tokio::time::Instant> {
        match (self.deadline, self.navigation_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub(super) fn command_id(&self) -> Option<u64> {
        match &self.wait {
            RuntimeWait::Dispatch(pending) => Some(pending.command_id()),
            RuntimeWait::Navigation => None,
        }
    }

    pub(super) async fn wait(&mut self) -> moli_protocol::CompletedDevToolsRuntimeCommandDispatch {
        match &mut self.wait {
            RuntimeWait::Dispatch(pending) => pending.wait().await,
            RuntimeWait::Navigation => std::future::pending().await,
        }
    }

    pub(super) async fn complete(
        mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        completed: moli_protocol::CompletedDevToolsRuntimeCommandDispatch,
    ) -> Option<Self> {
        let RuntimeWait::Dispatch(pending) =
            std::mem::replace(&mut self.wait, RuntimeWait::Navigation)
        else {
            unreachable!()
        };
        let progress = scheduler
            .complete_devtools_runtime_command(receivers, pending, completed)
            .await;
        self.apply(scheduler, progress)
    }

    pub(super) async fn renderer_response(
        mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        response: moli_protocol::conn::RuntimeInspectorResponseReady,
    ) -> Option<Self> {
        let RuntimeWait::Dispatch(pending) =
            std::mem::replace(&mut self.wait, RuntimeWait::Navigation)
        else {
            unreachable!()
        };
        let progress = scheduler
            .advance_devtools_runtime_command_after_renderer_response(receivers, pending, response)
            .await;
        self.apply(scheduler, progress)
    }

    pub(super) async fn poll(
        self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> Option<Self> {
        if matches!(self.wait, RuntimeWait::Navigation)
            && !scheduler.devtools_context_has_pending_document_navigation(self.command.context())
        {
            self.dispatch(scheduler, receivers).await
        } else {
            Some(self)
        }
    }

    pub(super) async fn expire(
        mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> Option<Self> {
        let navigation_wait = matches!(self.wait, RuntimeWait::Navigation);
        if let RuntimeWait::Dispatch(pending) =
            std::mem::replace(&mut self.wait, RuntimeWait::Navigation)
        {
            scheduler.cancel_devtools_runtime_command(*pending);
        }
        let error = if navigation_wait {
            classic_pending_navigation_timeout_error()
        } else {
            DevToolsError::new(DevToolsErrorKind::Timeout, "script timed out")
        };
        if self.terminate_on_timeout
            && matches!(self.phase, RuntimePhase::Command)
            && !navigation_wait
        {
            self.phase = RuntimePhase::Terminating(error);
            self.command = DevToolsCommand::TerminateExecution(DevToolsTerminateExecutionCommand {
                context: self.command.context().clone(),
            });
            self.expected_page = self.page_residence.clone();
            self.timeout = Some(CLASSIC_SCRIPT_TERMINATION_TIMEOUT);
            self.navigation_deadline = None;
            self.navigation_timeout = None;
            self.dispatch(scheduler, receivers).await
        } else {
            let error = match std::mem::replace(&mut self.phase, RuntimePhase::Command) {
                RuntimePhase::Terminating(original) => original,
                RuntimePhase::Command => error,
            };
            self.reply(Err(error));
            None
        }
    }

    pub(super) fn cancel(mut self, scheduler: &mut CdpScheduler) {
        if let RuntimeWait::Dispatch(pending) =
            std::mem::replace(&mut self.wait, RuntimeWait::Navigation)
        {
            scheduler.cancel_devtools_runtime_command(*pending);
        }
        self.reply(Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchSession,
            "Classic session ended during script execution",
        )));
    }

    fn reply(self, result: Result<DevToolsCommandResult, DevToolsError>) {
        let _ = self
            .response_tx
            .send(ClassicSessionRuntimeCommandExecution {
                result,
                page_residence: self.page_residence,
            });
    }
}
