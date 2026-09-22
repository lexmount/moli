use super::JsContextHost;
use crate::{
    StylesheetBlockingReadView, StylesheetElementRead, custom_elements,
    document_runtime::DomHandle,
    dom::native::{Attribute, DomMutationEffects, DomStylesheetOwnerChange, Node},
    frame_owner_model::FrameDocumentOwner,
    live_document_parser::{DocumentParserRunState, LiveDocumentParserOwner},
    parser::{
        ParserDomMutation, ParserDomMutationConsumer, ParserDomReadConsumer,
        ParserElementCreationConsumer, ParserElementCreationRequest, ParserMutationEffectConsumer,
        ParserPlanningReadView, ParserScriptRead,
    },
    parser_mutation_effects::{ParserMutationEffectsOwner, apply_parser_mutation_effects},
    window_document_identity::LightweightPopupDocumentOwner,
};
use html5ever::tree_builder::QuirksMode;
use url::Url;

pub(super) struct ContextDocumentParserOwner<'a, 'scope, 'pin> {
    host: &'a mut JsContextHost,
    scope: &'a mut v8::PinScope<'scope, 'pin>,
    document_handle: DomHandle,
    document_owner: Option<FrameDocumentOwner>,
    popup_owner: Option<LightweightPopupDocumentOwner>,
    parser_control: Option<crate::live_document_parser::DocumentParserSessionControlHandle>,
    mutation_effects: DocumentParserMutationEffects,
}

struct DocumentParserMutationEffects {
    document_handle: DomHandle,
    reaction_queue_active: bool,
}

impl ParserMutationEffectsOwner for DocumentParserMutationEffects {
    type Prepared = Vec<DomStylesheetOwnerChange>;

    fn prepare_parser_mutation_effects(&mut self, effects: &DomMutationEffects) -> Self::Prepared {
        effects.stylesheet_owners().changes().to_vec()
    }

    fn ensure_parser_reaction_queue(&mut self, host_ptr: *mut JsContextHost) {
        if !self.reaction_queue_active {
            custom_elements::push_parser_custom_element_reaction_queue(host_ptr);
            self.reaction_queue_active = true;
        }
    }

    fn finish_parser_mutation_effects(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        prepared: Self::Prepared,
    ) {
        unsafe { &mut *host_ptr }.sync_child_browsing_context_subtree(scope, self.document_handle);
        crate::native_bridge::document::apply_stylesheet_owner_css_projections(
            scope,
            unsafe { &*host_ptr },
            &prepared,
        );
    }
}

impl<'a, 'scope, 'pin> ContextDocumentParserOwner<'a, 'scope, 'pin> {
    pub(super) fn new_child(
        host: &'a mut JsContextHost,
        scope: &'a mut v8::PinScope<'scope, 'pin>,
        document_handle: DomHandle,
    ) -> Self {
        let document_owner = host
            .child_browsing_context_host_for_document_handle(document_handle)
            .and_then(|child| host.frame_owner_store.current_child_document_owner(child));
        let parser_control = document_owner
            .and_then(|owner| host.child_document_parsers.insertion_handle(owner))
            .map(|insertion| insertion.control_handle());
        Self {
            host,
            scope,
            document_handle,
            document_owner,
            popup_owner: None,
            parser_control,
            mutation_effects: DocumentParserMutationEffects {
                document_handle,
                reaction_queue_active: false,
            },
        }
    }

    pub(super) fn new_popup(
        host: &'a mut JsContextHost,
        scope: &'a mut v8::PinScope<'scope, 'pin>,
        document_handle: DomHandle,
        popup_owner: LightweightPopupDocumentOwner,
        parser_control: crate::live_document_parser::DocumentParserSessionControlHandle,
    ) -> Self {
        Self {
            host,
            scope,
            document_handle,
            document_owner: None,
            popup_owner: Some(popup_owner),
            parser_control: Some(parser_control),
            mutation_effects: DocumentParserMutationEffects {
                document_handle,
                reaction_queue_active: false,
            },
        }
    }

