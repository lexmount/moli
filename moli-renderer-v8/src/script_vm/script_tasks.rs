use super::*;

impl ScriptVm {
    #[cfg(test)]
    pub(crate) async fn advance_timers_until_deadline_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<()> {
        let deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(3_200))
            .unwrap_or_else(Instant::now);
        self.advance_timers_until_deadline_for_test_with_deadline(loader, deadline)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn advance_timers_until_deadline_for_test_with_deadline(
        &mut self,
        loader: &ResourceRequestClient,
        deadline: Instant,
    ) -> Result<()> {
        const MAX_TEST_ADVANCE_ROUNDS: usize = 10_000;
        let mut rounds = 0usize;

        while rounds < MAX_TEST_ADVANCE_ROUNDS {
            // This is an explicit low-level executor test helper. Production
            // callers must enter through the scheduler-selected PageTimer
            // turn, never through a generic ready-task drain.
            if self.has_ready_timeout() && self.run_next_due_timer_callback_for_test(loader).await?
            {
                rounds += 1;
                continue;
            }

            let Some(ms_to_next) = self.ms_to_next_timeout() else {
                break;
            };
            if ms_to_next == 0 {
                rounds += 1;
                continue;
            }

            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let sleep_for = std::time::Duration::from_millis(ms_to_next)
                .min(deadline.saturating_duration_since(now));
            if sleep_for.is_zero() {
                break;
            }
            tokio::time::sleep(sleep_for).await;
            rounds += 1;
        }

        Ok(())
    }

    /// Execute one due timer task body after the Page scheduler has validated
    /// its observed heap-head deadline.
    ///
    /// This method deliberately does not checkpoint, synchronize child
    /// records, or run runtime follow-up. The selected Page-task dispatcher
    /// commits that completion exactly once after the body returns.
    pub(crate) fn run_next_due_timer_callback_body(
        &mut self,
        selection: crate::page_task_queue::RendererPageTimerSelection,
    ) -> Result<HostTimeoutRunResult> {
        let result = self.run_next_timeout_body(selection)?;
        if let HostTimeoutRunResult::CallbackError(error) = &result {
            self.record_runtime_warning(format_args!("timer callback dispatch failed: {error}"));
        }
        Ok(result)
    }

    /// Execute one ready timer whose scheduling sequence belongs to a classic
    /// defer script, without committing its task-end callback completion.
    pub(crate) fn run_next_classic_defer_timer_callback_body(
        &mut self,
    ) -> Result<HostTimeoutRunResult> {
        let result = self.run_next_timeout_queued_by_classic_defer_script_body()?;
        if let HostTimeoutRunResult::CallbackError(error) = &result {
            self.record_runtime_warning(format_args!("timer callback dispatch failed: {error}"));
        }
        Ok(result)
    }

