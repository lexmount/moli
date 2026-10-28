use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{
    CommandDispatchContext, CommandWorkflowContext, PreparedCommandWorkflow, ProtocolStateAccess,
    ProtocolWorkflowDriver, RuntimeInspectorError,
};

pub(crate) enum PageRuntimeCompletion {
    Step(RuntimeCommandTaskStep),
    Wait(Box<PreparedPageRuntimeCommand>),
}

impl From<RuntimeCommandTaskStep> for PageRuntimeCompletion {
    fn from(step: RuntimeCommandTaskStep) -> Self {
        Self::Step(step)
    }
}

pub(crate) enum PreparedPageRuntimeCommand {
    Response {
        continuation: PageRuntimeCommandContinuation,
        receiver: RuntimeInspectorAsyncCompletionReceiver,
    },
    Bindings {
        continuation: PageRuntimeCommandContinuation,
        pending: PendingRuntimeBindingPageCommandDispatch,
    },
}

pub(crate) struct PageRuntimeCommandContinuation {
    pub(super) host_scope: Option<crate::conn::ProtocolPageHostScope>,
    pub(super) completed: RuntimeCommandCompletionMeta,
    pub(super) routed_output: RuntimeInspectorRoutedOutput,
    pub(super) session_response_succeeded: Option<bool>,
    pub(super) kind: PageRuntimeContinuationKind,
}

pub(super) enum PageRuntimeContinuationKind {
    Command {
        timing_started: Option<std::time::Instant>,
    },
    Binding(RuntimeBindingCommandTask),
}

impl PageRuntimeCompletion {
    /// Accepted output stays with the original command before a state reply
    /// can be discarded or its next renderer wait can be cancelled.
    pub(crate) fn retain(self, context: &mut CommandWorkflowContext) -> Self {
        match self {
            Self::Step(RuntimeCommandTaskStep::Complete(plan)) => {
                context.retain_output(plan);
                RuntimeCommandTaskStep::Complete(CommandOutputPlan::default()).into()
            }
            Self::Wait(mut pending) => {
                pending.retain_ready_events(context.context_mut());
                Self::Wait(pending)
            }
            step => step,
        }
    }
}

impl PreparedPageRuntimeCommand {
    fn retain_ready_events(&mut self, context: &mut CommandDispatchContext) {
        let continuation = match self {
            Self::Response { continuation, .. } | Self::Bindings { continuation, .. } => {
                continuation
            }
        };
        if let Some(predecessor) = continuation
            .routed_output
            .renderer_output_predecessor
            .take()
        {
            context.set_renderer_output_predecessor(predecessor);
        }
        if matches!(continuation.kind, PageRuntimeContinuationKind::Binding(_)) {
            return;
        }
        context.protocol_events_mut().extend(
            continuation
                .routed_output
                .take_ready_background_events_for_command(continuation.completed.command_id),
        );
    }

