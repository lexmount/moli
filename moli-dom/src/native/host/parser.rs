use super::*;

impl DomHost {
    pub fn push_parse_error(&mut self, error: String) {
        self.dom.push_parse_error(error);
    }

    pub fn set_html_quirks_mode_for_parser(
        &mut self,
        quirks_mode: html5ever::tree_builder::QuirksMode,
    ) {
        self.set_html_quirks_mode_for_parser_document(self.document_handle(), quirks_mode);
    }

    pub fn set_html_quirks_mode_for_parser_document(
        &mut self,
        document_handle: DomHandle,
        quirks_mode: html5ever::tree_builder::QuirksMode,
    ) {
        if let Some(document) = self
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        {
            document.set_html_quirks_mode(quirks_mode);
        }
    }

    pub fn create_parser_element(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        attributes: Vec<crate::dom::native::Attribute>,
    ) -> DomHandle {
        let node_id = self.create_parser_element_without_attributes(local_name, namespace, prefix);
        self.add_attrs_if_missing_for_parser(node_id, attributes);
        node_id
    }

    pub fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        self.create_parser_element_without_attributes_for_document(
            self.document_handle(),
            local_name,
            namespace,
            prefix,
        )
    }

    pub fn create_parser_element_without_attributes_for_document(
        &mut self,
        document_handle: DomHandle,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> DomHandle {
        let is_template = namespace == "http://www.w3.org/1999/xhtml" && local_name == "template";
        let is_child_browsing_context_host_candidate =
            is_html_frame_owner_candidate(&local_name, &namespace);
        let node_id = self.dom.create_node(
            NodeData::Element(Element::new_parser_created(
                local_name,
                namespace,
                prefix,
                Vec::new(),
            )),
            Some(document_handle),
            false,
            false,
        );
        if let Some(node) = self.node_mut(node_id) {
            node.set_parser_created(true);
        }
        self.record_element_query_index_candidate(node_id);
        if is_child_browsing_context_host_candidate {
            let mut candidates = self.child_browsing_context_host_candidates.borrow_mut();
            if !candidates.contains(&node_id) {
                candidates.push(node_id);
            }
        }

        if is_template {
            let template_contents = self
                .dom
                .create_template_contents_fragment_for_document(document_handle, Some(node_id));
            if let Some(element) = self
                .node_mut(node_id)
                .and_then(|node| node.data_mut().as_element_mut())
            {
                element.set_template_contents(Some(template_contents));
            }
        }

        node_id
    }

    pub fn parser_template_contents_handle(&self, node_id: DomHandle) -> Option<DomHandle> {
        self.node(node_id)
            .and_then(Node::as_element)
            .and_then(Element::template_contents)
    }

    pub fn associate_parser_form_owner(&mut self, target: DomHandle, form: DomHandle) -> bool {
        if !self.is_html_element_named(form, "form") {
            return false;
        }
        let Some(element) = self
            .node_mut(target)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return false;
        };
        if !is_parser_form_association_candidate(element) {
            return false;
        }
        element.set_parser_associated_form_owner(Some(form))
    }

    pub fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: DomHandle,
        template_id: DomHandle,
        attrs: &[crate::dom::native::Attribute],
    ) -> bool {
        let Some(mode) = attrs
            .iter()
            .find(|attr| attr.local_name() == "shadowrootmode")
            .map(crate::dom::native::Attribute::value)
            .filter(|value| matches!(*value, "open" | "closed"))
        else {
            return false;
        };
        if !self
            .node(template_id)
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_element("template"))
        {
            return false;
        }
        if self
            .node(host_id)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.local_name().contains('-')
                    && self.custom_element_definition_disables_shadow(element.local_name())
            })
        {
            return false;
        }
        let mut init = ShadowRootInit::new(mode);
        init.set_delegates_focus(
            attrs
                .iter()
                .any(|attr| attr.local_name() == "shadowrootdelegatesfocus"),
        );
        init.set_clonable(
            attrs
                .iter()
                .any(|attr| attr.local_name() == "shadowrootclonable"),
        );
        init.set_serializable(
            attrs
                .iter()
                .any(|attr| attr.local_name() == "shadowrootserializable"),
        );
        init.set_null_custom_element_registry(
            attrs
                .iter()
                .any(|attr| attr.local_name() == "shadowrootcustomelementregistry"),
        );
        if let Some(slot_assignment) = attrs
            .iter()
            .find(|attr| attr.local_name() == "shadowrootslotassignment")
            .map(crate::dom::native::Attribute::value)
            .filter(|value| {
                value.eq_ignore_ascii_case("named") || value.eq_ignore_ascii_case("manual")
            })
        {
            init.set_slot_assignment(&slot_assignment.to_ascii_lowercase());
        }
        if let Some(reference_target) = attrs
            .iter()
            .find(|attr| attr.local_name() == "shadowrootreferencetarget")
            .map(crate::dom::native::Attribute::value)
        {
            init.set_reference_target(Some(reference_target.to_owned()));
        }
        if let Some(adopted_style_sheets) = attrs
            .iter()
            .find(|attr| attr.local_name() == "shadowrootadoptedstylesheets")
            .map(crate::dom::native::Attribute::value)
        {
            init.set_adopted_style_sheets(Some(adopted_style_sheets.to_owned()));
        }
        let Some(shadow_root) = self.attach_declarative_shadow_root_with_init(host_id, init) else {
            return false;
        };
        if let Some(element) = self
            .node_mut(template_id)
            .and_then(|node| node.data_mut().as_element_mut())
        {
            element.set_template_contents(Some(shadow_root));
        }
        if let Some(parent) = self.node(template_id).and_then(Node::parent_node) {
            let _ = self.remove_child(parent, template_id);
        }
        true
    }

    pub fn add_attrs_if_missing_for_parser(
        &mut self,
        node_id: DomHandle,
        attrs: Vec<crate::dom::native::Attribute>,
    ) {
        let is_base_element = self.is_html_element_named(node_id, "base");
        let Some(element) = self
            .node_mut(node_id)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return;
        };

        let mut changed = false;
        let mut named_index_changed = false;
        let mut base_state_changed = false;
        for attribute in attrs {
            let already_exists = element.attributes().iter().any(|existing| {
                existing.namespace() == attribute.namespace()
                    && existing.local_name() == attribute.local_name()
                    && existing.prefix() == attribute.prefix()
            });

            if !already_exists {
                named_index_changed |= attribute.local_name().eq_ignore_ascii_case("id")
                    || attribute.local_name().eq_ignore_ascii_case("name");
                let attribute_changed = element.set_attribute(
                    attribute.local_name().to_owned(),
                    attribute.namespace().to_owned(),
                    attribute.prefix().map(str::to_owned),
                    attribute.value().to_owned(),
                );
                base_state_changed |= attribute_changed
                    && is_base_element
                    && attribute.namespace().is_empty()
                    && matches!(attribute.local_name(), "href" | "target");
                changed |= attribute_changed;
            }
        }
        changed |= element.resanitize_input_value_after_parser_attributes();
        changed |= element.mark_undefined_custom_element_candidate_from_identity();

        if changed {
            if named_index_changed {
                // Parser attribute repair bypasses the public setAttribute
                // helpers, so publish any newly introduced named candidate here.
                self.record_named_index_candidate(node_id);
            }
            if base_state_changed {
                self.dom.process_base_element_for_node(node_id);
            }
            self.record_mutation(MutationScope::QueryState);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::native::{Attribute, CustomElementState};

    use super::*;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://parser-element.test/").expect("test url"),
        ))
    }

    #[test]
    fn parser_element_without_attributes_keeps_parser_setup_separate_from_token_attrs() {
        let mut host = test_host();

        let element = host.create_parser_element_without_attributes(
            "x-card".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );

        assert!(
            host.node(element)
                .is_some_and(|node| node.flags().parser_created())
        );
        assert_eq!(host.get_attribute(element, "id"), None);

        host.add_attrs_if_missing_for_parser(
            element,
            vec![Attribute::new(
                "id".to_owned(),
                String::new(),
                None,
                "card".to_owned(),
            )],
        );

        assert_eq!(host.get_attribute(element, "id").as_deref(), Some("card"));
    }

    #[test]
    fn parser_input_value_sanitization_observes_the_complete_token_attribute_set() {
        let mut host = test_host();
        let input = host.create_parser_element_without_attributes(
            "input".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        let attribute = |name: &str, value: &str| {
            Attribute::new(name.to_owned(), String::new(), None, value.to_owned())
        };

        host.add_attrs_if_missing_for_parser(
            input,
            vec![
                attribute("type", "range"),
                attribute("id", "range"),
                attribute("min", "2"),
                attribute("max", "6"),
            ],
        );

        assert_eq!(
            host.node(input)
                .and_then(Node::as_element)
                .map(Element::input_value)
                .as_deref(),
            Some("4")
        );
    }

    #[test]
    fn parser_script_prepare_state_survives_parser_attribute_attach() {
        let mut host = test_host();

        let script = host.create_parser_element_without_attributes(
            "script".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );

        assert!(
            host.node(script)
                .and_then(Node::as_element)
                .is_some_and(|element| element.script_parser_inserted_for_prepare())
        );
        assert!(
            !host
                .node(script)
                .and_then(Node::as_element)
                .is_some_and(|element| element.script_async()),
            "parser-created executable script starts with force-async cleared"
        );

        host.add_attrs_if_missing_for_parser(
            script,
            vec![Attribute::new(
                "type".to_owned(),
                String::new(),
                None,
                "text/plain".to_owned(),
            )],
        );

        let element = host
            .node(script)
            .and_then(Node::as_element)
            .expect("parser-created script element");
        assert!(element.script_parser_inserted_for_prepare());
        assert!(
            !element.script_async(),
            "DOM storage does not classify parser-added script attributes"
        );
    }

    #[test]
    fn parser_added_is_attribute_marks_html_customized_builtin_candidate_undefined() {
        let mut host = test_host();

        let element = host.create_parser_element_without_attributes(
            "p".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert_eq!(
            host.node(element)
                .and_then(Node::as_element)
                .map(|element| element.custom_element_state()),
            Some(CustomElementState::Uncustomized)
        );

        host.add_attrs_if_missing_for_parser(
            element,
            vec![Attribute::new(
                "is".to_owned(),
                String::new(),
                None,
                String::new(),
            )],
        );

        assert_eq!(
            host.node(element)
                .and_then(Node::as_element)
                .map(|element| element.custom_element_state()),
            Some(CustomElementState::Undefined)
        );
    }

    #[test]
    fn parser_template_without_attributes_still_creates_template_contents() {
        let mut host = test_host();

        let template = host.create_parser_element_without_attributes(
            "template".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );

        assert!(host.parser_template_contents_handle(template).is_some());
    }
}