    /// Complete one timer in a standalone ScriptVm fixture.
    ///
    /// Production and PageVm behavior tests must use the selected Page-task
    /// dispatcher. Standalone domain fixtures have no Page owner slot, so this
    /// helper explicitly supplies the same bounded callback completion.
    #[cfg(test)]
    pub(crate) async fn run_next_due_timer_callback_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<bool> {
        let result = self.run_next_due_timer_callback_body(
            crate::page_task_queue::RendererPageTimerSelection::AnyReady,
        )?;
        if !result.consumed_heap_head() {
            return Ok(false);
        }
        // Timer turns are ordinary runtime activity. Runtime follow-up may
        // publish concrete Page work, but must not wait for network completion.
        self.finish_selected_page_callback_task(loader).await?;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn has_ready_window_message_task(&self) -> bool {
        self._context_host.borrow().has_pending_window_messages()
    }

    #[cfg(test)]
    pub(crate) fn pending_window_message_endpoints_for_test(
        &self,
    ) -> Vec<(
        crate::native_bridge::PendingWindowMessageEndpoint,
        crate::native_bridge::PendingWindowMessageEndpoint,
    )> {
        self._context_host
            .borrow()
            .pending_window_message_endpoints_for_test()
    }

    pub(crate) async fn finish_host_task_turn(
        &mut self,
        loader: &ResourceRequestClient,
        wait_for_dynamic_loads: bool,
    ) -> Result<()> {
        self.flush_pending_work(loader, wait_for_dynamic_loads)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub(crate) async fn run_prepared_script(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        dynamic_script_owner_id: Option<crate::dynamic_script_owner::DynamicScriptOwnerId>,
    ) -> std::result::Result<PreparedScriptExecutionOutcome, PreparedScriptExecutionError> {
        debug!(
            url = %script.url,
            mode = ?script.mode,
            kind = ?script.kind,
            "run_prepared_script begin"
        );
        let Some(run_input) = self.prepare_prepared_script_run(loader, script).await? else {
            return Ok(PreparedScriptExecutionOutcome::Dropped(
                PreparedScriptBodyActivity::NotEntered,
            ));
        };
        self.document_runtime
            .set_current_script_context(CurrentScriptContextSpec {
                handle: run_input.current_script,
                parser_write_insertion_point_active: run_input.parser_write_insertion_point_active,
                parser_bridge: None,
            });
        let document_owner_before_run =
            self.current_main_document_task_owner().ok_or_else(|| {
                PreparedScriptExecutionError::from_message(format!(
                    "prepared script `{}` has no current main Document owner",
                    script.url
                ))
            })?;
        let result = self
            .execute_prepared_script_run_body(script, run_input.body)
            .await;
        self.document_runtime.clear_current_script_handle();
        let outcome = result?;
        let body_activity = outcome.body_activity();
        if self.script_run_replaced_document(document_owner_before_run, script) {
            return Ok(PreparedScriptExecutionOutcome::Dropped(body_activity));
        }
        match outcome {
            LoadedScriptExecutionOutcome::Completed(_) => {}
            LoadedScriptExecutionOutcome::CompletedModuleGraph(graph) => {
                let continuation = self
                    .module_script_continuation_for_prepared_script(
                        script,
                        dynamic_script_owner_id,
                        document_owner_before_run,
                    )?
                    .with_completed_graph(graph);
                let actions = self.handle_module_script_graph_advance_for_owner(
                    ModuleScriptContinuationGraphAdvance::Ready(Box::new(continuation)),
                );
                debug_assert!(
                    actions.into_runtime_module_failures().is_empty(),
                    "completed module graph should only enqueue owner-ready work"
                );
                return Ok(PreparedScriptExecutionOutcome::DeferredModuleCompletion);
            }
            LoadedScriptExecutionOutcome::SuspendedModuleFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                let continuation = self.module_script_continuation_for_prepared_script(
                    script,
                    dynamic_script_owner_id,
                    document_owner_before_run,
                )?;
                let actions = self.handle_module_script_graph_advance_for_owner(
                    ModuleScriptContinuationGraphAdvance::NeedFetches {
                        continuation: Box::new(continuation),
                        job: Box::new(job),
                        fetches,
                    },
                );
                debug_assert!(
                    actions.into_runtime_module_failures().is_empty(),
                    "suspended module graph should only enqueue owner wait work"
                );
                return Ok(PreparedScriptExecutionOutcome::DeferredModuleCompletion);
            }
        }
        let uses_runtime_owned_page_task_execution =
            self.prepared_script_uses_runtime_owned_page_task_execution(script);
        // DynamicScriptOwner produces the observable terminal and exact
        // lifecycle settlement after this execution phase.
        let skip_current_script_load_enqueue = dynamic_script_owner_id.is_some();
        let finish_behavior = if uses_runtime_owned_page_task_execution {
            PreparedScriptFinishBehavior::QueueRuntimeContinuation
        } else {
            PreparedScriptFinishBehavior::FlushPendingWork
        };
        self.finish_run_prepared_script(
            loader,
            script,
            false,
            skip_current_script_load_enqueue,
            finish_behavior,
        )
        .await
        .map_err(|message| {
            PreparedScriptExecutionError::from_message(message).with_body_activity(body_activity)
        })?;
        Ok(PreparedScriptExecutionOutcome::Completed(body_activity))
    }

    pub(super) fn completion_owner_for_prepared_module_script(
        &self,
        script: &PreparedScript,
    ) -> ModuleScriptCompletionOwner {
        if self.prepared_script_uses_runtime_owned_page_task_execution(script) {
            ModuleScriptCompletionOwner::Runtime
        } else {
            ModuleScriptCompletionOwner::Parser
        }
    }

    pub(super) fn module_script_continuation_for_prepared_script(
        &self,
        script: &PreparedScript,
        dynamic_script_owner_id: Option<crate::dynamic_script_owner::DynamicScriptOwnerId>,
        document_owner_before_run: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> std::result::Result<ModuleScriptContinuation, PreparedScriptExecutionError> {
        match self.completion_owner_for_prepared_module_script(script) {
            ModuleScriptCompletionOwner::Parser => {
                let pending_script_id = self
                    .document_runtime
                    .parser_module_document_scripts()
                    .pending_script_id_for_script(script)
                    .ok_or_else(|| {
                    PreparedScriptExecutionError::from_message(format!(
                        "parser-owned module script `{}` has no unique registered PendingScript",
                        script.url
                    ))
                })?;
                debug_assert_eq!(
                    pending_script_id.owner().task_owner(),
                    document_owner_before_run,
                    "parser module continuation must retain its admitted Document owner"
                );
                Ok(ModuleScriptContinuation::new_parser(
                    script.clone(),
                    pending_script_id,
                ))
            }
            ModuleScriptCompletionOwner::Runtime => {
                let owner = dynamic_script_owner_id.ok_or_else(|| {
                    PreparedScriptExecutionError::from_message(format!(
                        "runtime-owned module script `{}` has no dynamic script owner",
                        script.url
                    ))
                })?;
                Ok(ModuleScriptContinuation::new_runtime(
                    script.clone(),
                    owner,
                    document_owner_before_run,
                ))
            }
        }
    }

    pub(crate) async fn settle_prepared_module_success(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        document_owner_before_run: crate::frame_owner_model::FrameDocumentTaskOwner,
        dynamic_script_owner_id: Option<crate::dynamic_script_owner::DynamicScriptOwnerId>,
        evaluation: ParserModuleEvaluationSettlement,
        terminal_disposition: ParserModuleTerminalDisposition,
        prepared_activity: PreparedScriptBodyActivity,
    ) -> std::result::Result<PreparedModuleSuccessSettlement, String> {
        if self.script_run_replaced_document(document_owner_before_run, script) {
            return Ok(PreparedModuleSuccessSettlement::Stale);
        }
        let uses_runtime_owned_page_task_execution =
            self.prepared_script_uses_runtime_owned_page_task_execution(script);
        if !uses_runtime_owned_page_task_execution
            && terminal_disposition == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
        {
            let script_event = self
                .plan_script_load_lifecycle_work_for_prepared_script(script)
                .and_then(|work| match work {
                    PostParseLifecycleWork::DispatchScriptEvent(task) => Some(task),
                    other => {
                        debug_assert!(
                            false,
                            "parser module load planning produced non-event work: {other:?}"
                        );
                        None
                    }
                });
            return Ok(PreparedModuleSuccessSettlement::ParserOwned(
                ParserOwnedModuleSuccessTerminal::new(evaluation, script_event, prepared_activity),
            ));
        }
        // DynamicScriptOwner produces the observable terminal and exact
        // lifecycle settlement after module evaluation starts.
        let skip_current_script_load_enqueue = dynamic_script_owner_id.is_some();
        let finish_behavior = if uses_runtime_owned_page_task_execution {
            PreparedScriptFinishBehavior::QueueRuntimeContinuation
        } else {
            PreparedScriptFinishBehavior::FlushPendingWork
        };
        self.finish_run_prepared_script(
            loader,
            script,
            false,
            skip_current_script_load_enqueue,
            finish_behavior,
        )
        .await?;
        Ok(if uses_runtime_owned_page_task_execution {
            PreparedModuleSuccessSettlement::RuntimeOwned
        } else {
            PreparedModuleSuccessSettlement::ParserOwnedCompleted
        })
    }

    pub(super) async fn finish_run_prepared_script(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        defer_script_event_dispatches: bool,
        skip_current_script_load_enqueue: bool,
        finish_behavior: PreparedScriptFinishBehavior,
    ) -> std::result::Result<(), String> {
        if self.has_pending_location_navigation() {
            debug!(url = %script.url, "run_prepared_script exiting due to pending location navigation");
            return Ok(());
        }
        if defer_script_event_dispatches {
            self.document_runtime
                .deferred_page_tasks_mut()
                .enter_scope();
        }
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| -> std::result::Result<(), String> {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let checkpoint_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
                Self::reset_dom_binding_trace_window();
                let dom_binding_checkpoint_started =
                    moli_trace::dom_binding_timing_enabled().then(Instant::now);
                Self::perform_microtask_checkpoints(scope, Some(&script.url))
                    .map_err(|error| error.to_string())?;
                if let Some(started) = dom_binding_checkpoint_started {
                    Self::emit_dom_binding_trace_window(
                        "renderer_prepared_script_dom_binding_summary",
                        "post_script_checkpoint",
                        Some(&script.url),
                        started.elapsed(),
                    );
                }
                if let Some(started) = checkpoint_started {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        url = %script.url,
                        stage = "renderer_prepared_script_checkpoint_done",
                        elapsed_ms = started.elapsed().as_millis(),
                    );
                }
                Ok(())
            })?;
        let current_script_load_disposition = if skip_current_script_load_enqueue {
            FollowupPageTaskDisposition::Skipped
        } else {
            self.enqueue_script_load_lifecycle_work_for_prepared_script_best_effort(script)
        };
        if defer_script_event_dispatches {
            self.document_runtime.deferred_page_tasks_mut().exit_scope();
        }
        if defer_script_event_dispatches && skip_current_script_load_enqueue {
            debug!(
                url = %script.url,
                "run_prepared_script defers followup runtime work until parser progress after handling current script load"
            );
            return Ok(());
        }
        if self.pause_runtime_script_work_at_followup_task_boundary(current_script_load_disposition)
        {
            debug!(
                url = %script.url,
                disposition = ?current_script_load_disposition,
                "run_prepared_script finished after enqueueing current script load page task"
            );
            return Ok(());
        }
        let result = match finish_behavior {
            PreparedScriptFinishBehavior::FlushPendingWork => {
                self.flush_pending_work(loader, !defer_script_event_dispatches)
                    .await
            }
            PreparedScriptFinishBehavior::QueueRuntimeContinuation => {
                self.enqueue_immediate_runtime_script_work_if_needed();
                Ok(())
            }
        };
        debug!(
            url = %script.url,
            result = ?result.as_ref().map(|_| ()).map_err(|error| error.as_str()),
            "run_prepared_script finished"
        );
        result
    }

