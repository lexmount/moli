use super::script_scheduling::DocumentWriteCurrentScriptEventBehavior;
use super::*;
use crate::document_script_scheduler::{
    DocumentScriptExecutionLane, MainParserAsyncModuleAdmission, PageOwnedDocumentScriptWork,
};
use crate::frame_owner_model::MainDocumentScriptLoadDelayKind;
use crate::host::{ScriptEventKind, ScriptEventTask};
use crate::live_document_parser::LiveDocumentParserOwner;
use crate::parser::{
    DocumentStream, ParserPumpStep, ParserScriptHandoff, ParserYield, PreparedImportMap,
    PreparedImportMapSource,
};
use crate::planning::SharedScriptSourceLoad;
use crate::stylesheet_blocking::DocumentBlockingStylesheetSignature;
use crate::types::{ScriptKind, ScriptMode};
use html5ever::tree_builder::QuirksMode;
use moli_parser::{
    ParserDomMutation, ParserDomMutationConsumer, ParserDomReadConsumer,
    ParserElementCreationConsumer, ParserElementCreationRequest, ParserMutationEffectConsumer,
    ParserPumpOutcome,
};
use std::collections::HashSet;
use tracing::debug;

struct DocumentWriteParserPumpStep {
    outcome: ParserPumpOutcome,
}

enum DocumentWriteParserPumpInput<'a> {
    Inserted(&'a str),
    Ordinary(&'a str),
    QueuedOrBuffered,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HtmlFragmentParserContextMode {
    Standard,
    RangeCreateContextualFragment,
}

struct DocumentWriteParserMutationOwner<'a, 'scope, 'pin> {
    runtime: &'a mut DocumentRuntime,
    scope: &'a mut v8::PinScope<'scope, 'pin>,
    host_ptr: *mut JsContextHost,
}

impl LiveDocumentParserOwner for DocumentWriteParserMutationOwner<'_, '_, '_> {}

impl ParserMutationEffectConsumer for DocumentWriteParserMutationOwner<'_, '_, '_> {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
        self.runtime
            .apply_parser_stream_mutation_effects_to_live_dom_host(
                self.scope,
                self.host_ptr,
                effects,
            );
    }
}

impl ParserDomReadConsumer for DocumentWriteParserMutationOwner<'_, '_, '_> {
    fn node_exists(&mut self, node_id: DomHandle) -> bool {
        self.runtime.parser_runtime_dom_node_exists(node_id)
    }

    fn is_connected(&mut self, node_id: DomHandle) -> bool {
        self.runtime.parser_runtime_dom_is_connected(node_id)
    }

    fn is_text_node(&mut self, node_id: DomHandle) -> bool {
        self.runtime.parser_runtime_dom_is_text_node(node_id)
    }

    fn owner_document(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.runtime.parser_runtime_dom_owner_document(node_id)
    }

    fn parent_node(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.runtime.parser_runtime_dom_parent_node(node_id)
    }

    fn previous_sibling(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.runtime.parser_runtime_dom_previous_sibling(node_id)
    }

    fn last_child(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.runtime.parser_runtime_dom_last_child(node_id)
    }

    fn child_handles(&mut self, node_id: DomHandle) -> Vec<DomHandle> {
        self.runtime.parser_runtime_dom_child_handles(node_id)
    }

    fn document_order_script_handles(&mut self, document_handle: DomHandle) -> Vec<DomHandle> {
        self.runtime
            .parser_runtime_dom_document_order_script_handles(document_handle)
    }

    fn document_order_stylesheet_candidate_handles_before(
        &mut self,
        document_handle: DomHandle,
        stop_at: Option<DomHandle>,
    ) -> Vec<DomHandle> {
        self.runtime
            .parser_runtime_dom_document_order_stylesheet_candidate_handles_before(
                document_handle,
                stop_at,
            )
    }

    fn document_body_handle_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.runtime
            .parser_runtime_dom_document_body_handle_for_document(document_handle)
    }

    fn document_base_url(&mut self, document_handle: DomHandle) -> Option<url::Url> {
        self.runtime
            .parser_runtime_dom_document_base_url(document_handle)
    }

    fn template_contents_handle(&mut self, node_id: DomHandle) -> Option<DomHandle> {
        self.runtime
            .parser_runtime_dom_template_contents_handle(node_id)
    }

    fn is_html_element_named(&mut self, node_id: DomHandle, local_name: &str) -> bool {
        self.runtime
            .parser_runtime_dom_is_html_element_named(node_id, local_name)
    }

    fn is_external_async_classic_candidate(&mut self, node_id: DomHandle) -> bool {
        self.runtime
            .parser_runtime_dom_is_external_async_classic_candidate(node_id)
    }

    fn parser_script_read(&mut self, node_id: DomHandle) -> Option<crate::ParserScriptRead> {
        self.runtime.parser_runtime_dom_parser_script_read(node_id)
    }

    fn stylesheet_element(&mut self, node_id: DomHandle) -> Option<crate::StylesheetElementRead> {
        self.runtime.parser_runtime_dom_stylesheet_element(node_id)
    }

    fn text_content(&mut self, node_id: DomHandle) -> Option<String> {
        self.runtime.parser_runtime_dom_text_content(node_id)
    }
}

impl ParserDomMutationConsumer for DocumentWriteParserMutationOwner<'_, '_, '_> {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) {
        self.runtime.apply_parser_dom_mutation_to_live_dom_host(
            self.scope,
            self.host_ptr,
            mutation,
        );
        self.runtime
            .run_pending_parser_post_step_runtime_work(self.scope, self.host_ptr);
    }

    fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.runtime
            .create_parser_element_without_attributes_in_live_dom_host(
                local_name, namespace, prefix,
            )
    }

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        document_handle: DomHandle,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.runtime
            .create_parser_element_for_document_without_attributes_in_live_dom_host(
                document_handle,
                local_name,
                namespace,
                prefix,
            )
    }

    fn add_attrs_if_missing_for_parser(
        &mut self,
        node_id: DomHandle,
        attrs: Vec<crate::dom::native::Attribute>,
    ) {
        self.runtime
            .add_attrs_if_missing_for_parser_in_live_dom_host(node_id, attrs);
    }

    fn create_text_node(&mut self, text: String) -> DomHandle {
        self.runtime.create_text_node_in_live_dom_host(text)
    }

    fn create_comment(&mut self, text: String) -> DomHandle {
        self.runtime.create_comment_in_live_dom_host(text)
    }

    fn create_processing_instruction(&mut self, target: String, data: String) -> DomHandle {
        self.runtime
            .create_processing_instruction_in_live_dom_host(target, data)
    }

    fn create_cdata_section(&mut self, data: String) -> DomHandle {
        self.runtime.create_cdata_section_in_live_dom_host(data)
    }

    fn create_document_type(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> DomHandle {
        self.runtime
            .create_document_type_in_live_dom_host(name, public_id, system_id)
    }

    fn prepend_text_to_text_node(&mut self, node_id: DomHandle, text: String) {
        self.runtime
            .prepend_text_to_text_node_in_live_dom_host(node_id, text);
    }

    fn append_text_to_text_node(&mut self, node_id: DomHandle, text: String) {
        self.runtime
            .append_text_to_text_node_in_live_dom_host(node_id, text);
    }

    fn push_parse_error(&mut self, error: String) {
        self.runtime.push_parse_error_in_live_dom_host(error);
    }

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode) {
        self.runtime
            .set_html_quirks_mode_for_parser_in_live_dom_host(quirks_mode);
    }

    fn mark_script_already_started_for_parser(&mut self, node_id: DomHandle) {
        self.runtime
            .mark_script_already_started_for_parser_in_live_dom_host(node_id);
    }

    fn finish_parsing_script_children(&mut self, node_id: DomHandle) {
        let _ = self
            .runtime
            .dom_host_mut()
            .finish_parsing_script_children(node_id);
    }

    fn finish_parsing_link_children(&mut self, node_id: DomHandle) {
        let _ = self
            .runtime
            .dom_host_mut()
            .finish_parsing_link_children(node_id);
    }

    fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: DomHandle,
        template_id: DomHandle,
        attrs: Vec<crate::dom::native::Attribute>,
    ) -> bool {
        self.runtime
            .attach_declarative_shadow_for_parser_in_live_dom_host(host_id, template_id, attrs)
    }

    fn associate_parser_form_owner(&mut self, target: DomHandle, form: DomHandle) -> bool {
        self.runtime
            .associate_parser_form_owner_in_live_dom_host(target, form)
    }
}

impl ParserElementCreationConsumer for DocumentWriteParserMutationOwner<'_, '_, '_> {
    fn create_parser_element(
        &mut self,
        request: ParserElementCreationRequest<'_>,
    ) -> Option<DomHandle> {
        let document_has_body = self
            .document_body_handle_for_document(request.document_handle)
            .is_some();
        let runtime = &mut *self.runtime;
        custom_elements::create_and_construct_parser_custom_element_direct_for_document(
            self.scope,
            self.host_ptr,
            request.document_handle,
            document_has_body,
            request.local_name,
            request.namespace,
            request.prefix,
            request.attributes,
            request.intended_parent,
            |document_handle, local_name, namespace, prefix| {
                runtime.create_parser_element_for_document_without_attributes_in_live_dom_host(
                    document_handle,
                    local_name,
                    namespace,
                    prefix,
                )
            },
        )
    }
}

// This slice groups the HTML-fragment and document.write-adjacent paths.
//
// The reason to isolate it as one unit is that these entrypoints all sit at the boundary between:
// - parsing HTML into a parser-owned fragment or document.write stream
// - applying produced nodes through the runtime mutation owner
// - running the follow-up effects that fragment insertion or document.write can trigger
//
// It is still intentionally *not* the general DOM-mutation facade. Plain append/insert/remove
// commands stay in `document_runtime.rs` for now. This keeps the file boundary aligned with
// "HTML string -> parser output -> document.write/fragment effects" rather than mixing two different
// mutation models in one refactor slice.
impl DocumentRuntime {
    pub(crate) fn start_root_document_parser_stream(&mut self) {
        debug_assert!(
            self.root_document_parser.is_none(),
            "opening a root document must discard the previous parser stream first"
        );
        let parser = DocumentParserSession::start_open_live_document(
            self.document_url().clone(),
            self.document_handle(),
        );
        self.root_document_parser = Some(parser);
    }

