use super::*;
use crate::domains::command_output::devtools_error_from_cdp_error_value;

pub(crate) type AutomationCommandResult =
    std::rc::Rc<std::cell::RefCell<Option<Result<AutomationResult, DevToolsError>>>>;

pub struct PendingAutomationCommandDispatch {
    internal_command_id: u64,
    context: crate::conn::CommandDispatchContext,
    result: AutomationCommandResult,
    protocol_success: Option<super::workflow::AutomationCommandSuccess>,
    pending: Option<crate::conn::PendingCdpCommandDispatch>,
    interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
}

pub struct CompletedAutomationCommandDispatch {
    owner: PendingAutomationCommandDispatch,
    completed: crate::conn::CompletedCdpCommandDispatch,
    interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
}

pub enum AutomationCommandTaskStep {
    Pending(Box<PendingAutomationCommandDispatch>),
    Complete(Box<AutomationCommandDispatchOutcome>),
}

impl PendingAutomationCommandDispatch {
    pub fn internal_command_id(&self) -> u64 {
        self.internal_command_id
    }
    pub fn append_interleaved_protocol_events(&mut self, events: Vec<BackgroundProtocolEvent>) {
        self.interleaved_protocol_events.extend(events);
    }
    pub fn take_scheduler_events(&mut self) -> Vec<CdpSchedulerEvent> {
        self.pending
            .as_mut()
            .expect("the owner retains its wait before dispatch")
            .take_scheduler_events()
    }
    pub fn wait_lifetime(&self) -> Option<crate::CommandWaitLifetime> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.wait_lifetime())
    }
    pub fn take_wait(&mut self) -> crate::conn::PendingCdpCommandDispatch {
        self.pending
            .take()
            .expect("one wait task per owning command")
    }
    pub fn with_completion(
        self,
        completed: crate::conn::CompletedCdpCommandDispatch,
    ) -> CompletedAutomationCommandDispatch {
        CompletedAutomationCommandDispatch {
            owner: self,
            completed,
            interleaved_protocol_events: Vec::new(),
        }
    }
    pub async fn wait(mut self) -> CompletedAutomationCommandDispatch {
        let completed = self.take_wait().wait().await;
        self.with_completion(completed)
    }
}

impl CompletedAutomationCommandDispatch {
    pub fn append_interleaved_protocol_events(&mut self, events: Vec<BackgroundProtocolEvent>) {
        self.interleaved_protocol_events.extend(events);
    }
}

impl CdpConnection {
    pub fn start_automation_command_dispatch(
        &mut self,
        command: AutomationCommand,
    ) -> AutomationCommandTaskStep {
        self.start_automation_command_dispatch_with_background_command_id(command, None)
    }

    pub fn start_automation_command_dispatch_with_background_command_id(
        &mut self,
        command: AutomationCommand,
        background_id: Option<u64>,
    ) -> AutomationCommandTaskStep {
        let command = match self.try_execute_immediate_automation_command(command) {
            Ok(outcome) => {
                let (result, scheduler, events, predecessor) = outcome.into_complete_parts();
                return AutomationCommandTaskStep::Complete(Box::new(
                    AutomationCommandDispatchOutcome {
                        result,
                        output: crate::conn::CdpTurnOutcome::new_with_protocol_events(
                            events, scheduler,
                        )
                        .with_renderer_output_predecessor(predecessor),
                    },
                ));
            }
            Err(command) => *command,
        };
        let access = self.protocol_state_access();
        let id = self.next_internal_runtime_command_id();
        let result: AutomationCommandResult = Default::default();
        let mut protocol_success = super::workflow::protocol_command_success(&command);
        let workflow = prepare_automation_workflow(
            self,
            command,
            access,
            id,
            result.clone(),
            &mut protocol_success,
            background_id,
        );
        let mut context = crate::conn::CommandDispatchContext::default();
        let step = self.start_command_workflow(workflow, Some(id), None, &mut context);
        let crate::conn::CdpCommandTaskStep::Pending(pending) = step else {
            unreachable!("an owned automation workflow starts as a task");
        };
        AutomationCommandTaskStep::Pending(Box::new(PendingAutomationCommandDispatch {
            internal_command_id: id,
            context,
            result,
            protocol_success,
            pending: Some(*pending),
            interleaved_protocol_events: Vec::new(),
        }))
    }

    pub fn complete_automation_command_dispatch(
        &mut self,
        completed: CompletedAutomationCommandDispatch,
    ) -> AutomationCommandTaskStep {
        let CompletedAutomationCommandDispatch {
            mut owner,
            completed,
            interleaved_protocol_events,
        } = completed;
        owner
            .interleaved_protocol_events
            .extend(interleaved_protocol_events);
        let step = self.complete_command_dispatch_with_context(completed, &mut owner.context);
        self.finish_automation_command_step(owner, step)
    }

    pub fn fail_automation_command_wait(
        &mut self,
        mut owner: PendingAutomationCommandDispatch,
        failure: crate::OwnedProtocolWaitFailure,
    ) -> AutomationCommandTaskStep {
        let step = self.complete_failed_command_wait_with_context(
            Some(owner.internal_command_id),
            None,
            failure.message(),
            &mut owner.context,
        );
        self.finish_automation_command_step(owner, step)
    }