    fn targets_current_document(&self) -> bool {
        if let Some(owner) = self.popup_owner {
            return self.parser_control.as_ref().is_some_and(|control| {
                self.host
                    .popup_document_parser_is_current(owner, control.session_id())
            }) && self
                .host
                .lightweight_popup_document_handle(owner.popup_id())
                == Some(self.document_handle);
        }
        self.host
            .child_browsing_context_host_for_document_handle(self.document_handle)
            .and_then(|child| {
                self.host
                    .frame_owner_store
                    .current_child_document_owner(child)
            })
            == self.document_owner
    }
}

impl LiveDocumentParserOwner for ContextDocumentParserOwner<'_, '_, '_> {}

impl StylesheetBlockingReadView for ContextDocumentParserOwner<'_, '_, '_> {
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
            .node(self.document_handle)
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        Some(self.host.document_base_url_for_handle(self.document_handle))
    }

    fn document_node_id(&self) -> DomHandle {
        self.document_handle
    }

    fn document_is_quirks_mode(&self) -> bool {
        self.host
            .dom_host()
            .node(self.document_handle)
            .and_then(Node::as_document)
            .is_some_and(|document| document.is_quirks_mode())
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<crate::dom::NodeId>,
    ) -> Vec<DomHandle> {
        self.host
            .dom_host()
            .stylesheet_candidate_handles_before_in_tree_scope(
                self.document_handle,
                target_node_id.map(|node_id| DomHandle::new(node_id.index())),
            )
    }
}

impl ParserMutationEffectConsumer for ContextDocumentParserOwner<'_, '_, '_> {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
        if !self.targets_current_document() {
            return;
        }
        for &root in effects.tree().connected_roots() {
            crate::native_bridge::element::initialize_parser_inserted_body_window_event_handlers(
                self.scope, self.host, root,
            );
        }
        apply_parser_mutation_effects(self.scope, self.host, &mut self.mutation_effects, &effects);
    }

    fn finish_parser_dom_mutations(&mut self) -> std::ops::ControlFlow<()> {
        if std::mem::take(&mut self.mutation_effects.reaction_queue_active) {
            custom_elements::flush_parser_custom_element_reaction_queue(self.scope, self.host);
        }
        if !self.targets_current_document()
            || self.parser_control.as_ref().is_some_and(|control| {
                !matches!(
                    control.run_state(),
                    DocumentParserRunState::Ready
                        | DocumentParserRunState::Pumping { .. }
                        | DocumentParserRunState::Finishing
                )
            })
        {
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    }
}

impl ParserDomReadConsumer for ContextDocumentParserOwner<'_, '_, '_> {
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

