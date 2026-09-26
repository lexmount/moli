use crate::document_runtime::DomHandle;
use crate::dom::forms::InputType;
use crate::dom::native::{Element, NodeType};
use crate::native_bridge::JsContextHost;
use crate::native_bridge::document::{MATHML_NS, XHTML_NS};
use crate::native_bridge::element::{StyleMode, style_property_value};

enum Visit {
    Node { node: DomHandle, block: DomHandle },
    EmptyBlock(DomHandle),
}

/// Resolve the initial DOM caret in an editing host. Walk editable content in
/// tree order, retaining block boundaries around non-editable islands and
/// element boundaries around line breaks and replaced content. This does not
/// use hit testing: the first DOM position need not be inside the viewport.
pub(super) fn initial_editing_caret(
    runtime: &JsContextHost,
    editing_host: DomHandle,
) -> Option<(DomHandle, u32)> {
    let dom = runtime.dom_host();
    let mut stack = vec![Visit::Node {
        node: editing_host,
        block: editing_host,
    }];
    while let Some(visit) = stack.pop() {
        let (handle, block) = match visit {
            Visit::Node { node, block } => (node, block),
            Visit::EmptyBlock(node) => {
                if node == editing_host || empty_block_has_caret_space(runtime, node) {
                    return Some((node, 0));
                }
                continue;
            }
        };
        let node = dom.node(handle)?;
        if matches!(node.node_type(), NodeType::Text | NodeType::CDataSection) {
            let parent = dom.parent_node(handle)?;
            if !is_visible(runtime, parent) {
                continue;
            }
            let collapse = computed(runtime, parent, "white-space-collapse");
            let preserve = matches!(collapse.as_str(), "preserve" | "break-spaces");
            let preserve_breaks = collapse == "preserve-breaks";
            let text = node.data_value()?;
            let first = text.encode_utf16().enumerate().find(|&(_, unit)| {
                preserve
                    || !is_collapsible_space(unit)
                    || (preserve_breaks && matches!(unit, 0x0a | 0x0d))
            });
            // An inline editing host can begin in the middle of a line. Its
            // first collapsed space is visible when it separates surrounding
            // inline content; the editing boundary itself is not a line break.
            if !text.is_empty()
                && first.is_none_or(|(offset, unit)| {
                    offset != 0 && !(preserve_breaks && matches!(unit, 0x0a | 0x0d))
                })
                && adjacent_inline_content(runtime, handle, false)
                && (first.is_some() || adjacent_inline_content(runtime, handle, true))
            {
                return Some((handle, 0));
            }
            if let Some((offset, _)) = first {
                return Some((handle, u32::try_from(offset).ok()?));
            }
            continue;
        }
        let Some(element) = node.as_element() else {
            continue;
        };
        let display = computed(runtime, handle, "display");
        if display == "none"
            || (element.is_html_input() && element.input_type() == InputType::Hidden)
        {
            continue;
        }
        let visible = is_visible(runtime, handle);
        // Every visited parent is editable in this host. False and inert
        // subtrees are islands, not absent content. Do not walk ancestors
        // again for each descendant of a deeply nested editor.
        if element.namespace() == XHTML_NS
            && (element.has_attribute("inert")
                || element
                    .attribute("contenteditable")
                    .is_some_and(|value| value.eq_ignore_ascii_case("false")))
        {
            if visible {
                return Some((block, 0));
            }
            continue;
        }
        if handle != editing_host
            && element.namespace() == XHTML_NS
            && (is_replaced_element(element)
                || matches!(element.local_name(), "br" | "hr" | "table"))
        {
            if visible {
                let parent = dom.parent_node(handle)?;
                return Some((
                    parent,
                    u32::try_from(dom.child_index(parent, handle)?).ok()?,
                ));
            }
            continue;
        }
        let is_block = display != "contents" && !display.starts_with("inline");
        let block = if is_block { handle } else { block };
        if visible && (is_block || handle == editing_host) {
            stack.push(Visit::EmptyBlock(handle));
        }
        // Use an explicit stack so deeply nested author content cannot exhaust
        // the Rust call stack during a synchronous focus operation.
        stack.extend(
            dom.child_handles(handle)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|node| Visit::Node { node, block }),
        );
    }
    None
}

