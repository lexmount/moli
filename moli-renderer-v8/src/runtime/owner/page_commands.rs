use super::*;

impl RendererOwnerHandle {
    pub(super) async fn run_live_page_command_turn(
        &self,
        token: RendererPageToken,
        command: RendererPageCommand,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        match command {
            RendererPageCommand::EvaluateExpression {
                expression,
                await_promise: true,
            } => self.begin_live_page_runtime_expression_await(
                token,
                RuntimeExpressionAwaitSpec {
                    execution_context_id: None,
                    expression,
                    result_mode: RuntimeEvaluateResultMode::RemoteObject,
                    navigation_policy: RuntimeEvaluationNavigationPolicy::DoNotFollow,
                },
                capture_policy,
            ),
            RendererPageCommand::EvaluateExpressionByValue { expression } => self
                .begin_live_page_runtime_expression_await(
                    token,
                    RuntimeExpressionAwaitSpec {
                        execution_context_id: None,
                        expression,
                        result_mode: RuntimeEvaluateResultMode::ByValue,
                        navigation_policy: RuntimeEvaluationNavigationPolicy::DoNotFollow,
                    },
                    capture_policy,
                ),
            RendererPageCommand::EvaluateExpressionAndFollowPendingNavigation {
                expression,
                await_promise: true,
            } => self.begin_live_page_runtime_expression_await(
                token,
                RuntimeExpressionAwaitSpec {
                    execution_context_id: None,
                    expression,
                    result_mode: RuntimeEvaluateResultMode::RemoteObject,
                    navigation_policy: RuntimeEvaluationNavigationPolicy::Follow,
                },
                capture_policy,
            ),
            RendererPageCommand::EvaluateExpressionInExecutionContext {
                execution_context_id,
                expression,
                await_promise: true,
            } => self.begin_live_page_runtime_expression_await(
                token,
                RuntimeExpressionAwaitSpec {
                    execution_context_id: Some(execution_context_id),
                    expression,
                    result_mode: RuntimeEvaluateResultMode::RemoteObject,
                    navigation_policy: RuntimeEvaluationNavigationPolicy::DoNotFollow,
                },
                capture_policy,
            ),
            RendererPageCommand::EvaluateExpressionInExecutionContextAndFollowPendingNavigation {
                execution_context_id,
                expression,
                await_promise: true,
            } => self.begin_live_page_runtime_expression_await(
                token,
                RuntimeExpressionAwaitSpec {
                    execution_context_id: Some(execution_context_id),
                    expression,
                    result_mode: RuntimeEvaluateResultMode::RemoteObject,
                    navigation_policy: RuntimeEvaluationNavigationPolicy::Follow,
                },
                capture_policy,
            ),
            RendererPageCommand::WaitForSelector {
                selector,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(timeout_ms, "selector") {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSelector {
                        token,
                        selector,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::WaitForScriptTruthy {
                expression,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(timeout_ms, "script truthy") {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageScriptTruthy {
                        token,
                        expression,
                        pending_call: None,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::WaitForSubresourceResponse {
                criteria,
                timeout_ms,
                loader,
            } => {
                let deadline =
                    match checked_live_page_wait_deadline(timeout_ms, "subresource response") {
                        Ok(deadline) => deadline,
                        Err(error) => return Err(error).into(),
                    };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                        token,
                        criteria,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort {
                timeout_ms,
                loader: _,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    timeout_ms,
                    "child-frame lifecycle best-effort observation",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageChildFrameLifecycle {
                        token,
                        deadline,
                        capture_policy,
                    },
                ))
            }
            command => {
                return self
                    .run_live_page_command_turn_inline(token, command, capture_policy)
                    .await;
            }
        }
    }

    pub(super) fn begin_live_page_runtime_expression_await(
        &self,
        token: RendererPageToken,
        spec: RuntimeExpressionAwaitSpec,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let deadline = match checked_live_page_wait_deadline(
            LIVE_PAGE_RUNTIME_EXPRESSION_AWAIT_TIMEOUT_MS,
            "runtime expression awaitPromise",
        ) {
            Ok(deadline) => deadline,
            Err(error) => return Err(error).into(),
        };
        RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
            RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                token,
                execution_context_id: spec.execution_context_id,
                expression: spec.expression,
                pending_call: None,
                deadline,
                result_mode: spec.result_mode,
                navigation_policy: spec.navigation_policy,
                capture_policy,
            },
        ))
    }

    pub(super) async fn run_live_page_command_turn_inline(
        &self,
        token: RendererPageToken,
        command: RendererPageCommand,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        if live_page_command_requires_materialized_child_realms(&command)
            && entry
                .page_vm()
                .vm()
                .has_pending_child_frame_realm_materialization()
        {
            // V8's Runtime.enable calls beginEnsureAllContextsInGroup() and
            // then reportAllContexts(). Chromium already has each live local
            // frame's ScriptState by this boundary. Moli creates child
            // realms lazily, so an earlier exact-Document materialization
            // task is a real prerequisite of that report, not an event to be
            // reconstructed from a later realm snapshot.
            //
            // Return the Page entry and let the ordinary scheduler consume
            // its typed child-frame task. The command resumes only after a
            // Page turn; this preserves the task's authorization, checkpoint
            // and concrete-output ownership instead of running its body from
            // the protocol command or manufacturing a second producer.
            self.restore_live_page_entry(token, entry);
            return RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                turn: Box::new(RenderRuntimeTurn::RunLivePageCommand {
                    token,
                    command,
                    capture_policy,
                }),
                wake_token: token,
            };
        }
        let timing_command_label = renderer_page_command_timing_label(&command);
        let timing_started = timing_command_label.and_then(|command_label| {
            moli_trace::cdp_nav_timing_enabled().then(|| {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    command = command_label,
                    page_id = token.page_id.as_u64(),
                    stage = "owner_command_turn_start",
                );
                Instant::now()
            })
        });
        let should_follow_pending_navigation =
            live_page_command_should_follow_pending_navigation(&command);
        let scope_before_dispatch = entry.page_vm().pending_runtime_command_output_scope_id();
        let (mut entry, reply_result) = dispatch_async_command_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            command,
        )
        .await;
        if let (Some(command_label), Some(started)) = (timing_command_label, timing_started) {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                page_id = token.page_id.as_u64(),
                elapsed_ms = started.elapsed().as_millis(),
                stage = "owner_command_vm_dispatch_done",
            );
        }
        let _ = entry.slot.cancel_in_flight_command(token.page_id);
        let owned_runtime_command_scope = runtime_command_output_scope_owned_by_dispatch(
            scope_before_dispatch,
            entry.page_vm().pending_runtime_command_output_scope_id(),
        );
        let RendererPageCommandDispatch {
            reply,
            replacement_lifecycle,
            turn_records,
        } = match reply_result {
            Ok(dispatch) => dispatch,
            Err(error) => {
                if let Some(scope_id) = owned_runtime_command_scope {
                    entry
                        .page_vm_mut()
                        .abandon_pending_runtime_command_lifecycle(scope_id);
                }
                self.restore_live_page_entry(token, entry);
                return Err(error).into();
            }
        };
        if let Some(scope_id) = owned_runtime_command_scope {
            let has_pending_document_lifecycle_turn =
                has_pending_document_lifecycle_turn_on_entry(&mut entry);
            self.restore_live_page_entry(token, entry);
            if has_pending_document_lifecycle_turn {
                self.signal_internal_document_lifecycle_turn(token);
            }
            return RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                turn: Box::new(RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                    token,
                    scope_id,
                    reply: Box::new(reply),
                    should_follow_pending_navigation,
                    turn_records,
                    capture_policy,
                }),
                wake_token: token,
            };
        }
        self.complete_live_page_command_turn(
            token,
            entry,
            reply,
            should_follow_pending_navigation,
            turn_records,
            replacement_lifecycle,
            capture_policy,
        )
        .await
    }

    pub(super) async fn continue_live_page_runtime_command_lifecycle_turn(
        &self,
        token: RendererPageToken,
        scope_id: PageVmRuntimeCommandOutputScopeId,
        reply: RendererPageReply,
        should_follow_pending_navigation: bool,
        turn_records: Vec<PendingRendererOutputRecord>,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let (entry, advance_result) = advance_runtime_command_lifecycle_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            scope_id,
        )
        .await;
        match advance_result {
            Ok(PageVmRuntimeCommandLifecycleAdvance::Pending) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                        token,
                        scope_id,
                        reply: Box::new(reply),
                        should_follow_pending_navigation,
                        turn_records,
                        capture_policy,
                    }),
                    wake_token: token,
                }
            }
            Ok(PageVmRuntimeCommandLifecycleAdvance::Completed) => {
                self.complete_live_page_command_turn(
                    token,
                    entry,
                    reply,
                    should_follow_pending_navigation,
                    turn_records,
                    None,
                    capture_policy,
                )
                .await
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn complete_live_page_command_turn(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        reply: RendererPageReply,
        should_follow_pending_navigation: bool,
        turn_records: Vec<PendingRendererOutputRecord>,
        replacement_lifecycle: Option<DocumentLifecycleTurnOutcome>,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        if should_follow_pending_navigation
            && entry.page_vm().vm().has_pending_location_navigation()
        {
            debug_assert!(
                turn_records.is_empty(),
                "commands that follow a pending navigation cannot drop command-turn output records"
            );
            return self.continue_live_page_pending_navigation(
                token,
                entry,
                PageVmInitStage::Load,
                0,
                LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                    reply: Box::new(reply),
                    capture_policy,
                },
            );
        }
        let deferred_document = match replacement_lifecycle.map(|outcome| outcome.readiness) {
            Some(
                DocumentLifecycleTurnReadiness::Runnable { document }
                | DocumentLifecycleTurnReadiness::Blocked { document },
            ) => {
                if let Err(error) = entry.defer_document_lifecycle_until_response(document) {
                    self.restore_live_page_entry(token, entry);
                    return Err(error).into();
                }
                Some(document)
            }
            Some(DocumentLifecycleTurnReadiness::Idle) | None => None,
        };
        let post_response_work = RendererPostResponseOwnerWork {
            document_lifecycle: deferred_document,
            navigation_handoff: entry.pending_javascript_navigation_handoff(),
        };
        let post_response_continuation = (!post_response_work.is_empty())
            .then(|| self.post_response_owner_work_continuation(token, post_response_work));
        self.finish_live_page_entry_with_page_state_and_continuation(
            token,
            entry,
            reply,
            turn_records,
            post_response_continuation,
            capture_policy,
        )
        .await
    }

    pub(super) async fn wait_live_page_network_idle_turn(
        &self,
        token: RendererPageToken,
        state: PageVmNetworkIdleWaitState,
        deadline: Instant,
        loader: ResourceRequestClient,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for networkidle")).into();
        }
        let (entry, wait_result) = advance_network_idle_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            state,
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmNetworkIdleWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                )
                .await
            }
            Ok(PageVmNetworkIdleWaitAdvance::TriggeredNavigation) => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    PageVmInitStage::Load,
                    0,
                    LivePagePendingNavigationCompletion::ContinueNetworkIdle { deadline, loader },
                ),
            Ok(PageVmNetworkIdleWaitAdvance::Progressed { state }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageNetworkIdle {
                        token,
                        state,
                        deadline,
                        loader,
                    },
                ))
            }
            Ok(PageVmNetworkIdleWaitAdvance::Waiting { sleep_for, state }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for networkidle")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageNetworkIdle {
                            token,
                            state,
                            deadline,
                            loader,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn wait_live_page_dom_stable_turn(
        &self,
        token: RendererPageToken,
        state: PageVmDomStableWaitState,
        deadline: Instant,
        loader: ResourceRequestClient,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for domstable")).into();
        }
        let (entry, wait_result) = advance_dom_stable_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            state,
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmDomStableWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                )
                .await
            }
            Ok(PageVmDomStableWaitAdvance::TriggeredNavigation) => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    PageVmInitStage::Load,
                    0,
                    LivePagePendingNavigationCompletion::ContinueDomStable { deadline, loader },
                ),
            Ok(PageVmDomStableWaitAdvance::Progressed { state }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageDomStable {
                        token,
                        state,
                        deadline,
                        loader,
                    },
                ))
            }
            Ok(PageVmDomStableWaitAdvance::Waiting { sleep_for, state }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for domstable")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageDomStable {
                            token,
                            state,
                            deadline,
                            loader,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn wait_live_page_selector_turn(
        &self,
        token: RendererPageToken,
        selector: String,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for selector `{selector}`")).into();
        }
        let (entry, wait_result) = advance_selector_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            selector.clone(),
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmCommandWaitAdvance::Completed { node }) => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::DocumentQuerySelectorNode(node),
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            Ok(PageVmCommandWaitAdvance::Progressed) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSelector {
                        token,
                        selector,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmCommandWaitAdvance::Waiting { sleep_for }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for selector `{selector}`")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageSelector {
                            token,
                            selector,
                            deadline,
                            loader,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn wait_live_page_child_frame_lifecycle_turn(
        &self,
        token: RendererPageToken,
        deadline: Instant,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        if entry.page_vm().child_frame_lifecycle_work_is_complete() {
            return self
                .finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Bool(true),
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await;
        }
        if Instant::now() >= deadline {
            return self
                .finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Bool(false),
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await;
        }
        self.restore_live_page_entry(token, entry);
        RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
            turn: Box::new(RenderRuntimeTurn::WaitLivePageChildFrameLifecycle {
                token,
                deadline,
                capture_policy,
            }),
            wake_token: token,
            ready_at: deadline,
        }
    }

    pub(super) async fn wait_live_page_script_truthy_turn(
        &self,
        token: RendererPageToken,
        expression: String,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            entry
                .page_vm_mut()
                .cancel_pending_runtime_evaluate(pending_call);
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for script to become truthy")).into();
        }
        let pending_call_for_error = pending_call;
        let wait_for = remaining.min(LIVE_PAGE_COMMAND_WAIT_TURN_SLICE);
        let (mut entry, wait_result) = advance_script_truthy_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            expression.clone(),
            pending_call,
            wait_for,
        )
        .await;
        match wait_result {
            Ok(PageVmScriptTruthyWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            Ok(PageVmScriptTruthyWaitAdvance::Progressed { pending_call }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageScriptTruthy {
                        token,
                        expression,
                        pending_call,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmScriptTruthyWaitAdvance::Waiting {
                sleep_for,
                pending_call,
            }) => {
                if Instant::now() >= deadline {
                    entry
                        .page_vm_mut()
                        .cancel_pending_runtime_evaluate(pending_call);
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for script to become truthy")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageScriptTruthy {
                            token,
                            expression,
                            pending_call,
                            deadline,
                            loader,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                entry
                    .page_vm_mut()
                    .cancel_pending_runtime_evaluate(pending_call_for_error);
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn wait_live_page_runtime_expression_await_turn(
        &self,
        token: RendererPageToken,
        execution_context_id: Option<i64>,
        expression: String,
        pending_call: Option<PendingRuntimeEvaluateCall>,
        deadline: Instant,
        result_mode: RuntimeEvaluateResultMode,
        navigation_policy: RuntimeEvaluationNavigationPolicy,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            entry
                .page_vm_mut()
                .cancel_pending_runtime_evaluate(pending_call);
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!(
                "timed out waiting for runtime expression awaitPromise"
            ))
            .into();
        }
        let pending_call_for_error = pending_call;
        let (mut entry, wait_result) =
            advance_runtime_expression_await_turn_on_entry_via_local_task(
                self.state.local_executor.clone(),
                entry,
                execution_context_id,
                expression.clone(),
                pending_call,
                remaining,
                result_mode,
            )
            .await;
        match wait_result {
            Ok(PageVmRuntimeExpressionAwaitAdvance::Completed { payload }) => {
                let reply = RendererPageReply::RuntimeEvaluationResult(payload);
                if navigation_policy.follows_pending_navigation()
                    && entry.page_vm().vm().has_pending_location_navigation()
                {
                    self.continue_live_page_pending_navigation(
                        token,
                        entry,
                        PageVmInitStage::Load,
                        0,
                        LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                            reply: Box::new(reply),
                            capture_policy,
                        },
                    )
                } else {
                    self.finish_live_page_entry_with_page_state_and_continuation(
                        token,
                        entry,
                        reply,
                        Vec::new(),
                        None,
                        capture_policy,
                    )
                    .await
                }
            }
            Ok(PageVmRuntimeExpressionAwaitAdvance::Progressed { pending_call }) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                        token,
                        execution_context_id,
                        expression,
                        pending_call,
                        deadline,
                        result_mode,
                        navigation_policy,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmRuntimeExpressionAwaitAdvance::Waiting {
                sleep_for,
                pending_call,
            }) => {
                if Instant::now() >= deadline {
                    entry
                        .page_vm_mut()
                        .cancel_pending_runtime_evaluate(pending_call);
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!(
                        "timed out waiting for runtime expression awaitPromise"
                    ))
                    .into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                            token,
                            execution_context_id,
                            expression,
                            pending_call,
                            deadline,
                            result_mode,
                            navigation_policy,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                entry
                    .page_vm_mut()
                    .cancel_pending_runtime_evaluate(pending_call_for_error);
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn wait_live_page_subresource_response_turn(
        &self,
        token: RendererPageToken,
        criteria: SubresourceResponseWaitCriteria,
        deadline: Instant,
        loader: ResourceRequestClient,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.restore_live_page_entry(token, entry);
            return Err(anyhow!("timed out waiting for subresource response")).into();
        }
        let (entry, wait_result) = advance_subresource_response_wait_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            criteria.clone(),
            remaining,
        )
        .await;
        match wait_result {
            Ok(PageVmSubresourceResponseWaitAdvance::Completed) => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    RendererPageReply::Unit,
                    Vec::new(),
                    None,
                    capture_policy,
                )
                .await
            }
            Ok(PageVmSubresourceResponseWaitAdvance::TriggeredNavigation) => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    PageVmInitStage::Load,
                    0,
                    LivePagePendingNavigationCompletion::ContinueSubresourceResponse {
                        criteria,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ),
            Ok(PageVmSubresourceResponseWaitAdvance::Progressed) => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                        token,
                        criteria,
                        deadline,
                        loader,
                        capture_policy,
                    },
                ))
            }
            Ok(PageVmSubresourceResponseWaitAdvance::Waiting { sleep_for }) => {
                if Instant::now() >= deadline {
                    self.restore_live_page_entry(token, entry);
                    Err(anyhow!("timed out waiting for subresource response")).into()
                } else {
                    self.restore_live_page_entry(token, entry);
                    let ready_at = Instant::now()
                        .checked_add(sleep_for)
                        .unwrap_or(deadline)
                        .min(deadline);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                        turn: Box::new(RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                            token,
                            criteria,
                            deadline,
                            loader,
                            capture_policy,
                        }),
                        wake_token: token,
                        ready_at,
                    }
                }
            }
            Err(error) => {
                self.restore_live_page_entry(token, entry);
                Err(error).into()
            }
        }
    }

    pub(super) async fn follow_live_page_pending_location_navigation_turn(
        &self,
        token: RendererPageToken,
        stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        if follow_count == 0 {
            entry.begin_renderer_navigation_follow();
        }
        let retire_page_on_failure = completion.retires_page_on_navigation_failure();
        if follow_count >= MAX_PENDING_LOCATION_NAVIGATION_TURNS {
            let failure = PageNavigationOwnerFailure::TooManyChainedLocationNavigations {
                context: completion.chain_limit_error_context(),
            };
            let disposition = match completion.failure_recipient() {
                LivePageNavigationFailureRecipient::PageCreationObserver => {
                    LivePageNavigationFailureDisposition::PublishToPageCreation(failure)
                }
                LivePageNavigationFailureRecipient::Background => {
                    LivePageNavigationFailureDisposition::ReportBackground(failure)
                }
                LivePageNavigationFailureRecipient::Initiator => {
                    LivePageNavigationFailureDisposition::ReturnToInitiator(anyhow!(
                        failure.to_string()
                    ))
                }
            };
            return self
                .finish_live_page_navigation_failure(
                    token,
                    entry,
                    retire_page_on_failure,
                    disposition,
                )
                .await;
        }
        let advance = follow_pending_location_navigation_one_turn_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            stage,
        )
        .await;
        let (entry, follow_result) = match advance {
            LivePageNavigationFollowEntryAdvance::Live { entry, result } => (entry, result),
            LivePageNavigationFollowEntryAdvance::Committed { entry, error } => {
                return self.finish_committed_navigation_failure(
                    token,
                    entry,
                    LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                );
            }
        };
        let LivePageNavigationFollowTurn {
            outcome: follow_outcome,
            document_commit,
        } = match follow_result {
            Ok(turn) => turn,
            Err(error) => {
                return self
                    .finish_live_page_navigation_failure(
                        token,
                        entry,
                        retire_page_on_failure,
                        LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                    )
                    .await;
            }
        };
        let dispatch = match follow_outcome {
            LivePageNavigationFollowOutcome::Completed => {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
            LivePageNavigationFollowOutcome::PostParseLifecycle {
                target_stage,
                outcome,
            } => {
                self.handle_live_page_post_parse_lifecycle_outcome(
                    token,
                    entry,
                    target_stage,
                    follow_count,
                    completion,
                    outcome,
                )
                .await
            }
            LivePageNavigationFollowOutcome::Download(download) => {
                self.finish_live_page_navigation_download(token, entry, completion, download)
                    .await
            }
            LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token } => {
                self.restore_live_page_entry(token, entry);
                let admission =
                    pending_phase_one_admission_after_restore_on_bound_owner_local_store(
                        wake_token,
                    );
                self.signal_pending_phase_one_admission(wake_token, admission);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                            token,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token,
                }
            }
            LivePageNavigationFollowOutcome::TriggeredNavigation { stage } => self
                .continue_live_page_pending_navigation(
                    token,
                    entry,
                    stage,
                    follow_count + 1,
                    completion,
                ),
        };
        if let Some(document_commit) = document_commit {
            tracing::debug!(
                page_id = token.page_id.as_u64(),
                navigation_handoff = ?document_commit.navigation_handoff,
                vm_creation_id = document_commit.vm_creation_id,
                view_generation = document_commit.view_generation,
                "published standalone replacement Document"
            );
            self.signal_replacement_document_view_settled(token, document_commit.vm_creation_id);
        }
        dispatch
    }

    pub(super) async fn handle_live_page_post_parse_lifecycle_outcome(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        target_stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
        outcome: DocumentLifecycleTurnOutcome,
    ) -> RenderRuntimeDispatchOutcome {
        match outcome {
            DocumentLifecycleTurnOutcome {
                action: DocumentLifecycleTurnAction::RequestedTopLevelNavigation { stage, .. },
                ..
            } => {
                if completion.returns_with_pending_location_navigation() {
                    self.finish_live_page_navigation_completion(token, entry, completion)
                        .await
                } else {
                    self.continue_live_page_pending_navigation(
                        token,
                        entry,
                        stage,
                        follow_count + 1,
                        completion,
                    )
                }
            }
            DocumentLifecycleTurnOutcome {
                readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                ..
            } => {
                self.restore_live_page_entry(token, entry);
                self.signal_internal_document_lifecycle_turn(token);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleTurnOutcome {
                readiness: DocumentLifecycleTurnReadiness::Blocked { document },
                ..
            } => {
                self.restore_live_page_entry(token, entry);
                RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count,
                            completion,
                        },
                    ),
                    wake_token: token,
                }
            }
            DocumentLifecycleTurnOutcome {
                readiness: DocumentLifecycleTurnReadiness::Idle,
                ..
            } => {
                self.finish_live_page_navigation_completion(token, entry, completion)
                    .await
            }
        }
    }
}
