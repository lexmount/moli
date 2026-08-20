use super::super::{ChildParserClassicScriptCandidate, JsContextHost};
use crate::{
    StylesheetBlockingReadView, StylesheetElementRead,
    content_security_policy::ContentSecurityPolicyScriptElementRequest,
    document_runtime::DomHandle,
    document_script_scheduler::FrameDocumentClassicScriptSchedulerWork,
    dom::native::{Attribute, DomMutationEffects, Node},
    frame_owner_model::{
        DocumentId, FrameClassicDocumentScriptExecutionStart,
        FrameDocumentClassicCompletionFinishAction, FrameDocumentClassicParserResumeApplication,
        FrameDocumentClassicParserResumeSkipReason, FrameDocumentInteractiveLifecycleAction,
        FrameDocumentOwner, FrameDocumentTaskOwner, FrameScriptJob, FrameScriptSource,
        LocalWindowId,
    },
    live_document_parser::{
        DocumentParserCloseDisposition, DocumentParserLifetime, DocumentParserRunState,
        DocumentParserSession, LiveDocumentParserOwner, LiveDocumentParserStepOutcome,
        ParserStopReason, ParserSuspensionCause,
    },
    modulepreload::{
        invalid_modulepreload_as_value, invalid_modulepreload_as_warning,
        modulepreload_fetch_candidate, modulepreload_href, resolve_parser_network_resource_url,
    },
    parser::{
        ParserDomMutation, ParserDomMutationConsumer, ParserDomReadConsumer,
        ParserElementCreationConsumer, ParserElementCreationRequest, ParserMutationEffectConsumer,
        ParserPlanningReadView, ParserScriptHandoff, ParserScriptRead, PreparedImportMapSource,
    },
    planning::ScriptSource,
    types::{ScriptKind, ScriptSourceKind},
};
use html5ever::tree_builder::QuirksMode;
use url::Url;

pub(in crate::native_bridge::context_host) struct ChildLiveDocumentParserStartResult {
    pub(crate) initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    pub(crate) parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
}

impl ChildLiveDocumentParserStartResult {
    fn parser_stopped(action: Option<FrameDocumentInteractiveLifecycleAction>) -> Self {
        Self {
            initial_classic_ready_work: None,
            parser_stop_action: action,
        }
    }

    fn parser_blocked(work: Option<FrameDocumentClassicScriptSchedulerWork>) -> Self {
        Self {
            initial_classic_ready_work: work,
            parser_stop_action: None,
        }
    }
}

struct ChildFrameLiveParserOwner<'a, 'scope, 'pin> {
    host: &'a mut JsContextHost,
    scope: &'a mut v8::PinScope<'scope, 'pin>,
    child_document_handle: DomHandle,
}

impl<'a, 'scope, 'pin> ChildFrameLiveParserOwner<'a, 'scope, 'pin> {
    fn new(
        host: &'a mut JsContextHost,
        scope: &'a mut v8::PinScope<'scope, 'pin>,
        child_document_handle: DomHandle,
    ) -> Self {
        Self {
            host,
            scope,
            child_document_handle,
        }
    }

    fn sync_child_parser_side_effects(&mut self, effects: Option<&DomMutationEffects>) {
        if effects.is_none_or(|effects| effects.did_change()) {
            self.host
                .sync_owner_style_sheet_texts_for_document_tree_scopes(self.child_document_handle);
            self.host
                .sync_child_browsing_context_subtree(self.scope, self.child_document_handle);
        }
    }
}

impl LiveDocumentParserOwner for ChildFrameLiveParserOwner<'_, '_, '_> {}

impl StylesheetBlockingReadView for ChildFrameLiveParserOwner<'_, '_, '_> {
    fn stylesheet_element(&self, node_id: DomHandle) -> Option<StylesheetElementRead> {
        self.host
            .dom_host()
            .node(node_id)
            .and_then(StylesheetElementRead::from_node)
    }

    fn child_ids(&self, node_id: DomHandle) -> Vec<DomHandle> {
        self.host.dom_host().child_handles(node_id).collect()
    }

    fn text_content(&self, node_id: DomHandle) -> Option<String> {
        self.host.dom_host().text_content(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.host
            .dom_host()
            .node(self.child_document_handle)
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        Some(
            self.host
                .document_base_url_for_handle(self.child_document_handle),
        )
    }

    fn document_node_id(&self) -> DomHandle {
        self.child_document_handle
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<crate::dom::NodeId>,
    ) -> Vec<DomHandle> {
        self.host
            .dom_host()
            .stylesheet_candidate_handles_before_in_tree_scope(
                self.child_document_handle,
                target_node_id.map(|node_id| DomHandle::new(node_id.index())),
            )
    }
}

impl ParserMutationEffectConsumer for ChildFrameLiveParserOwner<'_, '_, '_> {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
        self.sync_child_parser_side_effects(Some(&effects));
    }
}

impl ParserDomReadConsumer for ChildFrameLiveParserOwner<'_, '_, '_> {
    fn node_exists(&mut self, node_id: DomHandle) -> bool {
        self.host.dom_host().node(node_id).is_some()
    }

    fn is_connected(&mut self, node_id: DomHandle) -> bool {
        self.host.dom_host().is_connected(node_id)
    }

    fn is_text_node(&mut self, node_id: DomHandle) -> bool {
        self.host
            .dom_host()
            .node(node_id)
            .and_then(Node::as_text)
            .is_some()
    }

    fn owner_document(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.host.dom_host().owner_document_handle(node_id)
    }

    fn parent_node(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.host
            .dom_host()
            .node(node_id)
            .and_then(Node::parent_node)
    }

    fn previous_sibling(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.host
            .dom_host()
            .node(node_id)
            .and_then(Node::prev_sibling)
    }

    fn last_child(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.host
            .dom_host()
            .node(node_id)
            .and_then(Node::last_child)
    }

    fn child_handles(&mut self, node_id: DomHandle) -> Vec<DomHandle> {
        self.host.dom_host().child_handles(node_id).collect()
    }

    fn document_body_handle_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.host
            .dom_host()
            .document_body_handle_for_document(document_handle)
    }

    fn document_base_url(&mut self, document_handle: DomHandle) -> Option<Url> {
        self.host
            .dom_host()
            .document_base_url_for_handle(document_handle)
    }

    fn template_contents_handle(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.host
            .dom_host()
            .parser_template_contents_handle(node_id)
    }

    fn is_html_element_named(&mut self, node_id: DomHandle, local_name: &str) -> bool {
        self.host
            .dom_host()
            .dom()
            .is_html_element_named(node_id, local_name)
    }

    fn is_external_async_classic_candidate(&mut self, node_id: DomHandle) -> bool {
        let Some(element) = self
            .host
            .dom_host()
            .node(node_id)
            .and_then(Node::as_element)
        else {
            return false;
        };
        if !element.is_html_element("script")
            || element.attribute("src").is_none()
            || element.attribute("async").is_none()
            || element.attribute("nomodule").is_some()
        {
            return false;
        }
        let Some(script_type) = element.attribute("type") else {
            return true;
        };
        script_type.is_empty()
            || moli_script::classify_script_kind(Some(script_type))
                == crate::types::ScriptKind::Classic
    }

    fn parser_script_read(&mut self, node_id: DomHandle) -> Option<ParserScriptRead> {
        <crate::dom::native::DomHost as ParserPlanningReadView>::parser_script_read(
            self.host.dom_host(),
            node_id,
        )
    }

    fn stylesheet_element(&mut self, node_id: DomHandle) -> Option<StylesheetElementRead> {
        self.host
            .dom_host()
            .node(node_id)
            .and_then(StylesheetElementRead::from_node)
    }

    fn text_content(&mut self, node_id: DomHandle) -> Option<String> {
        self.host.dom_host().text_content(node_id)
    }
}

