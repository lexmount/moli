use crate::forms::{InputType, parse_html_integer_prefix};

use super::{Element, NativeDom, NativeNodeId, Node};

const HTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentEditableState {
    Inherit,
    True,
    False,
    PlaintextOnly,
}

impl ContentEditableState {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::True => "true",
            Self::False => "false",
            Self::PlaintextOnly => "plaintext-only",
        }
    }

    fn is_editable(self) -> bool {
        matches!(self, Self::True | Self::PlaintextOnly)
    }
}

/// Focus support from the DOM. The renderer determines whether the element
/// is rendered and can use geometry to admit a scroll container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusSupport {
    Blocked,
    Supported,
    LayoutDependent,
}

impl Element {
    /// The raw target of a navigable HTML or SVG hyperlink, including an empty
    /// target. URL resolution uses the element's owner document.
    pub fn hyperlink_href(&self) -> Option<&str> {
        match (self.namespace(), self.local_name()) {
            (HTML_NS, "a" | "area") => self.attribute_ns("", "href"),
            (SVG_NS, "a") => self
                .attribute_ns("", "href")
                .or_else(|| self.attribute_ns(XLINK_NS, "href")),
            _ => None,
        }
    }

    pub fn explicit_tab_index(&self) -> Option<i32> {
        self.attribute_ns("", "tabindex")
            .and_then(parse_html_integer_prefix)
    }

    pub fn content_editable_state(&self) -> ContentEditableState {
        let Some(value) = self
            .attribute_ns("", "contenteditable")
            .filter(|_| self.namespace() == HTML_NS)
        else {
            return ContentEditableState::Inherit;
        };
        if value.is_empty() || value.eq_ignore_ascii_case("true") {
            ContentEditableState::True
        } else if value.eq_ignore_ascii_case("false") {
            ContentEditableState::False
        } else if value.eq_ignore_ascii_case("plaintext-only") {
            ContentEditableState::PlaintextOnly
        } else {
            ContentEditableState::Inherit
        }
    }

    pub fn supports_disabled(&self) -> bool {
        self.namespace() == HTML_NS
            && matches!(
                self.local_name(),
                "button" | "input" | "select" | "textarea" | "option" | "optgroup" | "fieldset"
            )
    }
}

impl NativeDom {
    pub fn main_summary_child(&self, details: NativeNodeId) -> Option<NativeNodeId> {
        if !self.is_html_element_named(details, "details") {
            return None;
        }
        self.child_ids(details)
            .find(|child| self.is_html_element_named(*child, "summary"))
    }

    /// The nearest explicit editable element used by input and selection.
    pub fn contenteditable_editing_host(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.editable_ancestor(node_id, false)
    }

    pub fn is_content_editable(&self, node_id: NativeNodeId) -> bool {
        self.editable_ancestor(node_id, true).is_some()
    }

    fn editable_ancestor(
        &self,
        node_id: NativeNodeId,
        include_design_mode: bool,
    ) -> Option<NativeNodeId> {
        let mut current = Some(node_id);
        while let Some(id) = current {
            let node = self.node(id)?;
            if let Some(document) = node.as_document() {
                return (include_design_mode && document.design_mode_enabled()).then_some(id);
            }
            if let Some(element) = node.as_element() {
                match element.content_editable_state() {
                    ContentEditableState::True | ContentEditableState::PlaintextOnly => {
                        return Some(id);
                    }
                    ContentEditableState::False => return None,
                    ContentEditableState::Inherit => {}
                }
            }
            current = node.parent_node_id();
        }
        None
    }

    pub fn focus_support(&self, node_id: NativeNodeId) -> FocusSupport {
        let Some(element) = self.node(node_id).and_then(Node::as_element) else {
            return FocusSupport::Blocked;
        };
        if (element.is_html_input() && element.input_type() == InputType::Hidden)
            || self.is_disabled_form_control(node_id)
        {
            return FocusSupport::Blocked;
        }
        if element.explicit_tab_index().is_some() || self.has_default_focus_behavior(node_id) {
            return FocusSupport::Supported;
        }
        FocusSupport::LayoutDependent
    }

    pub fn has_default_focus_behavior(&self, node_id: NativeNodeId) -> bool {
        let Some(element) = self.node(node_id).and_then(Node::as_element) else {
            return false;
        };
        let native_behavior = match (element.namespace(), element.local_name()) {
            (HTML_NS, "button" | "input" | "select" | "textarea" | "frame" | "iframe" | "dialog" | "object") => true,
            (HTML_NS, "audio" | "video") => element.has_attribute_ns("", "controls"),
            (HTML_NS, "summary") => self.parent_node(node_id)
                .and_then(|parent| self.main_summary_child(parent)) == Some(node_id),
            ("http://www.w3.org/1998/Math/MathML", "a") => element.has_attribute_ns("", "href"),
            _ => element.hyperlink_href().is_some(),
        };
        native_behavior || (element.content_editable_state().is_editable()
            && self.parent_node(node_id).is_none_or(|parent| !self.is_content_editable(parent)))
    }

    /// Native disabled state. Form-associated custom elements use the same
    /// fieldset rule after the runtime has resolved their definitions.
    pub fn is_disabled_form_control(&self, node_id: NativeNodeId) -> bool {
        self.node(node_id)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.supports_disabled()
                    && (element.has_attribute_ns("", "disabled")
                        || (element.is_html_option() && self.option_is_disabled(node_id))
                        || self.disabled_fieldset_ancestor(node_id).is_some())
            })
    }

    pub fn disabled_fieldset_ancestor(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        let mut current = self.parent_node(node_id);
        while let Some(parent) = current {
            if self
                .node(parent)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.is_html_fieldset() && element.has_attribute_ns("", "disabled")
                })
            {
                let legend = self
                    .child_ids(parent)
                    .find(|child| self.is_html_element_named(*child, "legend"));
                if legend.is_none_or(|legend| !self.contains(legend, node_id)) {
                    return Some(parent);
                }
            }
            current = self.parent_node(parent);
        }
        None
    }
}