    pub(crate) fn close_root_document_parser_stream(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> bool {
        if self.root_document_parser.is_none() {
            return false;
        }
        if self
            .root_document_parser
            .as_ref()
            .is_some_and(|parser| parser.lifetime() != DocumentParserLifetime::Closing)
        {
            let closed = self.document.close_live_document_stream();
            debug_assert!(
                closed,
                "a live root parser must own an open document stream"
            );
            self.root_document_parser
                .as_mut()
                .expect("root document parser existence was checked")
                .request_close();
        }
        let _ = self.finish_root_document_parser_stream_if_ready(scope, host_ptr);
        true
    }

    fn finish_root_document_parser_stream_if_ready(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> bool {
        if self
            .root_document_parser
            .as_ref()
            .is_none_or(|parser| parser.lifetime() != DocumentParserLifetime::Closing)
            || self.has_pending_document_write_parser_blocking_work()
            || self
                .root_document_parser
                .as_ref()
                .is_some_and(|parser| parser.run_state() != DocumentParserRunState::Ready)
            || self
                .root_document_parser
                .as_ref()
                .is_some_and(|parser| !parser.input_is_empty())
        {
            return false;
        }
        // A speculative script preload can complete before a blocking
        // stylesheet. Resuming that stylesheet may then consume the ready
        // script synchronously while an outer parser pump still owns this
        // stream. EOF is real, but finalization must wait until that recovery
        // stack unwinds and the root controller is again the sole owner.
        if self
            .root_document_parser
            .as_ref()
            .is_some_and(|parser| !parser.has_exclusive_stream_handle())
        {
            return false;
        }
        let Some(mut parser) = self.root_document_parser.take() else {
            return false;
        };
        let finish_signals = self.with_dom_host_parse_step(|runtime| {
            let mut mutation_owner = DocumentWriteParserMutationOwner {
                runtime,
                scope,
                host_ptr,
            };
            parser.finish(&mut mutation_owner)
        });
        custom_elements::apply_parser_created_null_registry_associations(
            host_ptr,
            &finish_signals.parser_created_null_registry_elements,
        );
        self.accept_document_write_parser_modulepreloads(
            scope,
            host_ptr,
            finish_signals
                .discovery_signals
                .modulepreload_link_candidates,
        );
        self.note_discovered_document_owned_blocking_stylesheet_inputs(
            finish_signals
                .discovery_signals
                .blocking_stylesheet_inputs
                .iter(),
        );
        unsafe { &mut *host_ptr }.resync_child_browsing_contexts(scope);
        self.run_pending_parser_post_step_runtime_work(scope, host_ptr);
        self.style_source_document_sync_pending = true;
        true
    }

    fn frameset_fragment_child_is_ignored(is_frameset_context: bool, child: &Node) -> bool {
        is_frameset_context && child.is_html_element_named("template")
    }

    fn inner_html_document_handle(&self, handle: DomHandle) -> Option<DomHandle> {
        if self.dom_host().node(handle).is_some_and(Node::is_document) {
            Some(handle)
        } else {
            self.dom_host()
                .node(handle)
                .and_then(Node::owner_document)
                .or_else(|| Some(self.dom_host().document_handle()))
        }
    }

    pub(crate) fn fragment_context_custom_element_registry_association(
        &self,
        host_ptr: *mut JsContextHost,
        context_handle: DomHandle,
    ) -> custom_elements::CustomElementRegistryAssociation {
        if self
            .dom_host()
            .node(context_handle)
            .is_some_and(|node| node.is_html_element_named("template"))
        {
            return custom_elements::CustomElementRegistryAssociation::Null;
        }
        unsafe { &*host_ptr }.effective_custom_element_registry_association(context_handle)
    }

    fn set_fragment_root_custom_element_registry_association(
        &mut self,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
        association: custom_elements::CustomElementRegistryAssociation,
    ) {
        if self.dom_host().node(root).is_some_and(|node| {
            node.is_document() || node.is_document_fragment() || node.is_element()
        }) {
            unsafe { &mut *host_ptr }.set_custom_element_registry_association(root, association);
        }
    }

    fn set_fragment_null_registry_associations_from_native_dom(
        &self,
        host_ptr: *mut JsContextHost,
        source: &NativeDom,
        source_root: DomHandle,
        imported_root: DomHandle,
    ) {
        let mut stack = vec![(source_root, imported_root)];
        while let Some((source_handle, imported_handle)) = stack.pop() {
            if source_node_has_null_registry_attribute(source.node(source_handle)) {
                unsafe { &mut *host_ptr }.set_custom_element_registry_association(
                    imported_handle,
                    custom_elements::CustomElementRegistryAssociation::Null,
                );
            }

            let source_children = source.child_ids(source_handle).collect::<Vec<_>>();
            let imported_children = self
                .dom_host()
                .child_handles(imported_handle)
                .collect::<Vec<_>>();
            for (source_child, imported_child) in
                source_children.into_iter().zip(imported_children).rev()
            {
                stack.push((source_child, imported_child));
            }
            if let (Some(source_template), Some(imported_template)) = (
                source
                    .node(source_handle)
                    .and_then(Node::as_element)
                    .and_then(|element| element.template_contents()),
                self.dom_host()
                    .node(imported_handle)
                    .and_then(Node::as_element)
                    .and_then(|element| element.template_contents()),
            ) {
                stack.push((source_template, imported_template));
            }
        }
    }

    fn set_fragment_null_registry_associations_from_dom_host(
        &self,
        host_ptr: *mut JsContextHost,
        source: &DomHost,
        source_root: DomHandle,
        imported_root: DomHandle,
    ) {
        let mut stack = vec![(source_root, imported_root)];
        while let Some((source_handle, imported_handle)) = stack.pop() {
            if source_node_has_null_registry_attribute(source.node(source_handle)) {
                unsafe { &mut *host_ptr }.set_custom_element_registry_association(
                    imported_handle,
                    custom_elements::CustomElementRegistryAssociation::Null,
                );
            }

            let source_children = source.child_handles(source_handle).collect::<Vec<_>>();
            let imported_children = self
                .dom_host()
                .child_handles(imported_handle)
                .collect::<Vec<_>>();
            for (source_child, imported_child) in
                source_children.into_iter().zip(imported_children).rev()
            {
                stack.push((source_child, imported_child));
            }
            if let (Some(source_template), Some(imported_template)) = (
                source
                    .node(source_handle)
                    .and_then(Node::as_element)
                    .and_then(|element| element.template_contents()),
                self.dom_host()
                    .node(imported_handle)
                    .and_then(Node::as_element)
                    .and_then(|element| element.template_contents()),
            ) {
                stack.push((source_template, imported_template));
            }
            if let (Some(source_shadow), Some(imported_shadow)) = (
                source.shadow_root_handle(source_handle),
                self.dom_host().shadow_root_handle(imported_handle),
            ) {
                self.set_fragment_declarative_shadow_root_registry_association(
                    host_ptr,
                    source,
                    source_shadow,
                    imported_shadow,
                );
                stack.push((source_shadow, imported_shadow));
            }
        }
    }

    fn set_fragment_declarative_shadow_root_registry_association(
        &self,
        host_ptr: *mut JsContextHost,
        source: &DomHost,
        source_shadow: DomHandle,
        imported_shadow: DomHandle,
    ) {
        if !source
            .shadow_root_is_declarative(source_shadow)
            .unwrap_or(false)
        {
            return;
        }
        let association = if source
            .shadow_root_uses_null_custom_element_registry(source_shadow)
            .unwrap_or(false)
        {
            custom_elements::CustomElementRegistryAssociation::Null
        } else {
            let Some(owner_document) = self.dom_host().owner_document_handle(imported_shadow)
            else {
                return;
            };
            unsafe { &*host_ptr }.effective_custom_element_registry_association(owner_document)
        };
        unsafe { &mut *host_ptr }
            .set_custom_element_registry_association(imported_shadow, association);
    }

    fn build_fragment_from_html_with_context_mode(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        context_handle: DomHandle,
        html: &str,
        scripts_already_started: bool,
        custom_element_upgrade_timing: HtmlFragmentCustomElementUpgradeTiming,
        context_mode: HtmlFragmentParserContextMode,
        scripting_enabled: bool,
    ) -> Option<DomHandle> {
        let first_node_index = self.dom_host().dom().len();
        let parser = HtmlParser;
        let context_node = self.dom_host().node(context_handle);
        let context_namespace = context_node
            .and_then(Node::namespace)
            .unwrap_or("http://www.w3.org/1999/xhtml")
            .to_owned();
        let mut context_local_name = context_node
            .and_then(Node::local_name)
            .unwrap_or("body")
            .to_owned();
        if context_mode == HtmlFragmentParserContextMode::RangeCreateContextualFragment
            && context_namespace == "http://www.w3.org/1999/xhtml"
            && context_local_name.eq_ignore_ascii_case("html")
        {
            context_local_name = "body".to_owned();
        }
        let parsed = parser.parse_fragment_without_declarative_shadow_roots_with_scripting(
            self.document_url().clone(),
            &context_namespace,
            &context_local_name,
            html.to_owned(),
            scripting_enabled,
        );
        let is_frameset_context = context_namespace == "http://www.w3.org/1999/xhtml"
            && context_local_name.eq_ignore_ascii_case("frameset");
        let registry_association =
            self.fragment_context_custom_element_registry_association(host_ptr, context_handle);
        let fragment = self.create_document_fragment();
        self.initialize_new_native_node_owner_document(document_handle, fragment)
            .map(|_| ())?;
        let preserves_html_context_wrappers = custom_element_upgrade_timing
            == HtmlFragmentCustomElementUpgradeTiming::AfterInsertion
            && context_namespace == "http://www.w3.org/1999/xhtml"
            && context_local_name.eq_ignore_ascii_case("html");
        let source_root = if preserves_html_context_wrappers {
            let document_root = parsed.document_node_id();
            parsed
                .child_ids(document_root)
                .find(|child| {
                    parsed
                        .node(*child)
                        .is_some_and(|node| node.is_html_element_named("html"))
                })
                .unwrap_or(document_root)
        } else {
            parsed.body_node_id().unwrap_or_else(|| {
                let document_root = parsed.document_node_id();
                let document_children = parsed.child_ids(document_root).collect::<Vec<_>>();
                if document_children.len() == 1
                    && parsed
                        .node(document_children[0])
                        .is_some_and(|node| node.is_html_element_named("html"))
                {
                    document_children[0]
                } else {
                    document_root
                }
            })
        };
        for child in parsed.child_ids(source_root) {
            if parsed.node(child).is_some_and(|node| {
                Self::frameset_fragment_child_is_ignored(is_frameset_context, node)
            }) {
                continue;
            }
            let imported =
                self.dom_host_mut()
                    .import_foreign_node(document_handle, &parsed, child, true)?;
            self.set_fragment_root_custom_element_registry_association(
                host_ptr,
                imported,
                registry_association,
            );
            self.set_fragment_null_registry_associations_from_native_dom(
                host_ptr, &parsed, child, imported,
            );
            self.dom_host_mut()
                .set_subtree_script_already_started(imported, scripts_already_started);
            let _ = self.dom_host_mut().append_child(fragment, imported);
            if custom_element_upgrade_timing
                == HtmlFragmentCustomElementUpgradeTiming::InReturnedFragment
                && !custom_elements::upgrade_subtree_if_defined(scope, host_ptr, imported)
            {
                return None;
            }
        }
        unsafe { &mut *host_ptr }.capture_node_creation_stack_traces_since(scope, first_node_index);
        Some(fragment)
    }

    pub(crate) fn build_range_contextual_fragment_from_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        context_handle: DomHandle,
        html: &str,
        scripting_enabled: bool,
    ) -> Option<DomHandle> {
        self.build_fragment_from_html_with_context_mode(
            scope,
            host_ptr,
            document_handle,
            context_handle,
            html,
            false,
            HtmlFragmentCustomElementUpgradeTiming::InReturnedFragment,
            HtmlFragmentParserContextMode::RangeCreateContextualFragment,
            scripting_enabled,
        )
    }

    pub(crate) fn build_fragment_from_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        context_handle: DomHandle,
        html: &str,
        scripts_already_started: bool,
        custom_element_upgrade_timing: HtmlFragmentCustomElementUpgradeTiming,
    ) -> Option<DomHandle> {
        self.build_fragment_from_html_with_context_mode(
            scope,
            host_ptr,
            document_handle,
            context_handle,
            html,
            scripts_already_started,
            custom_element_upgrade_timing,
            HtmlFragmentParserContextMode::Standard,
            true,
        )
    }

