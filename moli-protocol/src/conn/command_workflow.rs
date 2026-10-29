use std::{cell::RefCell, future::Future, panic::AssertUnwindSafe, rc::Rc};

use anyhow::Result;
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};
use tokio::sync::Notify;

use super::{
    CommandDispatchContext, OwnedProtocolWaitFailure, ProtocolWorkflowDriver, ProtocolWorkflowState,
};
use crate::domains::command_output::CommandOutputPlan;

/// The command's existing output context travels with its owned task. A
/// cancelled task or discarded ready result returns it to the same recovery
/// scope, including events and exact renderer fences already accepted.
struct CommandContextSlot {
    payload: RefCell<Option<CommandWorkflowPayload>>,
    returned: Notify,
}

type WorkflowStateCompletion = Box<
    dyn for<'state, 'context> FnOnce(
        ProtocolWorkflowState<'state>,
        &'context mut CommandDispatchContext,
    ) -> CommandOutputPlan,
>;

struct CommandWorkflowPayload {
    context: CommandDispatchContext,
    terminal: Option<CommandOutputPlan>,
    retained_output: CommandOutputPlan,
    state_completion: Option<WorkflowStateCompletion>,
}

#[derive(Clone)]
pub(crate) struct CommandWorkflowContextReturn {
    slot: Rc<CommandContextSlot>,
    command_id: Option<u64>,
    session_id: Option<String>,
    continuation: Option<Shared<LocalBoxFuture<'static, Result<(), OwnedProtocolWaitFailure>>>>,
}

/// A complete business workflow, supervised by the existing command entry.
/// Once cleanup is admitted, cancellation of its waiter transfers polling to
/// that entry's recovery; it does not abandon half of a resource retirement.
pub(crate) struct PreparedCommandWorkflow {
    run: Box<
        dyn FnOnce(CommandWorkflowContext) -> LocalBoxFuture<'static, CompletedCommandWorkflow>,
    >,
    failure_cleanup: Option<CommandWorkflowFailureCleanup>,
    start_protocol_events: Vec<super::BackgroundProtocolEvent>,
    start_terminal: Option<CommandOutputPlan>,
    cancel_on_waiter_drop: bool,
    wait_lifetime: Option<super::CommandWaitLifetime>,
}

pub(crate) struct CommandWorkflowFailureCleanup(
    Box<dyn FnOnce(CommandOutputPlan) -> PreparedCommandWorkflow>,
);

impl CommandWorkflowFailureCleanup {
    pub(crate) fn new(
        cleanup: impl FnOnce(CommandOutputPlan) -> PreparedCommandWorkflow + 'static,
    ) -> Self {
        Self(Box::new(cleanup))
    }

    pub(super) fn recover(self, plan: CommandOutputPlan) -> PreparedCommandWorkflow {
        (self.0)(plan)
    }
}