impl ParserDomMutationConsumer for ChildFrameLiveParserOwner<'_, '_, '_> {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) {
        let effects = mutation.apply_to_dom_host(self.host.dom_host_mut());
        self.sync_child_parser_side_effects(Some(&effects));
    }

    fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_parser_element_without_attributes(local_name, namespace, prefix)
    }

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        document_handle: DomHandle,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_parser_element_without_attributes_for_document(
                document_handle,
                local_name,
                namespace,
                prefix,
            )
    }

    fn add_attrs_if_missing_for_parser(&mut self, node_id: DomHandle, attrs: Vec<Attribute>) {
        self.host
            .dom_host_mut()
            .add_attrs_if_missing_for_parser(node_id, attrs);
    }

    fn create_text_node(&mut self, text: String) -> DomHandle {
        self.host.dom_host_mut().create_text_node(&text)
    }

    fn create_comment(&mut self, text: String) -> DomHandle {
        self.host.dom_host_mut().create_comment(&text)
    }

    fn create_processing_instruction(&mut self, target: String, data: String) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_processing_instruction(&target, &data)
    }

    fn create_cdata_section(&mut self, data: String) -> DomHandle {
        self.host.dom_host_mut().create_cdata_section(&data)
    }

    fn create_document_type(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_document_type(&name, &public_id, &system_id)
    }

    fn prepend_text_to_text_node(&mut self, node_id: DomHandle, text: String) {
        if let Some(text_node) = self
            .host
            .dom_host_mut()
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_text_mut())
        {
            let mut merged = text;
            merged.push_str(text_node.data());
            text_node.set_data(merged);
        }
    }

    fn append_text_to_text_node(&mut self, node_id: DomHandle, text: String) {
        if let Some(text_node) = self
            .host
            .dom_host_mut()
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_text_mut())
        {
            let mut merged = text_node.data().to_owned();
            merged.push_str(&text);
            text_node.set_data(merged);
        }
    }

    fn push_parse_error(&mut self, error: String) {
        self.host.dom_host_mut().push_parse_error(error);
    }

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode) {
        self.host
            .dom_host_mut()
            .set_html_quirks_mode_for_parser_document(self.child_document_handle, quirks_mode);
    }

    fn mark_script_already_started_for_parser(&mut self, node_id: DomHandle) {
        self.host
            .dom_host_mut()
            .set_script_already_started(node_id, true);
    }

    fn finish_parsing_script_children(&mut self, node_id: DomHandle) {
        let _ = self
            .host
            .dom_host_mut()
            .finish_parsing_script_children(node_id);
    }

    fn finish_parsing_link_children(&mut self, node_id: DomHandle) {
        let _ = self
            .host
            .dom_host_mut()
            .finish_parsing_link_children(node_id);
    }

    fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: DomHandle,
        template_id: DomHandle,
        attrs: Vec<Attribute>,
    ) -> bool {
        self.host
            .dom_host_mut()
            .attach_declarative_shadow_for_parser(host_id, template_id, &attrs)
    }

    fn associate_parser_form_owner(&mut self, target: DomHandle, form: DomHandle) -> bool {
        self.host
            .dom_host_mut()
            .associate_parser_form_owner(target, form)
    }
}

impl ParserElementCreationConsumer for ChildFrameLiveParserOwner<'_, '_, '_> {
    fn create_parser_element(
        &mut self,
        _request: ParserElementCreationRequest<'_>,
    ) -> Option<DomHandle> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParserProgress {
    /// Every parser-owned input frame is empty and this parser's lifetime
    /// requires finalization.
    ReadyToFinish,
    /// Every parser-owned input frame is empty, but an explicitly open parser
    /// remains resident for a later `document.write()` or `document.close()`.
    WaitingForInput,
    /// The parser queued a parser-blocking classic script and must be saved until
    /// that script completes or reports source failure.
    BlockedOnParserScript {
        ready_work: Option<Box<FrameDocumentClassicScriptSchedulerWork>>,
    },
    /// The parser stopped immediately after a parser-created blocking
    /// stylesheet in the body. The exact parser entry remains resident until
    /// that document owner's stylesheet readiness settles.
    BlockedOnStylesheetPause,
    /// Parser ownership changed while a synchronous handoff was being applied.
    /// The stale parser must be stopped rather than finalized or parked.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptDisposition {
    /// The handoff was applied, skipped, or otherwise settled synchronously.
    Continue,
    /// A parser-blocking classic script owns the next parser wake.
    BlockedOnParserScript {
        cause: ParserSuspensionCause,
        ready_work: Option<Box<FrameDocumentClassicScriptSchedulerWork>>,
    },
    /// The script could not enter its selected scheduler. The parser driver
    /// decides whether to skip it for the current Document or stop a stale
    /// parser.
    AdmissionFailed { script_handle: DomHandle },
}

impl JsContextHost {
    fn suspend_live_child_parser(parser: &mut DocumentParserSession, cause: ParserSuspensionCause) {
        let _ = parser.suspend(cause);
    }

    fn live_child_parser_document_is_current(
        &self,
        child_handle: DomHandle,
        document_handle: DomHandle,
    ) -> bool {
        self.child_browsing_context_host_for_document_handle(document_handle) == Some(child_handle)
    }