    pub(crate) fn build_unsafe_fragment_from_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        context_handle: DomHandle,
        html: &str,
        custom_element_upgrade_timing: HtmlFragmentCustomElementUpgradeTiming,
    ) -> Option<DomHandle> {
        let first_node_index = self.dom_host().dom().len();
        let parser = HtmlParser;
        let context_node = self.dom_host().node(context_handle);
        let context_namespace = context_node
            .and_then(Node::namespace)
            .unwrap_or("http://www.w3.org/1999/xhtml")
            .to_owned();
        let context_local_name = context_node
            .and_then(Node::local_name)
            .unwrap_or("body")
            .to_owned();
        let parsed = parser.parse_fragment_dom_host(
            self.document_url().clone(),
            &context_namespace,
            &context_local_name,
            html.to_owned(),
        );
        let is_frameset_context = context_namespace == "http://www.w3.org/1999/xhtml"
            && context_local_name.eq_ignore_ascii_case("frameset");
        let registry_association =
            self.fragment_context_custom_element_registry_association(host_ptr, context_handle);
        let fragment = self.create_document_fragment();
        self.initialize_new_native_node_owner_document(document_handle, fragment)
            .map(|_| ())?;
        let preserves_html_context_wrappers = custom_element_upgrade_timing
            == HtmlFragmentCustomElementUpgradeTiming::AfterInsertion
            && context_namespace == "http://www.w3.org/1999/xhtml"
            && context_local_name.eq_ignore_ascii_case("html");
        let source_root = if preserves_html_context_wrappers {
            let document_root = parsed.document_handle();
            parsed
                .child_handles(document_root)
                .find(|child| {
                    parsed
                        .node(*child)
                        .is_some_and(|node| node.is_html_element_named("html"))
                })
                .unwrap_or(document_root)
        } else {
            parsed.dom().body_node_id().unwrap_or_else(|| {
                let document_root = parsed.document_handle();
                let document_children = parsed.child_handles(document_root).collect::<Vec<_>>();
                if document_children.len() == 1
                    && parsed
                        .node(document_children[0])
                        .is_some_and(|node| node.is_html_element_named("html"))
                {
                    document_children[0]
                } else {
                    document_root
                }
            })
        };
        for child in parsed.child_handles(source_root) {
            if parsed.node(child).is_some_and(|node| {
                Self::frameset_fragment_child_is_ignored(is_frameset_context, node)
            }) {
                continue;
            }
            let imported = self.dom_host_mut().import_foreign_node_with_shadow_roots(
                document_handle,
                &parsed,
                child,
                true,
            )?;
            self.set_fragment_root_custom_element_registry_association(
                host_ptr,
                imported,
                registry_association,
            );
            self.set_fragment_null_registry_associations_from_dom_host(
                host_ptr, &parsed, child, imported,
            );
            self.dom_host_mut()
                .set_subtree_script_already_started(imported, true);
            let _ = self.dom_host_mut().append_child(fragment, imported);
            if custom_element_upgrade_timing
                == HtmlFragmentCustomElementUpgradeTiming::InReturnedFragment
                && !custom_elements::upgrade_subtree_if_defined(scope, host_ptr, imported)
            {
                return None;
            }
        }
        unsafe { &mut *host_ptr }.capture_node_creation_stack_traces_since(scope, first_node_index);
        Some(fragment)
    }

    fn is_template_contents_fragment(&self, handle: DomHandle) -> bool {
        self.dom_host()
            .node(handle)
            .is_some_and(Node::is_document_fragment)
            && self.dom_host().nodes().iter().any(|node| {
                node.as_element()
                    .and_then(|element| element.template_contents())
                    == Some(handle)
            })
    }

    fn upgrade_inserted_html_fragment_custom_elements(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_parent: DomHandle,
        roots: &[DomHandle],
    ) -> bool {
        if roots.is_empty()
            || self.is_template_contents_fragment(insertion_parent)
            || unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent()
        {
            return true;
        }
        for &root in roots {
            custom_elements::enqueue_upgrade_reactions_for_subtree(scope, host_ptr, root);
        }
        true
    }

    pub(crate) fn set_inner_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        html: &str,
    ) -> bool {
        let context = handle;
        let target = self
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .and_then(|element| element.template_contents())
            .unwrap_or(handle);
        let Some(document_handle) = self.inner_html_document_handle(target) else {
            return false;
        };
        let existing_children = self
            .dom_host()
            .node(target)
            .map(|node| node.child_ids(self.dom_host().dom()).collect::<Vec<_>>())
            .unwrap_or_default();
        if html.is_empty() && existing_children.is_empty() {
            return false;
        }
        let Some(fragment) = self.build_fragment_from_html(
            scope,
            host_ptr,
            document_handle,
            context,
            html,
            true,
            HtmlFragmentCustomElementUpgradeTiming::AfterInsertion,
        ) else {
            return false;
        };
        let added_children = self.dom_host().child_handles(fragment).collect::<Vec<_>>();
        let records_enabled = self.dom_host().mutation_records_enabled();
        let removes_existing_children = !existing_children.is_empty();
        for &child in &existing_children {
            let _ = self
                .remove_child_appending_to_current_reaction_queue(scope, host_ptr, target, child);
        }
        let changed = self.append_html_fragment_child_appending_to_current_reaction_queue(
            scope, host_ptr, target, fragment,
        ) || removes_existing_children;
        if changed
            && !self.upgrade_inserted_html_fragment_custom_elements(
                scope,
                host_ptr,
                target,
                &added_children,
            )
        {
            return false;
        }
        if changed && records_enabled {
            crate::observer_runtime::coalesce_child_list_replacement_records(
                host_ptr,
                target,
                &added_children,
                &existing_children,
                None,
                None,
            );
        }
        changed
    }

    pub(crate) fn set_html_unsafe(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        html: &str,
    ) -> bool {
        let context = handle;
        let target = self
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .and_then(|element| element.template_contents())
            .unwrap_or(handle);
        let Some(document_handle) = self.inner_html_document_handle(target) else {
            return false;
        };
        let Some(fragment) = self.build_unsafe_fragment_from_html(
            scope,
            host_ptr,
            document_handle,
            context,
            html,
            HtmlFragmentCustomElementUpgradeTiming::AfterInsertion,
        ) else {
            return false;
        };
        let added_children = self.dom_host().child_handles(fragment).collect::<Vec<_>>();
        let existing_children = self
            .dom_host()
            .node(target)
            .map(|node| node.child_ids(self.dom_host().dom()).collect::<Vec<_>>())
            .unwrap_or_default();
        for child in existing_children {
            let _ = self
                .remove_child_appending_to_current_reaction_queue(scope, host_ptr, target, child);
        }
        let changed = self.append_html_fragment_child_appending_to_current_reaction_queue(
            scope, host_ptr, target, fragment,
        );
        if changed
            && !self.upgrade_inserted_html_fragment_custom_elements(
                scope,
                host_ptr,
                target,
                &added_children,
            )
        {
            return false;
        }
        changed
    }

    pub(crate) fn set_outer_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        html: &str,
    ) -> bool {
        let Some(parent) = self.dom_host().node(handle).and_then(Node::parent_node) else {
            return false;
        };
        let Some(document_handle) = self.inner_html_document_handle(handle) else {
            return false;
        };
        let previous_sibling = self.dom_host().node(handle).and_then(Node::prev_sibling);
        let next_sibling = self.dom_host().node(handle).and_then(Node::next_sibling);
        let records_enabled = self.dom_host().mutation_records_enabled();
        let removed =
            self.remove_child_appending_to_current_reaction_queue(scope, host_ptr, parent, handle);
        if !removed {
            return false;
        }
        if html.is_empty() {
            return true;
        }
        let Some(fragment) = self.build_fragment_from_html(
            scope,
            host_ptr,
            document_handle,
            parent,
            html,
            true,
            HtmlFragmentCustomElementUpgradeTiming::AfterInsertion,
        ) else {
            return false;
        };
        let added_children = self.dom_host().child_handles(fragment).collect::<Vec<_>>();
        let changed = self.insert_html_fragment_child_appending_to_current_reaction_queue(
            scope,
            host_ptr,
            parent,
            fragment,
            next_sibling,
        );
        if changed
            && !self.upgrade_inserted_html_fragment_custom_elements(
                scope,
                host_ptr,
                parent,
                &added_children,
            )
        {
            return false;
        }
        if changed && records_enabled {
            crate::observer_runtime::coalesce_child_list_replacement_records(
                host_ptr,
                parent,
                &added_children,
                std::slice::from_ref(&handle),
                previous_sibling,
                next_sibling,
            );
        }
        changed
    }

    pub(crate) fn insert_adjacent_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: DomHandle,
        document_handle: DomHandle,
        context_handle: DomHandle,
        html: &str,
        insert: impl FnOnce(&mut Self, &mut v8::PinScope<'_, '_>, *mut JsContextHost, DomHandle) -> bool,
    ) -> bool {
        let Some(fragment) = self.build_fragment_from_html(
            scope,
            host_ptr,
            document_handle,
            context_handle,
            html,
            true,
            HtmlFragmentCustomElementUpgradeTiming::AfterInsertion,
        ) else {
            return false;
        };
        let added_children = self.dom_host().child_handles(fragment).collect::<Vec<_>>();
        let _ = target;
        let changed = insert(self, scope, host_ptr, fragment);
        if changed
            && !self.upgrade_inserted_html_fragment_custom_elements(
                scope,
                host_ptr,
                context_handle,
                &added_children,
            )
        {
            return false;
        }
        changed
    }

    pub(crate) fn has_pending_document_write_external_script_load(&self) -> bool {
        self.pending_document_write_external_script_load.is_some()
    }

    #[cfg(test)]
    pub(crate) fn pending_document_write_external_script_fetch_target(
        &self,
    ) -> Option<crate::types::DocumentWriteExternalScriptFetchTarget> {
        self.pending_document_write_external_script_load
            .as_ref()
            .map(|pending| pending.target)
    }

    pub(crate) fn has_document_write_external_script_fetch_target(
        &self,
        target: crate::types::DocumentWriteExternalScriptFetchTarget,
    ) -> bool {
        self.pending_document_write_external_script_load
            .as_ref()
            .is_some_and(|pending| pending.target == target)
            || self
                .document_write_script_preloads
                .values()
                .any(|preload| preload.target == target)
    }

    pub(crate) fn has_pending_document_write_parser_blocking_work(&self) -> bool {
        self.pending_document_write_external_script_load.is_some()
            || self
                .pending_document_write_stylesheet_blocked_script
                .is_some()
            || self
                .pending_document_write_stylesheet_parser_pause
                .is_some()
    }

    pub(crate) fn has_unfinished_root_document_parser_stream(&self) -> bool {
        self.root_document_parser.is_some()
    }

    pub(crate) fn has_pending_document_write_stylesheet_blocked_script(&self) -> bool {
        self.pending_document_write_stylesheet_blocked_script
            .is_some()
            || self
                .pending_document_write_stylesheet_parser_pause
                .is_some()
            || self
                .pending_document_write_external_script_load
                .as_ref()
                .is_some_and(|pending| !pending.blocking_signatures_before.is_empty())
    }

    pub(crate) fn has_pending_document_write_parser_created_style_import_pause(&self) -> bool {
        self.pending_document_write_stylesheet_parser_pause
            .as_ref()
            .is_some_and(|pending| {
                !pending.blocking_signatures.is_empty()
                    && pending.blocking_signatures.iter().all(|signature| {
                        matches!(
                            signature,
                            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { .. }
                        )
                    })
            })
    }

    pub(crate) fn document_write_stylesheet_blocked_script_is_ready(&mut self) -> bool {
        let Some(signatures) = self
            .pending_document_write_stylesheet_parser_pause
            .as_ref()
            .map(|pending| pending.blocking_signatures.clone())
            .or_else(|| {
                self.pending_document_write_stylesheet_blocked_script
                    .as_ref()
                    .map(|pending| pending.blocking_signatures_before.clone())
            })
            .or_else(|| {
                self.pending_document_write_external_script_load
                    .as_ref()
                    .filter(|pending| !pending.blocking_signatures_before.is_empty())
                    .map(|pending| pending.blocking_signatures_before.clone())
            })
        else {
            return false;
        };
        !self.has_pending_parser_script_blocking_stylesheet_signatures(signatures.iter())
    }

    fn allocate_document_write_external_script_load_id(&mut self) -> u64 {
        let load_id = self.next_document_write_external_script_load_id;
        self.next_document_write_external_script_load_id = self
            .next_document_write_external_script_load_id
            .checked_add(1)
            .expect("document.write external-script load id space exhausted");
        load_id
    }

    fn scan_document_write_script_preloads(
        &mut self,
        host_ptr: *mut JsContextHost,
        html: &str,
        reset_scanner: bool,
    ) {
        if html.is_empty() || unsafe { &*host_ptr }.fetch_subresource_interception_enabled() {
            return;
        }
        let Some(resource_loader) = self.current_document_resource_loader() else {
            return;
        };
        let request_client = resource_loader.request_client().clone();
        if reset_scanner || self.document_write_script_preload_scanner.is_none() {
            let initiator_url = self
                .dom_host
                .document_base_url()
                .unwrap_or_else(|| self.document_url().clone());
            self.document_write_script_preload_scanner = Some(Box::new(
                crate::runtime::IncrementalBufferedScriptPreloadScanner::new(initiator_url),
            ));
        }
        let requests = self
            .document_write_script_preload_scanner
            .as_mut()
            .expect("document-write preload scanner must be installed")
            .push_html(html);
        let completion_tx = unsafe { &*host_ptr }.resource_completion_sender();
        let document_character_set = self.document_character_set().to_owned();
        let Some(task_owner) = unsafe { &*host_ptr }.current_main_document_task_owner() else {
            return;
        };
        let document_url = self.document_url().clone();
        for request in requests {
            if !request.is_parser_blocking_classic() {
                continue;
            }
            if self
                .script_element_request_csp_violation_with_request(
                    &request.url,
                    crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
                        nonce: request.fetch_metadata().nonce.as_deref(),
                        integrity: request.fetch_metadata().integrity.as_deref(),
                        parser_inserted: true,
                    },
                )
                .is_some()
            {
                // The real parser-owned script element remains responsible for
                // violation reporting and its eventual error transition.
                continue;
            }
            let key = request.cache_key();
            if self.document_write_script_preloads.contains_key(&key) {
                continue;
            }
            let load_id = self.allocate_document_write_external_script_load_id();
            let target =
                crate::types::DocumentWriteExternalScriptFetchTarget::new(task_owner, load_id);
            let script = request.to_preload_script();
            let resource_type_hint = request.resource_type_hint();
            self.document_write_script_preloads.insert(
                key,
                DocumentWriteScriptPreload {
                    request,
                    target,
                    ready_completion: None,
                },
            );
            let request_client = request_client.clone();
            let completion_tx = completion_tx.clone();
            let document_character_set = document_character_set.clone();
            let network_attribution =
                crate::types::DocumentWriteExternalScriptNetworkAttribution::new(
                    document_url.clone(),
                    script.url.clone(),
                );
            resource_loader.spawn_resource_task(async move {
                let outcome = crate::planning::load_prepared_script_source_outcome_with_document_character_set(
                    &script,
                    &request_client,
                    Some(&document_character_set),
                    Some(resource_type_hint),
                )
                .await;
                let _ = completion_tx.send_document_write_external_script(
                    crate::types::DocumentWriteExternalScriptLoadCompletion::new(
                        target,
                        outcome.source_result,
                        outcome.network_result,
                        network_attribution,
                    ),
                );
            });
        }
    }

    fn take_matching_document_write_script_preload(
        &mut self,
        script: &PreparedScript,
    ) -> Option<DocumentWriteScriptPreload> {
        let key = crate::runtime::BufferedScriptPreloadKey::from_script(script)?;
        let preload = self.document_write_script_preloads.remove(&key)?;
        if preload.request.matches_script(script) {
            Some(preload)
        } else {
            self.document_write_script_preloads.insert(key, preload);
            None
        }
    }

    fn append_to_pending_document_write_external_script_load(&mut self, html: &str) -> bool {
        let Some(pending) = self.pending_document_write_external_script_load.as_mut() else {
            return false;
        };
        pending
            .insertion
            .parser_insertion_controller
            .input_session()
            .enqueue_script_input_html(html.to_owned());
        pending
            .insertion
            .parser_insertion_controller
            .input_session()
            .enqueue_script_input_preload_html(html.to_owned());
        true
    }

    fn append_to_pending_document_write_stylesheet_blocked_script(&mut self, html: &str) -> bool {
        let Some(pending) = self
            .pending_document_write_stylesheet_blocked_script
            .as_mut()
        else {
            return false;
        };
        pending
            .insertion
            .parser_insertion_controller
            .input_session()
            .enqueue_script_input_html(html.to_owned());
        pending
            .insertion
            .parser_insertion_controller
            .input_session()
            .enqueue_script_input_preload_html(html.to_owned());
        true
    }

    fn append_to_pending_document_write_stylesheet_parser_pause(&mut self, html: &str) -> bool {
        let Some(pending) = self.pending_document_write_stylesheet_parser_pause.as_mut() else {
            return false;
        };
        pending
            .insertion
            .parser_insertion_controller
            .input_session()
            .enqueue_script_input_html(html.to_owned());
        pending
            .insertion
            .parser_insertion_controller
            .input_session()
            .enqueue_script_input_preload_html(html.to_owned());
        true
    }

    fn start_document_write_external_script_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        start: DocumentWriteExternalScriptStart,
        insertion: SuspendedDocumentWriteInsertion,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        if let Some(pending) = self.pending_document_write_external_script_load.as_mut() {
            tracing::error!(
                pending_load_id = pending.target.load_id(),
                queued_url = %start.script.url,
                "document.write external script load attempted while another load is pending"
            );
            pending.resume_after_completion.push_back(
                SuspendedDocumentWriteContinuation::StartExternal {
                    start,
                    insertion,
                    blocking_signatures_before,
                },
            );
            return true;
        }
        let Some(resource_loader) = unsafe { &*host_ptr }.current_main_document_resource_loader()
        else {
            return false;
        };
        let loader = resource_loader.request_client().clone();
        let preload = self.take_matching_document_write_script_preload(&start.script);
        let (target, preload_was_started, ready_completion) = match preload {
            Some(preload) => (preload.target, true, preload.ready_completion),
            None => {
                let Some(task_owner) = unsafe { &*host_ptr }.current_main_document_task_owner()
                else {
                    return false;
                };
                let load_id = self.allocate_document_write_external_script_load_id();
                (
                    crate::types::DocumentWriteExternalScriptFetchTarget::new(task_owner, load_id),
                    false,
                    None,
                )
            }
        };
        let network_attribution = crate::types::DocumentWriteExternalScriptNetworkAttribution::new(
            self.document_url().clone(),
            start.script.url.clone(),
        );
        let script_for_load = start.script.clone();
        self.pending_document_write_external_script_load =
            Some(PendingDocumentWriteExternalScriptLoad {
                target,
                start,
                insertion,
                blocking_signatures_before,
                ready_completion: None,
                resume_after_completion: VecDeque::new(),
            });
        if let Some(completion) = ready_completion {
            if self
                .pending_document_write_external_script_load
                .as_ref()
                .is_some_and(|pending| !pending.blocking_signatures_before.is_empty())
            {
                self.pending_document_write_external_script_load
                    .as_mut()
                    .expect("preloaded script must retain its pending owner")
                    .ready_completion = Some(completion);
                return true;
            }
            let pending = self
                .pending_document_write_external_script_load
                .take()
                .expect("ready preloaded script must retain its pending owner");
            let _ = self
                .finish_document_write_external_script_load(scope, host_ptr, pending, completion);
            return true;
        }
        if preload_was_started {
            return true;
        }
        let completion_tx = unsafe { &*host_ptr }.resource_completion_sender();
        let document_character_set = self.document_character_set().to_owned();
        resource_loader.spawn_resource_task(async move {
            let outcome =
                crate::planning::load_prepared_script_source_outcome_with_document_character_set(
                    &script_for_load,
                    &loader,
                    Some(&document_character_set),
                    None,
                )
                .await;
            let _ = completion_tx.send_document_write_external_script(
                crate::types::DocumentWriteExternalScriptLoadCompletion::new(
                    target,
                    outcome.source_result,
                    outcome.network_result,
                    network_attribution,
                ),
            );
        });
        true
    }

    fn queue_document_write_continuations_after_pending(
        &mut self,
        mut continuations: VecDeque<SuspendedDocumentWriteContinuation>,
    ) {
        let Some(pending) = self.pending_document_write_external_script_load.as_mut() else {
            debug_assert!(
                continuations.is_empty(),
                "document.write continuations should only be queued behind an active pending load"
            );
            return;
        };
        pending.resume_after_completion.append(&mut continuations);
    }

    fn resume_document_write_continuation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        continuation: SuspendedDocumentWriteContinuation,
    ) -> bool {
        match continuation {
            SuspendedDocumentWriteContinuation::ResumeAfterCompleted { start, insertion } => {
                let parser_insertion_controller = insertion.parser_insertion_controller.clone();
                self.set_current_script_context(CurrentScriptContextSpec {
                    handle: Some(start.node),
                    parser_write_insertion_point_active: true,
                    parser_insertion_controller: Some(parser_insertion_controller),
                });
                let changed =
                    self.resume_suspended_document_write_insertion(scope, host_ptr, insertion);
                self.clear_current_script_handle();
                changed
            }
            SuspendedDocumentWriteContinuation::StartExternal {
                start,
                insertion,
                blocking_signatures_before,
            } => self.start_document_write_external_script_load(
                scope,
                host_ptr,
                start,
                insertion,
                blocking_signatures_before,
            ),
        }
    }

    fn resume_document_write_continuations(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        first: SuspendedDocumentWriteContinuation,
        mut remaining: VecDeque<SuspendedDocumentWriteContinuation>,
    ) -> bool {
        let mut changed = false;
        let mut next = Some(first);
        while let Some(continuation) = next {
            if self.resume_document_write_continuation(scope, host_ptr, continuation) {
                changed = true;
            }
            if self.pending_document_write_external_script_load.is_some() {
                self.queue_document_write_continuations_after_pending(remaining);
                return true;
            }
            next = remaining.pop_front();
        }
        changed
    }

    fn resume_suspended_document_write_insertion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        mut insertion: SuspendedDocumentWriteInsertion,
    ) -> bool {
        if self.park_suspended_insertion_behind_reentrant_parser_work(&mut insertion) {
            return true;
        }
        if !Self::resume_document_write_insertion_permit(&mut insertion) {
            tracing::debug!(
                document_handle = ?insertion.document_handle,
                permit = ?insertion.resume_permit,
                run_state = ?insertion.parser_insertion_controller.run_state(),
                "dropping stale document.write parser continuation"
            );
            return false;
        }
        let mut changed = false;

        let parser_input_session = insertion.parser_insertion_controller.input_session();
        while let Some(script_input_html) = parser_input_session.take_next_script_input_html() {
            if self.write_html_via_parser_stream(
                scope,
                host_ptr,
                insertion.document_handle,
                &script_input_html,
                false,
                &insertion.parser_insertion_controller,
            ) {
                changed = true;
            }
            if self.has_pending_document_write_parser_blocking_work() {
                return true;
            }
        }

        // The blocker yielded from an insertion frame that is still resident
        // in HtmlParserSession. Draining one frame restores its parent and
        // reports an input boundary before that parent is pumped. Keep driving
        // the parser-owned segment stack until the complete input is drained
        // or another parser-blocking boundary takes ownership.
        loop {
            if self.write_html_via_parser_stream(
                scope,
                host_ptr,
                insertion.document_handle,
                "",
                true,
                &insertion.parser_insertion_controller,
            ) {
                changed = true;
            }
            if self.has_pending_document_write_parser_blocking_work() {
                return true;
            }
            if !insertion
                .parser_insertion_controller
                .parser_stream()
                .borrow()
                .has_pending_input()
            {
                break;
            }
        }

        changed
    }

    fn resume_document_write_insertion_permit(
        insertion: &mut SuspendedDocumentWriteInsertion,
    ) -> bool {
        if insertion.resume_permit_consumed {
            return insertion.parser_insertion_controller.run_state()
                == DocumentParserRunState::Ready;
        }
        let resumed = match insertion.parser_insertion_controller.run_state() {
            DocumentParserRunState::Ready => false,
            DocumentParserRunState::Suspended { .. } => insertion
                .parser_insertion_controller
                .resume(insertion.resume_permit),
            DocumentParserRunState::Pumping { .. }
            | DocumentParserRunState::Finishing
            | DocumentParserRunState::Finished
            | DocumentParserRunState::Stopped(_) => false,
        };
        if resumed {
            insertion.resume_permit_consumed = true;
        }
        resumed
    }

    fn resuspend_document_write_insertion(
        insertion: &mut SuspendedDocumentWriteInsertion,
        cause: ParserSuspensionCause,
    ) {
        insertion.resume_permit = insertion.parser_insertion_controller.suspend(cause);
        insertion.resume_permit_consumed = false;
    }

    /// A resumed parser script may synchronously create a newer blocking
    /// owner through document.write(). The older suspension's parser tail then
    /// belongs behind that new owner; consuming it immediately would parse
    /// past a stylesheet or script boundary that Blink keeps closed.
    fn park_suspended_insertion_behind_reentrant_parser_work(
        &mut self,
        _insertion: &mut SuspendedDocumentWriteInsertion,
    ) -> bool {
        // Every insertion frame and parent tail already resides in the shared
        // HtmlParserSession stack. A newer blocking owner therefore parks the
        // older continuation by ownership alone; there is no tail to move.
        self.has_pending_document_write_parser_blocking_work()
    }

    fn complete_document_write_script_preload(
        &mut self,
        completion: crate::types::DocumentWriteExternalScriptLoadCompletion,
    ) -> bool {
        let target = completion.target();
        let Some(preload) = self
            .document_write_script_preloads
            .values_mut()
            .find(|preload| preload.target == target)
        else {
            return false;
        };
        debug_assert!(
            preload.ready_completion.is_none(),
            "one speculative parser-script load must produce exactly one completion"
        );
        preload.ready_completion = Some(completion);
        true
    }

    pub(crate) fn complete_document_write_external_script_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        completion: crate::types::DocumentWriteExternalScriptLoadCompletion,
    ) -> super::DocumentWriteExternalScriptLoadApplication {
        let completion_target = completion.target();
        if self
            .pending_document_write_external_script_load
            .as_ref()
            .is_none_or(|pending| pending.target != completion_target)
        {
            return if self.complete_document_write_script_preload(completion) {
                super::DocumentWriteExternalScriptLoadApplication::Applied
            } else {
                super::DocumentWriteExternalScriptLoadApplication::RejectedStaleTarget
            };
        }
        if unsafe { &*host_ptr }.current_main_document_task_owner()
            != Some(completion_target.task_owner())
        {
            return super::DocumentWriteExternalScriptLoadApplication::RejectedStaleTarget;
        }
        let mut pending = self
            .pending_document_write_external_script_load
            .take()
            .expect("matching document-write script completion must retain its pending owner");
        if pending.target != completion_target
            || unsafe { &*host_ptr }.current_main_document_task_owner()
                != Some(completion_target.task_owner())
        {
            self.pending_document_write_external_script_load = Some(pending);
            return super::DocumentWriteExternalScriptLoadApplication::RejectedStaleTarget;
        }
        if !pending.blocking_signatures_before.is_empty() {
            debug_assert!(
                pending.ready_completion.is_none(),
                "one external parser script load must produce exactly one completion"
            );
            pending.ready_completion = Some(completion);
            self.pending_document_write_external_script_load = Some(pending);
            return super::DocumentWriteExternalScriptLoadApplication::Applied;
        }

        self.finish_document_write_external_script_load(scope, host_ptr, pending, completion)
    }

    fn finish_document_write_external_script_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        pending: PendingDocumentWriteExternalScriptLoad,
        completion: crate::types::DocumentWriteExternalScriptLoadCompletion,
    ) -> super::DocumentWriteExternalScriptLoadApplication {
        let completion_target = completion.target();
        let PendingDocumentWriteExternalScriptLoad {
            start,
            mut insertion,
            mut resume_after_completion,
            ..
        } = pending;
        if !Self::resume_document_write_insertion_permit(&mut insertion) {
            tracing::debug!(
                target = ?completion_target,
                permit = ?insertion.resume_permit,
                run_state = ?insertion.parser_insertion_controller.run_state(),
                "rejecting document.write external-script terminal for a stale parser suspension"
            );
            return super::DocumentWriteExternalScriptLoadApplication::RejectedStaleTarget;
        }
        match completion.into_result() {
            Ok(source) => {
                let csp_request =
                    crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
                        nonce: start.script.fetch_metadata.nonce.as_deref(),
                        integrity: start.script.fetch_metadata.integrity.as_deref(),
                        parser_inserted: true,
                    };
                if let Some(violation) = self
                    .script_element_request_csp_report_only_violation_with_request(
                        &start.script.url,
                        csp_request,
                    )
                {
                    self.queue_content_security_policy_violation_event_best_effort(
                        scope, host_ptr, &violation,
                    );
                }
                if let Some(violation) = self.script_element_request_csp_violation_with_request(
                    &start.script.url,
                    csp_request,
                ) {
                    self.queue_content_security_policy_violation_event_best_effort(
                        scope, host_ptr, &violation,
                    );
                    let task =
                        ScriptEventTask::new(ScriptEventKind::Error, &start.host_script_handle);
                    if let Err(error) = self.host_dispatch_script_event(scope, host_ptr, &task) {
                        tracing::debug!(
                            host_script_handle = start.host_script_handle.as_str(),
                            url = %start.script.url,
                            error,
                            "document.write external script CSP error event dispatch failed"
                        );
                    }
                } else {
                    self.execute_document_write_immediate_script(
                        scope,
                        host_ptr,
                        start.node,
                        &start.host_script_handle,
                        source,
                        Some(insertion.parser_insertion_controller.clone()),
                        DocumentWriteCurrentScriptEventBehavior::DispatchImmediately(
                            ScriptEventKind::Load,
                        ),
                    );
                }
            }
            Err(error_message) => {
                tracing::debug!(
                    host_script_handle = start.host_script_handle.as_str(),
                    url = %start.script.url,
                    error = error_message.as_str(),
                    "document.write external script load failed"
                );
                let task = ScriptEventTask::new(ScriptEventKind::Error, &start.host_script_handle);
                if let Err(error) = self.host_dispatch_script_event(scope, host_ptr, &task) {
                    tracing::debug!(
                        host_script_handle = start.host_script_handle.as_str(),
                        url = %start.script.url,
                        error,
                        "document.write external script error event dispatch failed"
                    );
                }
            }
        }

        if unsafe { &*host_ptr }.current_main_document_task_owner()
            != Some(completion_target.task_owner())
        {
            return super::DocumentWriteExternalScriptLoadApplication::SupersededDuringApplication;
        }

        let first = SuspendedDocumentWriteContinuation::ResumeAfterCompleted { start, insertion };
        if self.pending_document_write_external_script_load.is_some() {
            resume_after_completion.push_front(first);
            self.queue_document_write_continuations_after_pending(resume_after_completion);
            return super::DocumentWriteExternalScriptLoadApplication::Applied;
        }

        let _ = self.resume_document_write_continuations(
            scope,
            host_ptr,
            first,
            resume_after_completion,
        );
        let _ = self.finish_root_document_parser_stream_if_ready(scope, host_ptr);
        if unsafe { &*host_ptr }.current_main_document_task_owner()
            == Some(completion_target.task_owner())
        {
            super::DocumentWriteExternalScriptLoadApplication::Applied
        } else {
            super::DocumentWriteExternalScriptLoadApplication::SupersededDuringApplication
        }
    }

    fn pump_document_write_parser_step(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        stream: &mut DocumentStream,
        input: DocumentWriteParserPumpInput<'_>,
    ) -> DocumentWriteParserPumpStep {
        let outcome =
            self.with_dom_host_parse_step(|runtime| {
                let mut mutation_owner = DocumentWriteParserMutationOwner {
                    runtime,
                    scope,
                    host_ptr,
                };
                match input {
                    DocumentWriteParserPumpInput::Inserted(chunk) => stream
                        .pump_parser_inserted_step_with_runtime_dom_consumer(
                            chunk,
                            &mut mutation_owner,
                        ),
                    DocumentWriteParserPumpInput::Ordinary(chunk) => stream
                        .pump_parser_step_with_runtime_dom_consumer(chunk, &mut mutation_owner),
                    DocumentWriteParserPumpInput::QueuedOrBuffered => stream
                        .pump_next_parser_step_with_runtime_dom_consumer(0, &mut mutation_owner),
                }
            });
        let null_custom_element_registry_elements =
            stream.take_parser_stream_null_custom_element_registry_elements();
        custom_elements::apply_parser_created_null_registry_associations(
            host_ptr,
            &null_custom_element_registry_elements,
        );
        unsafe { &mut *host_ptr }.resync_child_browsing_contexts(scope);
        DocumentWriteParserPumpStep { outcome }
    }

    fn take_suspended_document_write_insertion(
        &mut self,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        cause: ParserSuspensionCause,
    ) -> SuspendedDocumentWriteInsertion {
        SuspendedDocumentWriteInsertion {
            document_handle,
            parser_insertion_controller: parser_insertion_controller.clone(),
            resume_permit: parser_insertion_controller.suspend(cause),
            resume_permit_consumed: false,
        }
    }

    fn suspend_document_write_stylesheet_blocked_script(
        &mut self,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        node: DomHandle,
        start_line: u64,
        start_column: u64,
        script: PreparedScript,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        debug_assert!(
            self.pending_document_write_stylesheet_blocked_script
                .is_none(),
            "a live parser can only have one pending parser-blocking script"
        );
        self.note_parser_script_start_position(node, start_line, start_column);
        let _ = self.dom_host_mut().set_script_already_started(node, true);
        let insertion = self.take_suspended_document_write_insertion(
            document_handle,
            parser_insertion_controller,
            ParserSuspensionCause::ParserClassicStylesheets { script: node },
        );
        self.pending_document_write_stylesheet_blocked_script =
            Some(PendingDocumentWriteStylesheetBlockedScript {
                node,
                start_line,
                start_column,
                script,
                blocking_signatures_before,
                insertion,
            });
        true
    }

    fn suspend_document_write_stylesheet_parser_pause(
        &mut self,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        stylesheet_owner: DomHandle,
        blocking_signatures: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        debug_assert!(
            self.pending_document_write_stylesheet_parser_pause
                .is_none(),
            "a live parser can only have one stylesheet parser boundary"
        );
        let insertion = self.take_suspended_document_write_insertion(
            document_handle,
            parser_insertion_controller,
            ParserSuspensionCause::ParserCreatedStylesheet {
                owner: stylesheet_owner,
            },
        );
        let preload_html = parser_insertion_controller
            .parser_stream()
            .borrow()
            .snapshot_pending_input();
        self.scan_document_write_script_preloads(host_ptr, &preload_html, true);
        self.pending_document_write_stylesheet_parser_pause =
            Some(PendingDocumentWriteStylesheetParserPause {
                blocking_signatures,
                insertion,
            });
        true
    }

    fn start_document_write_stylesheet_blocked_external_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        node: DomHandle,
        start_line: u64,
        start_column: u64,
        script: PreparedScript,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        self.note_parser_script_start_position(node, start_line, start_column);
        let _ = self.dom_host_mut().set_script_already_started(node, false);
        let DocumentWriteScriptRunOutcome::Suspend(start) = self
            .run_prepared_document_write_connected_script(
                scope,
                host_ptr,
                node,
                script,
                Some(parser_insertion_controller.clone()),
            )
        else {
            return false;
        };
        let insertion = self.take_suspended_document_write_insertion(
            document_handle,
            parser_insertion_controller,
            ParserSuspensionCause::DocumentWriteExternalScript { script: node },
        );
        self.start_document_write_external_script_load(
            scope,
            host_ptr,
            *start,
            insertion,
            blocking_signatures_before,
        );
        true
    }

    pub(crate) fn resume_document_write_stylesheet_blocked_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> bool {
        if !self.document_write_stylesheet_blocked_script_is_ready() {
            return false;
        }
        if let Some(pending) = self.pending_document_write_stylesheet_parser_pause.take() {
            self.document_write_script_preload_scanner = None;
            let _ =
                self.resume_suspended_document_write_insertion(scope, host_ptr, pending.insertion);
            let _ = self.finish_root_document_parser_stream_if_ready(scope, host_ptr);
            return true;
        }
        if self
            .pending_document_write_stylesheet_blocked_script
            .is_none()
        {
            let ready_completion = {
                let Some(pending) = self.pending_document_write_external_script_load.as_mut()
                else {
                    return false;
                };
                pending.blocking_signatures_before.clear();
                pending.ready_completion.take()
            };
            let Some(completion) = ready_completion else {
                // The stylesheet side is ready and its owner events have run;
                // the same pending parser script now waits only for its source.
                return true;
            };
            let pending = self
                .pending_document_write_external_script_load
                .take()
                .expect("ready external parser script must retain its pending owner");
            let _ = self
                .finish_document_write_external_script_load(scope, host_ptr, pending, completion);
            return true;
        }
        let Some(pending) = self.pending_document_write_stylesheet_blocked_script.take() else {
            return false;
        };
        let PendingDocumentWriteStylesheetBlockedScript {
            node,
            start_line,
            start_column,
            script,
            blocking_signatures_before: _,
            mut insertion,
        } = pending;
        if !Self::resume_document_write_insertion_permit(&mut insertion) {
            return false;
        }
        self.note_parser_script_start_position(node, start_line, start_column);
        let _ = self.dom_host_mut().set_script_already_started(node, false);
        match self.run_prepared_document_write_connected_script(
            scope,
            host_ptr,
            node,
            script,
            Some(insertion.parser_insertion_controller.clone()),
        ) {
            DocumentWriteScriptRunOutcome::Complete => {
                let parser_insertion_controller = insertion.parser_insertion_controller.clone();
                self.set_current_script_context(CurrentScriptContextSpec {
                    handle: Some(node),
                    parser_write_insertion_point_active: true,
                    parser_insertion_controller: Some(parser_insertion_controller),
                });
                let _ = self.resume_suspended_document_write_insertion(scope, host_ptr, insertion);
                self.clear_current_script_handle();
                let _ = self.finish_root_document_parser_stream_if_ready(scope, host_ptr);
                true
            }
            DocumentWriteScriptRunOutcome::Suspend(start) => {
                Self::resuspend_document_write_insertion(
                    &mut insertion,
                    ParserSuspensionCause::DocumentWriteExternalScript { script: node },
                );
                self.start_document_write_external_script_load(
                    scope,
                    host_ptr,
                    *start,
                    insertion,
                    HashSet::new(),
                );
                true
            }
        }
    }

    fn run_prepared_document_write_parser_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        node: DomHandle,
        start_line: u64,
        start_column: u64,
        script: PreparedScript,
    ) -> bool {
        self.note_parser_script_start_position(node, start_line, start_column);
        let _ = self.dom_host_mut().set_script_already_started(node, false);
        match self.run_prepared_document_write_connected_script(
            scope,
            host_ptr,
            node,
            script,
            Some(parser_insertion_controller.clone()),
        ) {
            DocumentWriteScriptRunOutcome::Complete => {
                self.has_pending_document_write_parser_blocking_work()
            }
            DocumentWriteScriptRunOutcome::Suspend(start) => self
                .suspend_document_write_parser_handoff(
                    scope,
                    host_ptr,
                    document_handle,
                    parser_insertion_controller,
                    start,
                ),
        }
    }
    fn queue_document_write_parser_owned_post_parse_script(
        &mut self,
        host_ptr: *mut JsContextHost,
        node: DomHandle,
        start_line: u64,
        start_column: u64,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        mut script: PreparedScript,
    ) {
        self.note_parser_script_start_position(node, start_line, start_column);
        if script.host_script_handle.is_none() {
            script.host_script_handle = Some(self.bind_parser_owned_script_handle_for_node(node));
        }
        let _ = self.dom_host_mut().set_script_already_started(node, true);

        if matches!(script.mode, ScriptMode::Defer | ScriptMode::ModuleDefer) {
            let script_node_id = script.node_id;
            let script_url = script.url.clone();
            let Some(task_owner) = (unsafe { &*host_ptr }).current_main_document_task_owner()
            else {
                debug!(
                    ?script_node_id,
                    url = %script_url,
                    "dropping document.write parser-deferred script without a main document owner"
                );
                return;
            };
            let Some(load_delay_token) = (unsafe { &mut *host_ptr })
                .acquire_current_main_parser_deferred_script_load_delay(task_owner)
            else {
                debug!(
                    ?task_owner,
                    ?script_node_id,
                    url = %script_url,
                    "dropping document.write parser-deferred script without lifecycle ownership"
                );
                return;
            };
            let shared_preload = self
                .main_document_script_preloads
                .shared_preload_for_script(&script);
            let document_character_set = self.document_character_set().to_owned();
            let Some(start) = self.accept_main_parser_deferred_script(
                task_owner,
                script,
                shared_preload,
                Some(&document_character_set),
                blocking_signatures_before,
                load_delay_token,
            ) else {
                let released = (unsafe { &mut *host_ptr })
                    .release_main_parser_deferred_script_load_delay(task_owner, load_delay_token);
                debug_assert!(
                    released,
                    "rejected document.write parser-deferred acceptance must release its lifecycle token"
                );
                return;
            };
            self.enqueue_main_parser_deferred_script_start(start);
            tracing::debug!(
                ?task_owner,
                ?load_delay_token,
                ?script_node_id,
                url = %script_url,
                "accepted document.write parser-deferred PendingScript before graph/source start"
            );
            return;
        }

        if script.mode == ScriptMode::Normal {
            self.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
                parser_prepared_script_page_owned_work(script, blocking_signatures_before),
            ]);
            return;
        }

        assert_eq!(
            script.mode,
            ScriptMode::Async,
            "defer-like parser scripts must enter the parser-deferred scheduler"
        );
        let task_owner = (unsafe { &*host_ptr })
            .current_main_document_task_owner()
            .expect("document.write async script requires the current main Document owner");
        let load_delay_kind = if script.kind == ScriptKind::Module {
            MainDocumentScriptLoadDelayKind::Module
        } else {
            MainDocumentScriptLoadDelayKind::Classic
        };
        let load_delay_binding = (unsafe { &mut *host_ptr })
            .acquire_current_main_document_script_load_delay(task_owner, load_delay_kind)
            .expect("document.write async script requires exact lifecycle ownership");

        if script.kind == ScriptKind::Module {
            let admission = MainParserAsyncModuleAdmission::new(script, load_delay_binding);
            let result = self.enqueue_main_parser_async_module_admission(admission);
            assert!(
                result.is_ok(),
                "a live document.write module must retain its exact main-runtime admission route"
            );
            return;
        }

        let resource_loader = (unsafe { &*host_ptr })
            .current_main_document_resource_loader()
            .expect("document.write async classic requires its exact Document resource loader");
        let source_load = SharedScriptSourceLoad::spawn_with_request_resource_type(
            script.clone(),
            resource_loader.request_client().clone(),
            resource_loader.task_runner(),
            Some(self.document_character_set().to_owned()),
            None,
        );
        let work = PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::parser_async_script_waiting_for_source(
                DocumentScriptExecutionLane::AsyncPhase,
                script,
                source_load,
                Some(load_delay_binding),
            ),
        );
        let result = self.enqueue_main_document_post_parse_work(work);
        assert!(
            result.is_ok(),
            "a live document.write async classic must retain its exact main-runtime route"
        );
    }

    fn run_prepared_document_write_import_map(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        node: DomHandle,
        start_line: u64,
        start_column: u64,
        import_map: PreparedImportMap,
    ) {
        self.note_parser_script_start_position(node, start_line, start_column);
        let host_script_handle = self.bind_parser_owned_script_handle_for_node(import_map.node_id);
        let _ = self.dom_host_mut().set_script_already_started(node, true);
        match import_map.source {
            PreparedImportMapSource::Inline(source) => {
                if let Err(message) =
                    self.register_import_map_source_with_base_url(&source, &import_map.base_url)
                    && let Err(error) = context_bootstrap::dispatch_window_report_error_message(
                        scope,
                        host_ptr,
                        &message,
                        Some(import_map.initiator_url.as_str()),
                    )
                {
                    debug!(
                        host_script_handle,
                        error, "document.write inline importmap window error dispatch failed"
                    );
                }
            }
            PreparedImportMapSource::ExternalUnsupported => {
                let _ = self.enqueue_script_event_lifecycle_work(
                    ScriptEventKind::Error,
                    &host_script_handle,
                );
            }
        }
    }

    fn suspend_document_write_parser_handoff(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        start: Box<DocumentWriteExternalScriptStart>,
    ) -> bool {
        let script = start.node;
        let insertion = self.take_suspended_document_write_insertion(
            document_handle,
            parser_insertion_controller,
            ParserSuspensionCause::DocumentWriteExternalScript { script },
        );
        self.start_document_write_external_script_load(
            scope,
            host_ptr,
            *start,
            insertion,
            HashSet::new(),
        )
    }

    fn handle_document_write_parser_handoff(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        parser_insertion_controller: &ParserInsertionController,
        handoff: ParserScriptHandoff,
    ) -> bool {
        match handoff {
            ParserScriptHandoff::BlockingClassic {
                node_id,
                start_line,
                start_column,
                blocking_signatures_before,
                script,
            } => {
                if self.has_pending_parser_script_blocking_stylesheet_signatures(
                    blocking_signatures_before.iter(),
                ) {
                    if matches!(script.source, crate::planning::ScriptSource::External) {
                        self.start_document_write_stylesheet_blocked_external_script(
                            scope,
                            host_ptr,
                            document_handle,
                            parser_insertion_controller,
                            node_id,
                            start_line,
                            start_column,
                            script,
                            blocking_signatures_before,
                        )
                    } else {
                        self.suspend_document_write_stylesheet_blocked_script(
                            document_handle,
                            parser_insertion_controller,
                            node_id,
                            start_line,
                            start_column,
                            script,
                            blocking_signatures_before,
                        )
                    }
                } else {
                    self.run_prepared_document_write_parser_script(
                        scope,
                        host_ptr,
                        document_handle,
                        parser_insertion_controller,
                        node_id,
                        start_line,
                        start_column,
                        script,
                    )
                }
            }
            ParserScriptHandoff::AsyncPostParse {
                node_id,
                start_line,
                start_column,
                script,
            } => {
                self.queue_document_write_parser_owned_post_parse_script(
                    host_ptr,
                    node_id,
                    start_line,
                    start_column,
                    HashSet::new(),
                    script,
                );
                false
            }
            ParserScriptHandoff::NonAsyncPostParse {
                node_id,
                start_line,
                start_column,
                blocking_signatures_before,
                script,
            } => {
                self.queue_document_write_parser_owned_post_parse_script(
                    host_ptr,
                    node_id,
                    start_line,
                    start_column,
                    blocking_signatures_before,
                    script,
                );
                false
            }
            ParserScriptHandoff::ImportMap {
                node_id,
                start_line,
                start_column,
                import_map,
            } => {
                self.run_prepared_document_write_import_map(
                    scope,
                    host_ptr,
                    node_id,
                    start_line,
                    start_column,
                    import_map,
                );
                false
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
                false
            }
            ParserScriptHandoff::PreparationFailure {
                node_id, failure, ..
            } => {
                crate::host::apply_parser_script_element_state_transition(
                    self.dom_host_mut(),
                    node_id,
                    failure.element_state_transition(),
                );
                self.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
                    parser_script_preparation_failure_page_owned_work(failure),
                ]);
                false
            }
        }
    }

    fn write_html_via_parser_stream(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        html: &str,
        resume_existing_insertion: bool,
        parser_insertion_controller: &ParserInsertionController,
    ) -> bool {
        let stream = parser_insertion_controller.parser_stream();
        let parser_input_session = parser_insertion_controller.input_session();
        parser_input_session.enqueue_script_input_preload_html(html.to_owned());

        let mut chunk = parser_input_session.take_current_script_input_html();
        chunk.push_str(html);
        if chunk.is_empty() && !resume_existing_insertion {
            return true;
        }
        let _parser_insertion_session = self.enter_parser_insertion_session();
        let mut begin_insertion = !chunk.is_empty();

        loop {
            let parser_step = {
                let _pump_guard = parser_insertion_controller.begin_pump();
                let input = if begin_insertion {
                    DocumentWriteParserPumpInput::Inserted(chunk.as_str())
                } else if resume_existing_insertion && chunk.is_empty() {
                    // Resuming a parser-inserted frame can expose either the
                    // tokenizer's buffered parent frame or a later root input
                    // segment. The parser owns that ordering; selecting its
                    // next queued input is required to make progress once the
                    // tokenizer buffer itself is empty.
                    DocumentWriteParserPumpInput::QueuedOrBuffered
                } else {
                    DocumentWriteParserPumpInput::Ordinary(chunk.as_str())
                };
                self.pump_document_write_parser_step(
                    scope,
                    host_ptr,
                    &mut stream.borrow_mut(),
                    input,
                )
            };
            let DocumentWriteParserPumpStep {
                outcome:
                    ParserPumpOutcome {
                        result,
                        discovered_async_prefetch_scripts: _,
                        discovered_modulepreload_link_candidates,
                        discovered_blocking_stylesheet_inputs,
                    },
            } = parser_step;
            let discovered_parser_meta_csp_candidates = stream
                .borrow_mut()
                .drain_discovered_parser_meta_csp_candidates();
            for handle in &discovered_parser_meta_csp_candidates {
                self.process_parser_meta_content_security_policy(*handle);
            }
            parser_input_session
                .note_processed_insertion_meta_csp(discovered_parser_meta_csp_candidates.len());
            self.accept_document_write_parser_modulepreloads(
                scope,
                host_ptr,
                discovered_modulepreload_link_candidates,
            );
            self.run_pending_parser_post_step_runtime_work(scope, host_ptr);
            chunk.clear();
            begin_insertion = false;

            self.note_discovered_document_owned_blocking_stylesheet_inputs(
                discovered_blocking_stylesheet_inputs.iter(),
            );

            match result {
                ParserPumpStep::InputDrained => {
                    return true;
                }
                ParserPumpStep::Yield(ParserYield::CustomElementConstruction(_handoff)) => {
                    // The handoff data is parser-side infrastructure for the future token-time
                    // construction owner. document.write keeps current post-sync construction
                    // behavior until that owner can safely run constructors from a shallow V8
                    // entry.
                }
                ParserPumpStep::Yield(ParserYield::BlockingStylesheet(pause)) => {
                    if self.current_document_resource_loader().is_none() {
                        continue;
                    }
                    let blocking_signatures = discovered_blocking_stylesheet_inputs
                        .iter()
                        .map(|input| input.signature().clone())
                        .collect::<HashSet<_>>();
                    if !blocking_signatures.is_empty()
                        && self.suspend_document_write_stylesheet_parser_pause(
                            host_ptr,
                            document_handle,
                            parser_insertion_controller,
                            pause.node_id,
                            blocking_signatures,
                        )
                    {
                        return true;
                    }
                }
                ParserPumpStep::Yield(ParserYield::Script(handoff)) => {
                    if self.handle_document_write_parser_handoff(
                        scope,
                        host_ptr,
                        document_handle,
                        parser_insertion_controller,
                        *handoff,
                    ) {
                        return true;
                    }
                }
            }
        }
    }

    /// Applies parser-discovered modulepreload insertion work before the
    /// document.write parser resumes. This mirrors the normal main-parser
    /// boundary: module-map registration and fetch start are synchronous with
    /// link insertion; only link errors and network terminals are queued.
    fn accept_document_write_parser_modulepreloads(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        link_handles: Vec<DomHandle>,
    ) {
        if link_handles.is_empty() {
            return;
        }
        let (document_owner, resource_scheduler) = unsafe {
            let host = &*host_ptr;
            let Some(document_owner) = host.current_main_document_task_owner() else {
                tracing::error!(
                    "document.write parser discovered modulepreload without a current Document owner"
                );
                return;
            };
            if host.current_main_document_resource_loader().is_none() {
                tracing::error!(
                    ?document_owner,
                    "document.write parser discovered modulepreload without a Document resource authority"
                );
                return;
            }
            (document_owner, host.resource_scheduler())
        };

        let (requests, runtime_warnings, _) = self
            .accept_parser_discovered_modulepreload_links(link_handles)
            .into_parts();
        for runtime_warning in runtime_warnings {
            record_document_write_modulepreload_warning(scope, host_ptr, runtime_warning);
        }
        for request in requests {
            match self.start_main_document_modulepreload_fetch(
                document_owner,
                &resource_scheduler,
                request,
            ) {
                Ok(outcome) => {
                    let (_, csp_violations, runtime_warning) = outcome.into_parts();
                    for violation in csp_violations {
                        self.queue_content_security_policy_violation_event_best_effort(
                            scope, host_ptr, &violation,
                        );
                    }
                    if let Some(runtime_warning) = runtime_warning {
                        record_document_write_modulepreload_warning(
                            scope,
                            host_ptr,
                            runtime_warning,
                        );
                    }
                }
                Err(error) => record_document_write_modulepreload_warning(
                    scope,
                    host_ptr,
                    format!(
                        "parser-discovered modulepreload failed before fetch scheduling: {}",
                        error.message()
                    ),
                ),
            }
        }
    }

    pub(crate) fn write_html(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        html: &str,
    ) -> bool {
        if self.has_pending_document_write_parser_blocking_work() {
            self.scan_document_write_script_preloads(host_ptr, html, false);
        }
        if self.append_to_pending_document_write_external_script_load(html) {
            return true;
        }
        if self.append_to_pending_document_write_stylesheet_blocked_script(html) {
            return true;
        }
        if self.append_to_pending_document_write_stylesheet_parser_pause(html) {
            return true;
        }

        let parser_insertion_controller = self
            .root_document_parser
            .as_ref()
            .and_then(ParserInsertionController::for_session)
            .or_else(|| self.current_parser_insertion_controller())
            .expect("document.write() must have a live root stream or parser insertion controller");
        self.write_html_via_parser_stream(
            scope,
            host_ptr,
            document_handle,
            html,
            false,
            &parser_insertion_controller,
        )
    }
}

