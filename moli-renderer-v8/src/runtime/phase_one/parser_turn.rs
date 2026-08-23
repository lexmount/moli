use super::parser_blocking_execution::{
    MainParserBlockingExecutionOutcome, resolve_main_parser_blocking_classic_after_runtime_gate,
};
use super::parser_blocking_pending::{
    PendingParsingBlockingClassicScriptRunner, main_parser_blocking_classic_script_item,
};
use super::parser_blocking_source::{
    MainParserBlockingSourceDisposition, apply_pending_parser_blocking_source_load_if_ready,
    prepare_main_parser_blocking_source_load,
    record_main_parser_blocking_applied_preload_network_result,
};
use super::parser_blocking_task::{
    PendingParsingBlockingClassicScriptBlockedOnExecution,
    PendingParsingBlockingClassicScriptBlockedOnSourceLoad,
    source_load_blocked_main_parser_blocking_classic_script,
    stylesheet_blocked_main_parser_blocking_classic_script,
};
use super::*;
use crate::document_runtime::parser_script_preparation_failure_page_owned_work;
use crate::dom::native::{Attribute, DomMutationEffects, NativeNodeId};
use crate::live_document_parser::{
    LiveDocumentParserOwner, LiveDocumentParserStepOutcome, ParserSuspensionCause,
};
use crate::parser::{
    ParserDomMutation, ParserDomMutationConsumer, ParserDomReadConsumer,
    ParserElementCreationConsumer, ParserElementCreationRequest, ParserMutationEffectConsumer,
    ParserScriptHandoff,
};
use crate::parser_script::payload::{ParserClassicScriptMetadata, ParserPreparedClassicScript};
use crate::planning::PreparedScript;
use html5ever::tree_builder::QuirksMode;

struct PhaseOneParserOwner<'a> {
    vm: &'a mut ScriptVm,
}

impl LiveDocumentParserOwner for PhaseOneParserOwner<'_> {}

impl ParserMutationEffectConsumer for PhaseOneParserOwner<'_> {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
        let _ = self
            .vm
            .apply_parser_stream_mutation_effects_to_live_dom_host_in_default_context(effects);
    }
}

impl ParserDomReadConsumer for PhaseOneParserOwner<'_> {
    fn snapshot_parser_document(&mut self) -> Option<crate::dom::native::NativeDom> {
        Some(self.vm.document_runtime.snapshot_document())
    }

    fn node_exists(&mut self, node_id: NativeNodeId) -> bool {
        self.vm
            .document_runtime
            .parser_runtime_dom_node_exists(node_id)
    }

    fn is_connected(&mut self, node_id: NativeNodeId) -> bool {
        self.vm
            .document_runtime
            .parser_runtime_dom_is_connected(node_id)
    }

    fn is_text_node(&mut self, node_id: NativeNodeId) -> bool {
        self.vm
            .document_runtime
            .parser_runtime_dom_is_text_node(node_id)
    }

    fn owner_document(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_owner_document(node_id)
    }

    fn parent_node(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_parent_node(node_id)
    }

    fn previous_sibling(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_previous_sibling(node_id)
    }

    fn last_child(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_last_child(node_id)
    }

    fn child_handles(&mut self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_child_handles(node_id)
    }

    fn document_order_script_handles(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Vec<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_document_order_script_handles(document_handle)
    }

    fn document_order_stylesheet_candidate_handles_before(
        &mut self,
        document_handle: NativeNodeId,
        stop_at: Option<NativeNodeId>,
    ) -> Vec<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_document_order_stylesheet_candidate_handles_before(
                document_handle,
                stop_at,
            )
    }

    fn document_body_handle_for_document(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Option<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_document_body_handle_for_document(document_handle)
    }

    fn document_base_url(&mut self, document_handle: NativeNodeId) -> Option<url::Url> {
        self.vm
            .document_runtime
            .parser_runtime_dom_document_base_url(document_handle)
    }

    fn template_contents_handle(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.vm
            .document_runtime
            .parser_runtime_dom_template_contents_handle(node_id)
    }

    fn is_html_element_named(&mut self, node_id: NativeNodeId, local_name: &str) -> bool {
        self.vm
            .document_runtime
            .parser_runtime_dom_is_html_element_named(node_id, local_name)
    }

    fn is_external_async_classic_candidate(&mut self, node_id: NativeNodeId) -> bool {
        self.vm
            .document_runtime
            .parser_runtime_dom_is_external_async_classic_candidate(node_id)
    }

    fn parser_script_read(
        &mut self,
        node_id: NativeNodeId,
    ) -> Option<crate::planning::ParserScriptRead> {
        self.vm
            .document_runtime
            .parser_runtime_dom_parser_script_read(node_id)
    }

    fn stylesheet_element(
        &mut self,
        node_id: NativeNodeId,
    ) -> Option<crate::StylesheetElementRead> {
        self.vm
            .document_runtime
            .parser_runtime_dom_stylesheet_element(node_id)
    }

    fn text_content(&mut self, node_id: NativeNodeId) -> Option<String> {
        self.vm
            .document_runtime
            .parser_runtime_dom_text_content(node_id)
    }
}

impl ParserDomMutationConsumer for PhaseOneParserOwner<'_> {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) {
        let _ = self
            .vm
            .apply_parser_dom_mutation_to_live_dom_host_in_default_context(mutation);
        let _ = self
            .vm
            .run_pending_parser_post_step_runtime_work_in_default_context();
    }

    fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_parser_element_without_attributes_in_live_dom_host(
                local_name, namespace, prefix,
            )
    }

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        document_handle: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_parser_element_for_document_without_attributes_in_live_dom_host(
                document_handle,
                local_name,
                namespace,
                prefix,
            )
    }

    fn add_attrs_if_missing_for_parser(&mut self, node_id: NativeNodeId, attrs: Vec<Attribute>) {
        self.vm
            .document_runtime
            .add_attrs_if_missing_for_parser_in_live_dom_host(node_id, attrs);
    }

    fn create_text_node(&mut self, text: String) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_text_node_in_live_dom_host(text)
    }

    fn create_comment(&mut self, text: String) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_comment_in_live_dom_host(text)
    }

    fn create_processing_instruction(&mut self, target: String, data: String) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_processing_instruction_in_live_dom_host(target, data)
    }

    fn create_cdata_section(&mut self, data: String) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_cdata_section_in_live_dom_host(data)
    }

    fn create_document_type(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        self.vm
            .document_runtime
            .create_document_type_in_live_dom_host(name, public_id, system_id)
    }

    fn prepend_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        self.vm
            .document_runtime
            .prepend_text_to_text_node_in_live_dom_host(node_id, text);
    }

    fn append_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        self.vm
            .document_runtime
            .append_text_to_text_node_in_live_dom_host(node_id, text);
    }

    fn push_parse_error(&mut self, error: String) {
        self.vm
            .document_runtime
            .push_parse_error_in_live_dom_host(error);
    }

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode) {
        self.vm
            .document_runtime
            .set_html_quirks_mode_for_parser_in_live_dom_host(quirks_mode);
    }

    fn mark_script_already_started_for_parser(&mut self, node_id: NativeNodeId) {
        self.vm
            .document_runtime
            .mark_script_already_started_for_parser_in_live_dom_host(node_id);
    }

    fn finish_parsing_script_children(&mut self, node_id: NativeNodeId) {
        let _ = self
            .vm
            .document_runtime
            .dom_host_mut()
            .finish_parsing_script_children(node_id);
    }

    fn finish_parsing_link_children(&mut self, node_id: NativeNodeId) {
        let _ = self
            .vm
            .document_runtime
            .dom_host_mut()
            .finish_parsing_link_children(node_id);
    }

    fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: Vec<Attribute>,
    ) -> bool {
        self.vm
            .document_runtime
            .attach_declarative_shadow_for_parser_in_live_dom_host(host_id, template_id, attrs)
    }

    fn associate_parser_form_owner(&mut self, target: NativeNodeId, form: NativeNodeId) -> bool {
        self.vm
            .document_runtime
            .associate_parser_form_owner_in_live_dom_host(target, form)
    }
}