impl PreparedCommandWorkflow {
    pub(crate) fn new<F: Future<Output = CompletedCommandWorkflow> + 'static>(
        run: impl FnOnce(CommandWorkflowContext) -> F + 'static,
    ) -> Self {
        Self {
            run: Box::new(move |context| run(context).boxed_local()),
            failure_cleanup: None,
            start_protocol_events: Vec::new(),
            start_terminal: None,
            cancel_on_waiter_drop: false,
            wait_lifetime: None,
        }
    }

    pub(crate) fn with_failure_cleanup(
        mut self,
        cleanup: impl FnOnce(CommandOutputPlan) -> Self + 'static,
    ) -> Self {
        self.failure_cleanup = Some(CommandWorkflowFailureCleanup::new(cleanup));
        self
    }

    pub(crate) fn with_start_protocol_events(
        mut self,
        events: Vec<super::BackgroundProtocolEvent>,
    ) -> Self {
        self.start_protocol_events = events;
        self
    }

    pub(crate) fn with_start_terminal(mut self, response: CommandOutputPlan) -> Self {
        self.start_terminal = Some(response);
        self
    }

    /// Ordinary operations may release their wait immediately on cancellation.
    /// Their original context still returns through the same recovery slot;
    /// resource retirement keeps the default completion-owned continuation.
    pub(crate) fn cancel_on_waiter_drop(mut self) -> Self {
        self.cancel_on_waiter_drop = true;
        self
    }

    pub(crate) fn with_wait_lifetime(
        mut self,
        lifetime: Option<super::CommandWaitLifetime>,
    ) -> Self {
        self.wait_lifetime = lifetime;
        self
    }

    pub(crate) fn start(
        self,
        actor_context: &mut CommandDispatchContext,
        command_id: Option<u64>,
        session_id: Option<String>,
    ) -> PendingCommandWorkflowRecovery {
        let wait_lifetime = self.wait_lifetime.clone();
        let (mut context, mut scope) =
            CommandWorkflowContext::take_from(actor_context, command_id, session_id);
        if let Some(response) = self.start_terminal {
            context.retain_terminal(response);
        }
        context
            .context_mut()
            .protocol_events_mut()
            .extend(self.start_protocol_events);
        if let Some(cleanup) = self.failure_cleanup {
            context.context_mut().register_command_failure_cleanup(
                super::CommandFailureCleanup::Continuation(cleanup),
            );
        }
        if self.cancel_on_waiter_drop {
            actor_context.register_command_failure_cleanup(super::CommandFailureCleanup::Workflow(
                scope.clone(),
            ));
            return PendingCommandWorkflowRecovery {
                scope,
                plan: CommandOutputPlan::error(-32000, OwnedProtocolWaitFailure::Cancelled),
                run: Some((self.run)(context)),
                wait_lifetime,
            };
        }
        // The task owns a scope without this continuation. Only the original
        // in-flight recovery owns the future, so there is no Rc cycle through
        // the command context and no independent task registry.
        scope.continuation = Some(
            async move {
                AssertUnwindSafe(async move {
                    drop((self.run)(context).await);
                })
                .catch_unwind()
                .await
                .map_err(|_| OwnedProtocolWaitFailure::Panicked)
            }
            .boxed_local()
            .shared(),
        );
        actor_context.register_command_failure_cleanup(super::CommandFailureCleanup::Workflow(
            scope.clone(),
        ));
        let mut pending = scope.recover(CommandOutputPlan::error(
            -32000,
            OwnedProtocolWaitFailure::Cancelled,
        ));
        pending.wait_lifetime = wait_lifetime;
        pending
    }
}

pub(crate) struct CommandWorkflowContext {
    payload: Option<CommandWorkflowPayload>,
    scope: CommandWorkflowContextReturn,
}

pub(crate) struct PendingCommandWorkflowRecovery {
    scope: CommandWorkflowContextReturn,
    plan: CommandOutputPlan,
    run: Option<LocalBoxFuture<'static, CompletedCommandWorkflow>>,
    wait_lifetime: Option<super::CommandWaitLifetime>,
}

