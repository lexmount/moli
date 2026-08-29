use crate::{CssDirection, first_strong_text_direction};
use dom::{ElementState, HEADING_LEVEL_OFFSET};

use crate::{
    dom::{
        NodeId,
        forms::{
            form_control_type_supports_intrinsic_validation, input_range_overflow,
            input_range_underflow, parse_input_numeric_value, parse_non_negative_integer_prefix,
        },
        native::{DomHost, Element, Node},
    },
    stylo::{atoms::normalized_direction, query::QueryElement},
};

impl<'a> QueryElement<'a> {
    pub(in crate::stylo) fn computed_state(self) -> ElementState {
        let element = self.element();
        let mut state = ElementState::empty();
        let local_name = element.local_name();

        if self.matches_defined_pseudo() {
            state |= ElementState::DEFINED;
        }
        if matches!(local_name, "a" | "area" | "link") && element.attribute("href").is_some() {
            state |= ElementState::UNVISITED;
        }
        if self.matches_checked_pseudo() {
            state |= ElementState::CHECKED;
        }
        if self.matches_indeterminate_pseudo() {
            state |= ElementState::INDETERMINATE;
        }
        if self.matches_disabled_pseudo() {
            state |= ElementState::DISABLED;
        } else if self.is_disableable_element() {
            state |= ElementState::ENABLED;
        }
        if self.matches_required_pseudo() {
            state |= ElementState::REQUIRED;
        } else if self.matches_optional_pseudo() {
            state |= ElementState::OPTIONAL_;
        }
        if self.matches_read_only_pseudo() {
            state |= ElementState::READONLY;
        } else if self.matches_read_write_pseudo() {
            state |= ElementState::READWRITE;
        }
        if self.matches_placeholder_shown_pseudo() {
            state |= ElementState::PLACEHOLDER_SHOWN;
        }
        if element.autofilled() {
            state |= ElementState::AUTOFILL;
        }
        if self.matches_validity_pseudo() {
            if self.is_invalid() {
                state |= ElementState::INVALID;
            } else {
                state |= ElementState::VALID;
            }
        }
        if self.matches_in_range_pseudo() {
            state |= ElementState::INRANGE;
        } else if self.matches_out_of_range_pseudo() {
            state |= ElementState::OUTOFRANGE;
        }
        if self.matches_default_pseudo() {
            state |= ElementState::DEFAULT;
        }
        if self.host.element_matches_focus(self.handle) {
            state |= ElementState::FOCUS | ElementState::FOCUSRING;
        }
        if self.host.element_matches_focus_within(self.handle) {
            state |= ElementState::FOCUS_WITHIN;
        }
        if self.host.element_matches_hover(self.handle) {
            state |= ElementState::HOVER;
        }
        if self.matches_target_pseudo() {
            state |= ElementState::URLTARGET;
        }
        if element.popover_open() && self.host.is_connected(self.handle) {
            state |= ElementState::POPOVER_OPEN;
        }
        state |= self.heading_state();
        if element.is_html_media() {
            if element.media_paused() {
                state |= ElementState::PAUSED;
            }
            if element.media_muted() {
                state |= ElementState::MUTED;
            }
            if element.media_seeking() {
                state |= ElementState::SEEKING;
            }
        }
        match self.resolved_direction() {
            CssDirection::Ltr => state |= ElementState::LTR,
            CssDirection::Rtl => state |= ElementState::RTL,
        }
        state
    }

    pub(super) fn heading_state(self) -> ElementState {
        const HEADING_NAMES: [(&str, u64); 6] = [
            ("h1", 1),
            ("h2", 2),
            ("h3", 3),
            ("h4", 4),
            ("h5", 5),
            ("h6", 6),
        ];
        HEADING_NAMES
            .iter()
            .find_map(|(name, level)| self.element().is_html_element(name).then_some(*level))
            .map(|level| ElementState::from_bits_retain(level << HEADING_LEVEL_OFFSET))
            .unwrap_or_else(ElementState::empty)
    }

    pub(in crate::stylo) fn resolved_direction(self) -> CssDirection {
        html_directionality(self.host, self.handle)
    }

    pub(super) fn matches_target_pseudo(self) -> bool {
        self.host.element_matches_target(self.handle)
    }