    pub(crate) fn into_workflow(
        mut self,
        access: ProtocolStateAccess,
        context: &mut CommandDispatchContext,
    ) -> PreparedCommandWorkflow {
        self.retain_ready_events(context);
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
        let continuation = match &self {
            Self::Response { continuation, .. } | Self::Bindings { continuation, .. } => {
                continuation
            }
        };
        let id = continuation.completed.command_id;
        let session = continuation.completed.session_id().map(str::to_owned);
        let mut local = CommandDispatchContext::new(flush.clone());
        let (mut context, _scope) = CommandWorkflowContext::take_from(&mut local, id, session);
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

    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let mut step = PageRuntimeCompletion::Wait(Box::new(self));
        loop {
            step = match step {
                PageRuntimeCompletion::Step(RuntimeCommandTaskStep::Complete(plan)) => {
                    context.retain_output(plan);
                    return Ok(());
                }
                PageRuntimeCompletion::Step(RuntimeCommandTaskStep::Pending(pending)) => {
                    // Deferred Inspector replies return to the original scheduler
                    // at preparation. Only finite binding followups enter here.
                    assert!(!pending.waits_for_scheduler_deferred_inspector_reply());
                    let completed = pending.wait().await;
                    context
                        .with_state(driver, move |state, context| {
                            state.prepare_page_runtime_completion(completed, context)
                        })
                        .await?
                }
                PageRuntimeCompletion::Wait(pending) => match *pending {
                    Self::Response {
                        continuation,
                        receiver,
                    } => {
                        let response = receiver
                            .await
                            .context(RuntimeInspectorError::ResponseCanceled);
                        context
                            .with_state(driver, move |state, context| {
                                state.finish_page_runtime_response(continuation, response, context)
                            })
                            .await?
                    }
                    Self::Bindings {
                        continuation,
                        pending,
                    } => {
                        let completed = pending.wait().await;
                        context
                            .with_state(driver, move |state, context| {
                                state.finish_page_runtime_binding_state(
                                    continuation,
                                    completed,
                                    context,
                                )
                            })
                            .await?
                    }
                },
            };
        }
    }
}

impl PageRuntimeCommandContinuation {
    fn owns_current_host(&self, conn: &CdpConnection) -> bool {
        self.host_scope
            .as_ref()
            .is_none_or(|scope| scope.is_current(conn))
    }

    pub(super) fn finish_retired(self) -> PageRuntimeCompletion {
        let mut plan = if self.session_response_succeeded.is_none()
            && !self.routed_output.events().iter().any(|event| {
                event.protocol_message_id() == self.completed.command_id
                    && self.completed.command_id.is_some()
            }) {
            runtime_inspector_error_plan(self.completed.command_id, crate::conn::NoDocumentLoaded)
        } else {
            CommandOutputPlan::default()
        };
        self.routed_output
            .push_ordered_into_plan(&mut plan, self.completed.command_id);
        RuntimeCommandTaskStep::Complete(plan).into()
    }

    pub(crate) fn finish_response(
        mut self,
        conn: &mut CdpConnection,
        response: Result<moli_core::RendererRuntimeInspectorAsyncCompletion>,
    ) -> PageRuntimeCompletion {
        if !self.owns_current_host(conn) {
            if let Ok(response) = response
                && let Some(fence) = response.renderer_output_predecessor()
            {
                self.routed_output.set_renderer_output_predecessor(fence);
            }
            return self.finish_retired();
        }
        if let Some(id) = self.completed.command_id {
            let response = RuntimeInspectorResponseReady::for_owner(
                id,
                &self.completed.owner_scope,
                response.map_err(|error| error.to_string()),
            );
            if let Some(response) = conn.resolve_runtime_inspector_response_ready(response) {
                let (_, output, predecessor) = response.into_renderer_command_output();
                if let Some(predecessor) = predecessor {
                    self.routed_output
                        .set_renderer_output_predecessor(predecessor);
                }
                route_runtime_command_output_into_routed_output(
                    conn,
                    output,
                    Some(id),
                    &self.completed.owner_scope,
                    &mut self.routed_output,
                );
            }
        }
        self.finish_routed(conn)
    }