pub(crate) struct CompletedCommandWorkflow {
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
                payload: RefCell::new(None),
                returned: Notify::new(),
            }),
            command_id,
            session_id,
            continuation: None,
        };
        (
            Self {
                payload: Some(CommandWorkflowPayload {
                    context,
                    terminal: None,
                    retained_output: CommandOutputPlan::default(),
                    state_completion: None,
                }),
                scope: scope.clone(),
            },
            scope,
        )
    }

    pub fn context_mut(&mut self) -> &mut CommandDispatchContext {
        &mut self.payload_mut().context
    }

    fn payload(&self) -> &CommandWorkflowPayload {
        self.payload
            .as_ref()
            .expect("live workflow owns its payload")
    }

    fn payload_mut(&mut self) -> &mut CommandWorkflowPayload {
        self.payload
            .as_mut()
            .expect("live workflow owns its payload")
    }

    /// Merge retained projection output and its exact renderer fence into the
    /// original command context without synthesizing a protocol response.
    pub(crate) fn finish_projection(
        self,
        connection: &mut super::CdpConnection,
        target: &mut CommandDispatchContext,
    ) {
        let mut local = CommandDispatchContext::new(target.response_flush().clone());
        let plan = self
            .complete(CommandOutputPlan::default())
            .finish(connection, &mut local);
        let (events, boundary, after_boundary) = local.take_renderer_fenced_protocol_events();
        target.append_renderer_fenced_protocol_events(events, boundary, after_boundary);
        super::dispatch::append_command_output_plan(&mut Vec::new(), target, plan, None, None);
        target.extend_post_response_events(local.take_post_response_events());
        for permit in local.take_runtime_output_projection_permits() {
            target.retain_runtime_output_projection_permit(permit);
        }
        if let Some(predecessor) = local.take_renderer_output_predecessor() {
            target.set_renderer_output_predecessor(predecessor);
        }
    }

    /// Borrow the command context only for this synchronous state operation.
    pub(crate) fn with_state<T: 'static>(
        &mut self,
        driver: &mut ProtocolWorkflowDriver,
        operation: impl for<'state, 'context> FnOnce(
            ProtocolWorkflowState<'state>,
            &'context mut Self,
        ) -> T,
    ) -> Result<T> {
        driver.with_state(|state| operation(state, self))
    }

    pub(crate) fn retain_output(&mut self, plan: CommandOutputPlan) {
        self.payload_mut().retained_output.extend(plan);
    }

    /// The dispatcher has already serialized this command's terminal with its
    /// original wire identity. Keep it durable if a later wait is cancelled.
    pub(super) fn retain_finalized_command(&mut self, plan: CommandOutputPlan) {
        let payload = self.payload_mut();
        payload
            .terminal
            .get_or_insert_with(CommandOutputPlan::default);
        payload.retained_output.extend(plan);
    }

    /// A finite operation has accepted the current command's terminal. Later
    /// waits and recovery may add side effects, but cannot replace that reply.
    pub(crate) fn retain_terminal(&mut self, plan: CommandOutputPlan) {
        debug_assert!(plan.command_status().is_some());
        let payload = self.payload_mut();
        if payload.terminal.is_none() {
            payload.terminal = Some(plan);
        } else {
            payload
                .retained_output
                .extend(plan.into_composite_command_prefix());
        }
    }

    pub(crate) fn has_retained_terminal(&self) -> bool {
        self.payload()
            .terminal
            .as_ref()
            .is_some_and(|plan| plan.command_status().is_some())
    }

    /// Keep completed projection slots in their original event order before
    /// the next slot writes directly into the same command context.
    pub(crate) fn materialize_projection_output(&mut self) {
        let output = std::mem::take(&mut self.payload_mut().retained_output);
        super::dispatch::append_command_output_plan(
            &mut Vec::new(),
            self.context_mut(),
            output,
            None,
            None,
        );
    }

    /// A terminal built by state remains durable before the reply channel
    /// delivers it. Losing that ready result cannot replace success with a
    /// second, invented terminal during recovery.
    #[cfg(test)]
    async fn finish_with_state(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        operation: impl for<'state, 'context> FnOnce(
            ProtocolWorkflowState<'state>,
            &'context mut Self,
        ) -> CommandOutputPlan
        + 'static,
    ) -> CompletedCommandWorkflow {
        let recovery = self.scope.clone();
        match driver.with_state(move |state| {
            let plan = operation(state, &mut self);
            self.complete(plan)
        }) {
            Ok(completed) => completed,
            Err(error) => {
                recovery
                    .recover(CommandOutputPlan::error(-32000, error))
                    .wait()
                    .await
            }
        }
    }

    /// Apply the final finite operation in the consuming actor turn, directly
    /// before its output is handed to the publisher. No other input turn can
    /// publish lifecycle work between this operation and the command result.
    pub(crate) fn complete_in_owner_turn(
        mut self,
        plan: CommandOutputPlan,
        operation: impl for<'state, 'context> FnOnce(
            ProtocolWorkflowState<'state>,
            &'context mut CommandDispatchContext,
        ) -> CommandOutputPlan
        + 'static,
    ) -> CompletedCommandWorkflow {
        self.defer_owner_turn_completion(operation);
        self.complete_retaining_cleanup(plan)
    }

    pub(crate) fn defer_owner_turn_completion(
        &mut self,
        operation: impl for<'state, 'context> FnOnce(
            ProtocolWorkflowState<'state>,
            &'context mut CommandDispatchContext,
        ) -> CommandOutputPlan
        + 'static,
    ) {
        assert!(
            self.payload().state_completion.is_none(),
            "one final state operation per command"
        );
        self.payload_mut().state_completion = Some(Box::new(operation));
    }

    pub(crate) fn fail(self, plan: CommandOutputPlan) -> CompletedCommandWorkflow {
        self.complete_retaining_cleanup(plan)
    }

    pub fn complete(mut self, plan: CommandOutputPlan) -> CompletedCommandWorkflow {
        if self.payload().state_completion.is_none() {
            self.context_mut().take_command_failure_cleanup();
        }
        self.complete_retaining_cleanup(plan)
    }

    fn complete_retaining_cleanup(mut self, plan: CommandOutputPlan) -> CompletedCommandWorkflow {
        let payload = self.payload_mut();
        if let Some(terminal) = payload.terminal.as_mut() {
            terminal.extend(plan.into_composite_command_prefix());
        } else {
            payload.terminal = Some(plan);
        }
        CompletedCommandWorkflow {
            command_id: self.scope.command_id,
            session_id: self.scope.session_id.clone(),
            context: self,
        }
    }
}

