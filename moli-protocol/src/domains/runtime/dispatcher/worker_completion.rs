use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{
    CommandWorkflowContext, PreparedCommandWorkflow, ProtocolStateAccess, ProtocolWorkflowDriver,
    WorkerRuntimeScope,
};

enum WorkerDispatch {
    Shared(CompletedSharedWorkerRuntimeProtocolMessageDispatch),
    Service(CompletedServiceWorkerRuntimeProtocolMessageDispatch),
}

pub(crate) struct PreparedWorkerRuntimeCommand {
    scope: WorkerRuntimeScope,
    meta: RuntimeCommandCompletionMeta,
    service: bool,
    dispatch: Result<WorkerDispatch>,
    binding_effect: Option<SharedWorkerRuntimeBindingEffect>,
}

pub(crate) struct WorkerRuntimeContinuation {
    scope: WorkerRuntimeScope,
    meta: RuntimeCommandCompletionMeta,
    service: bool,
    binding_effect: Option<SharedWorkerRuntimeBindingEffect>,
    routed: RuntimeInspectorRoutedOutput,
    receiver: Option<RuntimeInspectorAsyncCompletionReceiver>,
    session_success: Option<bool>,
    claimed: Option<ClaimedPendingInspectorAwait>,
    access: Option<ProtocolStateAccess>,
    replay: Option<PreparedWorkerBindingReplay>,
    succeeded: bool,
}

impl PreparedWorkerRuntimeCommand {
    pub(crate) fn from_completed(
        completed: Box<CompletedRuntimeCommandDispatch>,
    ) -> Result<Self, Box<CompletedRuntimeCommandDispatch>> {
        if !matches!(
            &completed.completed,
            CompletedRuntimeCommandKind::SharedWorkerInspector { .. }
                | CompletedRuntimeCommandKind::ServiceWorkerInspector { .. }
        ) {
            return Err(completed);
        }
        let completed = *completed;
        let meta = RuntimeCommandCompletionMeta::from(&completed);
        let (scope, service, dispatch, binding_effect) = match completed.completed {
            CompletedRuntimeCommandKind::SharedWorkerInspector {
                scope,
                completed,
                binding_effect,
            } => (
                scope,
                false,
                completed.map(WorkerDispatch::Shared),
                binding_effect,
            ),
            CompletedRuntimeCommandKind::ServiceWorkerInspector { scope, completed } => {
                (scope, true, completed.map(WorkerDispatch::Service), None)
            }
            _ => unreachable!("only Worker completions enter this workflow"),
        };
        Ok(Self {
            scope,
            meta,
            service,
            dispatch,
            binding_effect,
        })
    }

