use crate::{CssDirection, first_strong_text_direction};
use dom::{ElementState, HEADING_LEVEL_OFFSET};

use crate::{
    dom::{
        NodeId,
        forms::{
            InputType, form_control_type_supports_intrinsic_validation, input_range_overflow,
            input_range_underflow, parse_input_numeric_value, parse_non_negative_integer_prefix,
        },
        native::{DomHost, Element, Node},
    },
    stylo::query::QueryElement,
};

impl<'a> QueryElement<'a> {
    pub(in crate::stylo) fn computed_state(self) -> ElementState {
        let element = self.element();
        let mut state = ElementState::empty();

        if self.matches_defined_pseudo() {
            state |= ElementState::DEFINED;
        }
        if element.hyperlink_href().is_some()
            || (element.is_html_element("link") && element.has_attribute_ns("", "href"))
        {
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
        if element.web_mcp_form_active() {
            state |= ElementState::TOOL_FORM_ACTIVE;
        }
        if element.web_mcp_submit_active() {
            state |= ElementState::TOOL_SUBMIT_ACTIVE;
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
            state |= ElementState::FOCUS;
        }
        if self.host.element_matches_focus_visible(self.handle) {
            state |= ElementState::FOCUSRING;
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
        if self.matches_modal_pseudo() {
            state |= ElementState::MODAL;
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

    pub(super) fn matches_modal_pseudo(self) -> bool {
        self.element().dialog_modal()
            && self.element().has_attribute("open")
            && self.host.is_connected(self.handle)
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

    pub(in crate::stylo) fn directionality_hint(self) -> Option<CssDirection> {
        directionality_hint(self.host, self.handle, self.element())
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
                    if ty.is_checkable() {
                        return !self.element().checked();
                    }
                    return self.element().input_value().is_empty();
                }
                self.element().local_name() == "input"
                    && self.input_type() == InputType::Number
                    && !self.element().input_value().is_empty()
                    && parse_input_numeric_value(InputType::Number, &self.element().input_value())
                        .is_none()
            }
            _ => false,
        }
    }

    pub(super) fn input_type(self) -> InputType {
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
        self.element().supports_disabled()
    }

    pub(super) fn matches_disabled_pseudo(self) -> bool {
        self.is_disableable_element()
            && (self.host.dom().is_disabled_form_control(self.handle)
                || (matches!(self.element().local_name(), "option" | "optgroup")
                    && self.disabled_select_ancestor().is_some()))
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

    pub(super) fn can_match_required_pseudo(self) -> bool {
        match self.element().local_name() {
            "select" | "textarea" => true,
            "input" => self.input_type().supports_required(),
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
                self.input_type().supports_readonly() && !self.element().has_attribute("readonly")
            }
            _ => self.is_editable(),
        }
    }

    pub(super) fn matches_read_only_pseudo(self) -> bool {
        !self.matches_read_write_pseudo()
    }

    pub(super) fn is_editable(self) -> bool {
        self.host.dom().is_content_editable(self.handle)
    }

    pub(super) fn matches_placeholder_shown_pseudo(self) -> bool {
        if self.element().attribute("placeholder").is_none()
            || !self.element().input_value().is_empty()
        {
            return false;
        }
        match self.element().local_name() {
            "textarea" => true,
            "input" => self.input_type().supports_placeholder(),
            _ => false,
        }
    }

    pub(super) fn matches_checked_pseudo(self) -> bool {
        match self.element().local_name() {
            "option" => self.element().selected(),
            "input" => self.input_type().is_checkable() && self.element().checked(),
            _ => false,
        }
    }

    pub(super) fn matches_indeterminate_pseudo(self) -> bool {
        match self.element().local_name() {
            "progress" => !self.element().has_attribute("value"),
            "input" if self.input_type() == InputType::Checkbox => self.element().indeterminate(),
            "input" if self.input_type() == InputType::Radio => {
                !self.radio_group_has_checked_input()
            }
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
                && element.input_type() == InputType::Radio
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
            self.element()
                .is_html_input()
                .then(|| self.element().input_type()),
            self.host.button_is_submit_button(self.handle),
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
        let value = parse_input_numeric_value(input_type, &self.element().input_value())?;
        if input_type != InputType::Range {
            return Some(value);
        }
        let min = self
            .element()
            .attribute("min")
            .and_then(|value| parse_input_numeric_value(InputType::Range, value))
            .unwrap_or(0.0);
        let max = self
            .element()
            .attribute("max")
            .and_then(|value| parse_input_numeric_value(InputType::Range, value))
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
        self.input_type().supports_value_as_number()
            && (self.input_type() == InputType::Range
                || self.element().attribute("min").is_some()
                || self.element().attribute("max").is_some())
    }

    pub(super) fn matches_range_underflow_pseudo(self) -> bool {
        self.has_range_limitations()
            && self.numeric_input_value().is_some_and(|value| {
                input_range_underflow(
                    self.input_type(),
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
                    self.input_type(),
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
            "input" if self.input_type().is_checkable() => self.element().has_attribute("checked"),
            "input" if self.input_type().is_submit_button() => {
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
        if let Some(element) = host.node(handle).and_then(Node::as_element)
            && let Some(direction) = directionality_hint(host, handle, element)
        {
            return direction;
        }
        current = host
            .node(handle)
            .and_then(Node::parent_node)
            .or_else(|| host.shadow_root_host(handle));
    }
    CssDirection::Ltr
}

#[derive(PartialEq, Eq)]
enum SpecifiedDirectionality {
    Fixed(CssDirection),
    Auto,
    Inherited,
}

fn specified_directionality(element: &Element) -> SpecifiedDirectionality {
    match element.attribute_ns("", "dir") {
        Some(dir) if dir.eq_ignore_ascii_case("ltr") => {
            SpecifiedDirectionality::Fixed(CssDirection::Ltr)
        }
        Some(dir) if dir.eq_ignore_ascii_case("rtl") => {
            SpecifiedDirectionality::Fixed(CssDirection::Rtl)
        }
        Some(dir) if dir.eq_ignore_ascii_case("auto") => SpecifiedDirectionality::Auto,
        _ if element.is_html_element("bdi") => SpecifiedDirectionality::Auto,
        _ => SpecifiedDirectionality::Inherited,
    }
}

fn directionality_hint(host: &DomHost, handle: NodeId, element: &Element) -> Option<CssDirection> {
    match specified_directionality(element) {
        SpecifiedDirectionality::Fixed(direction) => Some(direction),
        SpecifiedDirectionality::Auto => {
            Some(auto_direction_for_element(host, handle).unwrap_or(CssDirection::Ltr))
        }
        SpecifiedDirectionality::Inherited
            if element.is_html_input() && element.input_type() == InputType::Tel =>
        {
            Some(CssDirection::Ltr)
        }
        SpecifiedDirectionality::Inherited => None,
    }
}

/// Finds the closest auto-direction element whose text scan can include
/// content inside `start`, including `start` itself.
pub fn html_auto_directionality_root(host: &DomHost, start: NodeId) -> Option<NodeId> {
    let mut current = Some(start);
    while let Some(handle) = current {
        let node = host.node(handle)?;
        if let Some(element) = node.as_element() {
            let directionality = specified_directionality(element);
            if directionality == SpecifiedDirectionality::Auto {
                return Some(handle);
            }
            if descendant_is_directionally_isolated_for_auto(element, directionality) {
                return None;
            }
        }
        current = node.parent_node();
    }
    None
}

fn auto_direction_for_element(host: &DomHost, root: NodeId) -> Option<CssDirection> {
    if let Some(element) = host.node(root).and_then(Node::as_element) {
        if element.is_html_textarea() {
            let value = if element.input_value_dirty() {
                element.input_value()
            } else {
                host.direct_text_content(root).unwrap_or_default()
            };
            return first_strong_text_direction(&value);
        }
        if element.is_html_input() {
            return input_auto_direction(element);
        }
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
        if descendant_is_directionally_isolated_for_auto(element, specified_directionality(element))
        {
            continue;
        }
        let mut children = host.child_handles(handle).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
    }
    None
}

fn descendant_is_directionally_isolated_for_auto(
    element: &Element,
    directionality: SpecifiedDirectionality,
) -> bool {
    directionality != SpecifiedDirectionality::Inherited
        || (matches!(element.local_name(), "script" | "style" | "textarea")
            && element.namespace() == "http://www.w3.org/1999/xhtml")
}

fn input_auto_direction(element: &Element) -> Option<CssDirection> {
    element
        .input_type()
        .uses_value_for_auto_direction()
        .then(|| first_strong_text_direction(&element.input_value()))
        .flatten()
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
        "input" => element.input_type().is_submit_button(),
        _ => false,
    }
}