impl Drop for CommandWorkflowContext {
    fn drop(&mut self) {
        if let Some(payload) = self.payload.take() {
            let previous = self.scope.slot.payload.borrow_mut().replace(payload);
            debug_assert!(previous.is_none(), "one command has one context owner");
            self.scope.slot.returned.notify_one();
        }
    }
}

impl CommandWorkflowContextReturn {
    pub(super) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(super) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(super) fn recover(self, plan: CommandOutputPlan) -> PendingCommandWorkflowRecovery {
        PendingCommandWorkflowRecovery {
            scope: self,
            plan,
            run: None,
            wait_lifetime: None,
        }
    }
}

impl PendingCommandWorkflowRecovery {
    pub(crate) fn wait_lifetime(&self) -> Option<super::CommandWaitLifetime> {
        self.wait_lifetime.clone()
    }

    pub(crate) fn continues_after_session_detachment(&self) -> bool {
        self.run.is_none()
    }

    pub async fn wait(mut self) -> CompletedCommandWorkflow {
        if let Some(run) = self.run.take() {
            match AssertUnwindSafe(run).catch_unwind().await {
                Ok(completed) => return completed,
                Err(_) => {
                    self.plan =
                        CommandOutputPlan::error(-32000, OwnedProtocolWaitFailure::Panicked);
                }
            }
        }
        let fallback = match self.scope.continuation.as_ref() {
            Some(continuation) => match continuation.clone().await {
                Ok(()) => self.plan,
                Err(error) => CommandOutputPlan::error(-32000, error),
            },
            None => self.plan,
        };
        loop {
            let returned = self.scope.slot.returned.notified();
            let payload = self.scope.slot.payload.borrow_mut().take();
            if let Some(mut payload) = payload {
                let plan = payload.terminal.take().unwrap_or(fallback);
                for event in payload.context.take_deferred_command_events() {
                    payload.retained_output.push_background_event(event);
                }
                return CommandWorkflowContext {
                    payload: Some(payload),
                    scope: self.scope.clone(),
                }
                .complete_retaining_cleanup(plan);
            }
            returned.await;
        }
    }
}