    pub(super) fn is_barred_from_constraint_validation(self) -> bool {
        !self.matches_validity_pseudo()
    }

    pub(super) fn has_invalid_descendant(self) -> bool {
        let mut stack = self.host.child_handles(self.handle).collect::<Vec<_>>();
        while let Some(handle) = stack.pop() {
            stack.extend(self.host.child_handles(handle));
            if self.host.node(handle).is_some_and(Node::is_element)
                && (QueryElement {
                    host: self.host,
                    handle,
                    shared_lock: self.shared_lock,
                    style_data: self.style_data,
                    atom_cache: self.atom_cache,
                })
                .is_locally_invalid()
            {
                return true;
            }
        }
        false
    }

    pub(super) fn is_invalid(self) -> bool {
        if self.is_locally_invalid() {
            return true;
        }
        matches!(self.element().local_name(), "form" | "fieldset") && self.has_invalid_descendant()
    }

    fn is_locally_invalid(self) -> bool {
        if self.is_readonly_barred_from_constraint_validation() {
            return false;
        }
        if !self.element().custom_validation_message().is_empty() {
            return true;
        }
        match self.element().local_name() {
            "form" | "fieldset" => false,
            "select" => {
                if !self.element().has_attribute("required") {
                    return false;
                }
                self.select_suffers_required_value_missing()
            }
            "input" | "textarea" => {
                if self.matches_range_underflow_pseudo() || self.matches_range_overflow_pseudo() {
                    return true;
                }
                if self.matches_required_pseudo() {
                    let ty = self.input_type();
                    if matches!(ty.as_str(), "checkbox" | "radio") {
                        return !self.element().checked();
                    }
                    return self.element().input_value().is_empty();
                }
                if self.element().local_name() == "input"
                    && self.input_type() == "number"
                    && !self.element().input_value().is_empty()
                    && parse_input_numeric_value("number", &self.element().input_value()).is_none()
                {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    pub(super) fn input_type(self) -> String {
        self.element().input_type()
    }

    pub(super) fn select_suffers_required_value_missing(self) -> bool {
        let selected_options = self.host.select_selected_option_elements(self.handle);
        if selected_options.is_empty() {
            return true;
        }
        selected_options.len() == 1
            && self.select_placeholder_label_option() == Some(selected_options[0])
    }

    pub(super) fn select_placeholder_label_option(self) -> Option<NodeId> {
        if self.element().has_attribute("multiple") || self.select_display_size() != 1 {
            return None;
        }
        let first = self
            .host
            .select_option_elements(self.handle)
            .first()
            .copied()?;
        if self.host.parent_node(first) != Some(self.handle) {
            return None;
        }
        self.host
            .option_value(first)
            .is_some_and(|value| value.is_empty())
            .then_some(first)
    }

    pub(super) fn select_display_size(self) -> i32 {
        self.element()
            .attribute("size")
            .map(parse_non_negative_integer_prefix)
            .unwrap_or(0)
            .max(1)
    }

    pub(super) fn is_disableable_element(self) -> bool {
        matches!(
            self.element().local_name(),
            "button" | "input" | "select" | "textarea" | "fieldset" | "optgroup" | "option"
        )
    }

    pub(super) fn matches_disabled_pseudo(self) -> bool {
        if !self.is_disableable_element() {
            return false;
        }
        if self.element().has_attribute("disabled") {
            return true;
        }
        if self.element().local_name() == "option"
            && self.parent_element().is_some_and(|(_, parent)| {
                parent.local_name() == "optgroup" && parent.has_attribute("disabled")
            })
        {
            return true;
        }
        if matches!(self.element().local_name(), "option" | "optgroup")
            && self.disabled_select_ancestor().is_some()
        {
            return true;
        }
        self.disabled_fieldset_ancestor().is_some()
    }

    pub(super) fn parent_element(self) -> Option<(NodeId, &'a Element)> {
        let parent = self.node().parent_node()?;
        let element = self.host.node(parent)?.as_element()?;
        Some((parent, element))
    }

    pub(super) fn disabled_fieldset_ancestor(self) -> Option<NodeId> {
        let mut current = self.node().parent_node();
        while let Some(parent) = current {
            if let Some(element) = self.host.node(parent).and_then(Node::as_element)
                && element.local_name() == "fieldset"
                && element.has_attribute("disabled")
                && !self.is_inside_first_legend_child(parent)
            {
                return Some(parent);
            }
            current = self.host.node(parent).and_then(Node::parent_node);
        }
        None
    }

    pub(super) fn disabled_select_ancestor(self) -> Option<NodeId> {
        let mut current = self.node().parent_node();
        while let Some(parent) = current {
            let Some(element) = self.host.node(parent).and_then(Node::as_element) else {
                current = self.host.node(parent).and_then(Node::parent_node);
                continue;
            };
            match element.local_name() {
                "select" if element.has_attribute("disabled") => return Some(parent),
                "select" | "option" => return None,
                "optgroup" if self.element().local_name() == "optgroup" => return None,
                _ => {}
            }
            current = self.host.node(parent).and_then(Node::parent_node);
        }
        None
    }

    pub(super) fn is_inside_first_legend_child(self, fieldset: NodeId) -> bool {
        let Some(legend) = self.host.child_handles(fieldset).find(|child| {
            self.host
                .node(*child)
                .and_then(Node::as_element)
                .is_some_and(|element| element.local_name() == "legend")
        }) else {
            return false;
        };
        self.handle == legend || self.is_descendant_of(legend)
    }

    pub(super) fn is_descendant_of(self, ancestor: NodeId) -> bool {
        let mut current = self.node().parent_node();
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.host.node(parent).and_then(Node::parent_node);
        }
        false
    }

    pub(super) fn can_match_required_pseudo(self) -> bool {
        match self.element().local_name() {
            "select" | "textarea" => true,
            "input" => !matches!(
                self.input_type().as_str(),
                "hidden" | "button" | "submit" | "reset" | "image"
            ),
            _ => false,
        }
    }

    pub(super) fn matches_required_pseudo(self) -> bool {
        self.can_match_required_pseudo() && self.element().has_attribute("required")
    }

    pub(super) fn matches_optional_pseudo(self) -> bool {
        // Blink treats every built-in input/button control as optional when it
        // is not required, including input states such as hidden for which the
        // required attribute does not apply. Keep this membership separate
        // from can_match_required_pseudo().
        matches!(
            self.element().local_name(),
            "button" | "input" | "select" | "textarea"
        ) && !self.matches_required_pseudo()
    }

    pub(super) fn matches_read_write_pseudo(self) -> bool {
        if self.matches_disabled_pseudo() {
            return false;
        }
        match self.element().local_name() {
            "textarea" => !self.element().has_attribute("readonly"),
            "input" => {
                matches!(
                    self.input_type().as_str(),
                    "text"
                        | "search"
                        | "url"
                        | "tel"
                        | "email"
                        | "password"
                        | "date"
                        | "month"
                        | "week"
                        | "time"
                        | "datetime-local"
                        | "number"
                ) && !self.element().has_attribute("readonly")
            }
            _ => self.is_editable(),
        }
    }

    pub(super) fn matches_read_only_pseudo(self) -> bool {
        !self.matches_read_write_pseudo()
    }

    pub(super) fn is_editable(self) -> bool {
        let mut current = Some(self.handle);
        while let Some(handle) = current {
            if let Some(element) = self.host.node(handle).and_then(Node::as_element)
                && let Some(value) = element.attribute("contenteditable")
                && let Some(is_editable) = contenteditable_value_is_editable(value)
            {
                return is_editable;
            }
            current = self.host.node(handle).and_then(Node::parent_node);
        }
        false
    }

    pub(super) fn matches_placeholder_shown_pseudo(self) -> bool {
        if self.element().attribute("placeholder").is_none()
            || !self.element().input_value().is_empty()
        {
            return false;
        }
        match self.element().local_name() {
            "textarea" => true,
            "input" => matches!(
                self.input_type().as_str(),
                "text" | "search" | "url" | "tel" | "email" | "password"
            ),
            _ => false,
        }
    }

    pub(super) fn matches_checked_pseudo(self) -> bool {
        match self.element().local_name() {
            "option" => self.element().selected(),
            "input" => {
                matches!(self.input_type().as_str(), "checkbox" | "radio")
                    && self.element().checked()
            }
            _ => false,
        }
    }

    pub(super) fn matches_indeterminate_pseudo(self) -> bool {
        match self.element().local_name() {
            "progress" => !self.element().has_attribute("value"),
            "input" if self.input_type() == "checkbox" => self.element().indeterminate(),
            "input" if self.input_type() == "radio" => !self.radio_group_has_checked_input(),
            _ => false,
        }
    }

    pub(super) fn radio_group_has_checked_input(self) -> bool {
        let name = self.element().attribute("name").unwrap_or_default();
        let mut stack = self
            .host
            .child_handles_reversed(self.host.document_handle())
            .collect::<Vec<_>>();
        while let Some(handle) = stack.pop() {
            stack.extend(self.host.child_handles_reversed(handle));
            if let Some(element) = self.host.node(handle).and_then(Node::as_element)
                && element.local_name() == "input"
                && element.input_type() == "radio"
                && element.attribute("name").unwrap_or_default() == name
                && element.checked()
            {
                return true;
            }
        }
        false
    }

    pub(super) fn is_constraint_validation_candidate(self) -> bool {
        if self.matches_disabled_pseudo() || self.is_readonly_barred_from_constraint_validation() {
            return false;
        }
        form_control_type_supports_intrinsic_validation(
            self.element().local_name(),
            self.element().attribute("type"),
            self.element().attribute("type"),
        )
    }

    pub(super) fn is_readonly_barred_from_constraint_validation(self) -> bool {
        if !self.element().has_attribute("readonly") {
            return false;
        }
        matches!(self.element().local_name(), "input" | "textarea")
    }

    pub(super) fn matches_validity_pseudo(self) -> bool {
        matches!(self.element().local_name(), "form" | "fieldset")
            || self.is_constraint_validation_candidate()
    }

    pub(super) fn numeric_input_value(self) -> Option<f64> {
        if self.element().local_name() != "input"
            || !self.is_constraint_validation_candidate()
            || self.is_readonly_barred_from_constraint_validation()
        {
            return None;
        }
        let input_type = self.input_type();
        let value = parse_input_numeric_value(&input_type, &self.element().input_value())?;
        if input_type != "range" {
            return Some(value);
        }
        let min = self
            .element()
            .attribute("min")
            .and_then(|value| parse_input_numeric_value("range", value))
            .unwrap_or(0.0);
        let max = self
            .element()
            .attribute("max")
            .and_then(|value| parse_input_numeric_value("range", value))
            .unwrap_or(100.0);
        Some(if min <= max {
            value.clamp(min, max)
        } else {
            value
        })
    }

    pub(super) fn has_range_limitations(self) -> bool {
        if self.element().local_name() != "input" {
            return false;
        }
        matches!(
            self.input_type().as_str(),
            "date" | "time" | "datetime-local" | "month" | "week" | "number" | "range"
        ) && (self.input_type() == "range"
            || self.element().attribute("min").is_some()
            || self.element().attribute("max").is_some())
    }

    pub(super) fn matches_range_underflow_pseudo(self) -> bool {
        self.has_range_limitations()
            && self.numeric_input_value().is_some_and(|value| {
                input_range_underflow(
                    &self.input_type(),
                    value,
                    self.element().attribute("min"),
                    self.element().attribute("max"),
                )
            })
    }

    pub(super) fn matches_range_overflow_pseudo(self) -> bool {
        self.has_range_limitations()
            && self.numeric_input_value().is_some_and(|value| {
                input_range_overflow(
                    &self.input_type(),
                    value,
                    self.element().attribute("min"),
                    self.element().attribute("max"),
                )
            })
    }

    pub(super) fn matches_in_range_pseudo(self) -> bool {
        self.has_range_limitations()
            && self.numeric_input_value().is_some()
            && !self.matches_range_underflow_pseudo()
            && !self.matches_range_overflow_pseudo()
    }

    pub(super) fn matches_out_of_range_pseudo(self) -> bool {
        self.matches_range_underflow_pseudo() || self.matches_range_overflow_pseudo()
    }

    pub(super) fn matches_default_pseudo(self) -> bool {
        match self.element().local_name() {
            "option" => self.element().selected(),
            "input" if matches!(self.input_type().as_str(), "checkbox" | "radio") => {
                self.element().has_attribute("checked")
            }
            "input" if matches!(self.input_type().as_str(), "submit" | "image") => {
                self.is_first_default_submit_button()
            }
            "button" => {
                let ty = self
                    .element()
                    .attribute("type")
                    .unwrap_or("submit")
                    .trim()
                    .to_ascii_lowercase();
                !matches!(ty.as_str(), "button" | "reset") && self.is_first_default_submit_button()
            }
            _ => false,
        }
    }

    pub(super) fn is_first_default_submit_button(self) -> bool {
        let Some(form) = self.nearest_ancestor_form() else {
            return false;
        };
        self.first_default_submit_button_in_subtree(form) == Some(self.handle)
    }

    pub(super) fn nearest_ancestor_form(self) -> Option<NodeId> {
        let mut current = self.node().parent_node();
        while let Some(parent) = current {
            if self
                .host
                .node(parent)
                .and_then(Node::as_element)
                .is_some_and(|element| element.local_name() == "form")
            {
                return Some(parent);
            }
            current = self.host.node(parent).and_then(Node::parent_node);
        }
        None
    }

    pub(super) fn first_default_submit_button_in_subtree(self, root: NodeId) -> Option<NodeId> {
        let mut stack = self.host.child_handles_reversed(root).collect::<Vec<_>>();
        while let Some(child) = stack.pop() {
            if self
                .host
                .node(child)
                .and_then(Node::as_element)
                .is_some_and(default_submit_button_element)
            {
                return Some(child);
            }
            stack.extend(self.host.child_handles_reversed(child));
        }
        None
    }
}

pub(crate) fn html_directionality(host: &DomHost, handle: NodeId) -> CssDirection {
    let mut current = Some(handle);
    while let Some(handle) = current {
        if let Some(element) = host.node(handle).and_then(Node::as_element) {
            if let Some(direction) = element.attribute("dir").and_then(normalized_direction) {
                return direction;
            }
            if element
                .attribute("dir")
                .is_some_and(|value| value.eq_ignore_ascii_case("auto"))
                || element.is_html_element("bdi")
            {
                return auto_direction_for_element(host, handle).unwrap_or(CssDirection::Ltr);
            }
            if element.is_html_input() && element.input_type() == "tel" {
                return CssDirection::Ltr;
            }
        }
        current = host
            .node(handle)
            .and_then(Node::parent_node)
            .or_else(|| host.shadow_root_host(handle));
    }
    CssDirection::Ltr
}

fn auto_direction_for_element(host: &DomHost, root: NodeId) -> Option<CssDirection> {
    if let Some(element) = host.node(root).and_then(Node::as_element)
        && element.is_html_input()
    {
        return input_auto_direction(element);
    }

    let mut stack = host.child_handles(root).collect::<Vec<_>>();
    stack.reverse();
    while let Some(handle) = stack.pop() {
        let Some(node) = host.node(handle) else {
            continue;
        };
        if let Some(text) = node.as_text() {
            if let Some(direction) = first_strong_text_direction(text.data()) {
                return Some(direction);
            }
            continue;
        }
        let Some(element) = node.as_element() else {
            continue;
        };
        if descendant_is_directionally_isolated_for_auto(element) {
            continue;
        }
        let mut children = host.child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn descendant_is_directionally_isolated_for_auto(element: &Element) -> bool {
    if element.is_html_element("bdi") {
        return true;
    }
    element.attribute("dir").is_some_and(|value| {
        normalized_direction(value).is_some() || value.eq_ignore_ascii_case("auto")
    })
}

fn input_auto_direction(element: &Element) -> Option<CssDirection> {
    input_type_uses_value_for_auto_direction(&element.input_type())
        .then(|| first_strong_text_direction(&element.input_value()))
        .flatten()
}

fn input_type_uses_value_for_auto_direction(input_type: &str) -> bool {
    matches!(
        input_type,
        "hidden"
            | "text"
            | "search"
            | "tel"
            | "url"
            | "email"
            | "password"
            | "submit"
            | "reset"
            | "button"
    )
}

fn default_submit_button_element(element: &Element) -> bool {
    match element.local_name() {
        "button" => {
            let ty = element
                .attribute("type")
                .unwrap_or("submit")
                .trim()
                .to_ascii_lowercase();
            !matches!(ty.as_str(), "button" | "reset")
        }
        "input" => matches!(element.input_type().as_str(), "submit" | "image"),
        _ => false,
    }
}

fn contenteditable_value_is_editable(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "true" | "plaintext-only" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