fn computed(runtime: &JsContextHost, node: DomHandle, property: &str) -> String {
    style_property_value(runtime, node, StyleMode::Computed, property)
}

fn is_collapsible_space(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

fn is_replaced_element(element: &Element) -> bool {
    element.namespace() == XHTML_NS
        && matches!(
            element.local_name(),
            "img"
                | "input"
                | "textarea"
                | "select"
                | "iframe"
                | "object"
                | "embed"
                | "canvas"
                | "audio"
                | "video"
        )
}

fn is_inline_container(display: &str) -> bool {
    matches!(display, "inline" | "contents")
}

/// Inspect the neighboring inline stream, including content outside an inline
/// editor. Stop at formatting-context boundaries; skip comments and display:none
/// subtrees. Hidden text still participates in whitespace collapsing.
fn adjacent_inline_content(runtime: &JsContextHost, mut node: DomHandle, forward: bool) -> bool {
    let dom = runtime.dom_host();
    loop {
        let mut adjacent = loop {
            let sibling = if forward {
                dom.next_sibling(node)
            } else {
                dom.previous_sibling(node)
            };
            if let Some(sibling) = sibling {
                break sibling;
            }
            let Some(parent) = dom.parent_node(node) else {
                return false;
            };
            // SVG and MathML have their own text layout; HTML's surrounding
            // inline stream cannot preserve their leading collapsed space.
            if dom
                .node(parent)
                .and_then(|node| node.as_element())
                .is_none_or(|element| element.namespace() != XHTML_NS)
                || !is_inline_container(&computed(runtime, parent, "display"))
            {
                return false;
            }
            node = parent;
        };
        loop {
            node = adjacent;
            let Some(candidate) = dom.node(node) else {
                return false;
            };
            if matches!(
                candidate.node_type(),
                NodeType::Text | NodeType::CDataSection
            ) {
                let text = candidate.data_value().unwrap_or_default();
                let Some(parent) = dom.parent_node(node) else {
                    return false;
                };
                let collapse = computed(runtime, parent, "white-space-collapse");
                let preserve = matches!(collapse.as_str(), "preserve" | "break-spaces");
                let preserve_breaks = preserve || collapse == "preserve-breaks";
                if forward {
                    for unit in text.encode_utf16() {
                        if preserve_breaks && matches!(unit, 0x0a | 0x0d) {
                            return false;
                        }
                        if preserve || !is_collapsible_space(unit) {
                            return true;
                        }
                    }
                } else if let Some(byte) = text.as_bytes().last() {
                    return (preserve && !matches!(byte, b'\n' | b'\r'))
                        || !is_collapsible_space(u16::from(*byte));
                }
                break;
            }
            let Some(element) = candidate.as_element() else {
                break;
            };
            let display = computed(runtime, node, "display");
            if display == "none"
                || matches!(
                    computed(runtime, node, "position").as_str(),
                    "absolute" | "fixed"
                )
            {
                break;
            }
            if element.is_html_element("br") {
                return false;
            }
            if !is_inline_container(&display) {
                return display.starts_with("inline");
            }
            // A foreign root occupies an inline box in the surrounding HTML,
            // although an initial caret can enter its text descendants.
            if is_replaced_element(element)
                || element.is_svg_element("svg")
                || (element.namespace() == MATHML_NS && element.local_name() == "math")
            {
                return true;
            }
            let child = if forward {
                dom.first_child(node)
            } else {
                dom.last_child(node)
            };
            let Some(child) = child else {
                break;
            };
            adjacent = child;
        }
    }
}

fn is_visible(runtime: &JsContextHost, node: DomHandle) -> bool {
    !matches!(
        computed(runtime, node, "visibility").as_str(),
        "hidden" | "collapse"
    )
}

fn empty_block_has_caret_space(runtime: &JsContextHost, node: DomHandle) -> bool {
    [
        "height",
        "min-height",
        "padding-top",
        "padding-bottom",
        "border-top-width",
        "border-bottom-width",
    ]
    .iter()
    .any(|property| {
        computed(runtime, node, property)
            .strip_suffix("px")
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value > 0.0)
    })
}
