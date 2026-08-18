use super::mutation_commands::{
    apply_runtime_mutation_effects_to_dom_host, finish_runtime_mutation_effects,
};
use super::*;
use crate::parser::{ParserPlanningReadView, ParserScriptRead};
use crate::stylesheet_blocking::StylesheetElementRead;
use html5ever::tree_builder::QuirksMode;
// This slice collects the remaining low-level facade methods that mostly forward into `DomHost`
// or expose stable document handles/state to the rest of the JS runtime.
//
// These methods are intentionally kept separate from selector/query and from mutation commands:
// they are the "basic DOM access" layer that other modules depend on, but they do not define
// selector semantics or orchestrate mutation side effects on their own.
impl DocumentRuntime {
    pub(crate) fn document_design_mode_enabled(&self, document: DomHandle) -> bool {
        self.design_mode_documents.contains(&document)
    }

    pub(crate) fn set_document_design_mode_enabled(&mut self, document: DomHandle, enabled: bool) {
        if !self.dom_host.node(document).is_some_and(Node::is_document) {
            return;
        }
        if enabled {
            self.design_mode_documents.insert(document);
        } else {
            self.design_mode_documents.remove(&document);
        }
    }

    pub(crate) fn snapshot_document(&self) -> NativeDom {
        self.dom_host.snapshot_document()
    }

