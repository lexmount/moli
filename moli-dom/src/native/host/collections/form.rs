use super::*;
use crate::forms::{ButtonTypeState, InputType, parse_non_negative_integer_prefix};

impl DomHost {
    pub fn option_value(&self, handle: DomHandle) -> Option<String> {
        self.dom.option_value(handle)
    }

    pub fn input_datalist_handle(&self, handle: DomHandle) -> Option<DomHandle> {
        let input = self.node(handle).and_then(Node::as_element)?;
        if !input.is_html_input()
            || !matches!(
                input.input_type(),
                InputType::Text
                    | InputType::Search
                    | InputType::Tel
                    | InputType::Url
                    | InputType::Email
                    | InputType::Date
                    | InputType::Month
                    | InputType::Week
                    | InputType::Time
                    | InputType::DatetimeLocal
                    | InputType::Number
                    | InputType::Range
                    | InputType::Color
            )
        {
            return None;
        }
        let list_id = input.attribute("list").filter(|id| !id.is_empty())?;
        let tree_root = self.root_node_handle(handle)?;
        let candidate = self.element_handle_by_id_in_subtree(tree_root, list_id)?;
        let resolved = self.resolve_reference_target_chain(candidate)?;
        self.node(resolved)
            .and_then(Node::as_element)
            .filter(|element| element.is_html_element("datalist"))
            .map(|_| candidate)
    }

    pub fn form_control_elements(&self, root: DomHandle) -> Vec<DomHandle> {
        if self.is_html_element_named(root, "fieldset") {
            return self.collect_matching_elements(root, false, |handle| {
                self.node(handle)
                    .and_then(Node::as_element)
                    .is_some_and(is_listed_form_control_element)
            });
        }

        if !self.is_html_element_named(root, "form") {
            return Vec::new();
        }

        if self.is_connected(root) {
            let form_tree_root = self
                .root_node_handle(root)
                .unwrap_or_else(|| self.document_handle());
            let document = self.document_handle();
            let reference_source_roots = self
                .shadow_roots_by_host
                .borrow()
                .keys()
                .filter_map(|host| {
                    (self.resolve_reference_target_chain(*host) == Some(root))
                        .then(|| self.root_node_handle(*host))
                        .flatten()
                })
                .collect::<Vec<_>>();

            let mut roots = Vec::new();
            if form_tree_root != document && reference_source_roots.contains(&document) {
                roots.push(document);
            }
            roots.push(form_tree_root);
            for tree_root in reference_source_roots {
                if tree_root != document && !roots.contains(&tree_root) {
                    roots.push(tree_root);
                }
            }

            roots
                .into_iter()
                .flat_map(|tree_root| {
                    self.collect_matching_elements(tree_root, false, |handle| {
                        self.is_listed_form_control_handle(handle)
                            && self.form_control_owner(handle) == Some(root)
                    })
                })
                .collect()
        } else {
            self.collect_matching_elements(root, false, |handle| {
                self.is_listed_form_control_handle(handle)
                    && self.form_control_owner(handle) == Some(root)
            })
        }
    }

    pub fn button_is_submit_button(&self, handle: DomHandle) -> bool {
        let Some(element) = self.node(handle).and_then(Node::as_element) else {
            return false;
        };
        if !element.is_html_button() {
            return false;
        }
        match element.button_type_state() {
            ButtonTypeState::Submit => true,
            ButtonTypeState::Auto => {
                !element.has_attribute("command")
                    && !element.has_attribute("commandfor")
                    && !self
                        .parent_node(handle)
                        .is_some_and(|parent| self.is_html_element_named(parent, "select"))
            }
            ButtonTypeState::Reset | ButtonTypeState::Button => false,
        }
    }