fn record_document_write_modulepreload_warning(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    message: String,
) {
    tracing::warn!(warning = %message, "document.write modulepreload warning");
    let Some(context_token) = crate::native_bridge::current_runtime_observable_context_token(scope)
    else {
        return;
    };
    let execution_context_id = i64::from(v8::inspector::V8Inspector::execution_context_id(
        scope.get_current_context(),
    ));
    if execution_context_id == 0 {
        return;
    }
    unsafe {
        (*host_ptr).record_runtime_observable_console_source_event(
            context_token,
            execution_context_id,
            message.clone(),
            vec![serde_json::Value::String(message)],
            None,
        );
    }
}

fn source_node_has_null_registry_attribute(node: Option<&Node>) -> bool {
    node.and_then(Node::as_element).is_some_and(|element| {
        element.attributes().iter().any(|attribute| {
            attribute.namespace().is_empty()
                && attribute
                    .local_name()
                    .eq_ignore_ascii_case("customelementregistry")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document_script_scheduler::ParserDeferredScriptStartAction,
        frame_owner_model::{
            DocumentId, DocumentLoadDelayTokenId, FrameDocumentTaskOwner, FrameSchedulerLaneId,
            LocalWindowId,
        },
        parser::HtmlParser,
        planning::{PreparedScript, ScriptFetchMetadata, ScriptSource},
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };
    use url::Url;

    fn prepared_script(
        node_id: DomHandle,
        kind: ScriptKind,
        mode: ScriptMode,
        url: Url,
    ) -> PreparedScript {
        PreparedScript {
            position: node_id.index(),
            node_id,
            kind,
            mode,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: url.clone(),
            base_url: url.clone(),
            initiator_url: url,
            host_script_handle: None,
        }
    }

    #[test]
    fn root_document_parser_is_owned_by_shared_open_session() {
        let document_url = Url::parse("https://document-write.test/root-owner.html").unwrap();
        let document = HtmlParser.parse(document_url, "<!doctype html>".to_owned());
        let mut runtime = DocumentRuntime::new(&document);

        runtime.start_root_document_parser_stream();

        let parser = runtime
            .root_document_parser
            .as_ref()
            .expect("root replacement parser session");
        assert_eq!(parser.lifetime(), DocumentParserLifetime::Open);
        assert!(parser.has_exclusive_stream_handle());
    }

    #[test]
    #[should_panic(expected = "document.write external-script load id space exhausted")]
    fn document_write_external_script_load_ids_never_wrap() {
        let document_url = Url::parse("https://document-write.test/load-id.html").unwrap();
        let document = HtmlParser.parse(document_url, "<!doctype html>".to_owned());
        let mut runtime = DocumentRuntime::new(&document);
        runtime.next_document_write_external_script_load_id = u64::MAX;

        let _ = runtime.allocate_document_write_external_script_load_id();
    }

    #[test]
    fn document_write_deferred_handoff_claims_parser_pending_before_start() {
        for (index, (html, kind, mode)) in [
            (
                "<!doctype html><script defer src='/classic.js'></script>",
                ScriptKind::Classic,
                ScriptMode::Defer,
            ),
            (
                "<!doctype html><script type='module' src='/module.js'></script>",
                ScriptKind::Module,
                ScriptMode::ModuleDefer,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let document_url = Url::parse("https://document-write.test/page.html").unwrap();
            let document = HtmlParser.parse(document_url.clone(), html.to_owned());
            let node = document.script_handles()[0];
            let mut runtime = DocumentRuntime::new(&document);
            let script = prepared_script(node, kind, mode, document_url);
            let owner = FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(1),
                LocalWindowId(2),
                DocumentId(3),
            );

            let start = runtime
                .accept_main_parser_deferred_script(
                    owner,
                    script,
                    None,
                    Some("UTF-8"),
                    HashSet::new(),
                    DocumentLoadDelayTokenId(index as u64 + 1),
                )
                .expect("parser-deferred handoff should establish PendingScript ownership");
            let (accepted_owner, _, action) = start.into_parts();

            assert_eq!(accepted_owner, owner);
            assert!(matches!(
                (kind, action),
                (
                    ScriptKind::Classic,
                    ParserDeferredScriptStartAction::ClassicSource(_)
                ) | (
                    ScriptKind::Module,
                    ParserDeferredScriptStartAction::ModuleGraph(_)
                )
            ));
            assert!(
                runtime
                    .pop_parser_owned_pre_domcontentloaded_action()
                    .is_none(),
                "parser-deferred acceptance must not create broad page-owned execution work"
            );
        }
    }
}