    pub(super) fn finish_routed(self, conn: &mut CdpConnection) -> PageRuntimeCompletion {
        if !self.owns_current_host(conn) {
            return self.finish_retired();
        }
        let Self {
            host_scope,
            completed,
            routed_output,
            session_response_succeeded,
            kind,
        } = self;
        let timing_started = match kind {
            PageRuntimeContinuationKind::Command { timing_started } => timing_started,
            PageRuntimeContinuationKind::Binding(mut task) => {
                if task.response_delivery == RendererInspectorResponseDelivery::AdapterReply {
                    record_runtime_binding_command_response_from_routed_events(
                        &mut task,
                        routed_output.events(),
                        completed.command_id,
                    );
                }
                if routed_output.background_event_count() > 0 {
                    tracing::debug!(
                        events = routed_output.background_event_count(),
                        "dropping unexpected Runtime binding background events during inspector phase"
                    );
                }
                if !runtime_binding_command_response_succeeded(&task) {
                    return RuntimeCommandTaskStep::Complete(runtime_binding_output_plan(task))
                        .into();
                }
                if task.skip_live_page_update_after_inspector_success {
                    return complete_runtime_binding_after_live_update(conn, completed, task)
                        .into();
                }
                return match start_pending_runtime_binding_page_phase(
                    conn,
                    completed.command_id,
                    task.clone(),
                    completed.owner_scope.clone(),
                ) {
                    Some(pending) => RuntimeCommandTaskStep::Pending(Box::new(pending)),
                    None => complete_runtime_binding_after_live_update(conn, completed, task),
                }
                .into();
            }
        };
        let succeeded = session_response_succeeded
            .unwrap_or_else(|| routed_output.command_response_succeeded(completed.command_id));
        if succeeded {
            routed_output.register_object_group_for_success(
                conn,
                &completed.owner_scope,
                completed.object_group.as_deref(),
            );
            if let Some(action) = console_action_from_protocol_method(completed.action)
                && !apply_console_output_state_for_owner(conn, &completed.owner_scope, action)
            {
                if session_response_succeeded.is_none() {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        completed.command_id,
                        RuntimeInspectorError::ConsoleCompletion(completed.action),
                    ))
                    .into();
                }
                tracing::warn!(
                    command_id = completed.command_id,
                    session_id = completed.session_id(),
                    action = completed.action,
                    "could not apply console projection after the terminal renderer response was published"
                );
            }
            if completed.action == "discardConsoleEntries" {
                advance_runtime_observable_cursors_to_current_for_owner(
                    conn,
                    &completed.owner_scope,
                );
            }
            if completed.action == "disable" {
                let pending = apply_runtime_disable_projection_after_success_for_owner(
                    conn,
                    &completed.owner_scope,
                )
                .map_err(anyhow::Error::msg)
                .and_then(|()| {
                    conn.start_apply_stored_runtime_bindings_for_owner(&completed.owner_scope)
                });
                match pending {
                    Ok(pending) => {
                        return PageRuntimeCompletion::Wait(Box::new(
                            PreparedPageRuntimeCommand::Bindings {
                                continuation: Self {
                                    host_scope,
                                    completed,
                                    routed_output,
                                    session_response_succeeded,
                                    kind: PageRuntimeContinuationKind::Command { timing_started },
                                },
                                pending,
                            },
                        ));
                    }
                    Err(error) => {
                        if session_response_succeeded.is_none() {
                            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                                completed.command_id,
                                error,
                            ))
                            .into();
                        }
                        tracing::warn!(%error, "could not apply Runtime disable projection after the terminal renderer response was published");
                    }
                }
            }
        }
        RuntimeCommandTaskStep::Complete(finish_runtime_inspector_tail(
            conn,
            completed,
            routed_output,
            succeeded,
            timing_started,
        ))
        .into()
    }

    pub(crate) fn finish_binding_state(
        self,
        conn: &mut CdpConnection,
        result: Result<CompletedRuntimeBindingPageCommandDispatch>,
    ) -> PageRuntimeCompletion {
        if !self.owns_current_host(conn) {
            return self.finish_retired();
        }
        let PageRuntimeContinuationKind::Command { timing_started } = self.kind else {
            unreachable!("only Runtime.disable waits for its stored binding state");
        };
        if let Err(error) =
            result.and_then(|completed| conn.complete_runtime_binding_page_command(completed))
        {
            if self.session_response_succeeded.is_none() {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    self.completed.command_id,
                    error,
                ))
                .into();
            }
            tracing::warn!(%error, "could not apply Runtime bindings after the terminal renderer response was published");
        }
        RuntimeCommandTaskStep::Complete(finish_runtime_inspector_tail(
            conn,
            self.completed,
            self.routed_output,
            true,
            timing_started,
        ))
        .into()
    }
}

