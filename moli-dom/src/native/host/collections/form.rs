use super::*;
use crate::forms::{InputType, parse_non_negative_integer_prefix};

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

    fn is_listed_form_control_handle(&self, handle: DomHandle) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(is_listed_form_control_element)
    }

    pub fn form_control_owner(&self, handle: DomHandle) -> Option<DomHandle> {
        let element = self.node(handle).and_then(Node::as_element)?;
        if !matches!(
            element.local_name(),
            "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
        ) || element.namespace() != "http://www.w3.org/1999/xhtml"
        {
            return None;
        }

        if let Some(form_id) = element.attribute("form") {
            if form_id.is_empty() {
                return None;
            }
            let tree_root = self.root_node_handle(handle)?;
            if self.is_shadow_root(tree_root) && !self.is_connected(tree_root) {
                return None;
            }
            let candidate = self.element_handle_by_id_in_subtree(tree_root, form_id)?;
            let candidate = self.resolve_reference_target_chain(candidate)?;
            return self
                .is_html_element_named(candidate, "form")
                .then_some(candidate);
        }

        if let Some(owner) = element.parser_associated_form_owner()
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

    pub fn owner_select_for_option(&self, handle: DomHandle) -> Option<DomHandle> {
        if !self.is_html_element_named(handle, "option") {
            return None;
        }
        let mut current = self.parent_node(handle);
        while let Some(parent) = current {
            let Some(element) = self.node(parent).and_then(Node::as_element) else {
                current = self.parent_node(parent);
                continue;
            };
            if element.is_html_select() {
                return Some(parent);
            }
            current = self.parent_node(parent);
        }
        None
    }

    pub fn select_option_elements(&self, select_handle: DomHandle) -> Vec<DomHandle> {
        if !self.is_html_element_named(select_handle, "select") {
            return Vec::new();
        }
        self.collect_matching_elements(select_handle, false, |handle| {
            self.is_html_element_named(handle, "option")
                && self.option_belongs_to_select(handle, select_handle)
        })
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

    fn option_belongs_to_select(&self, option: DomHandle, select_handle: DomHandle) -> bool {
        let mut current = self.parent_node(option);
        let mut seen_optgroup = false;
        while let Some(parent) = current {
            if parent == select_handle {
                return true;
            }
            let Some(element) = self.node(parent).and_then(Node::as_element) else {
                current = self.parent_node(parent);
                continue;
            };
            match element.local_name() {
                "option" | "hr" | "select" => return false,
                "optgroup" if seen_optgroup => return false,
                "optgroup" => seen_optgroup = true,
                _ => {}
            }
            current = self.parent_node(parent);
        }
        false
    }

    fn option_is_disabled(&self, handle: DomHandle) -> bool {
        let mut current = Some(handle);
        while let Some(candidate) = current {
            let Some(element) = self.node(candidate).and_then(Node::as_element) else {
                current = self.parent_node(candidate);
                continue;
            };
            if matches!(element.local_name(), "option" | "optgroup")
                && element.has_attribute("disabled")
            {
                return true;
            }
            if element.is_html_select() {
                return false;
            }
            current = self.parent_node(candidate);
        }
        false
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