    pub(super) fn parser_owned_external_classic_has_completion_event(
        &self,
        script: &PreparedScript,
    ) -> bool {
        script.kind == ScriptKind::Classic
            && script.source_kind == ScriptSourceKind::External
            && matches!(script.mode, ScriptMode::Normal | ScriptMode::Defer)
            && script.host_script_handle.as_deref().is_some_and(|handle| {
                self.document_runtime.script_handle_source(handle)
                    == ScriptHandleSource::ParserOwned
            })
    }

    pub(super) fn plan_parser_owned_external_classic_completion_event(
        &mut self,
        script: &PreparedScript,
        kind: ScriptEventKind,
    ) -> Option<ScriptEventTask> {
        if !self.parser_owned_external_classic_has_completion_event(script) {
            return None;
        }
        let followup = match kind {
            ScriptEventKind::Load => "parser-owned classic script load",
            ScriptEventKind::Error => "parser-owned classic script error",
        };
        let handle =
            self.required_host_script_handle_for_observable_script_followup(script, followup)?;
        self.document_runtime
            .plan_script_event_task_for_script(kind, script, handle)
    }

    pub(super) fn prepared_script_followup_lane(
        &self,
        script: &PreparedScript,
    ) -> DeferredPageTaskLane {
        if let Some(handle) = script.host_script_handle.as_deref()
            && let Some(lane) = self.document_runtime.script_handle_followup_lane(handle)
        {
            return lane;
        }
        crate::host::HostScriptScheduler::followup_lane_for_script(
            ScriptHandleSource::Unknown,
            script.mode,
        )
    }