    fn is_listed_form_control_handle(&self, handle: DomHandle) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(is_listed_form_control_element)
    }

    pub fn builtin_form_associated_owner(&self, handle: DomHandle) -> Option<DomHandle> {
        let element = self.node(handle).and_then(Node::as_element)?;
        if !is_parser_form_association_candidate(element) {
            return None;
        }
        if is_builtin_reassociateable_form_associated_element(element) {
            return self.form_control_owner(handle);
        }
        self.parser_or_ancestor_form_owner(handle)
    }

    pub fn form_control_owner(&self, handle: DomHandle) -> Option<DomHandle> {
        let element = self.node(handle).and_then(Node::as_element)?;
        if !is_builtin_reassociateable_form_associated_element(element) {
            return None;
        }

        if let Some(form_id) = element.attribute("form")
            && self.is_connected_to_document(handle)
        {
            if form_id.is_empty() {
                return None;
            }
            let tree_root = self.root_node_handle(handle)?;
            let candidate = self.element_handle_by_id_in_subtree(tree_root, form_id)?;
            let candidate = self.resolve_reference_target_chain(candidate)?;
            return self
                .is_html_element_named(candidate, "form")
                .then_some(candidate);
        }

        self.parser_or_ancestor_form_owner(handle)
    }

    fn parser_or_ancestor_form_owner(&self, handle: DomHandle) -> Option<DomHandle> {
        if let Some(owner) = self
            .node(handle)
            .and_then(Node::as_element)
            .and_then(Element::parser_associated_form_owner)
            && self.is_html_element_named(owner, "form")
            && self.root_node_handle(handle) == self.root_node_handle(owner)
        {
            return Some(owner);
        }

        let mut current = self.parent_node(handle);
        while let Some(parent) = current {
            if self.is_html_element_named(parent, "form") {
                return Some(parent);
            }
            current = self.parent_node(parent);
        }
        None
    }

    pub fn option_nearest_ancestor_select(&self, handle: DomHandle) -> Option<DomHandle> {
        self.dom.option_nearest_ancestor_select(handle)
    }

    pub fn optgroup_nearest_ancestor_select(&self, handle: DomHandle) -> Option<DomHandle> {
        self.dom.optgroup_nearest_ancestor_select(handle)
    }

    pub fn selectedcontent_nearest_ancestor_select(&self, handle: DomHandle) -> Option<DomHandle> {
        self.dom.selectedcontent_nearest_ancestor_select(handle)
    }

    pub fn select_selectedcontent_elements(&self, handle: DomHandle) -> Vec<DomHandle> {
        if !self.is_html_element_named(handle, "select") {
            return Vec::new();
        }
        self.elements_by_tag_name_ns(
            handle,
            Some("http://www.w3.org/1999/xhtml"),
            "selectedcontent",
            false,
        )
        .into_iter()
        .filter(|selectedcontent| {
            self.selectedcontent_nearest_ancestor_select(*selectedcontent) == Some(handle)
        })
        .collect()
    }

    pub fn option_is_disabled(&self, handle: DomHandle) -> bool {
        self.dom.option_is_disabled(handle)
    }

    pub fn radio_group_members(&self, handle: DomHandle) -> Vec<DomHandle> {
        let Some(element) = self.node(handle).and_then(Node::as_element) else {
            return Vec::new();
        };
        if !element.is_html_input() || element.input_type() != InputType::Radio {
            return Vec::new();
        }
        let Some(name) = element.name_attribute() else {
            return Vec::new();
        };
        let Some(tree_root) = self.root_node_handle(handle) else {
            return vec![handle];
        };
        let form_owner = self.form_control_owner(handle);
        self.collect_matching_elements(tree_root, true, |candidate| {
            self.node(candidate)
                .and_then(Node::as_element)
                .is_some_and(|candidate_element| {
                    candidate_element.is_html_input()
                        && candidate_element.input_type() == InputType::Radio
                        && candidate_element.matches_name(name)
                        && self.form_control_owner(candidate) == form_owner
                })
        })
    }

    pub fn select_option_elements(&self, select_handle: DomHandle) -> Vec<DomHandle> {
        self.dom.select_option_elements(select_handle)
    }

    pub fn select_selected_option_elements(&self, select_handle: DomHandle) -> Vec<DomHandle> {
        let options = self.select_option_elements(select_handle);
        let Some(select) = self.node(select_handle).and_then(Node::as_element) else {
            return Vec::new();
        };
        if select.has_attribute("multiple") {
            return options
                .into_iter()
                .filter(|handle| {
                    self.node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(Element::selected)
                })
                .collect();
        }

        if let Some(selected) = options.iter().rev().copied().find(|handle| {
            self.node(*handle)
                .and_then(Node::as_element)
                .is_some_and(Element::selected)
        }) {
            return vec![selected];
        }

        if select.select_explicit_none() || select_display_size(select) != 1 {
            return Vec::new();
        }

        options
            .into_iter()
            .find(|handle| !self.option_is_disabled(*handle))
            .into_iter()
            .collect()
    }
}

fn select_display_size(select: &Element) -> i32 {
    select
        .attribute("size")
        .map(parse_non_negative_integer_prefix)
        .unwrap_or(0)
        .max(1)
}

fn is_listed_form_control_element(element: &Element) -> bool {
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return false;
    }

    match element.local_name() {
        "input" => element.input_type() != InputType::Image,
        "button" | "fieldset" | "object" | "output" | "select" | "textarea" => true,
        _ => false,
    }
}