    #[cfg(test)]
    pub(crate) fn replace_live_document_with_dom_host(&mut self, mut dom_host: DomHost) {
        let started_scripts: Vec<_> = {
            let current = self.dom_host.borrow();
            current
                .elements_by_tag_name(current.document_handle(), "script", true)
                .into_iter()
                .filter(|handle| {
                    current
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.script_already_started())
                })
                .collect()
        };
        let mut shadow_bindings = DomHost::snapshot_shadow_root_bindings(self.dom_host.borrow());
        if let Some(parked_bindings) = self.parked_live_shadow_root_bindings.take() {
            shadow_bindings.extend(parked_bindings);
        }
        dom_host.restore_shadow_root_bindings(shadow_bindings);
        self.dom_host = LiveRuntimeDomHost::from_dom_host(dom_host);
        self.style_source_document_sync_pending = true;
        for handle in started_scripts {
            let _ = self.dom_host.set_script_already_started(handle, true);
        }
        let _ = self.dom_host.set_document_url(self.document.url().clone());
        let _ = self
            .dom_host
            .set_document_ready_state(self.document.ready_state());
        for context in &mut self.script_context_stack {
            context.handle = context
                .handle
                .filter(|handle| self.dom_host.node(*handle).is_some());
            context.parser_connected = None;
        }
        let active_element = self.document.active_element().filter(|handle| {
            self.dom_host
                .node(*handle)
                .is_some_and(|node| node.is_element() && node.is_connected())
        });
        self.document.set_active_element(active_element);
        self.dom_host.set_active_element_handle(active_element);
        self.stylesheet_lifecycle
            .pending_connected_loads
            .retain(|queued| self.dom_host.node(queued.owner()).is_some());
        self.stylesheet_lifecycle
            .pre_initial_scan_processed_owners
            .clear();
        self.initial_connected_style_loads_queued = false;
    }

    #[cfg(test)]
    pub(crate) fn replace_live_document_with_document(&mut self, document: NativeDom) {
        // Test helper for document replacement paths. Replace only the DomHost
        // so runtime-owned state on the already-created V8 world stays alive:
        // event listeners, timers, dynamic scripts, mutation/resource queues,
        // shadow-root bindings, and monotonic script already-started bits.
        self.replace_live_document_with_dom_host(DomHost::from_dom(document));
    }

    pub(crate) fn apply_parser_stream_mutation_effects_to_live_dom_host(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        effects: DomMutationEffects,
    ) {
        let connected_roots = effects.tree().connected_roots().to_vec();
        let form_owner_effects = effects.clone();
        self.assert_active_parser_document_incarnation();
        let result = {
            let dom_host = self.dom_host.borrow_mut();
            apply_runtime_mutation_effects_to_dom_host(
                &mut self.mutations,
                &self.document,
                self.script_lifecycle.scripts_mut(),
                &mut self.events,
                scope,
                host_ptr,
                dom_host,
                effects,
                RuntimeMutationOptions::parser_tree_sink(),
            )
        };
        let _ = finish_runtime_mutation_effects(self, scope, host_ptr, result);
        if !connected_roots.is_empty() {
            self.ensure_parser_custom_element_reaction_queue(host_ptr);
            crate::custom_elements::enqueue_connected_and_form_callbacks_for_already_upgraded_subtrees(
                scope,
                host_ptr,
                &connected_roots,
            );
        }
        if self.form_owner_mutation_effects_touch_html_form(&form_owner_effects) {
            self.ensure_parser_custom_element_reaction_queue(host_ptr);
            crate::custom_elements::enqueue_form_association_callbacks_for_all(scope, host_ptr);
        }
    }

    fn form_owner_mutation_effects_touch_html_form(&self, effects: &DomMutationEffects) -> bool {
        let dom_host = self.dom_host();
        effects
            .tree()
            .connected_roots()
            .iter()
            .chain(effects.tree().disconnected_roots())
            .copied()
            .any(|root| subtree_contains_html_form(dom_host, root))
            || effects
                .style()
                .child_list_mutations()
                .iter()
                .any(|mutation| {
                    mutation
                        .added_nodes()
                        .iter()
                        .chain(mutation.removed_nodes())
                        .copied()
                        .any(|root| subtree_contains_html_form(dom_host, root))
                })
    }

    pub(crate) fn parser_runtime_dom_node_exists(&mut self, node_id: DomHandle) -> bool {
        self.dom_host_mut_for_active_parser_step()
            .node(node_id)
            .is_some()
    }

    pub(crate) fn parser_runtime_dom_is_connected(&mut self, node_id: DomHandle) -> bool {
        self.dom_host_mut_for_active_parser_step()
            .is_connected(node_id)
    }

    pub(crate) fn parser_runtime_dom_is_text_node(&mut self, node_id: DomHandle) -> bool {
        self.dom_host_mut_for_active_parser_step()
            .node(node_id)
            .and_then(Node::as_text)
            .is_some()
    }

    pub(crate) fn parser_runtime_dom_owner_document(
        &mut self,
        node_id: DomHandle,
    ) -> Option<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .owner_document_handle(node_id)
    }

    pub(crate) fn parser_runtime_dom_parent_node(
        &mut self,
        node_id: DomHandle,
    ) -> Option<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .node(node_id)
            .and_then(Node::parent_node)
    }

    pub(crate) fn parser_runtime_dom_previous_sibling(
        &mut self,
        node_id: DomHandle,
    ) -> Option<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .node(node_id)
            .and_then(Node::prev_sibling)
    }

    pub(crate) fn parser_runtime_dom_last_child(
        &mut self,
        node_id: DomHandle,
    ) -> Option<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .node(node_id)
            .and_then(Node::last_child)
    }

    pub(crate) fn parser_runtime_dom_child_handles(
        &mut self,
        node_id: DomHandle,
    ) -> Vec<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .child_handles(node_id)
            .collect()
    }

    pub(crate) fn parser_runtime_dom_document_order_script_handles(
        &mut self,
        document_handle: DomHandle,
    ) -> Vec<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .script_handles_in_light_subtree(document_handle)
    }

    pub(crate) fn parser_runtime_dom_document_order_stylesheet_candidate_handles_before(
        &mut self,
        document_handle: DomHandle,
        stop_at: Option<DomHandle>,
    ) -> Vec<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .stylesheet_candidate_handles_before_in_tree_scope(document_handle, stop_at)
    }

    pub(crate) fn parser_runtime_dom_document_body_handle_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .document_body_handle_for_document(document_handle)
    }

    pub(crate) fn parser_runtime_dom_document_base_url(
        &mut self,
        document_handle: DomHandle,
    ) -> Option<url::Url> {
        self.dom_host_mut_for_active_parser_step()
            .document_base_url_for_handle(document_handle)
    }

    pub(crate) fn parser_runtime_dom_template_contents_handle(
        &mut self,
        node_id: DomHandle,
    ) -> Option<DomHandle> {
        self.dom_host_mut_for_active_parser_step()
            .parser_template_contents_handle(node_id)
    }

    pub(crate) fn parser_runtime_dom_is_html_element_named(
        &mut self,
        node_id: DomHandle,
        local_name: &str,
    ) -> bool {
        self.dom_host_mut_for_active_parser_step()
            .dom()
            .is_html_element_named(node_id, local_name)
    }

    pub(crate) fn parser_runtime_dom_is_external_async_classic_candidate(
        &mut self,
        node_id: DomHandle,
    ) -> bool {
        let Some(element) = self
            .dom_host_mut_for_active_parser_step()
            .node(node_id)
            .and_then(Node::as_element)
        else {
            return false;
        };
        if !element.is_script_element() {
            return false;
        }
        if element.script_source_attribute().is_none() || element.attribute("async").is_none() {
            return false;
        }
        if element.is_html_script() && element.attribute("nomodule").is_some() {
            return false;
        }
        let Some(script_type) = element.attribute("type") else {
            return true;
        };
        if script_type.is_empty() {
            return true;
        }
        moli_script::classify_script_kind(Some(script_type)) == moli_page_types::ScriptKind::Classic
    }

    pub(crate) fn parser_runtime_dom_parser_script_read(
        &mut self,
        node_id: DomHandle,
    ) -> Option<ParserScriptRead> {
        <DomHost as ParserPlanningReadView>::parser_script_read(
            self.dom_host_mut_for_active_parser_step(),
            node_id,
        )
    }

    pub(crate) fn parser_runtime_dom_stylesheet_element(
        &mut self,
        node_id: DomHandle,
    ) -> Option<StylesheetElementRead> {
        self.dom_host_mut_for_active_parser_step()
            .node(node_id)
            .and_then(StylesheetElementRead::from_node)
    }

    pub(crate) fn parser_runtime_dom_text_content(&mut self, node_id: DomHandle) -> Option<String> {
        self.dom_host_mut_for_active_parser_step()
            .text_content(node_id)
    }

    pub(crate) fn create_parser_element_without_attributes_in_live_dom_host(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_parser_element_without_attributes(local_name, namespace, prefix)
    }

    pub(crate) fn create_parser_element_for_document_without_attributes_in_live_dom_host(
        &mut self,
        document_handle: DomHandle,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_parser_element_without_attributes_for_document(
                document_handle,
                local_name,
                namespace,
                prefix,
            )
    }

    pub(crate) fn add_attrs_if_missing_for_parser_in_live_dom_host(
        &mut self,
        node_id: DomHandle,
        attrs: Vec<crate::dom::native::Attribute>,
    ) {
        let should_hide_nonce = !attrs.is_empty();
        let dom_host = self.dom_host_mut_for_active_parser_step();
        dom_host.add_attrs_if_missing_for_parser(node_id, attrs);
        if should_hide_nonce
            && dom_host.is_connected(node_id)
            && let Some(nonce) = dom_host.get_attribute(node_id, "nonce")
            && !nonce.is_empty()
        {
            let _ = dom_host.set_attribute(node_id, "nonce", "");
            let _ = dom_host.set_cryptographic_nonce(node_id, Some(nonce));
        }
    }

    pub(crate) fn create_text_node_in_live_dom_host(&mut self, text: String) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_text_node(&text)
    }

    pub(crate) fn create_comment_in_live_dom_host(&mut self, text: String) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_comment(&text)
    }

    pub(crate) fn create_processing_instruction_in_live_dom_host(
        &mut self,
        target: String,
        data: String,
    ) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_processing_instruction(&target, &data)
    }

    pub(crate) fn create_cdata_section_in_live_dom_host(&mut self, data: String) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_cdata_section(&data)
    }

    pub(crate) fn create_document_type_in_live_dom_host(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> DomHandle {
        self.dom_host_mut_for_active_parser_step()
            .create_document_type(&name, &public_id, &system_id)
    }

    pub(crate) fn prepend_text_to_text_node_in_live_dom_host(
        &mut self,
        node_id: DomHandle,
        text: String,
    ) {
        let host = self.dom_host_mut_for_active_parser_step();
        if let Some(text_node) = host
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_text_mut())
        {
            let mut merged = text;
            merged.push_str(text_node.data());
            text_node.set_data(merged);
        }
    }

    pub(crate) fn append_text_to_text_node_in_live_dom_host(
        &mut self,
        node_id: DomHandle,
        text: String,
    ) {
        let host = self.dom_host_mut_for_active_parser_step();
        if let Some(text_node) = host
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_text_mut())
        {
            let mut merged = text_node.data().to_owned();
            merged.push_str(&text);
            text_node.set_data(merged);
        }
    }

    pub(crate) fn push_parse_error_in_live_dom_host(&mut self, error: String) {
        self.dom_host_mut_for_active_parser_step()
            .push_parse_error(error);
    }

    pub(crate) fn set_html_quirks_mode_for_parser_in_live_dom_host(
        &mut self,
        quirks_mode: QuirksMode,
    ) {
        self.dom_host_mut_for_active_parser_step()
            .set_html_quirks_mode_for_parser(quirks_mode);
    }

    pub(crate) fn mark_script_already_started_for_parser_in_live_dom_host(
        &mut self,
        node_id: DomHandle,
    ) {
        let _ = self
            .dom_host_mut_for_active_parser_step()
            .set_script_already_started(node_id, true);
    }

    pub(crate) fn attach_declarative_shadow_for_parser_in_live_dom_host(
        &mut self,
        host_id: DomHandle,
        template_id: DomHandle,
        attrs: Vec<crate::dom::native::Attribute>,
    ) -> bool {
        self.dom_host_mut_for_active_parser_step()
            .attach_declarative_shadow_for_parser(host_id, template_id, &attrs)
    }

    pub(crate) fn associate_parser_form_owner_in_live_dom_host(
        &mut self,
        target: DomHandle,
        form: DomHandle,
    ) -> bool {
        self.dom_host_mut_for_active_parser_step()
            .associate_parser_form_owner(target, form)
    }

    #[cfg(test)]
    pub(crate) fn replace_live_document(&mut self, document: &NativeDom) {
        self.replace_live_document_with_document(document.clone());
    }

    pub(crate) fn dom_host(&self) -> &DomHost {
        self.dom_host.borrow()
    }

    pub(crate) fn dom_host_mut(&mut self) -> &mut DomHost {
        self.dom_host.borrow_mut()
    }

    pub(crate) fn create_element(&mut self, local_name: &str) -> DomHandle {
        let started = moli_trace::dom_binding_timing_enabled().then(std::time::Instant::now);
        let handle = self.dom_host.create_element(local_name);
        if let Some(started) = started {
            let op = if local_name.eq_ignore_ascii_case("script") {
                "dom.createElement.script"
            } else {
                "dom.createElement"
            };
            moli_trace::record_dom_binding_operation(op, started.elapsed());
        }
        handle
    }

    pub(crate) fn create_element_ns(
        &mut self,
        namespace: Option<&str>,
        qualified_name: &str,
    ) -> Option<DomHandle> {
        let started = moli_trace::dom_binding_timing_enabled().then(std::time::Instant::now);
        let handle = self.dom_host.create_element_ns(namespace, qualified_name);
        if let Some(started) = started {
            moli_trace::record_dom_binding_operation("dom.createElementNS", started.elapsed());
        }
        handle
    }

    pub(crate) fn create_text_node(&mut self, data: &str) -> DomHandle {
        let started = moli_trace::dom_binding_timing_enabled().then(std::time::Instant::now);
        let handle = self.dom_host.create_text_node(data);
        if let Some(started) = started {
            moli_trace::record_dom_binding_operation("dom.createTextNode", started.elapsed());
        }
        handle
    }

    pub(crate) fn create_text_node_for_document(
        &mut self,
        document_handle: DomHandle,
        data: &str,
    ) -> DomHandle {
        let started = moli_trace::dom_binding_timing_enabled().then(std::time::Instant::now);
        let handle = self
            .dom_host
            .create_text_node_for_document(document_handle, data);
        if let Some(started) = started {
            moli_trace::record_dom_binding_operation("dom.createTextNode", started.elapsed());
        }
        handle
    }

    pub(crate) fn create_cdata_section_for_document(
        &mut self,
        document_handle: DomHandle,
        data: &str,
    ) -> DomHandle {
        self.dom_host
            .create_cdata_section_for_document(document_handle, data)
    }

    pub(crate) fn create_comment_for_document(
        &mut self,
        document_handle: DomHandle,
        data: &str,
    ) -> DomHandle {
        self.dom_host
            .create_comment_for_document(document_handle, data)
    }

    pub(crate) fn create_document_type(
        &mut self,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> DomHandle {
        self.dom_host
            .create_document_type(name, public_id, system_id)
    }

    pub(crate) fn create_processing_instruction_for_document(
        &mut self,
        document_handle: DomHandle,
        target: &str,
        data: &str,
    ) -> DomHandle {
        self.dom_host
            .create_processing_instruction_for_document(document_handle, target, data)
    }

    pub(crate) fn create_document_fragment(&mut self) -> DomHandle {
        self.dom_host.create_document_fragment()
    }

    pub(crate) fn create_document_fragment_for_document(
        &mut self,
        document_handle: DomHandle,
    ) -> DomHandle {
        self.dom_host
            .create_document_fragment_for_document(document_handle)
    }

    pub(in crate::document_runtime) fn initialize_new_native_node_owner_document(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        // This is creation-time owner assignment for a freshly created native
        // node. User-visible adoption must go through adopt_node(...), which
        // snapshots registry/adoptedCallback side effects before mutating.
        self.dom_host.adopt_node(document_handle, handle)
    }

    pub(crate) fn create_detached_html_document(&mut self) -> DomHandle {
        self.dom_host.create_detached_html_document()
    }

    pub(crate) fn create_detached_xml_document(&mut self) -> DomHandle {
        self.dom_host.create_detached_xml_document()
    }

    pub(crate) fn create_detached_html_document_with_url(&mut self, url: url::Url) -> DomHandle {
        self.dom_host.create_detached_html_document_with_url(url)
    }

    pub(crate) fn create_detached_xml_document_with_url(&mut self, url: url::Url) -> DomHandle {
        self.dom_host.create_detached_xml_document_with_url(url)
    }

    pub(crate) fn clone_node(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        deep: bool,
    ) -> Option<DomHandle> {
        let clone = self.dom_host.clone_node(handle, deep)?;
        let registry_retargets =
            custom_elements::registry_association_retargets_for_clone(host_ptr, handle, clone);
        custom_elements::apply_registry_association_retargets(host_ptr, &registry_retargets);
        if !custom_elements::upgrade_subtree_if_defined(scope, host_ptr, clone) {
            return None;
        }
        Some(clone)
    }

    pub(crate) fn import_node(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        handle: DomHandle,
        deep: bool,
        fallback_registry: Option<custom_elements::CustomElementRegistryAssociation>,
    ) -> Option<DomHandle> {
        let clone = self.dom_host.import_node(document_handle, handle, deep)?;
        let registry_retargets = custom_elements::registry_association_retargets_for_import_clone(
            host_ptr,
            handle,
            clone,
            document_handle,
            fallback_registry,
        );
        custom_elements::apply_registry_association_retargets(host_ptr, &registry_retargets);
        if !custom_elements::upgrade_subtree_if_defined(scope, host_ptr, clone) {
            return None;
        }
        Some(clone)
    }

    pub(crate) fn adopt_node(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            if let Some(parent) = self.dom_host.node(handle).and_then(Node::parent_node) {
                let _ = self.remove_child_appending_to_current_reaction_queue(
                    scope, host_ptr, parent, handle,
                );
            }
            self.adopt_native_node_enqueuing_adoption_reactions(
                scope,
                host_ptr,
                document_handle,
                handle,
            )
        })
    }

    pub(crate) fn adopt_native_node_collecting_adoption_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            self.adopt_native_node_enqueuing_adoption_reactions(
                scope,
                host_ptr,
                document_handle,
                handle,
            )
        })
    }

    pub(crate) fn adopt_native_node_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        self.adopt_native_node_enqueuing_adoption_reactions(
            scope,
            host_ptr,
            document_handle,
            handle,
        )
    }

    fn adopt_native_node_enqueuing_adoption_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        let plan = self.tree_adoption_plan(host_ptr, document_handle, handle);
        self.apply_native_adoption_plan(scope, host_ptr, &plan)
    }

    fn tree_adoption_plan(
        &self,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> TreeAdoptionPlan {
        TreeAdoptionPlan::before_adoption(
            &self.dom_host,
            host_ptr,
            std::slice::from_ref(&handle),
            document_handle,
            true,
        )
    }

    fn apply_native_adoption_plan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        plan: &TreeAdoptionPlan,
    ) -> Option<DomHandle> {
        let root = plan.root()?;
        let new_document = plan.new_document()?;
        let (adopted, stylesheet_owner_changes) = self
            .dom_host
            .adopt_node_with_stylesheet_owner_changes(new_document, root)?;
        custom_elements::apply_registry_association_retargets(
            host_ptr,
            &plan.custom_elements().registry_retargets,
        );
        let previous_owner_document = plan.previous_owner_document_for(root);
        if previous_owner_document.is_some_and(|owner| owner != new_document) {
            self.queue_image_loads_after_owner_document_change(scope, host_ptr, root);
        }
        custom_elements::enqueue_adopted_callbacks(
            scope,
            host_ptr,
            &plan.custom_elements().targets,
        );
        self.sync_native_adoption_context_after_owner_change(
            scope,
            host_ptr,
            root,
            previous_owner_document,
            new_document,
            &stylesheet_owner_changes,
        );
        Some(adopted)
    }

    fn sync_native_adoption_context_after_owner_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        previous_owner_document: Option<DomHandle>,
        document_handle: DomHandle,
        stylesheet_owner_changes: &[crate::dom::native::DomStylesheetOwnerChange],
    ) {
        if !previous_owner_document.is_some_and(|owner| owner != document_handle) {
            return;
        }
        let runtime = unsafe { &mut *host_ptr };
        for shadow_root in runtime.shadow_roots_in_subtree(handle) {
            crate::native_bridge::element::clear_shadow_root_adopted_style_sheets(
                scope,
                runtime,
                shadow_root,
            );
        }
        runtime.apply_stylesheet_owner_changes(stylesheet_owner_changes);
        runtime.migrate_inline_style_metadata_in_subtree(handle);
        runtime.note_style_subtree_context_change(handle);
    }

    fn queue_image_loads_after_owner_document_change(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        root: DomHandle,
    ) {
        let mut stack = vec![root];
        let mut images = Vec::new();
        while let Some(handle) = stack.pop() {
            if self.dom_host.is_html_element_named(handle, "img") {
                images.push(handle);
            }
            if let Some(shadow_root) = self.dom_host.shadow_root_handle(handle) {
                stack.push(shadow_root);
            }
            stack.extend(self.dom_host.child_handles(handle));
        }
        for image in images {
            crate::native_bridge::element::reset_image_load_dispatch(
                unsafe { &mut *host_ptr },
                image,
            );
            crate::native_bridge::element::queue_image_load_event_after_document_adoption(
                scope, host_ptr, image,
            );
        }
    }

    pub(crate) fn get_element_by_id(&self, id: &str) -> Option<DomHandle> {
        self.dom_host.element_handle_by_id(id)
    }

    pub(crate) fn document_handle(&self) -> DomHandle {
        self.dom_host.document_handle()
    }

    pub(crate) fn mark_script_already_started_by_node_id(&mut self, node_id: NodeId) -> bool {
        let Some(handle) = self.dom_host.resolve_node(node_id) else {
            return false;
        };
        self.dom_host.set_script_already_started(handle, true)
    }

    pub(crate) fn active_element_handle(&self) -> Option<DomHandle> {
        self.document.active_element().filter(|handle| {
            self.dom_host
                .node(*handle)
                .is_some_and(|node| node.is_element() && node.is_connected())
        })
    }

    pub(crate) fn set_active_element_handle(&mut self, handle: Option<DomHandle>) {
        self.document.set_active_element(handle);
        self.dom_host.set_active_element_handle(handle);
    }

    pub(crate) fn document_focus_fallback_handle(&self) -> Option<DomHandle> {
        self.dom_host
            .document_body_handle()
            .or_else(|| self.dom_host.document_element_handle())
    }

    pub(crate) fn parent_node(&self, handle: DomHandle) -> Option<DomHandle> {
        self.dom_host.node(handle).and_then(Node::parent_node)
    }
}