    fn finish_automation_command_step(
        &mut self,
        mut owner: PendingAutomationCommandDispatch,
        step: crate::conn::CdpCommandTaskStep,
    ) -> AutomationCommandTaskStep {
        match step {
            crate::conn::CdpCommandTaskStep::Pending(pending) => {
                owner.pending = Some(*pending);
                AutomationCommandTaskStep::Pending(Box::new(owner))
            }
            crate::conn::CdpCommandTaskStep::Complete(outcome) => {
                let (mut events, after, boundary, post, scheduler, predecessor) =
                    outcome.into_renderer_owner_turn_parts();
                events.extend(owner.interleaved_protocol_events);
                let mut terminal = None;
                let mut take_terminal = |events: Vec<BackgroundProtocolEvent>| {
                    let mut retained = Vec::new();
                    for event in events {
                        if event.protocol_message_id() == Some(owner.internal_command_id)
                            && (event.command_response_payload_ref().is_some()
                                || event.protocol_message().is_some_and(|message| {
                                    message.get("result").is_some()
                                        || message.get("error").is_some()
                                }))
                        {
                            let (message, _) = event.into_parts();
                            assert!(
                                terminal.is_none(),
                                "one automation protocol terminal per owner"
                            );
                            terminal = Some(match message.field::<serde_json::Value>("error") {
                                Ok(Some(error)) => Err(devtools_error_from_cdp_error_value(&error)),
                                Ok(None) => Ok(()),
                                Err(error) => {
                                    Err(DevToolsError::new(DevToolsErrorKind::Internal, error))
                                }
                            });
                        } else {
                            retained.push(event);
                        }
                    }
                    retained
                };
                let prefix = take_terminal(events);
                let after = take_terminal(after);
                let post = take_terminal(post);
                let result = owner.result.borrow_mut().take().unwrap_or_else(|| {
                    match (terminal, owner.protocol_success) {
                        (Some(Ok(())), Some(success)) => success.finish(),
                        (Some(Err(error)), _) => Err(error),
                        _ => Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            crate::OwnedProtocolWaitFailure::Cancelled,
                        )),
                    }
                });
                let output =
                    crate::conn::CdpTurnOutcome::new_with_protocol_and_post_response_events(
                        prefix, post, scheduler,
                    )
                    .with_renderer_output_boundary(boundary, after)
                    .with_renderer_output_predecessor(predecessor);
                AutomationCommandTaskStep::Complete(Box::new(AutomationCommandDispatchOutcome {
                    result,
                    output,
                }))
            }
        }
    }
}

fn prepare_automation_workflow(
    conn: &mut CdpConnection,
    command: AutomationCommand,
    access: ProtocolStateHandle,
    id: u64,
    result: AutomationCommandResult,
    success: &mut Option<super::workflow::AutomationCommandSuccess>,
    background_id: Option<u64>,
) -> PreparedCommandWorkflow {
    match command {
        command @ (AutomationCommand::Navigate(_)
        | AutomationCommand::Reload(_)
        | AutomationCommand::TraverseHistory(_)) => {
            match PreparedProtocolCommand::for_automation_navigation(
                conn,
                command,
                id,
                background_id,
                Default::default(),
            ) {
                Ok((prepared, projection)) => {
                    *success = Some(super::workflow::AutomationCommandSuccess::Navigation(
                        projection,
                    ));
                    prepared.into_workflow(access)
                }
                Err(error) => PreparedCommandWorkflow::new(move |context| async move {
                    assert!(result.borrow_mut().replace(Err(error)).is_none());
                    context.complete(crate::domains::command_output::CommandOutputPlan::default())
                })
                .cancel_on_waiter_drop(),
            }
        }
        command @ (AutomationCommand::GetRealms(_)
        | AutomationCommand::EvaluateScript(_)
        | AutomationCommand::CallFunction(_)
        | AutomationCommand::TerminateExecution(_)
        | AutomationCommand::LocateNodes(_)
        | AutomationCommand::ReleaseObjects(_)) => {
            crate::domains::runtime::prepare_automation_runtime_workflow(
                conn, command, access, id, result,
            )
        }
        command @ (AutomationCommand::CloseTarget(_)
        | AutomationCommand::RemoveBrowserContext(_)
        | AutomationCommand::AddNetworkIntercept(_)
        | AutomationCommand::RemoveNetworkIntercept(_)
        | AutomationCommand::ContinueInterceptedRequest(_)
        | AutomationCommand::ContinueInterceptedResponse(_)
        | AutomationCommand::ContinueWithAuth(_)
        | AutomationCommand::FailInterceptedRequest(_)
        | AutomationCommand::FulfillInterceptedRequest(_)) => {
            match PreparedProtocolCommand::for_automation_command(
                conn,
                command,
                id,
                Default::default(),
            ) {
                Ok(prepared) => prepared.into_workflow(access),
                Err(error) => PreparedCommandWorkflow::new(move |context| async move {
                    assert!(result.borrow_mut().replace(Err(error)).is_none());
                    context.complete(crate::domains::command_output::CommandOutputPlan::default())
                })
                .cancel_on_waiter_drop(),
            }
        }
        command => super::workflow::prepare_workflow(conn, command, access, result),
    }
}
