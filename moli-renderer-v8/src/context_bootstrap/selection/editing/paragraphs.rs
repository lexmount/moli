use super::{is_inline_formatting, is_paragraph_or_heading};
use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, Node, NodeType};
use crate::native_bridge::{PageScriptHost, document::XHTML_NS};

pub(super) fn normalize_fragment_for_fixed_paragraph(
    runtime: &mut PageScriptHost,
    document: DomHandle,
    context: DomHandle,
    host: DomHandle,
    fragment: DomHandle,
) -> Option<()> {
    let dom = runtime.dom_host();
    let is_paragraph_host = dom.node(host).is_some_and(is_paragraph_or_heading);
    let is_preformatted =
        std::iter::successors(Some(context), |&node| dom.parent_node(node)).any(|node| {
            dom.node(node)
                .is_some_and(|node| node.is_html_element_named("pre"))
        });
    if !is_paragraph_host && !is_preformatted {
        return Some(());
    }

    // A paragraph editing host cannot be split into siblings. Preformatted
    // content also stays in its existing line container. Keep the incoming
    // inline nodes and express the pasted paragraph boundaries with breaks.
    let mut containers = vec![fragment];
    let mut index = 0;
    while let Some(&container) = containers.get(index) {
        containers.extend(dom.child_handles(container).filter(|&child| {
            dom.node(child)
                .is_some_and(|node| contains_paragraph_lines(node) || is_inline_formatting(node))
        }));
        index += 1;
    }
    let dom = runtime.dom_host_mut();
    for container in containers.into_iter().rev() {
        normalize_paragraph_children(dom, document, container, !is_preformatted)?;
    }
    Some(())
}

fn contains_paragraph_lines(node: &Node) -> bool {
    node.as_element().is_some_and(|element| {
        element.namespace() == XHTML_NS
            && !element.has_attribute("is")
            && matches!(
                element.local_name(),
                "address"
                    | "article"
                    | "aside"
                    | "div"
                    | "footer"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "header"
                    | "hgroup"
                    | "main"
                    | "nav"
                    | "p"
                    | "pre"
                    | "section"
            )
    })
}

fn normalize_paragraph_children(
    dom: &mut DomHost,
    document: DomHandle,
    container: DomHandle,
    apply_pre_whitespace_style: bool,
) -> Option<()> {
    if !dom
        .child_handles(container)
        .any(|child| dom.node(child).is_some_and(contains_paragraph_lines))
    {
        return Some(());
    }
    let children = dom.child_nodes(container)?;
    let has_adjacent_lines = children
        .iter()
        .filter(|&&child| {
            dom.node(child)
                .is_some_and(|node| !between_paragraphs(node))
        })
        .nth(1)
        .is_some();
    let mut output = Vec::with_capacity(children.len());
    let mut has_line = false;
    let mut previous_paragraph = false;
    let mut ends_in_break = false;
    for &child in &children {
        let node = dom.node(child)?;
        if between_paragraphs(node) {
            output.push(child);
            continue;
        }
        let paragraph = contains_paragraph_lines(node);
        let child_is_break = node.is_html_element_named("br");
        if has_line && (paragraph || previous_paragraph) && !ends_in_break && !child_is_break {
            output.push(dom.create_element_without_attributes_for_document(
                document,
                "br".to_owned(),
                XHTML_NS.to_owned(),
                None,
            ));
        }
        ends_in_break = if paragraph {
            append_paragraph_contents(
                dom,
                document,
                child,
                has_adjacent_lines,
                apply_pre_whitespace_style,
                &mut output,
            )?
        } else {
            output.push(child);
            child_is_break
        };
        previous_paragraph = paragraph;
        has_line = true;
    }
    // All of these nodes are still in the detached parser fragment.
    for child in children {
        dom.remove_child_without_mutation_effects(container, child)
            .then_some(())?;
    }
    for child in output {
        dom.append_child_without_mutation_effects(container, child)
            .then_some(())?;
    }
    Some(())
}

fn between_paragraphs(node: &Node) -> bool {
    node.node_type() == NodeType::Comment
        || node
            .as_text()
            .is_some_and(|text| text.data().chars().all(|ch| ch.is_ascii_whitespace()))
}

fn append_paragraph_contents(
    dom: &mut DomHost,
    document: DomHandle,
    paragraph: DomHandle,
    has_adjacent_lines: bool,
    apply_pre_whitespace_style: bool,
    output: &mut Vec<DomHandle>,
) -> Option<bool> {
    let mut contents = dom.child_nodes(paragraph)?;
    let is_break = |handle| {
        dom.node(handle)
            .is_some_and(|node| node.is_html_element_named("br"))
    };
    if has_adjacent_lines
        && contents.len() == 1
        && dom
            .node(contents[0])
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_element("br") && element.attributes().is_empty())
    {
        // The lone break is the empty paragraph's caret placeholder. The
        // surrounding paragraph boundaries already account for the blank line.
        contents.clear();
    }
    let ends_in_break = contents.last().copied().is_some_and(is_break);
    let element = dom.node(paragraph)?.as_element()?;
    let pre_style = (apply_pre_whitespace_style && element.is_html_element("pre")).then(|| {
        // The destination's collapsing whitespace must not discard the text
        // that the source pre element kept verbatim. Author styles follow the
        // default so that their white-space declarations retain precedence.
        format!(
            "white-space: pre;{}",
            element.attribute("style").unwrap_or_default()
        )
    });
    let attributes = element.attributes();
    if attributes.is_empty() && pre_style.is_none() {
        output.extend(contents);
    } else {
        let attributes = attributes.to_vec();
        let span = dom.create_element_without_attributes_for_document(
            document,
            "span".to_owned(),
            XHTML_NS.to_owned(),
            None,
        );
        dom.add_attrs_if_missing_for_parser(span, attributes);
        if let Some(style) = pre_style {
            dom.set_attribute(span, "style", &style);
        }
        for child in contents {
            dom.append_child_without_mutation_effects(span, child)
                .then_some(())?;
        }
        output.push(span);
    }
    Some(ends_in_break)
}