impl CompletedCommandWorkflow {
    pub fn finish(
        mut self,
        connection: &mut super::CdpConnection,
        target: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        target.take_command_failure_cleanup();
        let CommandWorkflowPayload {
            context,
            terminal,
            retained_output,
            state_completion,
        } = self
            .context
            .payload
            .take()
            .expect("completed context consumed once");
        *target = context;
        let mut plan = terminal.expect("completed workflow owns its terminal");
        let mut completion_output = None;
        if let Some(completion) = state_completion {
            // The final operation may discover that its original owner has
            // retired and install the corresponding resource rollback. Keep
            // that new cleanup; only the preceding phase's authority ends here.
            let previous_cleanup = target.take_command_failure_cleanup();
            match std::panic::catch_unwind(AssertUnwindSafe(|| {
                completion(ProtocolWorkflowState::new(connection), target)
            })) {
                Ok(output) => {
                    completion_output = Some(output);
                }
                Err(_) => {
                    if let Some(cleanup) = previous_cleanup
                        && target.command_failure_cleanup.borrow().is_none()
                    {
                        target.register_command_failure_cleanup(cleanup);
                    }
                    plan = CommandOutputPlan::error(-32000, OwnedProtocolWaitFailure::Panicked);
                }
            }
        }
        plan.extend(retained_output);
        if let Some(output) = completion_output {
            plan.extend(output);
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BackgroundProtocolEvent, CdpCommandTaskStep, CdpConnection};
    use serde_json::json;

    #[tokio::test]
    async fn accepted_transport_terminal_is_durable_before_first_poll_and_after_ready_loss() {
        for phase in ["before", "waiting", "ready"] {
            let mut connection = CdpConnection::new();
            let mut actor_context = CommandDispatchContext::default();
            let (release, held) = tokio::sync::oneshot::channel::<()>();
            let workflow = PreparedCommandWorkflow::new(move |context| async move {
                held.await.unwrap();
                context.complete(CommandOutputPlan::error(
                    -32000,
                    OwnedProtocolWaitFailure::Cancelled,
                ))
            })
            .cancel_on_waiter_drop()
            .with_start_terminal(CommandOutputPlan::success());
            let pending = workflow.start(&mut actor_context, Some(70), None);
            let mut waiting = Box::pin(pending.wait());
            if phase != "before" {
                assert!(waiting.as_mut().now_or_never().is_none());
            }
            if phase == "ready" {
                release.send(()).unwrap();
                drop(waiting.await);
            } else {
                drop(waiting);
                drop(release);
            }
            let Some(super::super::CommandFailureCleanup::Workflow(scope)) =
                actor_context.take_command_failure_cleanup()
            else {
                panic!("the original waiter owns its context");
            };
            let completed = scope
                .recover(CommandOutputPlan::error(
                    -32000,
                    OwnedProtocolWaitFailure::Cancelled,
                ))
                .wait()
                .await;
            let mut messages = Vec::new();
            completed
                .finish(&mut connection, &mut actor_context)
                .emit_into(&mut messages, Some(70), None);
            assert_eq!(messages, [json!({"id":70,"result":{}})], "{phase}");
        }
    }

    #[tokio::test]
    async fn accepted_terminal_survives_late_wait_failure_cancellation_and_ready_loss() {
        for phase in 0..5 {
            let owner = crate::conn::ProtocolStateOwner::new(CdpConnection::new());
            let access = owner.handle();
            let mut actor_context = CommandDispatchContext::default();
            let (release, held) = tokio::sync::oneshot::channel::<()>();
            let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
                context
                    .with_state(&mut ProtocolWorkflowDriver::new(access), |_, context| {
                        let mut output = CommandOutputPlan::result(json!({"accepted": 42}));
                        output.push_background_event(BackgroundProtocolEvent::immediate(
                            json!({"method":"Moli.accepted","params":{}}),
                        ));
                        context.retain_terminal(output);
                    })
                    .unwrap();
                held.await.unwrap();
                assert_ne!(phase, 4, "injected failure after accepting a terminal");
                let mut tail = CommandOutputPlan::error(-32000, "late failure");
                tail.push_background_event(BackgroundProtocolEvent::immediate(
                    json!({"method":"Moli.tail","params":{}}),
                ));
                if phase == 1 {
                    context.fail(tail)
                } else {
                    context.complete(tail)
                }
            });
            let pending = workflow.start(&mut actor_context, Some(89), None);
            let mut wait = Box::pin(pending.wait());
            assert!(wait.as_mut().now_or_never().is_none());

            assert!(wait.as_mut().now_or_never().is_none());
            if phase == 2 {
                drop(wait);
                let Some(super::super::CommandFailureCleanup::Workflow(scope)) =
                    actor_context.take_command_failure_cleanup()
                else {
                    panic!("original recovery scope");
                };
                wait = Box::pin(
                    scope
                        .recover(CommandOutputPlan::error(-32000, "cancelled"))
                        .wait(),
                );
                assert!(wait.as_mut().now_or_never().is_none());
            }
            release.send(()).unwrap();
            let mut completed = wait.await;
            if phase == 3 {
                drop(completed);
                let Some(super::super::CommandFailureCleanup::Workflow(scope)) =
                    actor_context.take_command_failure_cleanup()
                else {
                    panic!("original recovery scope");
                };
                completed = scope
                    .recover(CommandOutputPlan::error(-32000, "discarded"))
                    .wait()
                    .await;
            }
            let plan = owner
                .with_state(|state| state.finish_command_workflow(completed, &mut actor_context))
                .unwrap();
            let mut messages = Vec::new();
            plan.emit_into(&mut messages, Some(89), None);
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == 89)
                    .count(),
                1
            );
            assert_eq!(messages[0]["result"], json!({"accepted":42}));
            assert_eq!(messages[1]["method"], "Moli.accepted");
            assert_eq!(messages.len(), if phase == 4 { 2 } else { 3 });
            if phase != 4 {
                assert_eq!(messages[2]["method"], "Moli.tail");
            }
        }
    }

    #[tokio::test]
    async fn accepted_terminal_survives_cancellation_after_synchronous_state_commit() {
        let owner = crate::conn::ProtocolStateOwner::new(CdpConnection::new());
        let access = owner.handle();
        let mut actor_context = CommandDispatchContext::default();
        let (mut context, scope) =
            CommandWorkflowContext::take_from(&mut actor_context, Some(88), None);
        let mut wait = Box::pin(async move {
            context
                .with_state(&mut ProtocolWorkflowDriver::new(access), |_, context| {
                    context.retain_terminal(CommandOutputPlan::result(json!({"accepted":43})));
                })
                .unwrap();
            std::future::pending::<()>().await;
        });
        assert!(wait.as_mut().now_or_never().is_none());

        drop(wait);
        let completed = scope
            .recover(CommandOutputPlan::error(-32000, "discarded state reply"))
            .wait()
            .await;
        let mut messages = Vec::new();
        owner
            .with_state(|state| state.finish_command_workflow(completed, &mut actor_context))
            .unwrap()
            .emit_into(&mut messages, Some(88), None);
        assert_eq!(messages, vec![json!({"id":88,"result":{"accepted":43}})]);
    }

    #[tokio::test]
    async fn discarded_workflow_result_retains_its_finite_owner_completion_once() {
        for panic_in_completion in [false, true] {
            let mut actor_context = CommandDispatchContext::default();
            let calls = Rc::new(std::cell::Cell::new(0));
            let applied = calls.clone();
            let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
                let mut prefix = CommandOutputPlan::default();
                prefix.push_background_event(BackgroundProtocolEvent::page_file_chooser_opened(
                    None,
                    "FRAME-finite-completion",
                    "selectSingle",
                    41,
                    None,
                ));
                context.retain_output(prefix);
                context.complete_in_owner_turn(CommandOutputPlan::success(), move |_, _| {
                    applied.set(applied.get() + 1);
                    assert!(!panic_in_completion, "injected finite completion failure");
                    let mut output = CommandOutputPlan::default();
                    output.push_background_event(BackgroundProtocolEvent::immediate(json!({
                        "method": "Moli.afterFiniteCompletion", "params": {}
                    })));
                    output
                })
            });
            let pending = workflow.start(&mut actor_context, Some(90), None);
            let completed = pending.wait().await;
            assert_eq!(
                calls.get(),
                0,
                "waiting does not execute the final state operation"
            );
            drop(completed);
            let Some(super::super::CommandFailureCleanup::Workflow(scope)) =
                actor_context.take_command_failure_cleanup()
            else {
                panic!("the original command retains its completion authority");
            };
            let completed = scope
                .recover(CommandOutputPlan::error(
                    -32000,
                    OwnedProtocolWaitFailure::Cancelled,
                ))
                .wait()
                .await;
            let plan = completed.finish(&mut CdpConnection::new(), &mut actor_context);
            assert_eq!(calls.get(), 1);
            let mut messages = Vec::new();
            plan.emit_into(&mut messages, Some(90), None);
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["id"] == 90)
                    .count(),
                1
            );
            assert_eq!(messages[0].get("error").is_some(), panic_in_completion);
            assert_eq!(messages[1]["method"], "Page.fileChooserOpened");
            assert_eq!(messages.len(), if panic_in_completion { 2 } else { 3 });
        }
    }

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
        let plan = completed.finish(&mut CdpConnection::new(), &mut actor_context);
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

    #[tokio::test]
    async fn cancelled_workflow_keeps_synchronously_materialized_boundary_and_actual_terminal() {
        use crate::conn::ProtocolWorkflowDriver;
        use moli_core::{
            PageId, RendererOutputCursor, RendererOutputFence, RendererOutputStreamIdentity,
        };
        for terminal_ready in [false, true] {
            let owner = crate::conn::ProtocolStateOwner::new(CdpConnection::new());
            let access = owner.handle();
            let mut actor_context = CommandDispatchContext::default();
            let (mut task_context, recovery) =
                CommandWorkflowContext::take_from(&mut actor_context, Some(92), None);
            let before = BackgroundProtocolEvent::page_file_chooser_opened(
                None,
                "FRAME-state-result",
                "selectSingle",
                43,
                Some("NODE-state-result".into()),
            );
            let after = BackgroundProtocolEvent::immediate(
                json!({"method":"Moli.afterOriginalCursor","params":{}}),
            );
            let expected_before = before.clone();
            let expected_after = after.clone();
            let cursor = RendererOutputCursor::new_for_test(
                RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(
                    93,
                )),
                1,
            );
            let operation =
                move |_: ProtocolWorkflowState<'_>, context: &mut CommandWorkflowContext| {
                    let mut produced = CommandOutputPlan::default();
                    produced.push_background_event(before);
                    produced
                        .insert_renderer_output_boundary(RendererOutputFence::new_for_test(cursor));
                    produced.push_background_event(after);
                    context.retain_output(produced);
                };
            let mut task = Box::pin(async move {
                let mut driver = ProtocolWorkflowDriver::new(access);
                if terminal_ready {
                    task_context
                        .finish_with_state(&mut driver, move |state, context| {
                            operation(state, context);
                            CommandOutputPlan::result(json!({"original":"settled"}))
                        })
                        .await;
                } else {
                    task_context.with_state(&mut driver, operation).unwrap();
                }
                std::future::pending::<()>().await;
            });
            assert!(task.as_mut().now_or_never().is_none());
            // The synchronous operation has committed its original output.
            // Cancelling the later wait must preserve its terminal and fence.
            drop(task);
            let completed = recovery
                .recover(CommandOutputPlan::error(-32000, "cancelled"))
                .wait()
                .await;
            let plan = completed.finish(&mut CdpConnection::new(), &mut actor_context);
            let (before, boundary, after, post) =
                plan.into_renderer_fenced_background_and_post_response_events(Some(92), None);
            assert_eq!(boundary.unwrap().cursor(), cursor);
            assert_eq!(before.len(), 2);
            assert_eq!(before[1], expected_before);
            assert_eq!(after, vec![expected_after]);
            assert!(post.is_empty());
            let response = before[0].clone().into_protocol_message();
            assert_eq!(response["id"], 92);
            if terminal_ready {
                assert_eq!(response["result"], json!({"original":"settled"}));
                assert!(
                    response.get("error").is_none(),
                    "recovery cannot invent a second terminal"
                );
            } else {
                assert_eq!(response["error"]["message"], "cancelled");
            }
        }
    }

    #[tokio::test]
    async fn unpolled_state_operation_returns_context_without_applying_work() {
        use crate::conn::ProtocolWorkflowDriver;
        let owner = crate::conn::ProtocolStateOwner::new(CdpConnection::new());
        let access = owner.handle();
        let mut actor_context = CommandDispatchContext::default();
        let (mut task_context, recovery) =
            CommandWorkflowContext::take_from(&mut actor_context, Some(94), None);
        let applied = Rc::new(std::cell::Cell::new(false));
        let observed = applied.clone();
        let task = Box::pin(async move {
            task_context
                .with_state(&mut ProtocolWorkflowDriver::new(access), move |_, _| {
                    applied.set(true);
                })
                .unwrap();
        });
        drop(task);
        let returned = Box::pin(
            recovery
                .recover(CommandOutputPlan::error(-32000, "cancelled"))
                .wait(),
        );
        assert!(
            !observed.get(),
            "an operation cancelled before its first poll cannot mutate state"
        );
        let plan = returned
            .await
            .finish(&mut CdpConnection::new(), &mut actor_context);
        let (events, boundary, after, post) =
            plan.into_renderer_fenced_background_and_post_response_events(Some(94), None);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].clone().into_protocol_message()["error"]["message"],
            "cancelled"
        );
        assert!(boundary.is_none() && after.is_empty() && post.is_empty());
    }

    #[tokio::test]
    async fn admitted_workflow_survives_unpolled_cancel_wait_cancel_and_discarded_terminal() {
        use crate::conn::{CommandFailureCleanup, OwnedProtocolWaitFailure};
        for phase in 0..3 {
            let mut conn = CdpConnection::new();
            let mut actor_context = CommandDispatchContext::default();
            let (release, held) = tokio::sync::oneshot::channel::<()>();
            let finished = Rc::new(std::cell::Cell::new(0));
            let observed = finished.clone();
            let fact = BackgroundProtocolEvent::page_file_chooser_opened(
                None,
                "FRAME-admitted",
                "selectSingle",
                45,
                Some("NODE-admitted".into()),
            );
            let expected_fact = fact.clone();
            let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
                let mut prefix = CommandOutputPlan::default();
                prefix.push_background_event(fact);
                context.retain_output(prefix);
                held.await.unwrap();
                finished.set(finished.get() + 1);
                context.complete(CommandOutputPlan::result(json!({"original":"finished"})))
            });
            let pending = workflow.start(&mut actor_context, Some(95), None);
            let weak = match actor_context
                .command_failure_cleanup
                .borrow()
                .as_ref()
                .unwrap()
            {
                CommandFailureCleanup::Workflow(scope) => Rc::downgrade(&scope.slot),
                _ => panic!("the original command owns recovery"),
            };
            let mut waiting = Box::pin(pending.wait());
            if phase != 0 {
                assert!(waiting.as_mut().now_or_never().is_none());
            }
            if phase == 2 {
                release.send(()).unwrap();
                // The workflow has completed, but actor has not accepted its
                // terminal. Its original success must survive this ready loss.
                drop(waiting.await);
            } else {
                drop(waiting);
                release.send(()).unwrap();
            }
            let CdpCommandTaskStep::Pending(recovery) = conn
                .complete_failed_command_wait_with_context(
                    Some(95),
                    None,
                    OwnedProtocolWaitFailure::Cancelled.message(),
                    &mut actor_context,
                )
            else {
                panic!("recovery keeps the original in-flight responsibility")
            };
            let ready = recovery.wait().await;
            let CdpCommandTaskStep::Complete(outcome) =
                conn.complete_command_dispatch_with_context(ready, &mut actor_context)
            else {
                panic!("owned workflow has a finite terminal")
            };
            let (events, _, _) = outcome.into_protocol_event_parts();
            assert_eq!(events.len(), 2);
            assert_eq!(
                events[0].clone().into_protocol_message()["result"],
                json!({"original":"finished"})
            );
            assert_eq!(events[1], expected_fact);
            assert_eq!(
                observed.get(),
                1,
                "cancellation resumes, never restarts, admitted work"
            );
            assert!(actor_context.command_failure_cleanup.borrow().is_none());
            assert!(
                weak.upgrade().is_none(),
                "completed workflow must not retain an Rc cycle"
            );
        }
    }

    #[tokio::test]
    async fn admitted_workflow_panic_keeps_materialized_prefix_and_one_error() {
        fn fail_after_materialization() {
            panic!("controlled workflow failure after materialization");
        }
        let mut actor_context = CommandDispatchContext::default();
        let workflow = PreparedCommandWorkflow::new(|mut context| async move {
            let mut prefix = CommandOutputPlan::default();
            prefix.push_background_event(BackgroundProtocolEvent::page_file_chooser_opened(
                None,
                "FRAME-panicked",
                "selectSingle",
                46,
                Some("NODE-panicked".into()),
            ));
            context.retain_output(prefix);
            fail_after_materialization();
            context.complete(CommandOutputPlan::success())
        });
        let completed = workflow
            .start(&mut actor_context, Some(96), None)
            .wait()
            .await;
        let plan = completed.finish(&mut CdpConnection::new(), &mut actor_context);
        let (events, boundary, after, post) =
            plan.into_renderer_fenced_background_and_post_response_events(Some(96), None);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].clone().into_protocol_message()["error"]["message"],
            OwnedProtocolWaitFailure::Panicked.message()
        );
        let (_, typed) = events[1].clone().into_parts();
        assert!(matches!(
            typed,
            Some(crate::automation::AutomationEvent::PageFileChooserOpened(_))
        ));
        assert!(boundary.is_none() && after.is_empty() && post.is_empty());
    }
}