fn finish_runtime_inspector_tail(
    conn: &mut CdpConnection,
    completed: RuntimeCommandCompletionMeta,
    routed_output: RuntimeInspectorRoutedOutput,
    succeeded: bool,
    timing_started: Option<std::time::Instant>,
) -> CommandOutputPlan {
    if succeeded {
        if !completed.release_object_ids.is_empty() {
            conn.unregister_runtime_remote_object_ids_for_owner(
                &completed.owner_scope,
                &completed.release_object_ids,
            );
        }
        if let Some(object_group) = completed.release_object_group.as_deref() {
            conn.unregister_runtime_remote_object_group_for_owner(
                &completed.owner_scope,
                object_group,
            );
        }
        if completed.action == "runIfWaitingForDebugger"
            && conn.release_waiting_for_debugger_session(completed.session_id())
        {
            crate::domains::target::schedule_initial_document_target_url_navigation_after_debugger_resume(
                conn,
                completed.session_id(),
            );
        }
    }
    let mut plan = CommandOutputPlan::default();
    routed_output.push_ordered_into_plan(&mut plan, completed.command_id);
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            action = completed.action,
            stage = "runtime_inspector_plan_ready",
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{CdpCommandTaskStep, ProtocolStateOwner};
    use crate::testing::TestContext;
    use futures_util::FutureExt as _;

    struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[tokio::test]
    async fn page_runtime_response_preserves_original_call_when_waiter_or_ready_reply_is_lost() {
        for lose_ready in [false, true] {
            let mut conn = CdpConnection::new();
            let mut browser = crate::conn::BrowserContext::new("BID-runtime-response".into());
            browser.set_active_target_id("TID-runtime-response");
            browser.attach_active_session("SID-runtime-response");
            conn.install_browser_context_fixture_for_test(browser);
            let owner = CommandOwnerScope::for_session("SID-runtime-response");
            let registration = conn
                .try_register_renderer_call_for_session_owner(
                    Some("SID-runtime-response"),
                    1801,
                    None,
                    RendererCommandDescriptor::for_document_helper(
                        json!({
                            "id":1801,"method":"Runtime.evaluate","params":{"expression":"42"},
                        })
                        .to_string(),
                    )
                    .unwrap(),
                )
                .unwrap();
            let call_id = registration.correlation().renderer_call_id().get();
            let (send, receiver) = tokio::sync::oneshot::channel();
            let mut send = Some(send);
            let mut requests = conn.take_protocol_state_requests().unwrap();
            let mut dispatch = CommandDispatchContext::default();
            let mut routed_output = RuntimeInspectorRoutedOutput::default();
            routed_output.append_ordered_events(vec![BackgroundProtocolEvent::immediate(
                json!({"method":"Runtime.consoleAPICalled","params":{"type":"log","args":[]}}),
            )]);
            let prepared = PreparedPageRuntimeCommand::Response {
                continuation: PageRuntimeCommandContinuation {
                    host_scope: None,
                    completed: RuntimeCommandCompletionMeta {
                        command_id: Some(1801),
                        action: "evaluate",
                        owner_scope: owner.clone(),
                        object_group: None,
                        release_object_ids: Vec::new(),
                        release_object_group: None,
                        await_promise: false,
                        wait_for_deferred_reply: false,
                    },
                    routed_output,
                    session_response_succeeded: None,
                    kind: PageRuntimeContinuationKind::Command {
                        timing_started: None,
                    },
                },
                receiver,
            };
            let workflow =
                prepared.into_workflow(conn.protocol_state_access().unwrap(), &mut dispatch);
            let CdpCommandTaskStep::Pending(pending) = conn.start_command_workflow(
                workflow,
                Some(1801),
                Some("SID-runtime-response"),
                &mut dispatch,
            ) else {
                panic!("the reply is still explicitly held");
            };
            let mut waiting = Box::pin(pending.wait());
            assert!(waiting.as_mut().now_or_never().is_none());
            let peer = ParsedCdpCommand::parse_str(r#"{"id":1802,"method":"Browser.getVersion"}"#)
                .unwrap();
            assert!(matches!(
                conn.start_parsed_command_dispatch_with_context(
                    &peer,
                    &mut CommandDispatchContext::default(),
                ),
                CdpCommandTaskStep::Complete(_)
            ));
            if lose_ready {
                send.take()
                    .unwrap()
                    .send(
                        moli_core::RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                            call_id,
                            json!({"id":call_id,"result":{"result":{"type":"number","value":42}}}),
                        ),
                    )
                    .unwrap();
                assert!(waiting.as_mut().now_or_never().is_none());
                requests.recv().await.unwrap().apply(&mut conn);
                // The finite operation accepted the actual terminal, but its
                // caller has not polled the ready state reply.
            }
            drop(waiting);
            let CdpCommandTaskStep::Pending(recovery) = conn
                .complete_failed_command_wait_with_context(
                    Some(1801),
                    Some("SID-runtime-response"),
                    crate::OwnedProtocolWaitFailure::Cancelled.message(),
                    &mut dispatch,
                )
            else {
                panic!("the same command rejoins its admitted response wait");
            };
            let mut waiting = Box::pin(recovery.wait());
            if !lose_ready {
                assert!(waiting.as_mut().now_or_never().is_none());
                send.take()
                    .unwrap()
                    .send(
                        moli_core::RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                            call_id,
                            json!({"id":call_id,"result":{"result":{"type":"number","value":42}}}),
                        ),
                    )
                    .unwrap();
            }
            let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    tokio::select! {
                        completed = &mut waiting => return completed,
                        request = requests.recv() => request.unwrap().apply(&mut conn),
                    }
                }
            })
            .await
            .unwrap();
            let CdpCommandTaskStep::Complete(outcome) = conn
                .try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
                .unwrap_or_else(|_| panic!("the response handoff is finite"))
            else {
                panic!("the original response already settled");
            };
            let (events, _, _) = outcome.into_protocol_event_parts();
            let messages = events
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message)
                .collect::<Vec<_>>();
            assert_eq!(messages.len(), 2, "{messages:?}");
            assert_eq!(messages[0]["method"], "Runtime.consoleAPICalled");
            assert_eq!(
                messages[1],
                json!({"id":1801,"sessionId":"SID-runtime-response","result":{"result":{"type":"number","value":42}}})
            );
            assert!(
                conn.renderer_runtime_command_cause_for_owner(&owner, 1801)
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn page_runtime_disable_owns_binding_wait_and_ready_loss_recovery() {
        for phase in 0..3 {
            let mut fixture = TestContext::new();
            let mut browser = crate::conn::BrowserContext::new("BID-runtime-completion".into());
            browser.set_active_target_id("TID-runtime-completion");
            browser.attach_active_session("SID-runtime-completion");
            fixture
                .conn
                .install_browser_context_fixture_for_test(browser);
            fixture
                .install_navigation_fixture_for_session_owner(
                    "about:blank",
                    Some("SID-runtime-completion"),
                )
                .await;
            for (id, method, params) in [
                (1700, "Runtime.enable", json!({})),
                (1701, "Runtime.addBinding", json!({"name":"ownedBinding"})),
            ] {
                fixture.process_async(json!({
                    "id":id,"sessionId":"SID-runtime-completion","method":method,"params":params,
                })).await;
            }
            let command = ParsedCdpCommand::parse_str(
                json!({
                    "id":1702,"sessionId":"SID-runtime-completion","method":"Runtime.disable",
                })
                .to_string(),
            )
            .unwrap();
            let mut dispatch = CommandDispatchContext::default();
            dispatch.set_terminal_response_delivery_override(
                RendererInspectorResponseDelivery::AdapterReply,
            );
            let CdpCommandTaskStep::Pending(pending) = fixture
                .conn
                .start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            else {
                panic!("adapter Runtime.disable enters its Inspector dispatch");
            };
            let completed = pending.wait().await;

            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, held) = std::sync::mpsc::channel();
            let (gate, gate_guard) = fixture
                .conn
                .loaded_page_mut_for_protocol_access(Some("SID-runtime-completion"))
                .unwrap()
                .start_cdp_call(
                    1799,
                    None,
                    moli_core::RendererNativeOperation::then(
                        moli_core::RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            moli_core::RendererNativeOperationStep::Complete(
                                moli_core::RendererNativeProtocolResponse::success(json!({})),
                            )
                        },
                    ),
                )
                .unwrap();
            let release = ReleaseOnDrop(Some(release));
            tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .unwrap();
            drop(fixture.conn.take_protocol_state_requests());
            let owner = ProtocolStateOwner::new(fixture.conn);
            let handle = owner.handle();
            let CdpCommandTaskStep::Pending(workflow) = handle
                .with_state(|state| {
                    state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
                })
                .unwrap()
                .unwrap_or_else(|_| panic!("Runtime completion must leave the owner turn"))
            else {
                panic!("the binding state update still waits behind the held Native command");
            };
            assert_eq!(workflow.kind_name(), "Workflow");
            let mut waiting = Box::pin(workflow.wait());
            assert!(waiting.as_mut().now_or_never().is_none());
            let peer = ParsedCdpCommand::parse_str(r#"{"id":1703,"method":"Browser.getVersion"}"#)
                .unwrap();
            assert!(matches!(
                handle
                    .with_state(|state| {
                        state.start_parsed_command_dispatch_with_context(
                            &peer,
                            &mut CommandDispatchContext::default(),
                        )
                    })
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
            if phase == 1 {
                drop(waiting);
                let CdpCommandTaskStep::Pending(recovery) = handle
                    .with_state(|state| {
                        state.complete_failed_command_wait_with_context(
                            Some(1702),
                            Some("SID-runtime-completion"),
                            crate::OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut dispatch,
                        )
                    })
                    .unwrap()
                else {
                    panic!("the original workflow retains the admitted binding update");
                };
                waiting = Box::pin(recovery.wait());
                assert!(waiting.as_mut().now_or_never().is_none());
            }
            drop(release);
            gate.wait().await.unwrap();
            drop(gate_guard);
            let completed = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
                .await
                .unwrap();
            let completed = if phase == 2 {
                drop(completed);
                let CdpCommandTaskStep::Pending(recovery) = handle
                    .with_state(|state| {
                        state.complete_failed_command_wait_with_context(
                            Some(1702),
                            Some("SID-runtime-completion"),
                            crate::OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut dispatch,
                        )
                    })
                    .unwrap()
                else {
                    panic!("discarding the ready result rejoins the original workflow");
                };
                recovery.wait().await
            } else {
                completed
            };
            let CdpCommandTaskStep::Complete(outcome) = handle
                .with_state(|state| {
                    state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
                })
                .unwrap()
                .unwrap_or_else(|_| panic!("final Runtime handoff is finite"))
            else {
                panic!("all renderer waits have settled");
            };
            let (events, _, _) = outcome.into_protocol_event_parts();
            let messages = events
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message)
                .collect::<Vec<_>>();
            let replies = messages
                .iter()
                .filter(|message| message["id"] == 1702)
                .collect::<Vec<_>>();
            assert_eq!(
                replies,
                vec![&json!({"id":1702,"sessionId":"SID-runtime-completion","result":{}})],
                "{messages:?}"
            );
        }
    }
}
