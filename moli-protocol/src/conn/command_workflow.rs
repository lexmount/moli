use std::{cell::RefCell, rc::Rc};

use tokio::sync::Notify;

use super::{CommandDispatchContext, ProtocolStateAccess};
use crate::domains::command_output::CommandOutputPlan;

/// The command's existing output context travels with its owned task. A
/// cancelled task or discarded ready result returns it to the same recovery
/// scope, including events and exact renderer fences already accepted.
struct CommandContextSlot {
    context: RefCell<Option<(CommandDispatchContext, Option<CommandOutputPlan>)>>,
    returned: Notify,
}

#[derive(Clone)]
pub(crate) struct CommandWorkflowContextReturn {
    slot: Rc<CommandContextSlot>,
    command_id: Option<u64>,
    session_id: Option<String>,
}

pub(super) struct CommandWorkflowContext {
    context: Option<CommandDispatchContext>,
    terminal: Option<CommandOutputPlan>,
    scope: CommandWorkflowContextReturn,
}

pub(super) struct OwnedCommandWorkflow {
    pub access: ProtocolStateAccess,
    pub context: CommandWorkflowContext,
}

pub(super) struct PendingCommandWorkflowRecovery {
    scope: CommandWorkflowContextReturn,
    plan: CommandOutputPlan,
}

pub(super) struct CompletedCommandWorkflow {
    pub command_id: Option<u64>,
    pub session_id: Option<String>,
    context: CommandWorkflowContext,
}

impl CommandWorkflowContext {
    pub fn take_from(
        context: &mut CommandDispatchContext,
        command_id: Option<u64>,
        session_id: Option<String>,
    ) -> (Self, CommandWorkflowContextReturn) {
        assert!(context.command_failure_cleanup.borrow().is_none());
        let replacement = CommandDispatchContext::new(context.response_flush().clone());
        let context = std::mem::replace(context, replacement);
        let scope = CommandWorkflowContextReturn {
            slot: Rc::new(CommandContextSlot {
                context: RefCell::new(None),
                returned: Notify::new(),
            }),
            command_id,
            session_id,
        };
        (
            Self {
                context: Some(context),
                terminal: None,
                scope: scope.clone(),
            },
            scope,
        )
    }

    pub fn context_mut(&mut self) -> &mut CommandDispatchContext {
        self.context
            .as_mut()
            .expect("live workflow owns its context")
    }

    pub fn complete(mut self, plan: CommandOutputPlan) -> CompletedCommandWorkflow {
        self.terminal = Some(plan);
        CompletedCommandWorkflow {
            command_id: self.scope.command_id,
            session_id: self.scope.session_id.clone(),
            context: self,
        }
    }
}

impl Drop for CommandWorkflowContext {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            let previous = self
                .scope
                .slot
                .context
                .borrow_mut()
                .replace((context, self.terminal.take()));
            debug_assert!(previous.is_none(), "one command has one context owner");
            self.scope.slot.returned.notify_one();
        }
    }
}

impl CommandWorkflowContextReturn {
    pub(super) fn recover(self, plan: CommandOutputPlan) -> PendingCommandWorkflowRecovery {
        PendingCommandWorkflowRecovery { scope: self, plan }
    }
}

impl PendingCommandWorkflowRecovery {
    pub async fn wait(self) -> CompletedCommandWorkflow {
        loop {
            let returned = self.scope.slot.returned.notified();
            let context = self.scope.slot.context.borrow_mut().take();
            if let Some((mut context, terminal)) = context {
                let mut plan = terminal.unwrap_or(self.plan);
                for event in context.take_deferred_command_events() {
                    plan.push_background_event(event);
                }
                return CommandWorkflowContext {
                    context: Some(context),
                    terminal: None,
                    scope: self.scope.clone(),
                }
                .complete(plan);
            }
            returned.await;
        }
    }
}

impl CompletedCommandWorkflow {
    pub fn finish(mut self, target: &mut CommandDispatchContext) -> CommandOutputPlan {
        target.take_command_failure_cleanup();
        *target = self
            .context
            .context
            .take()
            .expect("completed context consumed once");
        self.context
            .terminal
            .take()
            .expect("completed workflow owns its terminal")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BackgroundProtocolEvent, CdpCommandTaskStep, CdpConnection};
    use futures_util::FutureExt as _;
    use serde_json::json;

    #[tokio::test]
    async fn cancelling_before_context_return_preserves_typed_side_effects_and_order() {
        let mut actor_context = CommandDispatchContext::default();
        let prefix = BackgroundProtocolEvent::immediate(
            json!({"method":"Page.frameStartedLoading","params":{"frameId":"FRAME-context"}}),
        );
        actor_context.push_protocol_event(prefix);
        let (mut task_context, scope) =
            CommandWorkflowContext::take_from(&mut actor_context, Some(91), None);
        task_context.context_mut().defer_command_events(vec![
            BackgroundProtocolEvent::page_file_chooser_opened(
                None,
                "FRAME-context",
                "selectSingle",
                42,
                Some("NODE-context".into()),
            ),
        ]);
        let mut recovery = Box::pin(
            scope
                .recover(CommandOutputPlan::error(-32000, "cancelled"))
                .wait(),
        );
        assert!(recovery.as_mut().now_or_never().is_none());
        drop(task_context);
        let completed = recovery.await;
        let plan = completed.finish(&mut actor_context);
        let mut connection = CdpConnection::new();
        let CdpCommandTaskStep::Complete(outcome) =
            connection.complete_with_output_plan(&mut actor_context, plan, Some(91), None)
        else {
            panic!("recovery is a finite terminal");
        };
        let (mut events, _, _) = outcome.into_protocol_event_parts();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events.remove(0).into_protocol_message()["method"],
            "Page.frameStartedLoading"
        );
        assert_eq!(
            events.remove(0).into_protocol_message()["error"]["code"],
            -32000
        );
        let (chooser, typed) = events.remove(0).into_parts();
        assert_eq!(chooser["method"], "Page.fileChooserOpened");
        assert!(matches!(
            typed,
            Some(crate::automation::AutomationEvent::PageFileChooserOpened(_))
        ));
    }
}