    fn recover_current_child_parser_script_admission_failure(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        script_handle: DomHandle,
    ) -> bool {
        if !self.live_child_parser_document_is_current(child_handle, document_handle) {
            tracing::debug!(
                ?child_handle,
                ?document_handle,
                ?script_handle,
                "stopping stale child parser after script scheduler admission failed"
            );
            return false;
        }
        let _ = self
            .dom_host_mut()
            .set_script_already_started(script_handle, true);
        tracing::warn!(
            ?child_handle,
            ?document_handle,
            ?script_handle,
            "skipping current child parser script after scheduler admission failed"
        );
        true
    }

    fn drive_live_child_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        document_handle: DomHandle,
        parser: &mut DocumentParserSession,
    ) -> ParserProgress {
        loop {
            if !self.live_child_parser_document_is_current(child_handle, document_handle) {
                parser.stop(ParserStopReason::DocumentReplacement);
                return ParserProgress::Stopped;
            }
            let outcome = {
                let mut owner = ChildFrameLiveParserOwner::new(self, scope, document_handle);
                parser.advance_queued_or_resume_step(&mut owner)
            };
            let discovery_signals = parser.take_discovery_signals();
            self.queue_live_child_parser_discovery_signals(
                child_handle,
                document_handle,
                discovery_signals,
            );
            match outcome {
                LiveDocumentParserStepOutcome::InputBoundary => {
                    if parser.input_is_empty() {
                        return if parser.finishes_on_empty_input() {
                            ParserProgress::ReadyToFinish
                        } else {
                            ParserProgress::WaitingForInput
                        };
                    }
                    // Draining one insertion frame restores its parent input
                    // inside the same feed and still reports an input
                    // boundary. Keep driving until the parser-owned input
                    // stack is truly empty.
                    continue;
                }
                LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(_) => {}
                LiveDocumentParserStepOutcome::BlockingStylesheetPause(pause) => {
                    tracing::debug!(
                        child_handle = child_handle.index(),
                        stylesheet_node_id = pause.node_id.index(),
                        "child document parser paused on a body blocking stylesheet"
                    );
                    let remains_blocked = self
                        .frame_owner_store
                        .current_child_document_owner(child_handle)
                        .is_some_and(|owner| {
                            self.frame_document_blocking_stylesheets.has_pending(owner)
                        });
                    if remains_blocked {
                        Self::suspend_live_child_parser(
                            parser,
                            ParserSuspensionCause::ParserCreatedStylesheet {
                                owner: pause.node_id,
                            },
                        );
                        return ParserProgress::BlockedOnStylesheetPause;
                    }
                }
                LiveDocumentParserStepOutcome::ScriptHandoff(handoff) => {
                    match self.queue_live_child_parser_script_handoff(
                        child_handle,
                        document_handle,
                        *handoff,
                    ) {
                        ScriptDisposition::Continue => {}
                        ScriptDisposition::BlockedOnParserScript { cause, ready_work } => {
                            Self::suspend_live_child_parser(parser, cause);
                            return ParserProgress::BlockedOnParserScript { ready_work };
                        }
                        ScriptDisposition::AdmissionFailed { script_handle } => {
                            if !self.recover_current_child_parser_script_admission_failure(
                                child_handle,
                                document_handle,
                                script_handle,
                            ) {
                                parser.stop(ParserStopReason::DocumentReplacement);
                                return ParserProgress::Stopped;
                            }
                        }
                    }
                }
            }
        }
    }

    fn queue_live_child_parser_script_handoff(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        handoff: ParserScriptHandoff,
    ) -> ScriptDisposition {
        let (script_handle, start_line, start_column) = match &handoff {
            ParserScriptHandoff::BlockingClassic {
                node_id,
                start_line,
                start_column,
                ..
            }
            | ParserScriptHandoff::AsyncPostParse {
                node_id,
                start_line,
                start_column,
                ..
            }
            | ParserScriptHandoff::NonAsyncPostParse {
                node_id,
                start_line,
                start_column,
                ..
            }
            | ParserScriptHandoff::ImportMap {
                node_id,
                start_line,
                start_column,
                ..
            }
            | ParserScriptHandoff::NoExecution {
                node_id,
                start_line,
                start_column,
                ..
            }
            | ParserScriptHandoff::PreparationFailure {
                node_id,
                start_line,
                start_column,
                ..
            } => (*node_id, *start_line, *start_column),
        };
        self.note_parser_script_start_position(script_handle, start_line, start_column);

        if !self.child_browsing_context_scripting_enabled(child_handle) {
            match handoff {
                ParserScriptHandoff::NoExecution {
                    node_id, outcome, ..
                } => {
                    crate::host::apply_parser_script_element_state_transition(
                        self.dom_host_mut(),
                        node_id,
                        outcome.element_state_transition(),
                    );
                }
                ParserScriptHandoff::PreparationFailure {
                    node_id, failure, ..
                } => {
                    crate::host::apply_parser_script_element_state_transition(
                        self.dom_host_mut(),
                        node_id,
                        failure.element_state_transition(),
                    );
                }
                ParserScriptHandoff::BlockingClassic { node_id, .. }
                | ParserScriptHandoff::AsyncPostParse { node_id, .. }
                | ParserScriptHandoff::NonAsyncPostParse { node_id, .. }
                | ParserScriptHandoff::ImportMap { node_id, .. } => {
                    let _ = self
                        .dom_host_mut()
                        .set_script_already_started(node_id, true);
                }
            }
            return ScriptDisposition::Continue;
        }

        match handoff {
            ParserScriptHandoff::BlockingClassic {
                node_id,
                start_line,
                blocking_signatures_before,
                script,
                ..
            } => {
                let waits_for_stylesheets = !blocking_signatures_before.is_empty();
                let queued = self.push_child_parser_classic_script_for_current_document(
                    child_handle,
                    document_handle,
                    ChildParserClassicScriptCandidate::from_parser_handoff(
                        node_id,
                        start_line,
                        blocking_signatures_before,
                        script,
                    ),
                );
                if let Some(queued) = queued {
                    let cause = if waits_for_stylesheets {
                        ParserSuspensionCause::ParserClassicStylesheets { script: node_id }
                    } else {
                        ParserSuspensionCause::ParserClassicSource { script: node_id }
                    };
                    ScriptDisposition::BlockedOnParserScript {
                        cause,
                        ready_work: queued.ready_work.map(Box::new),
                    }
                } else {
                    ScriptDisposition::AdmissionFailed {
                        script_handle: node_id,
                    }
                }
            }
            ParserScriptHandoff::AsyncPostParse {
                node_id,
                start_line,
                script,
                ..
            } => self.queue_live_child_parser_async_post_parse_script_handoff(
                child_handle,
                document_handle,
                node_id,
                start_line,
                script,
            ),
            ParserScriptHandoff::NonAsyncPostParse {
                node_id,
                start_line,
                blocking_signatures_before,
                script,
                ..
            } => self.queue_live_child_parser_non_async_post_parse_script_handoff(
                child_handle,
                document_handle,
                node_id,
                start_line,
                blocking_signatures_before,
                script,
            ),
            ParserScriptHandoff::ImportMap {
                node_id,
                import_map,
                ..
            } => {
                let _ = self
                    .dom_host_mut()
                    .set_script_already_started(node_id, true);
                match import_map.source {
                    PreparedImportMapSource::Inline(source) => {
                        if let Err(error) = self.register_current_child_document_import_map(
                            child_handle,
                            document_handle,
                            &source,
                            &import_map.base_url,
                        ) {
                            tracing::warn!(
                                child_handle = ?child_handle,
                                document_handle = ?document_handle,
                                script_handle = ?node_id,
                                %error,
                                "child parser import map registration failed"
                            );
                        }
                    }
                    PreparedImportMapSource::ExternalUnsupported => {
                        tracing::debug!(
                            child_handle = ?child_handle,
                            document_handle = ?document_handle,
                            script_handle = ?node_id,
                            "child parser external import map is unsupported"
                        );
                    }
                }
                ScriptDisposition::Continue
            }
            ParserScriptHandoff::NoExecution {
                node_id, outcome, ..
            } => {
                crate::host::apply_parser_script_element_state_transition(
                    self.dom_host_mut(),
                    node_id,
                    outcome.element_state_transition(),
                );
                if let (_, _, Some(run)) = outcome.into_parts() {
                    self.record_parser_no_execution_run(run);
                }
                ScriptDisposition::Continue
            }
            ParserScriptHandoff::PreparationFailure {
                node_id, failure, ..
            } => {
                crate::host::apply_parser_script_element_state_transition(
                    self.dom_host_mut(),
                    node_id,
                    failure.element_state_transition(),
                );
                ScriptDisposition::Continue
            }
        }
    }

    fn queue_live_child_parser_async_post_parse_script_handoff(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        script_handle: DomHandle,
        _start_line: u64,
        mut script: crate::planning::PreparedScript,
    ) -> ScriptDisposition {
        if script.kind == ScriptKind::Module {
            return self.queue_live_child_parser_post_parse_module_handoff(
                child_handle,
                script_handle,
                Default::default(),
                script,
            );
        }
        if script.kind != ScriptKind::Classic || script.source_kind != ScriptSourceKind::External {
            return ScriptDisposition::Continue;
        }
        if !matches!(script.source, ScriptSource::External) {
            return ScriptDisposition::Continue;
        }
        script.node_id = crate::dom::NodeId::new(script_handle.index());
        if !self.queue_child_external_classic_document_script_for_current_document(
            child_handle,
            document_handle,
            script_handle,
            script,
        ) {
            return ScriptDisposition::AdmissionFailed { script_handle };
        }
        ScriptDisposition::Continue
    }

    fn queue_live_child_parser_post_parse_module_handoff(
        &mut self,
        child_handle: DomHandle,
        script_handle: DomHandle,
        blocking_stylesheet_signatures: std::collections::HashSet<
            crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
        >,
        script: crate::planning::PreparedScript,
    ) -> ScriptDisposition {
        if script.kind != ScriptKind::Module {
            return ScriptDisposition::Continue;
        }
        if !self.queue_child_parser_module_root_for_current_document(
            child_handle,
            script_handle,
            blocking_stylesheet_signatures,
            script,
        ) {
            return ScriptDisposition::AdmissionFailed { script_handle };
        }
        let _ = self
            .dom_host_mut()
            .set_script_already_started(script_handle, true);
        ScriptDisposition::Continue
    }

    fn queue_live_child_parser_non_async_post_parse_script_handoff(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        script_handle: DomHandle,
        start_line: u64,
        blocking_stylesheet_signatures: std::collections::HashSet<
            crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
        >,
        script: crate::planning::PreparedScript,
    ) -> ScriptDisposition {
        if script.kind == ScriptKind::Module {
            return self.queue_live_child_parser_post_parse_module_handoff(
                child_handle,
                script_handle,
                blocking_stylesheet_signatures,
                script,
            );
        }
        if script.kind != ScriptKind::Classic || script.source_kind != ScriptSourceKind::External {
            return ScriptDisposition::Continue;
        }
        if !matches!(script.source, ScriptSource::External) {
            return ScriptDisposition::Continue;
        }
        if self
            .push_child_parser_classic_script_for_current_document(
                child_handle,
                document_handle,
                ChildParserClassicScriptCandidate::from_deferred_handoff(
                    script_handle,
                    start_line,
                    blocking_stylesheet_signatures,
                    script,
                ),
            )
            .is_none()
        {
            return ScriptDisposition::AdmissionFailed { script_handle };
        }
        ScriptDisposition::Continue
    }

    fn finish_live_child_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
        document_handle: DomHandle,
        mut parser: DocumentParserSession,
    ) -> Option<crate::frame_owner_model::FrameDocumentInteractiveLifecycleAction> {
        let finish_signals = {
            let mut owner = ChildFrameLiveParserOwner::new(self, scope, document_handle);
            parser.finish(&mut owner)
        };
        self.queue_live_child_parser_discovery_signals(
            child_handle,
            document_handle,
            finish_signals.discovery_signals,
        );
        self.sync_child_browsing_context_subtree(scope, document_handle);
        let _ = self
            .dom_host_mut()
            .update_document_target_from_url(document_handle);
        self.sync_owner_style_sheet_texts_for_document_tree_scopes(document_handle);
        self.dom_host_mut()
            .mark_subtree_connected_preserving_owner_document(document_handle);
        let document_url = self.document_url_for_handle(document_handle);
        let document_base_url = self.document_base_url_for_handle(document_handle);
        let _ = self.frame_owner_store.update_current_child_document_urls(
            child_handle,
            document_url,
            document_base_url,
        );
        self.frame_owner_store
            .finish_current_child_document_parsing(child_handle, owner)
    }

    fn finish_and_queue_live_child_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
        document_handle: DomHandle,
        parser: DocumentParserSession,
    ) {
        if let Some(action) = self.finish_live_child_document_parser(
            scope,
            child_handle,
            owner,
            document_handle,
            parser,
        ) {
            self.queue_child_document_interactive_lifecycle_action(action);
        }
    }

    fn queue_live_child_parser_discovery_signals(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        discovery_signals: crate::live_document_parser::LiveDocumentParserDiscoverySignals,
    ) {
        let async_prefetch_count = discovery_signals.async_prefetch_scripts.len();
        let modulepreload_link_count = discovery_signals.modulepreload_link_candidates.len();
        let blocking_stylesheet_count = discovery_signals.blocking_stylesheet_inputs.len();
        let modulepreload_link_candidates = discovery_signals.modulepreload_link_candidates;
        let blocking_stylesheet_inputs = discovery_signals.blocking_stylesheet_inputs;
        if async_prefetch_count != 0
            || modulepreload_link_count != 0
            || blocking_stylesheet_count != 0
        {
            tracing::debug!(
                child_document_handle = ?document_handle,
                async_prefetch_count,
                modulepreload_link_count,
                blocking_stylesheet_count,
                "child live parser emitted document-owned discovery signals"
            );
        }
        if let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
        {
            let accepted_stylesheet_count = self.accept_child_parser_blocking_stylesheet_inputs(
                child_handle,
                owner,
                blocking_stylesheet_inputs,
            );
            if accepted_stylesheet_count != 0 {
                tracing::debug!(
                    child_handle = ?child_handle,
                    document_handle = ?document_handle,
                    owner = ?owner,
                    accepted_stylesheet_count,
                    "child live parser discovery installed document-owned stylesheet readiness"
                );
            }
        }
        self.queue_child_parser_discovered_modulepreload_links(
            child_handle,
            document_handle,
            modulepreload_link_candidates,
        );
    }

    fn queue_child_parser_discovered_modulepreload_links(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        link_handles: Vec<DomHandle>,
    ) {
        if link_handles.is_empty() {
            return;
        }
        for link_handle in link_handles {
            if self.dom_host().owner_document_handle(link_handle) != Some(document_handle) {
                continue;
            }
            let Some(element) = self
                .dom_host()
                .node(link_handle)
                .and_then(crate::dom::native::Node::as_element)
            else {
                continue;
            };
            if let Some(invalid_as) = invalid_modulepreload_as_value(element) {
                tracing::debug!(
                    child_handle = ?child_handle,
                    document_handle = ?document_handle,
                    link_handle = ?link_handle,
                    warning = %invalid_modulepreload_as_warning(&invalid_as),
                    "queueing child parser-discovered modulepreload error task"
                );
                let _ = self.queue_child_modulepreload_link_error_for_current_document(
                    child_handle,
                    link_handle,
                );
                continue;
            }
            let Some(raw_href) = modulepreload_href(element) else {
                continue;
            };
            let document_url = self.document_url_for_handle(document_handle);
            let document_base_url = self.document_base_url_for_handle(document_handle);
            let Some(request_url) =
                resolve_parser_network_resource_url(&document_base_url, raw_href)
            else {
                continue;
            };
            let Some(candidate) =
                modulepreload_fetch_candidate(element, request_url, &document_url, None)
            else {
                continue;
            };
            tracing::debug!(
                child_handle = ?child_handle,
                document_handle = ?document_handle,
                link_handle = ?link_handle,
                url = %candidate.key.url(),
                "queueing child parser-discovered modulepreload fetch task"
            );
            let _ = self.queue_child_modulepreload_fetch_for_current_document(
                child_handle,
                link_handle,
                candidate.request,
            );
        }
    }

    pub(in crate::native_bridge::context_host) fn install_child_document_write_parser(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
        document_handle: DomHandle,
        document_url: Url,
    ) {
        assert_eq!(
            self.frame_owner_store
                .current_child_document_owner(child_handle),
            Some(owner),
            "committed child document-open parser owner must remain current"
        );
        assert_eq!(
            self.child_browsing_context_document_handle(child_handle),
            Some(document_handle),
            "committed child document-open parser must target the current Document"
        );
        let task_owner = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
            .expect("committed child document-open parser must have a task owner");
        assert_eq!(task_owner.document_owner(), owner);
        let parser = DocumentParserSession::start_open_live_document(document_url, document_handle);
        self.child_document_parsers.replace(owner, parser);
    }

    pub(in crate::native_bridge::context_host) fn child_document_parser_is_active(
        &self,
        child_handle: DomHandle,
    ) -> bool {
        self.frame_owner_store
            .current_child_document_owner(child_handle)
            .is_some_and(|owner| self.child_document_parsers.contains(owner))
    }

    pub(in crate::native_bridge::context_host) fn pump_child_document_write_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        script_context: v8::Local<'_, v8::Context>,
        child_handle: DomHandle,
        document_handle: DomHandle,
        chunk: Option<String>,
        close_requested: bool,
    ) -> bool {
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_owner(child_handle)
        else {
            return false;
        };
        let Some(mut entry) = self.child_document_parsers.take(owner) else {
            return !close_requested;
        };
        let executing_parser_script =
            chunk.is_some() && self.child_document_is_executing_parser_script(document_handle);
        let parser_insertion_only = chunk.is_some()
            && !close_requested
            && (entry.lifetime() == DocumentParserLifetime::Finite || executing_parser_script);
        let parser_ready_to_advance = if close_requested {
            entry.request_close() == DocumentParserCloseDisposition::DrainNow
        } else {
            entry.run_state() == DocumentParserRunState::Ready
        };
        if let Some(chunk) = chunk {
            if parser_ready_to_advance {
                entry
                    .stream_handle()
                    .borrow()
                    .script_input_session()
                    .enqueue_script_input_html(chunk);
            } else if executing_parser_script {
                if !entry.append_to_current_inserted_input(&chunk) {
                    tracing::warn!(
                        ?child_handle,
                        ?document_handle,
                        "child parser script write had no active inserted input frame"
                    );
                    entry
                        .stream_handle()
                        .borrow()
                        .script_input_session()
                        .enqueue_script_input_html(chunk);
                }
            } else if parser_insertion_only {
                entry
                    .stream_handle()
                    .borrow()
                    .script_input_session()
                    .enqueue_script_input_html(chunk);
            } else {
                // A write from outside the blocked child parser belongs after
                // every input frame already owned by that parser. Treating it
                // as a fresh script insertion would move it ahead of buffered
                // tails from nested document.write() calls.
                entry.queue_arrived_chunk(chunk);
            }
        }
        if !parser_ready_to_advance {
            self.child_document_parsers.replace(owner, entry);
            return true;
        }

        loop {
            if !self.live_child_parser_document_is_current(child_handle, document_handle) {
                entry.stop(ParserStopReason::DocumentReplacement);
                return false;
            }
            let outcome = {
                let mut parser_owner = ChildFrameLiveParserOwner::new(self, scope, document_handle);
                entry.advance_queued_or_resume_step(&mut parser_owner)
            };
            let discovery_signals = entry.take_discovery_signals();
            self.queue_live_child_parser_discovery_signals(
                child_handle,
                document_handle,
                discovery_signals,
            );
            match outcome {
                LiveDocumentParserStepOutcome::InputBoundary => {
                    if parser_insertion_only {
                        // Script-inserted input parks after one boundary; the
                        // post-script resume drains the remaining stack in its
                        // own turn so following scripts keep their scheduler
                        // ordering.
                        self.child_document_parsers.replace(owner, entry);
                        return true;
                    }
                    if !entry.input_is_empty() {
                        // Draining one insertion frame restores its parent
                        // input inside the same feed and still reports an
                        // input boundary. Keep driving until the parser-owned
                        // input stack is truly empty before finishing or
                        // parking.
                        continue;
                    }
                    if entry.finishes_on_empty_input() {
                        if close_requested {
                            self.finish_child_document_write_parser_on_current_stack(
                                scope,
                                script_context,
                                child_handle,
                                owner,
                                document_handle,
                                entry,
                            );
                        } else {
                            self.finish_and_queue_live_child_document_parser(
                                scope,
                                child_handle,
                                owner,
                                document_handle,
                                entry,
                            );
                        }
                    } else {
                        self.child_document_parsers.replace(owner, entry);
                    }
                    return true;
                }
                LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(handoff) => {
                    self.child_document_parsers.replace(owner, entry);
                    let host_ptr = self as *mut JsContextHost;
                    let _ = crate::custom_elements::construct_parser_created_autonomous_element_from_handoff(
                        scope,
                        host_ptr,
                        &handoff,
                    );
                    crate::custom_elements::flush_parser_custom_element_handoff_replacements(
                        scope, host_ptr,
                    );
                    let Some(next_entry) = self.child_document_parsers.take(owner) else {
                        return false;
                    };
                    entry = next_entry;
                }
                LiveDocumentParserStepOutcome::BlockingStylesheetPause(pause) => {
                    tracing::debug!(
                        child_handle = child_handle.index(),
                        stylesheet_node_id = pause.node_id.index(),
                        "child document.write parser paused on a body blocking stylesheet"
                    );
                    if self.frame_document_blocking_stylesheets.has_pending(owner) {
                        Self::suspend_live_child_parser(
                            &mut entry,
                            ParserSuspensionCause::ParserCreatedStylesheet {
                                owner: pause.node_id,
                            },
                        );
                        self.child_document_parsers.replace(owner, entry);
                        return false;
                    }
                }
                LiveDocumentParserStepOutcome::ScriptHandoff(handoff) => {
                    entry = match self.try_execute_child_document_write_inline_classic_handoff(
                        scope,
                        script_context,
                        child_handle,
                        document_handle,
                        owner,
                        &handoff,
                        entry,
                    ) {
                        None => {
                            let Some(next_entry) = self.child_document_parsers.take(owner) else {
                                return false;
                            };
                            if next_entry.is_suspended() {
                                self.child_document_parsers.replace(owner, next_entry);
                                return true;
                            }
                            next_entry
                        }
                        Some(mut entry) => {
                            let progress = self.queue_live_child_parser_script_handoff(
                                child_handle,
                                document_handle,
                                *handoff,
                            );
                            match progress {
                                ScriptDisposition::Continue => entry,
                                ScriptDisposition::BlockedOnParserScript { cause, ready_work } => {
                                    Self::suspend_live_child_parser(&mut entry, cause);
                                    self.child_document_parsers.replace(owner, entry);
                                    if let Some(ready_work) = ready_work {
                                        self.push_child_document_script_ready_input(*ready_work);
                                    }
                                    return true;
                                }
                                ScriptDisposition::AdmissionFailed { script_handle } => {
                                    if !self.recover_current_child_parser_script_admission_failure(
                                        child_handle,
                                        document_handle,
                                        script_handle,
                                    ) {
                                        entry.stop(ParserStopReason::DocumentReplacement);
                                        return false;
                                    }
                                    entry
                                }
                            }
                        }
                    };
                }
            }
        }
    }

    fn finish_child_document_write_parser_on_current_stack(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        script_context: v8::Local<'_, v8::Context>,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
        document_handle: DomHandle,
        parser: DocumentParserSession,
    ) {
        let Some(interactive) = self.finish_live_child_document_parser(
            scope,
            child_handle,
            owner,
            document_handle,
            parser,
        ) else {
            return;
        };
        let ready_work = self
            .apply_child_document_interactive_for_script_created_parser_close(scope, interactive);
        let Some(work) = ready_work else {
            return;
        };
        // Blink runs the eligible defer head during parser close, then posts a
        // separate script task for the next item instead of draining the queue.
        if let Some(next_work) = self.run_child_document_write_deferred_classic_on_current_stack(
            scope,
            script_context,
            work,
        ) {
            self.push_child_document_script_ready_input(next_work);
        }
    }

    fn run_child_document_write_deferred_classic_on_current_stack(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        script_context: v8::Local<'_, v8::Context>,
        work: FrameDocumentClassicScriptSchedulerWork,
    ) -> Option<FrameDocumentClassicScriptSchedulerWork> {
        let completion = match work {
            FrameDocumentClassicScriptSchedulerWork::Ready(ready) => {
                let start = self
                    .prepare_child_classic_script_execution(ready)
                    .into_start();
                match start {
                    FrameClassicDocumentScriptExecutionStart::Execute(action) => {
                        let (job, finish) = action.into_parts();
                        let child_handle = finish.child_handle;
                        if let Err(error) = self.execute_child_frame_script_job_on_current_stack(
                            scope,
                            script_context,
                            job,
                        ) {
                            tracing::warn!(
                                child_handle = ?child_handle,
                                %error,
                                "synchronous child document.close defer script failed"
                            );
                        }
                        self.finish_executing_child_classic_script(finish)
                    }
                    FrameClassicDocumentScriptExecutionStart::Complete(completion) => {
                        Some(*completion)
                    }
                    FrameClassicDocumentScriptExecutionStart::Dropped => None,
                }
            }
            FrameDocumentClassicScriptSchedulerWork::SourceFailed(failed) => {
                tracing::warn!(
                    child_handle = ?failed.target().child_handle(),
                    script_handle = ?failed.script_handle(),
                    url = %failed.script_url(),
                    error = failed.error(),
                    "synchronous child document.close defer source failed"
                );
                self.report_child_classic_script_source_failure(failed)
                    .into_completion()
            }
        }?;
        self.apply_child_document_write_classic_completion(scope, completion)
    }

    fn execute_child_frame_script_job_on_current_stack(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        script_context: v8::Local<'_, v8::Context>,
        mut job: FrameScriptJob,
    ) -> anyhow::Result<()> {
        let child_handle = self.frame_owner_child_handle_for_script_job(&job);
        let host_ptr = self as *mut JsContextHost;
        let allowed = {
            let script_scope = &mut v8::ContextScope::new(scope, script_context);
            crate::native_bridge::element::prepare_inline_classic_frame_script_job_for_execution(
                script_scope,
                host_ptr,
                &mut job,
            )?
        };
        if !allowed {
            if let Some(document_handle) = child_handle
                .and_then(|child_handle| self.child_browsing_context_document_handle(child_handle))
            {
                self.sync_child_browsing_context_subtree(scope, document_handle);
            }
            return Ok(());
        }
        let current_script = self.push_frame_script_job_current_script(&job);
        let FrameScriptJob {
            source,
            script_url,
            base_url,
            script_nonce,
            ..
        } = job;
        let result = match source {
            FrameScriptSource::SourceText(source) => {
                let script_scope = &mut v8::ContextScope::new(scope, script_context);
                crate::script_vm::execute_source_text_on_current_stack(
                    script_scope,
                    &source,
                    Some(&script_url),
                    Some(&base_url),
                    0,
                    script_nonce.as_deref(),
                    true,
                )
            }
            #[cfg(test)]
            FrameScriptSource::FunctionConstructor(_) => Err(anyhow::anyhow!(
                "child document.close classic work is not a source-text job"
            )),
        };
        if let Some(current_script) = current_script {
            self.pop_child_current_script(current_script);
        }
        if let Some(document_handle) = child_handle
            .and_then(|child_handle| self.child_browsing_context_document_handle(child_handle))
        {
            self.sync_child_browsing_context_subtree(scope, document_handle);
        }
        result
    }

    fn apply_child_document_write_classic_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: crate::frame_owner_model::FrameDocumentClassicScriptCompletionAction,
    ) -> Option<FrameDocumentClassicScriptSchedulerWork> {
        let action = FrameDocumentClassicCompletionFinishAction::from_completion(completion);
        if let Some(event_action) = action.script_element_event_action() {
            let _ = self.dispatch_child_script_element_event(scope, event_action.event());
        }
        self.complete_child_deferred_classic_script(action.target())
            .into_scheduler_work()
    }

    fn try_execute_child_document_write_inline_classic_handoff(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        script_context: v8::Local<'_, v8::Context>,
        child_handle: DomHandle,
        document_handle: DomHandle,
        owner: FrameDocumentOwner,
        handoff: &ParserScriptHandoff,
        entry: DocumentParserSession,
    ) -> Option<DocumentParserSession> {
        let ParserScriptHandoff::BlockingClassic {
            node_id,
            start_line,
            script,
            ..
        } = handoff
        else {
            return Some(entry);
        };
        let source = match (&script.kind, &script.source) {
            (ScriptKind::Classic, ScriptSource::Inline(source)) => source.clone(),
            _ => return Some(entry),
        };
        let Some(mut job) = self.frame_owner_child_parser_classic_script_job(
            child_handle,
            Some(*node_id),
            source.clone(),
        ) else {
            return Some(entry);
        };
        job.script_url = script.url.clone();
        job.base_url = script.base_url.clone();
        job.referrer_policy = script.fetch_metadata.referrer_policy.clone();
        if self.dom_host().containing_shadow_root(*node_id).is_some() {
            job.current_script = None;
        }
        self.child_document_parsers.replace(owner, entry);
        let _ = self
            .dom_host_mut()
            .set_script_already_started(*node_id, true);
        let host_ptr = self as *mut JsContextHost;
        let source = {
            let script_scope = &mut v8::ContextScope::new(scope, script_context);
            crate::native_bridge::element::inline_script_source_for_execution(
                script_scope,
                host_ptr,
                *node_id,
                &source,
                ContentSecurityPolicyScriptElementRequest {
                    nonce: script.fetch_metadata.nonce.as_deref(),
                    integrity: script.fetch_metadata.integrity.as_deref(),
                    parser_inserted: true,
                },
            )
        };
        let Some(source) = source else {
            self.sync_child_browsing_context_subtree(scope, document_handle);
            return None;
        };
        job.source = FrameScriptSource::SourceText(source.clone());
        let current_script = self.push_frame_script_job_current_script(&job);
        let parser_script_owner = self.current_child_document_task_owner(child_handle);
        let entered_parser_script_nesting = parser_script_owner.is_some_and(|task_owner| {
            self.enter_child_parser_script_nesting(child_handle, task_owner)
        });
        let result = {
            let script_scope = &mut v8::ContextScope::new(scope, script_context);
            crate::script_vm::execute_source_text_on_current_stack(
                script_scope,
                &source,
                Some(&job.script_url),
                Some(&job.base_url),
                (*start_line).saturating_sub(1).min(i32::MAX as u64) as i32,
                script.fetch_metadata.nonce.as_deref(),
                true,
            )
        };
        if entered_parser_script_nesting && let Some(task_owner) = parser_script_owner {
            self.exit_child_parser_script_nesting(child_handle, task_owner);
        }
        if let Some(current_script) = current_script {
            self.pop_child_current_script(current_script);
        }
        if let Err(error) = result {
            tracing::debug!(
                ?child_handle,
                script_handle = ?node_id,
                %error,
                "child document.write inline parser script failed"
            );
        }
        self.sync_child_browsing_context_subtree(scope, document_handle);
        None
    }

    pub(in crate::native_bridge::context_host) fn create_empty_live_child_html_document(
        &mut self,
        document_url: Url,
        content_type: Option<&str>,
    ) -> DomHandle {
        let document_handle = self
            .dom_host_mut()
            .create_detached_html_document_with_url(document_url);
        if let Some(content_type) = content_type {
            let _ = self.set_dom_document_content_type_for_handle(document_handle, content_type);
        }
        document_handle
    }

    pub(in crate::native_bridge::context_host) fn start_live_child_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        document_handle: DomHandle,
        owner_local_window_id: LocalWindowId,
        owner_document_id: DocumentId,
        document_base_url: Url,
        markup: &str,
        is_xml_document: bool,
    ) -> ChildLiveDocumentParserStartResult {
        let owner = FrameDocumentOwner::new(owner_local_window_id, owner_document_id);
        self.child_document_parsers.clear(owner);
        let mut parser = if is_xml_document {
            DocumentParserSession::start_finite_live_xml_document(
                document_base_url,
                document_handle,
            )
        } else {
            DocumentParserSession::start_finite_live_document(document_base_url, document_handle)
        };
        let task_owner = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
            .expect("committed child parser must have a current task owner");
        assert_eq!(task_owner.document_owner(), owner);
        parser.queue_arrived_chunk(markup.to_owned());
        parser.declare_eof();
        let outcome = self.drive_live_child_document_parser(
            scope,
            child_handle,
            document_handle,
            &mut parser,
        );
        match outcome {
            ParserProgress::ReadyToFinish => {
                let parser_stop_action = self.finish_live_child_document_parser(
                    scope,
                    child_handle,
                    owner,
                    document_handle,
                    parser,
                );
                ChildLiveDocumentParserStartResult::parser_stopped(parser_stop_action)
            }
            ParserProgress::WaitingForInput => {
                self.child_document_parsers.replace(owner, parser);
                ChildLiveDocumentParserStartResult::parser_blocked(None)
            }
            ParserProgress::BlockedOnParserScript { ready_work } => {
                self.child_document_parsers.replace(owner, parser);
                ChildLiveDocumentParserStartResult::parser_blocked(ready_work.map(|work| *work))
            }
            ParserProgress::BlockedOnStylesheetPause => {
                self.child_document_parsers.replace(owner, parser);
                ChildLiveDocumentParserStartResult::parser_blocked(None)
            }
            ParserProgress::Stopped => ChildLiveDocumentParserStartResult::parser_stopped(None),
        }
    }

    pub(crate) fn resume_live_child_document_parser_after_blocker(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        owner: FrameDocumentOwner,
    ) -> FrameDocumentClassicParserResumeApplication {
        if !self
            .frame_owner_store
            .child_document_owner_is_current(child_handle, owner)
        {
            self.child_document_parsers.clear(owner);
            return FrameDocumentClassicParserResumeApplication::skipped(
                FrameDocumentClassicParserResumeSkipReason::StaleDocumentOwner,
            );
        }
        let Some(snapshot) = self.frame_owner_current_child_snapshot(child_handle) else {
            self.child_document_parsers.clear(owner);
            return FrameDocumentClassicParserResumeApplication::skipped(
                FrameDocumentClassicParserResumeSkipReason::MissingCurrentChildSnapshot,
            );
        };
        let Some(mut entry) = self.child_document_parsers.take(owner) else {
            return FrameDocumentClassicParserResumeApplication::skipped(
                FrameDocumentClassicParserResumeSkipReason::MissingLiveParser,
            );
        };
        let parser_was_suspended_for_classic_script = matches!(
            entry.suspension_cause(),
            Some(
                ParserSuspensionCause::ParserClassicSource { .. }
                    | ParserSuspensionCause::ParserClassicStylesheets { .. }
            )
        );
        if parser_was_suspended_for_classic_script
            && self
                .frame_parser_classic_scripts
                .has_current_parser_blocking_script(owner)
        {
            if let Some(work) =
                self.take_child_classic_script_scheduler_work_for_current_document(child_handle)
            {
                self.child_document_parsers.replace(owner, entry);
                return FrameDocumentClassicParserResumeApplication::resumed(Some(work));
            }
            let _ = self.queue_child_classic_script_source_load_task(child_handle);
            self.child_document_parsers.replace(owner, entry);
            return FrameDocumentClassicParserResumeApplication::resumed(None);
        }
        match entry.run_state() {
            DocumentParserRunState::Ready => {}
            DocumentParserRunState::Suspended { .. } => {
                let Some(permit) = entry.current_resume_permit() else {
                    self.child_document_parsers.replace(owner, entry);
                    return FrameDocumentClassicParserResumeApplication::skipped(
                        FrameDocumentClassicParserResumeSkipReason::StaleParserSuspension,
                    );
                };
                if !entry.resume(permit) {
                    self.child_document_parsers.replace(owner, entry);
                    return FrameDocumentClassicParserResumeApplication::skipped(
                        FrameDocumentClassicParserResumeSkipReason::StaleParserSuspension,
                    );
                }
            }
            DocumentParserRunState::Pumping { .. }
            | DocumentParserRunState::Finishing
            | DocumentParserRunState::Finished
            | DocumentParserRunState::Stopped(_) => {
                self.child_document_parsers.replace(owner, entry);
                return FrameDocumentClassicParserResumeApplication::skipped(
                    FrameDocumentClassicParserResumeSkipReason::StaleParserSuspension,
                );
            }
        }
        let document_handle = snapshot.document_handle;
        let outcome =
            self.drive_live_child_document_parser(scope, child_handle, document_handle, &mut entry);
        match outcome {
            ParserProgress::ReadyToFinish => {
                self.finish_and_queue_live_child_document_parser(
                    scope,
                    child_handle,
                    owner,
                    document_handle,
                    entry,
                );
                FrameDocumentClassicParserResumeApplication::resumed(None)
            }
            ParserProgress::WaitingForInput => {
                self.child_document_parsers.replace(owner, entry);
                FrameDocumentClassicParserResumeApplication::resumed(None)
            }
            ParserProgress::BlockedOnParserScript { ready_work } => {
                self.child_document_parsers.replace(owner, entry);
                FrameDocumentClassicParserResumeApplication::resumed(ready_work.map(|work| *work))
            }
            ParserProgress::BlockedOnStylesheetPause => {
                self.child_document_parsers.replace(owner, entry);
                FrameDocumentClassicParserResumeApplication::resumed(None)
            }
            ParserProgress::Stopped => FrameDocumentClassicParserResumeApplication::resumed(None),
        }
    }

    pub(crate) fn resume_live_child_document_parser_for_classic_execution(
        &mut self,
        child_handle: DomHandle,
        task_owner: FrameDocumentTaskOwner,
        script_handle: DomHandle,
    ) -> bool {
        if self.current_child_document_task_owner(child_handle) != Some(task_owner) {
            return false;
        }
        let owner = task_owner.document_owner();
        let Some(mut entry) = self.child_document_parsers.take(owner) else {
            return false;
        };
        let resumed = match entry.run_state() {
            // Source completion may already have consumed the exact permit
            // while promoting ready scheduler work. The current task owner and
            // realm gate still authorize this parser execution.
            DocumentParserRunState::Ready => true,
            DocumentParserRunState::Suspended { .. }
                if matches!(
                    entry.suspension_cause(),
                    Some(ParserSuspensionCause::ParserClassicSource { script }
                        | ParserSuspensionCause::ParserClassicStylesheets { script })
                        if script == script_handle
                ) =>
            {
                entry
                    .current_resume_permit()
                    .is_some_and(|permit| entry.resume(permit))
            }
            DocumentParserRunState::Pumping { .. }
            | DocumentParserRunState::Suspended { .. }
            | DocumentParserRunState::Finishing
            | DocumentParserRunState::Finished
            | DocumentParserRunState::Stopped(_) => false,
        };
        self.child_document_parsers.replace(owner, entry);
        resumed
    }
}