    pub(crate) fn parser_owned_inline_importmap_reports_window_error_immediately(
        &self,
        script: &PreparedScript,
    ) -> bool {
        script.kind == ScriptKind::ImportMap
            && script.source_kind == ScriptSourceKind::Inline
            && script.host_script_handle.as_deref().is_some_and(|handle| {
                self.document_runtime.script_handle_source(handle)
                    == ScriptHandleSource::ParserOwned
            })
    }

    pub(crate) fn parser_owned_module_reports_failure_immediately(
        &self,
        script: &PreparedScript,
    ) -> bool {
        script.kind == ScriptKind::Module
            && script.host_script_handle.as_deref().is_some_and(|handle| {
                self.document_runtime.script_handle_source(handle)
                    == ScriptHandleSource::ParserOwned
            })
    }

    pub(super) fn script_run_replaced_document(
        &mut self,
        document_owner_before_run: crate::frame_owner_model::FrameDocumentTaskOwner,
        script: &PreparedScript,
    ) -> bool {
        if self.current_main_document_task_owner() == Some(document_owner_before_run) {
            return false;
        }
        self.refresh_script_vm_local_document_state();
        debug!(
            url = %script.url,
            mode = ?script.mode,
            kind = ?script.kind,
            "skipping stale script followup after document replacement during script execution"
        );
        true
    }

