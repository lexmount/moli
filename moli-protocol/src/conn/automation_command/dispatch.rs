use super::*;
use crate::domains::command_output::devtools_error_from_cdp_error_value;

pub(crate) type AutomationCommandResult =
    std::rc::Rc<std::cell::RefCell<Option<Result<AutomationResult, DevToolsError>>>>;

pub struct PendingAutomationCommandDispatch {
    internal_command_id: u64,
    context: crate::conn::CommandDispatchContext,
    result: AutomationCommandResult,
    pending: Option<crate::conn::PendingCdpCommandDispatch>,
    interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
}

pub struct CompletedAutomationCommandDispatch {
    owner: PendingAutomationCommandDispatch,
    completed: crate::conn::CompletedCdpCommandDispatch,
    interleaved_protocol_events: Vec<BackgroundProtocolEvent>,
}

pub struct OwnedAutomationCommandOutcome {
    result: Result<AutomationResult, DevToolsError>,
    output: crate::conn::CdpRendererOwnerTurnOutcome,
}

pub enum AutomationCommandTaskStep {
    Pending(Box<PendingAutomationCommandDispatch>),
    Complete(Box<OwnedAutomationCommandOutcome>),
}

impl OwnedAutomationCommandOutcome {
    pub fn into_result_and_owner_turn(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        crate::conn::CdpRendererOwnerTurnOutcome,
    ) {
        (self.result, self.output)
    }

    pub fn into_complete_parts(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let (mut events, after, boundary, post, scheduler, predecessor) =
            self.output.into_renderer_owner_turn_parts();
        assert!(
            boundary.is_none(),
            "an automation consumer must project its independent renderer boundary"
        );
        assert!(after.is_empty());
        events.extend(post);
        (self.result, scheduler, events, predecessor)
    }

    pub fn into_parts(
        self,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        Vec<CdpSchedulerEvent>,
    ) {
        let (result, scheduler, events, predecessor) = self.into_complete_parts();
        assert!(
            events.is_empty(),
            "automation output events require complete consumption"
        );
        assert!(
            predecessor.is_none(),
            "automation output retains its exact renderer cursor"
        );
        (result, scheduler)
    }
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
        let Some(access) = self.protocol_state_access() else {
            let output = crate::conn::CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                self.take_scheduler_events(),
            )
            .with_renderer_output_predecessor(None);
            return AutomationCommandTaskStep::Complete(Box::new(OwnedAutomationCommandOutcome {
                result: Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    AutomationWorkflowOwnerUnavailable,
                )),
                output,
            }));
        };
        let id = self.next_internal_runtime_command_id();
        let result: AutomationCommandResult = Default::default();
        let workflow = prepare_automation_workflow(self, command, access, id, result.clone());
        let mut context = crate::conn::CommandDispatchContext::default();
        let step = self.start_command_workflow(workflow, Some(id), None, &mut context);
        let crate::conn::CdpCommandTaskStep::Pending(pending) = step else {
            unreachable!("an owned automation workflow starts as a task");
        };
        AutomationCommandTaskStep::Pending(Box::new(PendingAutomationCommandDispatch {
            internal_command_id: id,
            context,
            result,
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
        let step = self
            .try_complete_ready_command_dispatch_with_context(completed, &mut owner.context)
            .unwrap_or_else(|completed| {
                panic!(
                    "owned automation {} completion must be finite",
                    completed.kind_name()
                )
            });
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
                let mut prefix = Vec::new();
                let mut failure = None;
                for event in events {
                    if event.protocol_message_id() == Some(owner.internal_command_id) {
                        let (message, _) = event.into_parts();
                        failure = message
                            .get("error")
                            .map(devtools_error_from_cdp_error_value);
                    } else {
                        prefix.push(event);
                    }
                }
                let result = owner.result.borrow_mut().take().unwrap_or_else(|| {
                    Err(failure.unwrap_or_else(|| {
                        DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            crate::OwnedProtocolWaitFailure::Cancelled,
                        )
                    }))
                });
                let output =
                    crate::conn::CdpTurnOutcome::new_with_protocol_and_post_response_events(
                        prefix, post, scheduler,
                    )
                    .with_renderer_output_boundary(boundary, after)
                    .with_renderer_output_predecessor(predecessor);
                AutomationCommandTaskStep::Complete(Box::new(OwnedAutomationCommandOutcome {
                    result,
                    output,
                }))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Automation command requires its protocol state owner")]
struct AutomationWorkflowOwnerUnavailable;

fn prepare_automation_workflow(
    conn: &mut CdpConnection,
    command: AutomationCommand,
    access: ProtocolStateAccess,
    id: u64,
    result: AutomationCommandResult,
) -> PreparedCommandWorkflow {
    match command {
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
        command => super::configuration::prepare_workflow(conn, command, access, result),
    }
}