impl ParserElementCreationConsumer for PhaseOneParserOwner<'_> {
    fn create_parser_element(
        &mut self,
        request: ParserElementCreationRequest<'_>,
    ) -> Option<NativeNodeId> {
        let document_has_body = self
            .document_body_handle_for_document(request.document_handle)
            .is_some();
        self.vm
            .create_and_construct_parser_custom_element_direct_in_default_context(
                request.document_handle,
                document_has_body,
                request.local_name,
                request.namespace,
                request.prefix,
                request.attributes,
                request.intended_parent,
            )
            .ok()
            .flatten()
    }
}

pub(super) enum PageTaskTurnResult {
    /// No parse-visible task was runnable.
    NoTask,
    /// One parse-visible task executed in this phase-one owner turn.
    ExecutedTask,
    /// A concrete task is resident in a stable Page source. Phase one must
    /// return the checked-out PageVm to its owner slot so the common Page
    /// scheduler can select it; phase one has no dequeue authority of its own.
    BlockedOnPageTask,
    /// Script execution stopped or replaced the current parser Document.
    StoppedCurrentDocument,
}

pub(super) enum ScriptHandoffOutcome {
    NoNavigation,
    StoppedCurrentDocument,
    BlockedOnStylesheet(Box<PendingParsingBlockingClassicScriptBlockedOnExecution>),
    BlockedOnDocumentWriteExternalLoad,
    BlockedOnExternalSource(Box<PendingParsingBlockingClassicScriptBlockedOnSourceLoad>),
}

pub(super) enum ParserStepAdvanceOutcome {
    Continue,
    StoppedCurrentDocument,
    BlockedOnStylesheet(Box<PendingParsingBlockingClassicScriptBlockedOnExecution>),
    BlockedOnStylesheetParserPause,
    BlockedOnDocumentWriteExternalLoad,
    BlockedOnExternalSource(Box<PendingParsingBlockingClassicScriptBlockedOnSourceLoad>),
}

fn suspend_parser_for_stylesheet_page_task(
    owner: &mut ParseTimeOwner,
    pending_wait: &mut PendingParsingBlockingWait,
) -> OwnerStepProgress {
    *owner = ParseTimeOwner::Document;
    *pending_wait = PendingParsingBlockingWait::PageTaskBlockingStylesheet;
    OwnerStepProgress::BlockedOnPageTask
}

fn script_handoff_from_main_parser_blocking_execution(
    outcome: MainParserBlockingExecutionOutcome,
) -> ScriptHandoffOutcome {
    match outcome {
        MainParserBlockingExecutionOutcome::NoNavigation => ScriptHandoffOutcome::NoNavigation,
        MainParserBlockingExecutionOutcome::StoppedCurrentDocument => {
            ScriptHandoffOutcome::StoppedCurrentDocument
        }
        MainParserBlockingExecutionOutcome::BlockedOnStylesheet(pending) => {
            ScriptHandoffOutcome::BlockedOnStylesheet(pending)
        }
        MainParserBlockingExecutionOutcome::BlockedOnDocumentWriteExternalLoad => {
            ScriptHandoffOutcome::BlockedOnDocumentWriteExternalLoad
        }
    }
}

pub(super) fn bind_parser_owned_script_handle(page_vm: &mut PageVm, script: &mut PreparedScript) {
    if script.host_script_handle.is_some() {
        return;
    }
    let handle = page_vm
        .vm_mut()
        .document_runtime
        .bind_parser_owned_script_handle_for_node(script.node_id);
    page_vm
        .vm_mut()
        .document_runtime
        .set_script_handle_followup_lane(
            &handle,
            crate::host::HostScriptScheduler::followup_lane_for_script(
                crate::host::ScriptHandleSource::ParserOwned,
                script.mode,
            ),
        );
    script.host_script_handle = Some(handle);
}

pub(super) struct ParserDriver<'loader, 'state> {
    pub(super) loader: &'loader ResourceRequestClient,
    pub(super) final_url: &'state Url,
    pub(super) parser_session: &'state mut DocumentParserSession,
    pub(super) scheduler: &'state mut DocumentScriptScheduler,
    pub(super) pending_parsing_blocking_script:
        &'state mut PendingParsingBlockingClassicScriptRunner,
    pub(super) buffered_document_preloads: &'state mut BufferedDocumentPreloadState,
    pub(super) service_worker_preload_context: Option<&'state ServiceWorkerScriptPreloadContext>,
    pub(super) input_closed: &'state bool,
}

impl<'loader, 'state> ParserDriver<'loader, 'state> {
    /// Bind parser-side speculative loads to the same executor and wake route
    /// as the `PageVm` that owns this parser turn.
    ///
    /// A `ParseTimeDriverState` can exist before the V8 realm is bootstrapped,
    /// so its preload scanner cannot manufacture an ambient executor. Once a
    /// parser operation has a live `PageVm`, that page is the authoritative
    /// source of runtime wiring. Rebinding here also keeps low-level parser
    /// fixtures on the production path: they must construct a real Page
    /// residence, and no inert or rejecting runner is substituted.
    fn bind_page_resource_runtime(&mut self, page_vm: &PageVm) {
        self.buffered_document_preloads.bind_resource_runtime(
            page_vm.runtime_hooks.owner_wake(),
            page_vm.runtime_hooks.resource_task_runner(),
        );
    }

    fn sync_parser_defined_custom_elements(&mut self, page_vm: &mut PageVm) {
        let names = page_vm
            .vm_mut()
            .drain_parser_defined_autonomous_custom_elements();
        if names.is_empty() {
            return;
        }
        self.parser_session
            .note_defined_autonomous_custom_elements(names);
    }

