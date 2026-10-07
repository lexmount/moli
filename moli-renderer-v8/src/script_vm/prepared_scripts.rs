use super::*;

impl ScriptVm {
    pub(crate) async fn prepare_prepared_script_run(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
    ) -> std::result::Result<Option<PreparedScriptRunInput>, PreparedScriptExecutionError> {
        self.prepare_prepared_script_run_with_options(loader, script, true, None)
            .await
    }

    pub(crate) async fn prepare_prepared_script_run_without_blocker_wait(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
    ) -> std::result::Result<Option<PreparedScriptRunInput>, PreparedScriptExecutionError> {
        self.prepare_prepared_script_run_with_options(loader, script, false, None)
            .await
    }

    pub(super) async fn prepare_prepared_script_run_with_options(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        wait_for_blocking_stylesheets: bool,
        blocking_signatures_before: Option<
            &std::collections::HashSet<
                crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
            >,
        >,
    ) -> std::result::Result<Option<PreparedScriptRunInput>, PreparedScriptExecutionError> {
        if !self.prepared_script_is_live_for_execution(script) {
            return Ok(None);
        }
        debug!(
            url = %script.url,
            mode = ?script.mode,
            kind = ?script.kind,
            source_kind = ?script.source_kind,
            "execute_prepared_script_once loading source"
        );
        let load_started = Instant::now();
        let current_script = script
            .host_script_handle
            .as_deref()
            .and_then(|handle| self.document_runtime.resolve_host_script_handle(handle))
            .or(Some(script.node_id));
        let csp_script_request = self.content_security_policy_script_element_request(script);
        if script.source_kind == ScriptSourceKind::External
            && let Some(violation) = self
                .document_runtime
                .script_element_request_csp_report_only_violation_with_request(
                    &script.url,
                    csp_script_request,
                )
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
        if script.source_kind == ScriptSourceKind::External
            && let Some(violation) = self
                .document_runtime
                .script_element_request_csp_violation_with_request(&script.url, csp_script_request)
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
            let message = format!(
                "Refused to load script `{}` because it violates the document Content Security Policy directive `{}`",
                script.url, violation.effective_directive
            );
            return Err(if script.kind == ScriptKind::Module {
                PreparedScriptExecutionError::from_top_level_module_source_load_failure(message)
            } else {
                PreparedScriptExecutionError::from_message(message)
            });
        }
        if wait_for_blocking_stylesheets
            && self
                .document_runtime
                .prepared_script_waits_for_blocking_stylesheets(script)
        {
            match blocking_signatures_before {
                Some(signatures) => {
                    self.document_runtime
                        .wait_for_document_owned_blocking_stylesheet_signatures(signatures.iter())
                        .await;
                }
                None => {
                    let completed_stylesheet_clients = self
                        .document_runtime
                        .wait_for_script_blockers_before(script.node_id)
                        .await;
                    self.settle_stylesheet_link_clients(completed_stylesheet_clients);
                }
            }
            self.record_ready_stylesheet_network_results();
        }
        if prepared_script_uses_external_module_graph(script) {
            debug!(
                url = %script.url,
                elapsed_ms = load_started.elapsed().as_millis(),
                "execute_prepared_script_once prepared external module graph"
            );
            if moli_trace::cdp_nav_timing_enabled() {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url = %script.url,
                    elapsed_ms = load_started.elapsed().as_millis(),
                    kind = ?script.kind,
                    mode = ?script.mode,
                    source_kind = ?script.source_kind,
                    stage = "renderer_prepared_script_external_module_graph_prepared",
                );
            }
            return Ok(Some(PreparedScriptRunInput {
                current_script,
                parser_write_insertion_point_active: false,
                body: PreparedScriptRunBody::ExternalModuleGraph,
            }));
        }
        let (source, source_bytes) = match &script.source {
            crate::planning::ScriptSource::Loaded(source) => (source.clone(), None),
            crate::planning::ScriptSource::LoadedBinary { source, bytes } => {
                (source.clone(), Some(bytes.clone()))
            }
            _ => {
                let document_character_set =
                    self.document_runtime.document_character_set().to_owned();
                let outcome =
                    crate::planning::load_prepared_script_source_outcome_with_document_character_set(
                        script,
&self.current_main_document_resource_loader().expect("script load requires its Document authority").fetch_context().request_origin(),
                        loader,
                        Some(&document_character_set),
                        None,
                    )
                    .await;
                if let Some(network_result) = outcome.network_result.as_deref() {
                    self._context_host
                        .borrow_mut()
                        .record_get_subresource_network_result_with_initiator(
                            None,
                            script.initiator_url.clone(),
                            script.url.clone(),
                            SubresourceResourceType::Script,
                            crate::types::SubresourceRequestInitiatorType::Parser,
                            network_result,
                        );
                }
                self.enforce_external_script_redirect_csp(
                    script,
                    outcome.network_result.as_deref(),
                )?;
                let source = outcome.source_result.map_err(|message| {
                    if script.kind == ScriptKind::Module {
                        PreparedScriptExecutionError::from_top_level_module_source_load_failure(
                            message,
                        )
                    } else {
                        PreparedScriptExecutionError::from_message(message)
                    }
                })?;
                (source, outcome.source_bytes)
            }
        };
        debug!(
            url = %script.url,
            source_len = source.len(),
            elapsed_ms = load_started.elapsed().as_millis(),
            "execute_prepared_script_once source loaded"
        );
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %script.url,
                elapsed_ms = load_started.elapsed().as_millis(),
                source_len = source.len(),
                kind = ?script.kind,
                mode = ?script.mode,
                source_kind = ?script.source_kind,
                stage = "renderer_prepared_script_source_loaded",
            );
        }
        Ok(Some(PreparedScriptRunInput {
            current_script,
            parser_write_insertion_point_active: script.kind == ScriptKind::Classic
                && script.mode == ScriptMode::Normal,
            body: PreparedScriptRunBody::LoadedSource {
                source,
                source_bytes,
            },
        }))
    }

    pub(super) fn enforce_external_script_redirect_csp(
        &mut self,
        script: &PreparedScript,
        network_result: Option<&std::result::Result<crate::types::NavigationResponse, String>>,
    ) -> std::result::Result<(), PreparedScriptExecutionError> {
        if script.source_kind != ScriptSourceKind::External {
            return Ok(());
        }
        let Some(Ok(response)) = network_result else {
            return Ok(());
        };
        if !response.redirected {
            return Ok(());
        }
        let redirect_status =
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect;
        let csp_script_request = self.content_security_policy_script_element_request(script);
        if let Some(violation) = self
            .document_runtime
            .script_element_request_csp_report_only_violation_with_redirect_status(
                &response.final_url,
                redirect_status,
                csp_script_request,
            )
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
        let Some(violation) = self
            .document_runtime
            .script_element_request_csp_violation_with_redirect_status(
                &response.final_url,
                redirect_status,
                csp_script_request,
            )
        else {
            return Ok(());
        };
        self.queue_content_security_policy_violation_event_best_effort(&violation);
        let message = format!(
            "Refused to load script `{}` because it violates the document Content Security Policy directive `{}`",
            response.final_url, violation.effective_directive
        );
        Err(if script.kind == ScriptKind::Module {
            PreparedScriptExecutionError::from_top_level_module_source_load_failure(message)
        } else {
            PreparedScriptExecutionError::from_message(message)
        })
    }

    pub(crate) async fn run_parser_owned_classic_script_without_blocker_wait(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        execution_context: &ParserOwnedClassicScriptExecutionContext,
    ) -> ParserOwnedClassicScriptExecutionReport {
        let run_input = match self
            .prepare_prepared_script_run_without_blocker_wait(loader, script)
            .await
        {
            Ok(Some(run_input)) => run_input,
            Ok(None) => {
                return ParserOwnedClassicScriptExecutionReport::new(
                    Ok(()),
                    None,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    PreparedScriptBodyActivity::NotEntered,
                );
            }
            Err(error) => {
                let script_element_event = self
                    .plan_parser_owned_external_classic_completion_event(
                        script,
                        ScriptEventKind::Error,
                    );
                tracing::debug!(
                    url = %script.url,
                    event_planned = script_element_event.is_some(),
                    error = error.message(),
                    "parser-owned classic preparation produced completion work"
                );
                return ParserOwnedClassicScriptExecutionReport::new(
                    Err(ParserOwnedClassicScriptExecutionError::new(
                        error.into_message(),
                    )),
                    script_element_event,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    PreparedScriptBodyActivity::NotEntered,
                );
            }
        };
        let parser_bridge = match (
            run_input.current_script,
            run_input.parser_write_insertion_point_active,
            execution_context.parser_bridge(),
        ) {
            (Some(_), true, Some(bridge)) => Some(bridge.clone()),
            _ => None,
        };
        // XML parser-blocking scripts have a currentScript and use the same
        // execution/lifecycle coordinator, but XML has no HTML insertion point.
        // Absence of a controller therefore means document.write is inactive;
        // it must not suppress otherwise valid XHTML/SVG script execution.
        let parser_write_insertion_point_active =
            run_input.parser_write_insertion_point_active && parser_bridge.is_some();
        self.document_runtime
            .set_current_script_context(CurrentScriptContextSpec {
                handle: run_input.current_script,
                parser_write_insertion_point_active,
                parser_bridge,
            });
        let document_owner_before_run = self
            .current_main_document_task_owner()
            .expect("parser-owned classic execution requires a current main Document owner");
        let result = {
            let _parser_script_nesting = execution_context
                .is_parser_blocking()
                .then(|| self.document_runtime.enter_parser_script_nesting());
            self.execute_prepared_script_run_body(script, run_input.body)
                .await
        };
        self.document_runtime.clear_current_script_handle();
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                let script_element_event = self
                    .plan_parser_owned_external_classic_completion_event(
                        script,
                        ScriptEventKind::Error,
                    );
                tracing::debug!(
                    url = %script.url,
                    event_planned = script_element_event.is_some(),
                    error = error.message(),
                    "parser-owned classic execution produced completion work"
                );
                return ParserOwnedClassicScriptExecutionReport::new(
                    Err(ParserOwnedClassicScriptExecutionError::new(
                        error.into_message(),
                    )),
                    script_element_event,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    PreparedScriptBodyActivity::Entered,
                );
            }
        };
        let body_activity = outcome.body_activity();
        match outcome {
            LoadedScriptExecutionOutcome::Completed(_) => {}
            LoadedScriptExecutionOutcome::CompletedModuleGraph(_)
            | LoadedScriptExecutionOutcome::SuspendedModuleFetches(_) => {
                let message = format!(
                    "parser-owned classic script `{}` produced a native ESM continuation",
                    script.url
                );
                let script_element_event = self
                    .plan_parser_owned_external_classic_completion_event(
                        script,
                        ScriptEventKind::Error,
                    );
                return ParserOwnedClassicScriptExecutionReport::new(
                    Err(ParserOwnedClassicScriptExecutionError::new(message)),
                    script_element_event,
                    ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                    body_activity,
                );
            }
        }
        if self.script_run_replaced_document(document_owner_before_run, script) {
            return ParserOwnedClassicScriptExecutionReport::new(
                Ok(()),
                None,
                ParserOwnedClassicScriptEvaluationSettlement::Settled,
                body_activity,
            );
        }
        let script_element_event =
            self.plan_parser_owned_external_classic_completion_event(script, ScriptEventKind::Load);
        tracing::debug!(
            url = %script.url,
            event_planned = script_element_event.is_some(),
            "parser-owned classic execution is awaiting owner completion"
        );
        ParserOwnedClassicScriptExecutionReport::new(
            Ok(()),
            script_element_event,
            ParserOwnedClassicScriptEvaluationSettlement::Settled,
            body_activity,
        )
    }

    pub(super) async fn execute_prepared_script_once(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
    ) -> std::result::Result<bool, PreparedScriptExecutionError> {
        self.execute_prepared_script_once_with_blocking_signatures(loader, script, None)
            .await
    }

    pub(super) async fn execute_prepared_script_once_with_blocking_signatures(
        &mut self,
        loader: &ResourceRequestClient,
        script: &PreparedScript,
        blocking_signatures_before: Option<
            &std::collections::HashSet<
                crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
            >,
        >,
    ) -> std::result::Result<bool, PreparedScriptExecutionError> {
        let Some(run_input) = self
            .prepare_prepared_script_run_with_options(
                loader,
                script,
                true,
                blocking_signatures_before,
            )
            .await?
        else {
            return Ok(false);
        };
        self.document_runtime
            .set_current_script_context(CurrentScriptContextSpec {
                handle: run_input.current_script,
                parser_write_insertion_point_active: run_input.parser_write_insertion_point_active,
                parser_bridge: None,
            });
        let result = self
            .execute_prepared_script_run_body(script, run_input.body)
            .await;
        self.document_runtime.clear_current_script_handle();
        match result? {
            LoadedScriptExecutionOutcome::Completed(_) => Ok(true),
            LoadedScriptExecutionOutcome::CompletedModuleGraph(_)
            | LoadedScriptExecutionOutcome::SuspendedModuleFetches(_) => {
                Err(PreparedScriptExecutionError::from_message(format!(
                    "runtime module script `{}` produced a native ESM continuation outside page-task execution",
                    script.url
                )))
            }
        }
    }

    pub(super) async fn execute_prepared_script_run_body(
        &mut self,
        script: &PreparedScript,
        body: PreparedScriptRunBody,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        match body {
            PreparedScriptRunBody::LoadedSource {
                source,
                source_bytes,
            } => {
                self.execute_loaded_prepared_script_source(script, &source, source_bytes.as_deref())
                    .await
            }
            PreparedScriptRunBody::ExternalModuleGraph => {
                self.execute_external_prepared_module_script_graph(script)
                    .await
            }
        }
    }

    pub(super) async fn execute_external_prepared_module_script_graph(
        &mut self,
        script: &PreparedScript,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        let completion_owner = self.completion_owner_for_prepared_module_script(script);
        match execute_external_module_script_graph(
            self,
            &script.url,
            &script.initiator_url,
            &script.fetch_metadata,
            completion_owner,
        )
        .await
        .map_err(PreparedScriptExecutionError::from_module_load_error)?
        {
            ModuleScriptExecutionOutcome::CompletedModuleGraph(graph) => {
                Ok(LoadedScriptExecutionOutcome::CompletedModuleGraph(graph))
            }
            ModuleScriptExecutionOutcome::SuspendedModuleFetches(continuation) => Ok(
                LoadedScriptExecutionOutcome::SuspendedModuleFetches(continuation),
            ),
        }
    }

    pub(crate) async fn execute_loaded_prepared_script_source(
        &mut self,
        script: &PreparedScript,
        source: &str,
        source_bytes: Option<&[u8]>,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        let inline_source = if script.source_kind == ScriptSourceKind::Inline {
            let request = self.content_security_policy_script_element_request(script);
            let Some(source) =
                self.inline_script_element_source_for_execution(script.node_id, source, request)
            else {
                return Ok(LoadedScriptExecutionOutcome::Completed(
                    PreparedScriptBodyActivity::NotEntered,
                ));
            };
            Some(source)
        } else {
            None
        };
        let source = inline_source.as_deref().unwrap_or(source);
        let selector_before = self.document_runtime.selector_debug_snapshot();
        let started = Instant::now();
        debug!(url = %script.url, "execute_prepared_script_once executing source");
        self.script_execution_memory.record(script, source.len());
        Self::reset_dom_binding_trace_window();
        let dom_binding_source_started =
            moli_trace::dom_binding_timing_enabled().then(Instant::now);
        let completion_owner = self.completion_owner_for_prepared_module_script(script);
        let result = self
            .execute_loaded_script(
                script.kind,
                script.source_kind,
                source,
                source_bytes,
                &script.url,
                &script.base_url,
                &script.initiator_url,
                script.node_id,
                &script.fetch_metadata,
                completion_owner,
            )
            .await;
        if let Some(started) = dom_binding_source_started {
            Self::emit_dom_binding_trace_window(
                "renderer_prepared_script_dom_binding_summary",
                "source_eval",
                Some(&script.url),
                started.elapsed(),
            );
        }
        let selector_after = self.document_runtime.selector_debug_snapshot();
        debug!(
            url = %script.url,
            result = ?result.as_ref().map(|_| ()).map_err(|error| error.message()),
            elapsed_ms = started.elapsed().as_millis(),
            source_len = source.len(),
            kind = ?script.kind,
            mode = ?script.mode,
            source_kind = ?script.source_kind,
            query_selector_delta = selector_after.query_selector.saturating_sub(selector_before.query_selector),
            query_selector_all_delta = selector_after.query_selector_all.saturating_sub(selector_before.query_selector_all),
            matches_delta = selector_after.matches.saturating_sub(selector_before.matches),
            closest_delta = selector_after.closest.saturating_sub(selector_before.closest),
            "execute_prepared_script_once finished"
        );
        if moli_trace::cdp_nav_timing_enabled() {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %script.url,
                result = ?result.as_ref().map(|_| ()).map_err(|error| error.message()),
                elapsed_ms = started.elapsed().as_millis(),
                source_len = source.len(),
                kind = ?script.kind,
                mode = ?script.mode,
                source_kind = ?script.source_kind,
                query_selector_delta = selector_after
                    .query_selector
                    .saturating_sub(selector_before.query_selector),
                query_selector_all_delta = selector_after
                    .query_selector_all
                    .saturating_sub(selector_before.query_selector_all),
                matches_delta = selector_after.matches.saturating_sub(selector_before.matches),
                closest_delta = selector_after.closest.saturating_sub(selector_before.closest),
                stage = "renderer_prepared_script_source_done",
            );
        }
        result
    }

    pub(super) async fn execute_loaded_script(
        &mut self,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        source: &str,
        source_bytes: Option<&[u8]>,
        script_url: &Url,
        script_base_url: &Url,
        initiator_url: &Url,
        script_node_id: NodeId,
        fetch_metadata: &crate::planning::ScriptFetchMetadata,
        completion_owner: ModuleScriptCompletionOwner,
    ) -> std::result::Result<LoadedScriptExecutionOutcome, PreparedScriptExecutionError> {
        match kind {
            ScriptKind::Classic => {
                let provenance =
                    CompiledStringProvenance::new(script_url.clone(), script_base_url.clone());
                let result = self.exec_in_enclosing_script_turn_with_provenance(
                    source,
                    &provenance,
                    if self
                        .document_runtime
                        .parser_script_start_line(script_node_id)
                        .is_some_and(|line| line > 1)
                        && script_url.as_str() == self.document_runtime.document_url().as_str()
                    {
                        self.document_runtime
                            .parser_script_start_line(script_node_id)
                            .map(|line| line.saturating_sub(1).min(i32::MAX as u64) as i32)
                            .unwrap_or(0)
                    } else {
                        0
                    },
                    fetch_metadata.nonce.as_deref(),
                    true,
                );
                match result {
                    Ok(()) => Ok(LoadedScriptExecutionOutcome::Completed(
                        PreparedScriptBodyActivity::Entered,
                    )),
                    Err(eval_exec::RawScriptExecutionError::Exception { report, .. }) => {
                        self.report_classic_script_exception_and_finish_evaluation_best_effort(
                            &report,
                        );
                        Ok(LoadedScriptExecutionOutcome::Completed(
                            PreparedScriptBodyActivity::Entered,
                        ))
                    }
                    Err(error) => Err(PreparedScriptExecutionError::from_entered_script_message(
                        error.into_anyhow().to_string(),
                    )),
                }
            }
            ScriptKind::ImportMap => {
                let _ = script_node_id;
                register_import_map_source(self, source)
                    .map_err(PreparedScriptExecutionError::from_message)?;
                Ok(LoadedScriptExecutionOutcome::Completed(
                    PreparedScriptBodyActivity::NotEntered,
                ))
            }
            ScriptKind::Module => {
                let module_source =
                    module_script_source_for_execution(script_url, source, source_bytes)
                        .map_err(PreparedScriptExecutionError::from_message)?;
                match execute_module_script_source(
                    self,
                    module_source,
                    script_base_url,
                    initiator_url,
                    fetch_metadata,
                    source_kind == ScriptSourceKind::External,
                    completion_owner,
                )
                .await
                .map_err(PreparedScriptExecutionError::from_module_load_error)?
                {
                    ModuleScriptExecutionOutcome::CompletedModuleGraph(graph) => {
                        Ok(LoadedScriptExecutionOutcome::CompletedModuleGraph(graph))
                    }
                    ModuleScriptExecutionOutcome::SuspendedModuleFetches(continuation) => Ok(
                        LoadedScriptExecutionOutcome::SuspendedModuleFetches(continuation),
                    ),
                }
            }
            ScriptKind::DataBlock => Err(PreparedScriptExecutionError::from_message(
                "data block should have been skipped",
            )),
        }
    }

    pub(crate) fn inline_script_element_source_for_execution(
        &mut self,
        node_id: DomHandle,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<String> {
        let context_host = self._context_host.clone();
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_runtime.context;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: context_ptr points to self.page_default_runtime.context, which
                // remains live for this non-escaping isolate closure.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                Ok(
                    crate::native_bridge::element::inline_script_source_for_execution(
                        scope, host_ptr, node_id, source, request,
                    ),
                )
            })
            .ok()
            .flatten()
    }
}