impl ParserDomMutationConsumer for ContextDocumentParserOwner<'_, '_, '_> {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) {
        if !self.targets_current_document() {
            return;
        }
        // The tree sink invokes this queue after releasing its structural
        // borrow, before returning to the tree builder. Nested document.write
        // can then use the insertion point preceding this element's children.
        self.mutation_effects
            .ensure_parser_reaction_queue(self.host);
        let effects = mutation.apply_to_dom_host(self.host.dom_host_mut());
        self.consume_parser_mutation_effects(effects);
    }

    fn create_element_for_document_without_attributes(
        &mut self,
        document_handle: DomHandle,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_element_without_attributes_for_document(
                document_handle,
                local_name,
                namespace,
                prefix,
            )
    }

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        construction: &moli_dom::native::ParserConstruction,
        document_handle: DomHandle,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        construction.create_element(
            self.host.dom_host_mut(),
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

    fn create_text_node(&mut self, document_handle: DomHandle, text: String) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_text_node_for_document(document_handle, &text)
    }

    fn create_comment(&mut self, document_handle: DomHandle, text: String) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_comment_for_document(document_handle, &text)
    }

    fn create_processing_instruction(
        &mut self,
        document_handle: DomHandle,
        target: String,
        data: String,
    ) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_processing_instruction_for_document(document_handle, &target, &data)
    }

    fn create_cdata_section(&mut self, document_handle: DomHandle, data: String) -> DomHandle {
        self.host
            .dom_host_mut()
            .create_cdata_section_for_document(document_handle, &data)
    }

    fn create_document_type(
        &mut self,
        document_handle: DomHandle,
        name: String,
        public_id: String,
        system_id: String,
    ) -> DomHandle {
        self.host.dom_host_mut().create_document_type_for_document(
            document_handle,
            &name,
            &public_id,
            &system_id,
        )
    }

    fn prepend_text_to_text_node(
        &mut self,
        node_id: DomHandle,
        text: String,
    ) -> DomMutationEffects {
        let host = self.host.dom_host_mut();
        let Some(previous) = host.node(node_id).and_then(Node::as_text) else {
            return DomMutationEffects::default();
        };
        let mut merged = text;
        merged.push_str(previous.data());
        host.set_text_content_effects(node_id, &merged)
    }

    fn append_text_to_text_node(&mut self, node_id: DomHandle, text: String) -> DomMutationEffects {
        let host = self.host.dom_host_mut();
        let Some(previous) = host.node(node_id).and_then(Node::as_text) else {
            return DomMutationEffects::default();
        };
        let mut merged = previous.data().to_owned();
        merged.push_str(&text);
        host.set_text_content_effects(node_id, &merged)
    }

    fn push_parse_error(&mut self, error: String) {
        self.host.dom_host_mut().push_parse_error(error);
    }

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode) {
        self.host
            .dom_host_mut()
            .set_html_quirks_mode_for_parser_document(self.document_handle, quirks_mode);
    }

    fn mark_script_already_started_for_parser(&mut self, node_id: DomHandle) {
        self.host
            .dom_host_mut()
            .set_script_already_started(node_id, true);
    }

    fn mark_unclosed_form_control_for_parser(&mut self, node_id: DomHandle) {
        if !self.targets_current_document() {
            return;
        }
        let _ = self
            .host
            .dom_host_mut()
            .set_blocks_form_submission(node_id, true);
    }

    fn finish_parsing_children(
        &mut self,
        construction: &moli_dom::native::ParserConstruction,
        node_id: DomHandle,
    ) {
        if !self.targets_current_document() {
            return;
        }
        let effects = construction.finish_children(self.host.dom_host_mut(), node_id);
        self.consume_parser_mutation_effects(effects);
    }

    fn maybe_clone_an_option_into_selectedcontent(&mut self, node_id: DomHandle) {
        let host_ptr = self.host as *mut JsContextHost;
        let _ = self
            .host
            .sync_selectedcontents_after_parser_option_finished(self.scope, host_ptr, node_id);
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

impl ParserElementCreationConsumer for ContextDocumentParserOwner<'_, '_, '_> {
    fn create_parser_element(
        &mut self,
        request: ParserElementCreationRequest<'_>,
    ) -> Option<DomHandle> {
        if !self.targets_current_document() {
            return None;
        }
        let context = if let Some(owner) = self.popup_owner {
            self.host
                .lightweight_popup_window(self.scope, owner.popup_id())?
                .get_creation_context(self.scope)?
        } else {
            let child_handle = self
                .host
                .child_browsing_context_host_for_document_handle(request.document_handle)?;
            // Keep a child parser lazy until its relevant realm exists.
            self.host
                .child_browsing_context_relevant_context(self.scope, child_handle)?
        };
        let scope = &mut v8::ContextScope::new(self.scope, context);
        let host_ptr = self.host as *mut JsContextHost;
        custom_elements::create_and_construct_parser_custom_element_direct_for_document(
            scope,
            host_ptr,
            request.document_handle,
            request.local_name,
            request.namespace,
            request.prefix,
            request.attributes,
            request.intended_parent,
            |document_handle, local_name, namespace, prefix| {
                request.construction.create_element(
                    self.host.dom_host_mut(), document_handle, local_name, namespace, prefix,
                )
            },
        )
    }
}