pub(super) fn sync_style_sources_from_dom_mutation_effects(
    host_ptr: *mut JsContextHost,
    effects: &DomMutationEffects,
) {
    if effects.stylesheet_owners().changes().is_empty() {
        return;
    }
    unsafe { &mut *host_ptr }.apply_stylesheet_owner_changes(effects.stylesheet_owners().changes());
}

fn subtree_contains_html_form(dom_host: &DomHost, root: DomHandle) -> bool {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        if dom_host.is_html_element_named(handle, "form") {
            return true;
        }
        let mut child = dom_host.first_child(handle);
        while let Some(current) = child {
            stack.push(current);
            child = dom_host.next_sibling(current);
        }
        if let Some(shadow_root) = dom_host.shadow_root_handle(handle) {
            stack.push(shadow_root);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::DomStylesheetOwnerChangeKind;

    fn runtime_with_body() -> (DocumentRuntime, DomHandle) {
        let parser = HtmlParser;
        let document = parser.parse(
            Url::parse("https://example.test/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let body = document.document_body_handle().expect("body handle");
        (DocumentRuntime::new(&document), body)
    }

    #[test]
    fn style_source_sync_roots_include_inserted_style_subtrees() {
        let (mut runtime, body) = runtime_with_body();
        let style = runtime.dom_host_mut().create_element("style");
        let text = runtime
            .dom_host_mut()
            .create_text_node(".target:has(.marker) { color: green; }");
        assert!(runtime.dom_host_mut().append_child(style, text));

        let effects = runtime.dom_host_mut().append_child_effects(body, style);
        assert!(effects.stylesheet_owners().changes().iter().any(|change| {
            change.owner() == style
                && matches!(change.kind(), DomStylesheetOwnerChangeKind::Registered)
        }));
    }

    #[test]
    fn style_source_sync_roots_include_style_character_data_parent() {
        let (mut runtime, body) = runtime_with_body();
        let style = runtime.dom_host_mut().create_element("style");
        let text = runtime
            .dom_host_mut()
            .create_text_node(".target { color: green; }");
        assert!(runtime.dom_host_mut().append_child(style, text));
        assert!(runtime.dom_host_mut().append_child(body, style));

        let effects = runtime
            .dom_host_mut()
            .set_text_content_effects(text, ".target + .marker { color: blue; }");
        assert!(effects.stylesheet_owners().changes().iter().any(|change| {
            change.owner() == style
                && matches!(change.kind(), DomStylesheetOwnerChangeKind::Contents)
        }));
    }

    #[test]
    fn disconnected_style_text_mutations_remain_directed_owner_changes() {
        let (mut runtime, body) = runtime_with_body();
        let style = runtime.dom_host_mut().create_element("style");
        let text = runtime
            .dom_host_mut()
            .create_text_node(".target { color: green; }");

        let disconnected_append = runtime.dom_host_mut().append_child_effects(style, text);
        assert!(
            disconnected_append
                .stylesheet_owners()
                .changes()
                .iter()
                .any(|change| {
                    change.owner() == style
                        && matches!(change.kind(), DomStylesheetOwnerChangeKind::Contents)
                })
        );

        assert!(runtime.dom_host_mut().append_child(body, style));
        assert!(runtime.dom_host_mut().remove_child(body, style));

        let disconnected_character_data = runtime
            .dom_host_mut()
            .set_text_content_effects(text, ".target { color: blue; }");
        assert!(
            disconnected_character_data
                .stylesheet_owners()
                .changes()
                .iter()
                .any(|change| {
                    change.owner() == style
                        && matches!(change.kind(), DomStylesheetOwnerChangeKind::Contents)
                })
        );
    }
}
