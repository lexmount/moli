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
            && scheduler.devtools_runtime_command_waits_for_document(&self.command)
        {
            return Some(self.await_document());
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
            DevToolsRuntimeCommandProgress::AwaitingDocument(command) => {
                self.command = *command;
                Some(self.await_document())
            }
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
                        Some(self.await_document())
                    }
                    RuntimePhase::Command => {
                        self.reply(execution.result);
                        None
                    }
                }
            }
        }
    }

    fn await_document(mut self) -> Self {
        if self.navigation_deadline.is_none() {
            self.navigation_deadline = self
                .navigation_timeout
                .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout));
        }
        self.deadline = self.navigation_deadline;
        self.wait = RuntimeWait::Navigation;
        self
    }

    pub(super) fn deadline(&self) -> Option<tokio::time::Instant> {
        // Dispatch and navigation retry already install their own deadline.
        // Retained navigation bookkeeping must not time out a later script.
        self.deadline
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
            && !scheduler.devtools_runtime_command_waits_for_document(&self.command)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_dispatch_does_not_inherit_completed_navigation_deadline() {
        let service = moli_core::browser::BrowserService::start().unwrap();
        let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            moli_protocol::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let initialized = scheduler.execute_internal_protocol_message(&mut receivers,
            serde_json::json!({"id": 1, "method": "Target.createTarget", "params": {"url": "about:blank"}}),
        ).await.unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1)).into_messages();
        let target = initialized
            .iter()
            .find(|message| message["id"] == 1)
            .and_then(|message| message["result"]["targetId"].as_str())
            .unwrap_or_else(|| panic!("created test Page: {initialized:?}"));
        for script_timeout in [None, Some(Duration::from_secs(30))] {
            let context = ClassicDevToolsCommandContext::with_target_id("deadline-test", target);
            let command = moli_protocol_webdriver_classic::execute_sync_command(
                &context,
                &serde_json::json!({"script": "return new Promise(() => {})", "args": []}),
            )
            .unwrap();
            let (response_tx, mut response) = oneshot::channel();
            // Model the owner boundary after the exact navigation has completed:
            // its old deadline is retained for navigation retries, not execution.
            let navigation_deadline = tokio::time::Instant::now();
            let pending = ClassicPendingRuntime {
                command,
                timeout: script_timeout,
                navigation_timeout: Some(Duration::from_millis(100)),
                navigation_deadline: Some(navigation_deadline),
                deadline: Some(navigation_deadline),
                expected_page: None,
                page_residence: None,
                terminate_on_timeout: false,
                phase: RuntimePhase::Command,
                response_tx,
                wait: RuntimeWait::Navigation,
            }
            .dispatch(&mut scheduler, &mut receivers)
            .await
            .unwrap_or_else(|| {
                panic!(
                    "script must remain pending: {:?}",
                    response.try_recv().map(|execution| execution.result)
                )
            });
            assert!(matches!(pending.wait, RuntimeWait::Dispatch(_)));
            assert_eq!(
                pending.deadline(),
                pending.deadline,
                "execution must use its own deadline, not the completed navigation deadline"
            );
            assert_eq!(pending.deadline().is_some(), script_timeout.is_some());
            if let Some(deadline) = pending.deadline() {
                assert!(deadline > navigation_deadline);
            }
            pending.cancel(&mut scheduler);
        }
        service.shutdown();
    }
}