    pub(super) fn required_host_script_handle_for_observable_script_followup<'a>(
        &mut self,
        script: &'a PreparedScript,
        followup: &'static str,
    ) -> Option<&'a str> {
        let handle = script.host_script_handle.as_deref();
        if handle.is_none() {
            let node_is_live_script = self
                .document_runtime
                .dom_host()
                .node(script.node_id)
                .is_some_and(|node| node.is_connected() && node.is_script_element());
            self.record_runtime_warning(format_args!(
                "skipping {followup} for `{}` because prepared script has no host handle",
                script.url
            ));
            debug_assert!(
                !node_is_live_script,
                "observable script followup requires a bound host handle"
            );
        }
        handle
    }

    // Main-document work retains its preparation owner until it is ready. Check
    // adoption when entering execution, without cancelling its fetch or order slot.
    pub(crate) fn prepared_script_changed_documents(&self, script: &PreparedScript) -> bool {
        let node = script
            .host_script_handle
            .as_deref()
            .and_then(|handle| self.document_runtime.resolve_host_script_handle(handle))
            .unwrap_or(script.node_id);
        self.document_runtime
            .dom_host()
            .owner_document_handle(node)
            .is_some_and(|document| document != self.document_runtime.document_handle())
    }

    pub(super) fn prepared_script_is_live_for_execution(
        &mut self,
        script: &PreparedScript,
    ) -> bool {
        if self.prepared_script_changed_documents(script) {
            return false;
        }
        let Some(handle) = script.host_script_handle.as_deref() else {
            let allow_missing_handle = script.kind == ScriptKind::Classic
                && script.source_kind == ScriptSourceKind::Inline
                && script.mode == ScriptMode::Normal;
            let node_is_live_script = self
                .document_runtime
                .dom_host()
                .node(script.node_id)
                .is_some_and(|node| node.is_connected() && node.is_script_element());
            if !allow_missing_handle {
                self.record_runtime_warning(format_args!(
                    "skipping stale prepared script `{}` because it has no bound host handle",
                    script.url
                ));
                debug_assert!(
                    !node_is_live_script,
                    "prepared script execution requires a bound host handle"
                );
            }
            return allow_missing_handle;
        };

        if self
            .document_runtime
            .resolve_host_script_handle(handle)
            .is_some()
        {
            return true;
        }

        self.record_runtime_warning(format_args!(
            "skipping stale prepared script `{}` because handle `{handle}` is no longer live",
            script.url
        ));
        debug_assert!(
            false,
            "prepared script execution requires a live registered handle"
        );
        false
    }

    pub(crate) fn perform_parser_script_preparation_checkpoint(&mut self) -> Result<()> {
        self.with_default_context_scope(|scope, _| {
            crate::script_cleanup::perform_parser_script_preparation_checkpoint(scope)
        })
    }

    /// Run one explicit page-task microtask checkpoint before a queued script task.
    ///
    /// This does not make the runtime a full browser task queue yet. The point is
    /// narrower: once parse-time classic async is modeled as a page-owned task, the
    /// task should get its own pre-task checkpoint instead of being executed as
    /// "whatever ready scripts coordinator had in a vector right now".
    ///
    /// Keeping this helper separate from `run_prepared_script(...)` makes the
    /// boundary explicit:
    /// - pre-task checkpoint belongs to the page task queue turn
    /// - post-script checkpoint still belongs to script execution / flush logic
    pub(crate) fn perform_script_task_checkpoint(
        &mut self,
        script_url: Option<&Url>,
    ) -> anyhow::Result<()> {
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                Self::reset_dom_binding_trace_window();
                let dom_binding_checkpoint_started =
                    moli_trace::dom_binding_timing_enabled().then(Instant::now);
                Self::perform_microtask_checkpoints(scope, script_url)?;
                if let Some(started) = dom_binding_checkpoint_started {
                    Self::emit_dom_binding_trace_window(
                        "renderer_script_task_dom_binding_summary",
                        "pre_task_checkpoint",
                        script_url,
                        started.elapsed(),
                    );
                }
                Ok(())
            })
    }
}
