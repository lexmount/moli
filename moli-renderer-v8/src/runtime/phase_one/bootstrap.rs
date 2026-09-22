use super::super::script_preloads::{ServiceWorkerScriptPreloadContext, admit_pending_preloads};
use super::scaffold::finish_phase_one_creation_on_execution_context;
use super::*;

impl ConcurrentParseTimeRuntime {
    pub(in crate::runtime) async fn continue_creation_from_phase_one_runtime(
        self,
        started: Instant,
    ) -> Result<ParseTimePageVmCreationOutcome> {
        finish_phase_one_creation_on_execution_context(self, started).await
    }

    pub(super) async fn bootstrap_page_vm_from_state_on_fresh_local_task(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: ResourceRequestClient,
        env: PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        mut state: ParseTimeDriverState,
        started: Instant,
        closed_message: &'static str,
    ) -> Result<(ParseTimeDriverState, PageVm, bool)> {
        let bootstrap_executor = local_executor.clone();
        let bootstrap = Box::pin(async move {
            let (page_vm, triggered_navigation) = {
                let buffered_document_preloads = &mut state.buffered_document_preloads;
                let service_worker_preload_context = state.service_worker_preload_context.as_ref();
                PageVm::new_from_parser_stream_and_run_document_start(
                    page_id,
                    local_executor,
                    &loader,
                    &env,
                    runtime_hooks,
                    &mut state.parser_session,
                    started,
                    |page_vm| {
                        admit_pending_preloads(
                            page_vm,
                            buffered_document_preloads,
                            &loader,
                            service_worker_preload_context,
                        );
                        Ok(())
                    },
                )?
            };
            Ok((state, page_vm, triggered_navigation))
        });
        PageVm::run_bootstrap_future_on_fresh_local_task(
            bootstrap_executor,
            closed_message,
            bootstrap,
        )
        .await
    }

    pub(in crate::runtime) async fn finish_creation_from_html_bootstrap(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        final_url: Url,
        stage: PageVmInitStage,
        html: String,
        started: Instant,
    ) -> Result<ParseTimePageVmCreationOutcome> {
        let mut state = ParseTimeDriverState::new_with_scripting_enabled(
            final_url.clone(),
            main_document_parser_scripting_enabled(env),
        );
        state
            .buffered_document_preloads
            .set_script_fetch_requires_owner_admission(script_preloads_require_owner_admission(
                env,
            ));
        state
            .buffered_document_preloads
            .set_response_csp_requires_parser_admission(
                !env.document_policy_container
                    .response_content_security_policies
                    .is_empty(),
            );
        state.buffered_document_preloads.bind_resource_runtime(
            runtime_hooks.owner_wake(),
            runtime_hooks.resource_task_runner(),
        );
        let service_worker_preload_context =
            env.reserved_service_worker_client_id.map(|client_id| {
                ServiceWorkerScriptPreloadContext::new(
                    runtime_hooks.browser_context_runtime.clone(),
                    client_id,
                    final_url.clone(),
                    runtime_hooks.owner_wake(),
                )
            });
        state.service_worker_preload_context = service_worker_preload_context.clone();
        state
            .buffered_document_preloads
            .append_to_main_document_scan_with_service_worker_context(
                &final_url,
                &html,
                loader,
                service_worker_preload_context.as_ref(),
            );
        state.parser_session.queue_arrived_chunk(html);
        state.close_input();
        let (state, page_vm, triggered_navigation) =
            Self::bootstrap_page_vm_from_state_on_fresh_local_task(
                page_id,
                local_executor.clone(),
                loader.clone(),
                env.clone(),
                runtime_hooks,
                state,
                started,
                "html bootstrap local task channel closed",
            )
            .await?;
        if triggered_navigation {
            return Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage });
        }
        // Keep the current phase-1 parse/runtime hand-off in one explicit runtime object.
        // That makes the control flow easier to evolve than leaving the parser stream, ready
        // queue drain, and DOM resynchronization as three unrelated helper calls in `from_html`.
        finish_phase_one_creation_on_execution_context(
            Self::new_parser_owner(loader.clone(), stage, state, page_vm),
            started,
        )
        .await
    }

    pub(in crate::runtime) async fn finish_creation_from_xml_bootstrap(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        final_url: Url,
        document_content_type: String,
        stage: PageVmInitStage,
        source: String,
        started: Instant,
    ) -> Result<ParseTimePageVmCreationOutcome> {
        let mut state = ParseTimeDriverState::new_xml(final_url.clone());
        state
            .parser_session
            .set_xml_document_content_type(document_content_type);
        state
            .buffered_document_preloads
            .set_script_fetch_requires_owner_admission(script_preloads_require_owner_admission(
                env,
            ));
        state
            .buffered_document_preloads
            .set_response_csp_requires_parser_admission(
                !env.document_policy_container
                    .response_content_security_policies
                    .is_empty(),
            );
        state.buffered_document_preloads.bind_resource_runtime(
            runtime_hooks.owner_wake(),
            runtime_hooks.resource_task_runner(),
        );
        let service_worker_preload_context =
            env.reserved_service_worker_client_id.map(|client_id| {
                ServiceWorkerScriptPreloadContext::new(
                    runtime_hooks.browser_context_runtime.clone(),
                    client_id,
                    final_url.clone(),
                    runtime_hooks.owner_wake(),
                )
            });
        state.service_worker_preload_context = service_worker_preload_context.clone();
        state
            .buffered_document_preloads
            .append_to_main_document_scan_with_service_worker_context(
                &final_url,
                &source,
                loader,
                service_worker_preload_context.as_ref(),
            );
        state.parser_session.queue_arrived_chunk(source);
        state.parser_session.declare_eof();
        state.close_input();
        let (state, page_vm, triggered_navigation) =
            Self::bootstrap_page_vm_from_state_on_fresh_local_task(
                page_id,
                local_executor.clone(),
                loader.clone(),
                env.clone(),
                runtime_hooks,
                state,
                started,
                "XML bootstrap local task channel closed",
            )
            .await?;
        if triggered_navigation {
            return Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage });
        }
        finish_phase_one_creation_on_execution_context(
            Self::new_parser_owner(loader.clone(), stage, state, page_vm),
            started,
        )
        .await
    }
}