    pub(super) fn finish_parser_blocking_pause(&mut self) {
        while let Some(html) = self.parser_session.take_next_insertion_preload_input() {
            self.buffered_document_preloads
                .append_to_insertion_scan_with_service_worker_context(
                    self.final_url,
                    &html,
                    self.loader,
                    self.service_worker_preload_context,
                );
        }
        self.buffered_document_preloads
            .note_parser_processed_meta_csp(
                self.parser_session
                    .take_processed_insertion_meta_csp_count(),
            );
        self.buffered_document_preloads.reset_insertion_scan();
    }

    pub(super) async fn drive_owner_step(
        &mut self,
        owner: &mut ParseTimeOwner,
        parser_step_ready: &mut bool,
        pending_parsing_blocking_wait: &mut PendingParsingBlockingWait,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        page_vm: &mut PageVm,
    ) -> Result<OwnerStepProgress> {
        if page_vm.vm().current_main_document_task_owner() != Some(parser_document_owner) {
            tracing::debug!(
                ?parser_document_owner,
                current_owner = ?page_vm.vm().current_main_document_task_owner(),
                "stopping stale main parser owner before its next parser turn"
            );
            return Ok(owner_step_progress_after_current_document_stop(page_vm));
        }
        match self.parser_session.suspension_cause() {
            Some(ParserSuspensionCause::ParserCreatedStylesheet { .. }) => {
                if !page_vm
                    .vm()
                    .document_runtime
                    .has_all_blocking_stylesheets_resolved()
                {
                    return Ok(suspend_parser_for_stylesheet_page_task(
                        owner,
                        pending_parsing_blocking_wait,
                    ));
                }
                let permit = self
                    .parser_session
                    .current_resume_permit()
                    .expect("a stylesheet-suspended parser must retain its resume permit");
                assert!(
                    self.parser_session.resume(permit),
                    "the admitted stylesheet continuation must resume its exact parser suspension"
                );
            }
            Some(ParserSuspensionCause::DocumentWriteExternalScript { .. }) => {
                if page_vm
                    .vm()
                    .document_runtime
                    .has_pending_document_write_external_script_load()
                {
                    *owner = ParseTimeOwner::Document;
                    *pending_parsing_blocking_wait =
                        PendingParsingBlockingWait::PageNetworkingDocumentWriteExternalScript;
                    return Ok(OwnerStepProgress::BlockedOnPageTask);
                }
                let permit = self
                    .parser_session
                    .current_resume_permit()
                    .expect("a document.write-suspended parser must retain its resume permit");
                assert!(
                    self.parser_session.resume(permit),
                    "the admitted document.write continuation must resume its exact parser suspension"
                );
            }
            Some(
                ParserSuspensionCause::ParserClassicSource { .. }
                | ParserSuspensionCause::ParserClassicStylesheets { .. },
            )
            | None => {}
        }
        debug_assert!(
            !pending_parsing_blocking_wait.is_pending(),
            "parser owner should not enqueue a new parsing turn while a blocking-wait document turn is still pending"
        );
        if !self
            .pending_parsing_blocking_script
            .has_parser_blocking_script()
        {
            if self.parser_session.input_is_empty() {
                if *self.input_closed {
                    return Ok(self.finish_main_parser_or_defer_borrowed_stream(
                        page_vm,
                        parser_document_owner,
                    ));
                }
                return Ok(OwnerStepProgress::NeedMoreInput);
            }
            if !*parser_step_ready {
                *owner = ParseTimeOwner::Document;
                return Ok(OwnerStepProgress::Continue);
            }
        }
        if self
            .pending_parsing_blocking_script
            .has_parser_blocking_script()
        {
            let parser_blocking_script_handle = self
                .pending_parsing_blocking_script
                .current_parser_blocking_script_handle();
            let preload_candidate = self
                .pending_parsing_blocking_script
                .current_parser_blocking_script()
                .filter(|pending| pending.context().source_load.is_none())
                .and_then(|pending| pending.runner_script().cloned());
            let (applied_preload, prepared_script) = if let Some(mut script) = preload_candidate {
                let applied = self
                    .buffered_document_preloads
                    .apply_preloaded_source_to_script_if_available(&mut script, false)
                    .await;
                (applied, Some(script))
            } else {
                (None, None)
            };
            if let Some(network_result) = applied_preload
                .as_ref()
                .and_then(|applied| applied.network_result.as_deref())
                && let Some(script) = prepared_script.as_ref()
            {
                page_vm.vm_mut().record_script_subresource_network_result(
                    script.initiator_url.clone(),
                    script.url.clone(),
                    network_result,
                );
            }
            if let (Some(script_handle), Some(prepared_script)) =
                (parser_blocking_script_handle, prepared_script)
                && applied_preload.is_some()
            {
                debug_assert!(
                    self.pending_parsing_blocking_script
                        .apply_current_parser_blocking_preloaded_script(
                            script_handle,
                            prepared_script,
                        ),
                    "a parser-blocking preload must complete the same external pending script"
                );
            }
            if !apply_pending_parser_blocking_source_load_if_ready(
                page_vm,
                self.pending_parsing_blocking_script,
            ) {
                return Ok(OwnerStepProgress::BlockedOnParserScriptSourceLoad);
            }
            let progress = match resolve_main_parser_blocking_classic_after_runtime_gate(
                self.parser_session,
                page_vm,
                self.pending_parsing_blocking_script,
                "executing stylesheet-unblocked parser-blocking classic script",
            )
            .await?
            {
                MainParserBlockingExecutionOutcome::BlockedOnStylesheet(pending) => {
                    self.pending_parsing_blocking_script
                        .install_parser_blocking_script_blocked_on_execution(*pending);
                    None
                }
                MainParserBlockingExecutionOutcome::StoppedCurrentDocument => {
                    Some(owner_step_progress_after_current_document_stop(page_vm))
                }
                MainParserBlockingExecutionOutcome::NoNavigation => {
                    Some(OwnerStepProgress::Continue)
                }
                MainParserBlockingExecutionOutcome::BlockedOnDocumentWriteExternalLoad => {
                    self.finish_parser_blocking_pause();
                    if let Some(script_handle) = parser_blocking_script_handle {
                        self.pending_parsing_blocking_script
                            .discard_current_parser_blocking_script_if_handle(script_handle);
                    }
                    *pending_parsing_blocking_wait = if page_vm.has_page_resource_completion_route()
                    {
                        PendingParsingBlockingWait::PageNetworkingDocumentWriteExternalScript
                    } else {
                        PendingParsingBlockingWait::LegacyDocumentProcessing
                    };
                    *owner = ParseTimeOwner::Document;
                    return Ok(if pending_parsing_blocking_wait.waits_for_page_task() {
                        OwnerStepProgress::BlockedOnPageTask
                    } else {
                        OwnerStepProgress::Continue
                    });
                }
            };
            if let Some(progress) = progress {
                self.finish_parser_blocking_pause();
                if let Some(script_handle) = parser_blocking_script_handle {
                    self.pending_parsing_blocking_script
                        .discard_current_parser_blocking_script_if_handle(script_handle);
                }
                return Ok(progress);
            }
            return Ok(suspend_parser_for_stylesheet_page_task(
                owner,
                pending_parsing_blocking_wait,
            ));
        }
        let default_chunk_bytes = self.parser_session.current_chunk_len();
        let ParseTimeTurn {
            parser_step_bytes,
            ready_task,
        } = self
            .scheduler
            .parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes,
            });
        *parser_step_ready = false;
        let progress = if self.parser_session.input_is_empty() {
            if *self.input_closed {
                self.finish_main_parser_or_defer_borrowed_stream(page_vm, parser_document_owner)
            } else {
                OwnerStepProgress::NeedMoreInput
            }
        } else {
            let parser_step_bytes =
                parser_step_bytes.expect("BeforeParserStep turn must retain parser step size");
            let parser_budget_exhausted =
                parser_step_bytes > 0 && default_chunk_bytes > parser_step_bytes;
            page_vm
                .page_task_queue
                .enqueue_parse_time_document_script_task(ready_task);

            match self
                .advance_next_parser_step_for_owner(
                    page_vm,
                    parser_document_owner,
                    parser_step_bytes,
                )
                .await?
            {
                ParserStepAdvanceOutcome::Continue => {
                    if !parser_budget_exhausted {
                        // This bounded step consumed the complete current
                        // chunk. The owner loop can immediately observe
                        // EOF, another already-buffered chunk, or the next
                        // real wait without manufacturing a continuation.
                        OwnerStepProgress::Continue
                    } else if page_vm
                        .vm()
                        .document_runtime
                        .request_main_parser_continuation_if_active()
                    {
                        // One bounded tokenizer turn has spent its parser
                        // budget. The exact parser state remains in this
                        // runtime; the Networking task only grants the next
                        // opportunity through the common Page scheduler.
                        OwnerStepProgress::BlockedOnPageTask
                    } else {
                        #[cfg(test)]
                        {
                            assert!(
                                page_vm.permits_direct_parser_budget_continuation_for_test(),
                                "only an explicit standalone parser fixture may bypass the Networking continuation"
                            );
                            OwnerStepProgress::Continue
                        }
                        #[cfg(not(test))]
                        {
                            panic!(
                                "an active production parser spent its budget without a bound Networking continuation"
                            );
                        }
                    }
                }
                ParserStepAdvanceOutcome::StoppedCurrentDocument => {
                    owner_step_progress_after_current_document_stop(page_vm)
                }
                ParserStepAdvanceOutcome::BlockedOnStylesheet(script) => {
                    self.pending_parsing_blocking_script
                        .install_parser_blocking_script_blocked_on_execution(*script);
                    suspend_parser_for_stylesheet_page_task(owner, pending_parsing_blocking_wait)
                }
                ParserStepAdvanceOutcome::BlockedOnStylesheetParserPause => {
                    suspend_parser_for_stylesheet_page_task(owner, pending_parsing_blocking_wait)
                }
                ParserStepAdvanceOutcome::BlockedOnExternalSource(script) => {
                    self.pending_parsing_blocking_script
                        .install_parser_blocking_script_blocked_on_source_load(*script);
                    *pending_parsing_blocking_wait = PendingParsingBlockingWait::None;
                    *owner = ParseTimeOwner::Parser;
                    OwnerStepProgress::NeedMoreInput
                }
                ParserStepAdvanceOutcome::BlockedOnDocumentWriteExternalLoad => {
                    *pending_parsing_blocking_wait = if page_vm.has_page_resource_completion_route()
                    {
                        PendingParsingBlockingWait::PageNetworkingDocumentWriteExternalScript
                    } else {
                        PendingParsingBlockingWait::LegacyDocumentProcessing
                    };
                    *owner = ParseTimeOwner::Document;
                    if pending_parsing_blocking_wait.waits_for_page_task() {
                        OwnerStepProgress::BlockedOnPageTask
                    } else {
                        OwnerStepProgress::Continue
                    }
                }
            }
        };
        Ok(progress)
    }

    fn catch_up_main_document_preload_scan(
        &mut self,
        page_vm: &mut PageVm,
        final_url: &Url,
        parser_tail_after_step: Option<&str>,
    ) {
        let mut upcoming_html = self.parser_session.snapshot_pending_input();
        if let Some(tail) = parser_tail_after_step {
            upcoming_html.push_str(tail);
        }
        self.buffered_document_preloads
            .catch_up_main_document_scan_if_absent(final_url, &upcoming_html, self.loader);
        super::super::script_preloads::admit_pending_preloads(
            page_vm,
            self.buffered_document_preloads,
            self.loader,
            self.service_worker_preload_context,
        );
    }

    #[cfg(test)]
    pub(super) async fn handle_parse_time_script_handoff(
        &mut self,
        page_vm: &mut PageVm,
        handoff: ParserScriptHandoff,
        parser_tail_after_step: Option<&str>,
    ) -> Result<ScriptHandoffOutcome> {
        let parser_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("parser handoff test requires an installed main document owner");
        self.handle_parse_time_script_handoff_for_owner(
            page_vm,
            parser_document_owner,
            handoff,
            parser_tail_after_step,
        )
        .await
    }

    async fn handle_parse_time_script_handoff_for_owner(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        handoff: ParserScriptHandoff,
        parser_tail_after_step: Option<&str>,
    ) -> Result<ScriptHandoffOutcome> {
        self.bind_page_resource_runtime(page_vm);
        if page_vm.vm().current_main_document_task_owner() != Some(parser_document_owner) {
            tracing::debug!(
                ?parser_document_owner,
                current_owner = ?page_vm.vm().current_main_document_task_owner(),
                "dropping stale main parser script handoff"
            );
            return Ok(ScriptHandoffOutcome::StoppedCurrentDocument);
        }
        match handoff {
            ParserScriptHandoff::BlockingClassic {
                node_id: handle,
                start_line,
                start_column,
                blocking_signatures_before,
                mut script,
            } => {
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_parser_script_start_position(handle, start_line, start_column);
                self.catch_up_main_document_preload_scan(
                    page_vm,
                    &script.initiator_url,
                    parser_tail_after_step,
                );
                self.buffered_document_preloads
                    .claim_pending_script_preload_for_parser(&script);
                let source_decision = prepare_main_parser_blocking_source_load(
                    page_vm,
                    self.loader,
                    self.buffered_document_preloads,
                    &mut script,
                );
                record_main_parser_blocking_applied_preload_network_result(
                    page_vm,
                    &script,
                    source_decision.applied_preload.as_ref(),
                );
                let source_load = match source_decision.disposition {
                    MainParserBlockingSourceDisposition::Ready => None,
                    MainParserBlockingSourceDisposition::Pending(source_load) => Some(source_load),
                    MainParserBlockingSourceDisposition::Suppressed => {
                        let _ = page_vm
                            .vm_mut()
                            .document_runtime
                            .dom_host_mut()
                            .set_script_already_started(handle, true);
                        self.finish_parser_blocking_pause();
                        return Ok(ScriptHandoffOutcome::NoNavigation);
                    }
                };
                bind_parser_owned_script_handle(page_vm, &mut script);
                let _ = page_vm
                    .vm_mut()
                    .document_runtime
                    .dom_host_mut()
                    .set_script_already_started(handle, true);
                let still_blocked_on_stylesheet = {
                    let vm = page_vm.vm_mut();
                    let still_blocked = vm
                        .document_runtime
                        .has_pending_parser_script_blocking_stylesheet_signatures(
                            blocking_signatures_before.iter(),
                        );
                    vm.record_ready_stylesheet_network_results();
                    still_blocked
                };
                if still_blocked_on_stylesheet {
                    let resume_permit = self.parser_session.suspend(
                        ParserSuspensionCause::ParserClassicStylesheets { script: handle },
                    );
                    let mut pending = main_parser_blocking_classic_script_item(
                        parser_document_owner,
                        ParserPreparedClassicScript::new(
                            ParserClassicScriptMetadata::new(handle, start_line),
                            script,
                        ),
                        blocking_signatures_before,
                        source_load,
                    );
                    pending.context_mut().install_resume_permit(resume_permit);
                    return Ok(ScriptHandoffOutcome::BlockedOnStylesheet(Box::new(
                        stylesheet_blocked_main_parser_blocking_classic_script(pending),
                    )));
                }
                if let Some(source_load) = source_load {
                    let resume_permit = self
                        .parser_session
                        .suspend(ParserSuspensionCause::ParserClassicSource { script: handle });
                    let mut pending = main_parser_blocking_classic_script_item(
                        parser_document_owner,
                        ParserPreparedClassicScript::new(
                            ParserClassicScriptMetadata::new(handle, start_line),
                            script,
                        ),
                        blocking_signatures_before,
                        Some(source_load),
                    );
                    pending.context_mut().install_resume_permit(resume_permit);
                    return Ok(ScriptHandoffOutcome::BlockedOnExternalSource(Box::new(
                        source_load_blocked_main_parser_blocking_classic_script(pending),
                    )));
                }
                let mut pending_runner =
                    PendingParsingBlockingClassicScriptRunner::from_parser_blocking_script(
                        main_parser_blocking_classic_script_item(
                            parser_document_owner,
                            ParserPreparedClassicScript::new(
                                ParserClassicScriptMetadata::new(handle, start_line),
                                script,
                            ),
                            blocking_signatures_before,
                            None,
                        ),
                    );
                let outcome = script_handoff_from_main_parser_blocking_execution(
                    resolve_main_parser_blocking_classic_after_runtime_gate(
                        self.parser_session,
                        page_vm,
                        &mut pending_runner,
                        "executing parser-inserted classic script during parse",
                    )
                    .await?,
                );
                if !matches!(
                    outcome,
                    ScriptHandoffOutcome::BlockedOnStylesheet(_)
                        | ScriptHandoffOutcome::BlockedOnExternalSource(_)
                ) {
                    self.finish_parser_blocking_pause();
                }
                Ok(outcome)
            }
            ParserScriptHandoff::AsyncPostParse {
                node_id: handle,
                start_line,
                start_column,
                mut script,
            } => {
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_parser_script_start_position(handle, start_line, start_column);
                self.buffered_document_preloads
                    .claim_pending_script_preload_for_parser(&script);
                let shared_preload = self
                    .buffered_document_preloads
                    .shared_preload_for_script(&script);
                if let Some(applied) = self
                    .buffered_document_preloads
                    .apply_preloaded_source_to_script_if_available(&mut script, false)
                    .await
                    && let Some(network_result) = applied.network_result.as_deref()
                {
                    page_vm.vm_mut().record_script_subresource_network_result(
                        script.initiator_url.clone(),
                        script.url.clone(),
                        network_result,
                    );
                }
                bind_parser_owned_script_handle(page_vm, &mut script);
                let _ = page_vm
                    .vm_mut()
                    .document_runtime
                    .dom_host_mut()
                    .set_script_already_started(handle, true);
                if script.kind == crate::types::ScriptKind::Module {
                    let accepted = page_vm
                        .vm_mut()
                        .accept_main_parser_async_module_script(parser_document_owner, &script)?;
                    if !accepted {
                        return Err(anyhow::anyhow!(
                            "current parser owner rejected async module PendingScript `{}`",
                            script.url
                        ));
                    }
                    return Ok(ScriptHandoffOutcome::NoNavigation);
                }
                let claimed = self
                    .scheduler
                    .claim_existing_parse_time_async_handoff(NodeId::new(handle.index()));
                let document_character_set = page_vm
                    .vm()
                    .document_runtime
                    .document_character_set()
                    .to_owned();
                let resource_task_runner = page_vm.resource_task_runner();
                let accepted = claimed
                    || self
                        .scheduler
                        .recover_parse_time_async_handoff_with_load_delay_binding(
                            script,
                            self.loader,
                            resource_task_runner,
                            shared_preload,
                            Some(&document_character_set),
                            |_| {
                                page_vm
                                    .vm_mut()
                                    .accept_main_document_script_load_delay_binding(
                                        parser_document_owner,
                                        crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
                                    )
                                    .expect("current parser async handoff must bind lifecycle ownership")
                            },
                        );
                if accepted {
                    let _ = self.scheduler.grant_parse_visible_reevaluation_credit();
                }
                Ok(ScriptHandoffOutcome::NoNavigation)
            }
            ParserScriptHandoff::NonAsyncPostParse {
                node_id: handle,
                start_line,
                start_column,
                mut script,
                blocking_signatures_before,
            } => {
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_parser_script_start_position(handle, start_line, start_column);
                self.buffered_document_preloads
                    .claim_pending_script_preload_for_parser(&script);
                let shared_preload = if script.kind == crate::types::ScriptKind::Module {
                    None
                } else {
                    self.buffered_document_preloads
                        .shared_preload_for_script(&script)
                };
                if script.kind != crate::types::ScriptKind::Module
                    && let Some(applied) = self
                        .buffered_document_preloads
                        .apply_preloaded_source_to_script_if_available(&mut script, false)
                        .await
                    && let Some(network_result) = applied.network_result.as_deref()
                {
                    page_vm.vm_mut().record_script_subresource_network_result(
                        script.initiator_url.clone(),
                        script.url.clone(),
                        network_result,
                    );
                }
                bind_parser_owned_script_handle(page_vm, &mut script);
                let _ = page_vm
                    .vm_mut()
                    .document_runtime
                    .dom_host_mut()
                    .set_script_already_started(handle, true);
                if self
                    .scheduler
                    .claim_existing_parse_time_async_handoff(NodeId::new(handle.index()))
                {
                    let _ = self.scheduler.grant_parse_visible_reevaluation_credit();
                    return Ok(ScriptHandoffOutcome::NoNavigation);
                }
                match script.kind {
                    crate::types::ScriptKind::Module => {}
                    crate::types::ScriptKind::Classic => {}
                    crate::types::ScriptKind::ImportMap | crate::types::ScriptKind::DataBlock => {
                        unreachable!("non-executable script kinds use dedicated parser handoffs")
                    }
                }
                let document_character_set = page_vm
                    .vm()
                    .document_runtime
                    .document_character_set()
                    .to_owned();
                let accepted = page_vm.vm_mut().claim_main_parser_deferred_script(
                    parser_document_owner,
                    script,
                    shared_preload,
                    Some(&document_character_set),
                    blocking_signatures_before,
                )?;
                debug_assert!(
                    accepted,
                    "current parser handoff owner must accept parser-deferred preparation"
                );
                Ok(ScriptHandoffOutcome::NoNavigation)
            }
            ParserScriptHandoff::ImportMap {
                node_id: handle,
                start_line,
                start_column,
                import_map,
            } => {
                crate::module_runtime::accept_parser_owned_import_map_handoff(
                    page_vm.vm_mut(),
                    handle,
                    start_line,
                    start_column,
                    import_map,
                );
                Ok(ScriptHandoffOutcome::NoNavigation)
            }
            ParserScriptHandoff::NoExecution {
                node_id: handle,
                start_line,
                start_column,
                outcome,
            } => {
                // HTML's parser script processing performs a microtask
                // checkpoint before PrepareScript, including for data blocks
                // and other non-executable script elements. The parser crate
                // has already classified the element, but classification does
                // not run JavaScript, so this is the equivalent observable
                // boundary on the renderer owner lane.
                page_vm
                    .perform_script_task_checkpoint_on_named_owner_local_task(None)
                    .await?;
                if page_vm.vm().current_main_document_task_owner() != Some(parser_document_owner) {
                    return Ok(ScriptHandoffOutcome::StoppedCurrentDocument);
                }
                crate::host::apply_parser_script_element_state_transition(
                    page_vm.vm_mut().document_runtime.dom_host_mut(),
                    handle,
                    outcome.element_state_transition(),
                );
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_parser_script_start_position(handle, start_line, start_column);
                let claimed_parse_time_async = self
                    .scheduler
                    .claim_existing_parse_time_async_handoff(NodeId::new(handle.index()));
                if claimed_parse_time_async {
                    let _ = page_vm
                        .vm_mut()
                        .document_runtime
                        .dom_host_mut()
                        .set_script_already_started(handle, true);
                    let _ = self.scheduler.grant_parse_visible_reevaluation_credit();
                }
                if !claimed_parse_time_async && let (_, _, Some(run)) = outcome.into_parts() {
                    page_vm.report.runs.push(run);
                }
                Ok(ScriptHandoffOutcome::NoNavigation)
            }
            ParserScriptHandoff::PreparationFailure {
                node_id: handle,
                start_line,
                start_column,
                failure,
            } => {
                page_vm
                    .perform_script_task_checkpoint_on_named_owner_local_task(None)
                    .await?;
                if page_vm.vm().current_main_document_task_owner() != Some(parser_document_owner) {
                    return Ok(ScriptHandoffOutcome::StoppedCurrentDocument);
                }
                crate::host::apply_parser_script_element_state_transition(
                    page_vm.vm_mut().document_runtime.dom_host_mut(),
                    handle,
                    failure.element_state_transition(),
                );
                page_vm
                    .vm_mut()
                    .document_runtime
                    .note_parser_script_start_position(handle, start_line, start_column);
                let claimed_parse_time_async = self
                    .scheduler
                    .claim_existing_parse_time_async_handoff(NodeId::new(handle.index()));
                if claimed_parse_time_async {
                    let _ = page_vm
                        .vm_mut()
                        .document_runtime
                        .dom_host_mut()
                        .set_script_already_started(handle, true);
                    let _ = self.scheduler.grant_parse_visible_reevaluation_credit();
                } else {
                    page_vm
                        .vm_mut()
                        .document_runtime
                        .send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
                            parser_script_preparation_failure_page_owned_work(failure),
                        ]);
                }
                Ok(ScriptHandoffOutcome::NoNavigation)
            }
        }
    }

    /// Executes a single parse pump step with the runtime-owned sink protocol:
    /// 1. Borrows the runtime owner for one parser step
    /// 2. Pumps the parser; TreeSink routes structural mutations and custom-element
    ///    construction through the scoped consumer traits
    /// 3. Clears the parser-side callback bundle before releasing the owner borrow
    /// 4. Processes parser-discovery signals (async prefetch discovery, blocking stylesheets)
    ///
    /// Returns only the flow-control signal (ScriptHandoff or InputDrained).
    /// Parser-discovery signals are processed internally before returning.
    #[cfg(test)]
    pub(super) fn pump_parse_step_with_signals(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        chunk: &str,
    ) -> LiveDocumentParserStepOutcome {
        self.sync_parser_defined_custom_elements(page_vm);
        let (outcome, null_custom_element_registry_elements) =
            page_vm.vm_mut().with_dom_host_parse_step(|vm| {
                let mut parser_owner = PhaseOneParserOwner { vm };
                self.parser_session
                    .advance_step_and_take_null_custom_element_registry_elements(
                        chunk,
                        &mut parser_owner,
                    )
            });
        self.finish_parser_pump_step(
            page_vm,
            parser_document_owner,
            outcome,
            null_custom_element_registry_elements,
        )
    }

    fn pump_next_parse_step_with_signals(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        max_bytes: usize,
    ) -> LiveDocumentParserStepOutcome {
        self.sync_parser_defined_custom_elements(page_vm);
        let (outcome, null_custom_element_registry_elements) =
            page_vm.vm_mut().with_dom_host_parse_step(|vm| {
                let mut parser_owner = PhaseOneParserOwner { vm };
                let outcome = self
                    .parser_session
                    .advance_next_step(max_bytes, &mut parser_owner);
                let null_custom_element_registry_elements = self
                    .parser_session
                    .take_parser_stream_null_custom_element_registry_elements();
                (outcome, null_custom_element_registry_elements)
            });
        self.finish_parser_pump_step(
            page_vm,
            parser_document_owner,
            outcome,
            null_custom_element_registry_elements,
        )
    }

    fn pump_queued_or_resume_parse_step_with_signals(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> LiveDocumentParserStepOutcome {
        self.sync_parser_defined_custom_elements(page_vm);
        let (outcome, null_custom_element_registry_elements) =
            page_vm.vm_mut().with_dom_host_parse_step(|vm| {
                let mut parser_owner = PhaseOneParserOwner { vm };
                let outcome = self
                    .parser_session
                    .advance_queued_or_resume_step(&mut parser_owner);
                let null_custom_element_registry_elements = self
                    .parser_session
                    .take_parser_stream_null_custom_element_registry_elements();
                (outcome, null_custom_element_registry_elements)
            });
        self.finish_parser_pump_step(
            page_vm,
            parser_document_owner,
            outcome,
            null_custom_element_registry_elements,
        )
    }

    fn finish_parser_pump_step(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        outcome: LiveDocumentParserStepOutcome,
        null_custom_element_registry_elements: Vec<NativeNodeId>,
    ) -> LiveDocumentParserStepOutcome {
        let discovery_signals = self.parser_session.take_discovery_signals();
        self.apply_parser_step_outputs(
            page_vm,
            parser_document_owner,
            null_custom_element_registry_elements,
            discovery_signals,
        );
        outcome
    }

    fn finish_main_parser_or_defer_borrowed_stream(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> OwnerStepProgress {
        if !self.parser_session.has_exclusive_stream_handle() {
            // A document.write recovery turn can drive this same main stream
            // to EOF while its temporary ParserInsertionController is still
            // live on the Page-task stack. EOF is final, but destroying the
            // stream is only valid after that stack has unwound. Cross the
            // stable scheduler boundary once; the selected continuation then
            // re-enters this sole parser runtime with exclusive ownership.
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .request_main_parser_continuation_if_active(),
                "main parser EOF with a borrowed stream requires an active continuation route"
            );
            return OwnerStepProgress::BlockedOnPageTask;
        }
        self.finish_main_parser(page_vm, parser_document_owner);
        OwnerStepProgress::AdvancePhase
    }

    fn finish_main_parser(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        self.sync_parser_defined_custom_elements(page_vm);
        let finish_signals = page_vm.vm_mut().with_dom_host_parse_step(|vm| {
            let mut parser_owner = PhaseOneParserOwner { vm };
            self.parser_session.finish(&mut parser_owner)
        });
        self.apply_parser_step_outputs(
            page_vm,
            parser_document_owner,
            finish_signals.parser_created_null_registry_elements,
            finish_signals.discovery_signals,
        );
    }

    fn apply_parser_step_outputs(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        null_custom_element_registry_elements: Vec<NativeNodeId>,
        discovery_signals: crate::live_document_parser::LiveDocumentParserDiscoverySignals,
    ) {
        let _ = page_vm
            .vm_mut()
            .apply_parser_created_null_registry_associations_in_default_context(
                &null_custom_element_registry_elements,
            );
        let _ = page_vm
            .vm_mut()
            .flush_parser_custom_element_handoff_replacements();
        page_vm.vm_mut().resync_child_browsing_contexts();

        // Process parser-discovery signals after the scoped parser step ends.
        let crate::live_document_parser::LiveDocumentParserDiscoverySignals {
            async_prefetch_scripts,
            modulepreload_link_candidates,
            parser_meta_csp_candidates,
            blocking_stylesheet_inputs,
        } = discovery_signals;
        self.buffered_document_preloads
            .claim_pending_stylesheet_preloads_for_parser(&blocking_stylesheet_inputs);
        for handle in &parser_meta_csp_candidates {
            page_vm
                .vm()
                .document_runtime
                .process_parser_meta_content_security_policy(*handle);
        }
        self.buffered_document_preloads
            .note_parser_processed_meta_csp(parser_meta_csp_candidates.len());
        super::super::script_preloads::admit_pending_preloads(
            page_vm,
            self.buffered_document_preloads,
            self.loader,
            self.service_worker_preload_context,
        );
        page_vm
            .vm_mut()
            .accept_parser_discovered_native_modulepreloads(modulepreload_link_candidates);
        for mut script in async_prefetch_scripts {
            bind_parser_owned_script_handle(page_vm, &mut script);
            self.buffered_document_preloads
                .claim_pending_script_preload_for_parser(&script);
            let shared_preload = self
                .buffered_document_preloads
                .shared_preload_for_script(&script);
            let document_character_set = page_vm
                .vm()
                .document_runtime
                .document_character_set()
                .to_owned();
            let resource_task_runner = page_vm.resource_task_runner();
            let _ = self.scheduler.accept_parser_discovered_async_candidate(
                script,
                self.loader,
                resource_task_runner,
                shared_preload,
                Some(&document_character_set),
                |script| {
                    let binding = page_vm
                        .vm_mut()
                        .accept_main_document_script_load_delay_binding(
                            parser_document_owner,
                            crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
                        )
                        .expect("current parser async discovery must bind lifecycle ownership");
                    tracing::debug!(
                        ?parser_document_owner,
                        script_node_id = ?script.node_id,
                        script_url = %script.url,
                        load_delay_token = ?binding.load_delay_token(),
                        "accepted main parser async classic lifecycle binding before source work"
                    );
                    binding
                },
            );
        }
        page_vm
            .vm_mut()
            .accept_main_document_blocking_stylesheet_inputs(
                parser_document_owner,
                &blocking_stylesheet_inputs,
            );
    }

    async fn advance_next_parser_step_for_owner(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        max_bytes: usize,
    ) -> Result<ParserStepAdvanceOutcome> {
        self.bind_page_resource_runtime(page_vm);
        let mut first_step = true;
        let outcome = loop {
            if page_vm.vm().current_main_document_task_owner() != Some(parser_document_owner) {
                tracing::debug!(
                    ?parser_document_owner,
                    current_owner = ?page_vm.vm().current_main_document_task_owner(),
                    "stopping stale main parser owner before advancing parser-owned input"
                );
                break ParserStepAdvanceOutcome::StoppedCurrentDocument;
            }
            let outcome = if first_step {
                first_step = false;
                self.pump_next_parse_step_with_signals(page_vm, parser_document_owner, max_bytes)
            } else {
                // Continue only bytes already admitted to the active backend for
                // this bounded parser turn. The next end segment remains owned by
                // the parser session until the scheduler grants another turn.
                self.pump_queued_or_resume_parse_step_with_signals(page_vm, parser_document_owner)
            };
            if page_vm
                .run_pending_parser_post_step_runtime_work_on_named_owner_local_task()
                .await?
            {
                break ParserStepAdvanceOutcome::StoppedCurrentDocument;
            }

            match outcome {
                LiveDocumentParserStepOutcome::InputBoundary => {
                    break ParserStepAdvanceOutcome::Continue;
                }
                LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(handoff) => {
                    page_vm
                        .construct_parser_custom_element_handoff_on_named_owner_local_task(*handoff)
                        .await?;
                }
                LiveDocumentParserStepOutcome::BlockingStylesheetPause(pause) => {
                    tracing::debug!(
                        stylesheet_node_id = pause.node_id.index(),
                        "main document parser paused on a body blocking stylesheet"
                    );
                    let final_url = self.final_url.clone();
                    self.catch_up_main_document_preload_scan(page_vm, &final_url, None);
                    let _ = self.parser_session.suspend(
                        ParserSuspensionCause::ParserCreatedStylesheet {
                            owner: pause.node_id,
                        },
                    );
                    break ParserStepAdvanceOutcome::BlockedOnStylesheetParserPause;
                }
                LiveDocumentParserStepOutcome::ScriptHandoff(handoff) => {
                    match self
                        .handle_parse_time_script_handoff_for_owner(
                            page_vm,
                            parser_document_owner,
                            *handoff,
                            None,
                        )
                        .await?
                    {
                        ScriptHandoffOutcome::StoppedCurrentDocument => {
                            break ParserStepAdvanceOutcome::StoppedCurrentDocument;
                        }
                        ScriptHandoffOutcome::NoNavigation => {
                            if let Some(outcome) = self.document_write_suspension_step_outcome() {
                                break outcome;
                            }
                        }
                        ScriptHandoffOutcome::BlockedOnDocumentWriteExternalLoad => {
                            break ParserStepAdvanceOutcome::BlockedOnDocumentWriteExternalLoad;
                        }
                        ScriptHandoffOutcome::BlockedOnStylesheet(script) => {
                            break ParserStepAdvanceOutcome::BlockedOnStylesheet(script);
                        }
                        ScriptHandoffOutcome::BlockedOnExternalSource(script) => {
                            break ParserStepAdvanceOutcome::BlockedOnExternalSource(script);
                        }
                    }
                }
            }
        };

        page_vm
            .drain_deferred_page_tasks_on_named_owner_local_task()
            .await?;
        Ok(outcome)
    }

    #[cfg(test)]
    pub(super) async fn advance_parser_step(
        &mut self,
        page_vm: &mut PageVm,
        parser_step: &str,
        parser_tail_after_step: Option<&str>,
    ) -> Result<ParserStepAdvanceOutcome> {
        let parser_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("parser-step test requires an installed main document owner");
        self.advance_parser_step_for_owner(
            page_vm,
            parser_document_owner,
            parser_step,
            parser_tail_after_step,
        )
        .await
    }

    #[cfg(test)]
    async fn advance_parser_step_for_owner(
        &mut self,
        page_vm: &mut PageVm,
        parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        parser_step: &str,
        parser_tail_after_step: Option<&str>,
    ) -> Result<ParserStepAdvanceOutcome> {
        self.bind_page_resource_runtime(page_vm);
        let mut next_chunk = parser_step;
        let mut handoff_tail = parser_tail_after_step;
        let outcome = loop {
            if page_vm.vm().current_main_document_task_owner() != Some(parser_document_owner) {
                tracing::debug!(
                    ?parser_document_owner,
                    current_owner = ?page_vm.vm().current_main_document_task_owner(),
                    "stopping stale main parser owner before advancing parser input"
                );
                break ParserStepAdvanceOutcome::StoppedCurrentDocument;
            }
            let outcome =
                self.pump_parse_step_with_signals(page_vm, parser_document_owner, next_chunk);
            if page_vm
                .run_pending_parser_post_step_runtime_work_on_named_owner_local_task()
                .await?
            {
                break ParserStepAdvanceOutcome::StoppedCurrentDocument;
            }

            match outcome {
                LiveDocumentParserStepOutcome::InputBoundary => {
                    break ParserStepAdvanceOutcome::Continue;
                }
                LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(handoff) => {
                    page_vm
                        .construct_parser_custom_element_handoff_on_named_owner_local_task(*handoff)
                        .await?;
                    next_chunk = "";
                }
                LiveDocumentParserStepOutcome::BlockingStylesheetPause(pause) => {
                    tracing::debug!(
                        stylesheet_node_id = pause.node_id.index(),
                        "main document parser paused on a body blocking stylesheet"
                    );
                    // Blink keeps token consumption paused while its preload scanner
                    // examines the unconsumed tail. Preserve that split here: the
                    // following DOM stays absent, while external scripts may begin
                    // fetching before the stylesheet settles.
                    let final_url = self.final_url.clone();
                    self.catch_up_main_document_preload_scan(
                        page_vm,
                        &final_url,
                        handoff_tail.take(),
                    );
                    let _ = self.parser_session.suspend(
                        ParserSuspensionCause::ParserCreatedStylesheet {
                            owner: pause.node_id,
                        },
                    );
                    break ParserStepAdvanceOutcome::BlockedOnStylesheetParserPause;
                }
                LiveDocumentParserStepOutcome::ScriptHandoff(handoff) => {
                    next_chunk = "";
                    match self
                        .handle_parse_time_script_handoff_for_owner(
                            page_vm,
                            parser_document_owner,
                            *handoff,
                            handoff_tail.take(),
                        )
                        .await?
                    {
                        ScriptHandoffOutcome::StoppedCurrentDocument => {
                            break ParserStepAdvanceOutcome::StoppedCurrentDocument;
                        }
                        ScriptHandoffOutcome::NoNavigation => {
                            if let Some(outcome) = self.document_write_suspension_step_outcome() {
                                break outcome;
                            }
                        }
                        ScriptHandoffOutcome::BlockedOnDocumentWriteExternalLoad => {
                            break ParserStepAdvanceOutcome::BlockedOnDocumentWriteExternalLoad;
                        }
                        ScriptHandoffOutcome::BlockedOnStylesheet(script) => {
                            break ParserStepAdvanceOutcome::BlockedOnStylesheet(script);
                        }
                        ScriptHandoffOutcome::BlockedOnExternalSource(script) => {
                            break ParserStepAdvanceOutcome::BlockedOnExternalSource(script);
                        }
                    }
                }
            };
        };

        page_vm
            .drain_deferred_page_tasks_on_named_owner_local_task()
            .await?;
        Ok(outcome)
    }

    fn document_write_suspension_step_outcome(&self) -> Option<ParserStepAdvanceOutcome> {
        match self.parser_session.suspension_cause() {
            Some(ParserSuspensionCause::ParserCreatedStylesheet { .. }) => {
                Some(ParserStepAdvanceOutcome::BlockedOnStylesheetParserPause)
            }
            Some(ParserSuspensionCause::DocumentWriteExternalScript { .. }) => {
                Some(ParserStepAdvanceOutcome::BlockedOnDocumentWriteExternalLoad)
            }
            Some(
                ParserSuspensionCause::ParserClassicSource { .. }
                | ParserSuspensionCause::ParserClassicStylesheets { .. },
            )
            | None => None,
        }
    }
}