    pub(crate) fn into_workflow(self, access: ProtocolStateAccess) -> PreparedCommandWorkflow {
        PreparedCommandWorkflow::new(move |mut context| async move {
            match self
                .run(&mut ProtocolWorkflowDriver::owned(access), &mut context)
                .await
            {
                Ok(()) => context.complete(CommandOutputPlan::default()),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
    }

    pub(crate) async fn run_inline(
        self,
        conn: &mut CdpConnection,
        flush: &crate::conn::CommandResponseFlushContext,
    ) -> CommandOutputPlan {
        let mut local = crate::conn::CommandDispatchContext::new(flush.clone());
        let (mut context, _scope) = CommandWorkflowContext::take_from(
            &mut local,
            self.meta.command_id,
            self.meta.session_id().map(str::to_owned),
        );
        let completed = match self
            .run(&mut ProtocolWorkflowDriver::inline(conn), &mut context)
            .await
        {
            Ok(()) => context.complete(CommandOutputPlan::default()),
            Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
        };
        let plan = completed.finish(conn, &mut local);
        let mut combined = CommandOutputPlan::default();
        combined.extend_background_events(local.take_protocol_events());
        combined.extend(plan);
        combined.extend_post_response_events(local.take_post_response_events());
        if let Some(predecessor) = local.take_renderer_output_predecessor() {
            combined.set_renderer_output_predecessor(predecessor);
        }
        combined
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        // SessionSink commands own a native settlement receipt. Waiting here
        // leaves the actor free to ingest the publication that settles it.
        if !self.meta.await_promise {
            let result = match &mut self.dispatch {
                Ok(WorkerDispatch::Shared(dispatch))
                    if dispatch.response_delivery()
                        == RendererInspectorResponseDelivery::SessionSink =>
                {
                    dispatch.wait_for_session_response().await
                }
                Ok(WorkerDispatch::Service(dispatch))
                    if dispatch.response_delivery()
                        == RendererInspectorResponseDelivery::SessionSink =>
                {
                    dispatch.wait_for_session_response().await
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                self.dispatch = Err(error);
            }
        }
        let Some(mut continuation) = context
            .with_state(driver, move |state, context| {
                state.finish_worker_runtime_dispatch(self, context)
            })
            .await?
        else {
            return Ok(());
        };
        if let Some(receiver) = continuation.receiver.take() {
            let response = receiver
                .await
                .context(crate::conn::RuntimeInspectorError::ResponseCanceled);
            let Some(next) = context
                .with_state(driver, move |state, context| {
                    state.finish_worker_runtime_response(continuation, response, context)
                })
                .await?
            else {
                return Ok(());
            };
            continuation = next;
        }
        if let Some(replay) = continuation.replay.take() {
            replay.run(driver).await;
        }
        context
            .with_state(driver, move |state, context| {
                state.finish_worker_runtime_tail(continuation, context)
            })
            .await
    }

    pub(crate) fn finish_dispatch(
        self,
        conn: &mut CdpConnection,
        context: &mut CommandWorkflowContext,
    ) -> Option<WorkerRuntimeContinuation> {
        if !self.scope.binding_is_current(conn) {
            return None;
        }
        let dispatch = match self.dispatch {
            Ok(dispatch) => dispatch,
            Err(error) => {
                retain_error(conn, &self.scope, self.service, self.meta, error, context);
                return None;
            }
        };
        let (messages, receiver, delivery, predecessor, session_success) = match dispatch {
            WorkerDispatch::Shared(mut dispatch) => {
                let delivery = dispatch.response_delivery();
                let predecessor = dispatch.session_response_predecessor();
                let session_success = dispatch.session_response_succeeded();
                let receiver = dispatch.take_deferred_response_receiver();
                (
                    conn.complete_shared_worker_runtime_protocol_message_for_session(dispatch),
                    receiver,
                    delivery,
                    predecessor,
                    session_success,
                )
            }
            WorkerDispatch::Service(mut dispatch) => {
                let delivery = dispatch.response_delivery();
                let predecessor = dispatch.session_response_predecessor();
                let session_success = dispatch.session_response_succeeded();
                let receiver = dispatch.take_deferred_response_receiver();
                (
                    conn.complete_service_worker_runtime_protocol_message_for_session(dispatch),
                    receiver,
                    delivery,
                    predecessor,
                    session_success,
                )
            }
        };
        let messages = match messages {
            Ok(messages) => messages,
            Err(error) => {
                retain_error(conn, &self.scope, self.service, self.meta, error, context);
                return None;
            }
        };
        let mut routed = RuntimeInspectorRoutedOutput::default();
        if let Some(predecessor) = predecessor {
            routed.set_renderer_output_predecessor(predecessor);
        }
        let saw_response = route_inspector_messages_into_routed_output(
            conn,
            messages,
            self.meta.command_id,
            &self.meta.owner_scope,
            &mut routed,
        );
        let receiver =
            if delivery == RendererInspectorResponseDelivery::AdapterReply && !saw_response {
                receiver
            } else {
                None
            };
        if delivery == RendererInspectorResponseDelivery::SessionSink
            && self.meta.await_promise
            && session_success.is_none()
        {
            let mut plan = CommandOutputPlan::default();
            routed.push_background_events_before_response_events(&mut plan, self.meta.command_id);
            context.retain_output(plan);
            return None;
        }
        let claimed = (receiver.is_some() && self.meta.wait_for_deferred_reply)
            .then(|| {
                self.meta.command_id.and_then(|id| {
                    conn.claim_pending_inspector_await_for_scheduler_deferred_reply(
                        id,
                        &self.meta.owner_scope,
                    )
                })
            })
            .flatten();
        let mut next = WorkerRuntimeContinuation {
            scope: self.scope,
            meta: self.meta,
            service: self.service,
            binding_effect: self.binding_effect,
            routed,
            receiver,
            session_success,
            claimed,
            access: conn.protocol_state_access(),
            replay: None,
            succeeded: false,
        };
        if next.receiver.is_some() {
            // Accepted notifications belong to this command's existing output
            // guard before a finite state reply can be lost.
            context.context_mut().protocol_events_mut().extend(
                next.routed
                    .take_ready_background_events_for_command(next.meta.command_id),
            );
        } else {
            next.finish_projection(conn, context);
        }
        Some(next)
    }
}

impl WorkerRuntimeContinuation {
    pub(crate) fn finish_response(
        mut self,
        conn: &mut CdpConnection,
        response: Result<moli_core::RendererRuntimeInspectorAsyncCompletion>,
        context: &mut CommandWorkflowContext,
    ) -> Option<Self> {
        if !self.scope.binding_is_current(conn) {
            return None;
        }
        if let Some(id) = self.meta.command_id {
            let response = RuntimeInspectorResponseReady::for_owner(
                id,
                &self.meta.owner_scope,
                response.map_err(|error| error.to_string()),
            );
            if let Some(response) = conn.resolve_runtime_inspector_response_ready(response) {
                let (_, output, predecessor) = response.into_renderer_command_output();
                if let Some(predecessor) = predecessor {
                    self.routed.set_renderer_output_predecessor(predecessor);
                }
                let mut events = Vec::new();
                conn.route_renderer_runtime_command_output_for_owner_into(
                    output,
                    Some(id),
                    &self.meta.owner_scope,
                    &mut events,
                );
                self.routed.append_ordered_events(events);
            }
        }
        conn.complete_claimed_pending_inspector_await_for_scheduler_deferred_reply(
            self.claimed.take(),
            self.routed.events(),
        );
        self.finish_projection(conn, context);
        Some(self)
    }

    fn finish_projection(
        &mut self,
        conn: &mut CdpConnection,
        context: &mut CommandWorkflowContext,
    ) {
        self.succeeded = self
            .session_success
            .unwrap_or_else(|| self.routed.command_response_succeeded(self.meta.command_id));
        let mut plan = CommandOutputPlan::default();
        if self.scope.can_configure(conn) {
            if self.service {
                apply_service_worker_runtime_completion_projection(
                    conn,
                    self.meta.session_id(),
                    self.meta.action,
                    self.succeeded,
                );
            } else {
                apply_shared_worker_runtime_completion_projection(
                    conn,
                    self.meta.session_id(),
                    self.meta.action,
                    self.succeeded,
                );
            }
            if self.succeeded {
                if let Some(action) = console_action_from_protocol_method(self.meta.action)
                    && !apply_console_output_state_for_session(conn, self.meta.session_id(), action)
                {
                    if self.session_success.is_some() {
                        tracing::warn!(
                            action = self.meta.action,
                            "Worker console projection owner disappeared after its native terminal"
                        );
                    } else {
                        context.retain_output(shared_worker_runtime_error_plan(
                            crate::conn::RuntimeInspectorError::ConsoleCompletion(self.meta.action),
                        ));
                        self.routed = RuntimeInspectorRoutedOutput::default();
                        return;
                    }
                }
                if let Some(effect) = self.binding_effect.take()
                    && let Err(error) = apply_shared_worker_runtime_binding_effect_after_success(
                        conn,
                        self.meta.session_id(),
                        effect,
                    )
                {
                    tracing::warn!(%error, "Worker binding succeeded before persistence update failed");
                }
                // Claim resources before yielding to binding replay. Closing
                // can now capture these objects while that replay is pending.
                self.routed.register_object_group_for_success(
                    conn,
                    &self.meta.owner_scope,
                    self.meta.object_group.as_deref(),
                );
                if !self.meta.release_object_ids.is_empty() {
                    conn.unregister_runtime_remote_object_ids_for_session_owner(
                        self.meta.session_id(),
                        &self.meta.release_object_ids,
                    );
                }
                if let Some(group) = self.meta.release_object_group.as_deref() {
                    conn.unregister_runtime_remote_object_group_for_session_owner(
                        self.meta.session_id(),
                        group,
                    );
                }
                self.replay = PreparedWorkerBindingReplay::prepare(conn, self.scope.clone());
            }
        }
        std::mem::take(&mut self.routed)
            .push_background_events_before_response_events(&mut plan, self.meta.command_id);
        context.retain_output(plan);
    }

    pub(crate) fn finish_tail(
        mut self,
        conn: &mut CdpConnection,
        context: &mut CommandWorkflowContext,
    ) {
        if self.succeeded && self.meta.action == "enable" && self.scope.can_configure(conn) {
            let mut plan = CommandOutputPlan::default();
            if self.service {
                append_service_worker_runtime_console_messages(
                    conn,
                    self.meta.session_id(),
                    &mut plan,
                );
                append_service_worker_runtime_exception_messages(
                    conn,
                    self.meta.session_id(),
                    &mut plan,
                );
            } else {
                append_shared_worker_runtime_console_messages(
                    conn,
                    self.meta.session_id(),
                    &mut plan,
                );
            }
            context.retain_output(plan);
        }
        self.access.take();
    }
}

impl Drop for WorkerRuntimeContinuation {
    fn drop(&mut self) {
        if let Some(access) = self.access.take()
            && let Some(claimed) = self.claimed.take()
        {
            let scope = self.scope.clone();
            access.defer_worker_runtime_claim_cleanup(scope, claimed);
        }
    }
}

fn retain_error(
    conn: &mut CdpConnection,
    scope: &WorkerRuntimeScope,
    service: bool,
    meta: RuntimeCommandCompletionMeta,
    error: anyhow::Error,
    context: &mut CommandWorkflowContext,
) {
    if !scope.can_configure(conn) {
        if let Some(id) = meta.command_id {
            if conn
                .take_renderer_call_for_frontend_for_session_owner(meta.session_id(), id)
                .is_none()
            {
                return;
            }
            conn.forget_pending_inspector_await(id, meta.session_id());
        }
        context.retain_output(CommandOutputPlan::error(-32000, error));
        return;
    }
    let step = if service {
        complete_service_worker_runtime_inspector_error(conn, meta, error)
    } else {
        complete_shared_worker_runtime_inspector_error(conn, meta, error)
    };
    let RuntimeCommandTaskStep::Complete(plan) = step else {
        unreachable!("Worker error projection is finite");
    };
    context.retain_output(plan);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        CdpCommandTaskStep, CommandDispatchContext, CompletedCdpCommandDispatch,
        OwnedProtocolWaitFailure, PendingCdpCommandDispatch, ProtocolStateRequestReceiver,
    };
    use futures_util::FutureExt as _;

    const SESSION: &str = "SID-owned-worker";

    async fn drive(
        conn: &mut CdpConnection,
        requests: &mut ProtocolStateRequestReceiver,
        pending: PendingCdpCommandDispatch,
    ) -> CompletedCdpCommandDispatch {
        let mut pending = Box::pin(pending.wait());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    completed = &mut pending => return completed,
                    request = requests.recv() => request.unwrap().apply(conn),
                }
            }
        })
        .await
        .unwrap()
    }

    async fn admit(
        conn: &mut CdpConnection,
        context: &mut CommandDispatchContext,
    ) -> PendingCdpCommandDispatch {
        let command = ParsedCdpCommand::parse_str(
            json!({"id":1501,"method":"Runtime.enable","sessionId":SESSION}).to_string(),
        )
        .unwrap();
        let CdpCommandTaskStep::Pending(native) =
            conn.start_parsed_command_dispatch_with_context(&command, context)
        else {
            panic!("Worker dispatch owns its native wait");
        };
        let completed = native.wait().await;
        let CdpCommandTaskStep::Pending(workflow) = conn
            .try_complete_ready_command_dispatch_with_context(completed, context)
            .unwrap_or_else(|_| {
                panic!("Worker completion must enter its owned workflow in a finite actor turn")
            })
        else {
            panic!("Worker completion has an ordinary async workflow");
        };
        *workflow
    }

    #[tokio::test]
    async fn worker_completion_retains_accepted_output_on_cancellation_and_ready_loss() {
        for phase in 0..4 {
            let mut conn = worker_workflow_fixture();
            let mut requests = conn.take_protocol_state_requests().unwrap();
            let mut context = CommandDispatchContext::default();
            let pending = admit(&mut conn, &mut context).await;
            let mut pending = Box::pin(pending.wait());
            if phase == 1 {
                assert!(pending.as_mut().now_or_never().is_none());
                requests.recv().await.unwrap().apply(&mut conn);
            }
            let completed = if phase >= 2 {
                let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        tokio::select! {
                            completed = &mut pending => return completed,
                            request = requests.recv() => request.unwrap().apply(&mut conn),
                        }
                    }
                })
                .await
                .unwrap();
                if phase == 3 {
                    Some(completed)
                } else {
                    drop(completed);
                    None
                }
            } else {
                drop(pending);
                None
            };
            let completed = match completed {
                Some(completed) => completed,
                None => {
                    let CdpCommandTaskStep::Pending(recovery) = conn
                        .complete_failed_command_wait_with_context(
                            Some(1501),
                            Some(SESSION),
                            OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut context,
                        )
                    else {
                        panic!("the original command resumes its workflow");
                    };
                    drive(&mut conn, &mut requests, *recovery).await
                }
            };
            let CdpCommandTaskStep::Complete(outcome) = conn
                .try_complete_ready_command_dispatch_with_context(completed, &mut context)
                .unwrap_or_else(|_| panic!("final Worker workflow completion is finite"))
            else {
                panic!("one workflow owns all Worker waits");
            };
            let (events, _, _) = outcome.into_protocol_event_parts();
            let messages = events
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message)
                .collect::<Vec<_>>();
            let replies = messages
                .iter()
                .filter(|message| message["id"] == 1501)
                .collect::<Vec<_>>();
            assert_eq!(replies.len(), 1, "{messages:?}");
            assert_eq!(replies[0]["result"], json!({}), "{messages:?}");
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Runtime.executionContextCreated")
                    .count(),
                1,
                "{messages:?}"
            );
            assert!(
                conn.shared_worker_target_for_session(Some(SESSION))
                    .unwrap()
                    .runtime_frontend_enabled(SESSION)
            );
        }
    }

    #[tokio::test]
    async fn closing_worker_cannot_be_reenabled_by_a_late_unavailable_runtime_fallback() {
        let mut conn = worker_workflow_fixture();
        let mut requests = conn.take_protocol_state_requests().unwrap();
        let mut context = CommandDispatchContext::default();
        let pending = admit(&mut conn, &mut context).await;
        let route = conn.session_route(Some(SESSION)).unwrap();
        let disposal =
            crate::conn::SessionDisposalPlan::for_session_route(SESSION, &route).unwrap();
        conn.mark_session_closing(&disposal);
        let completed = drive(&mut conn, &mut requests, pending).await;
        let CdpCommandTaskStep::Complete(outcome) = conn
            .try_complete_ready_command_dispatch_with_context(completed, &mut context)
            .unwrap_or_else(|_| panic!("finite Closing completion"))
        else {
            panic!("all waits are owned");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        assert!(
            !conn
                .shared_worker_target_for_session(Some(SESSION))
                .unwrap()
                .runtime_frontend_enabled(SESSION)
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Runtime.executionContextCreated")
                .count(),
            0,
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .find(|message| message["id"] == 1501)
                .unwrap()
                .get("error")
                .is_some(),
            "{messages:?}"
        );
    }
}
