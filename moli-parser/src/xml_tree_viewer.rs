use std::collections::{HashMap, HashSet};

use moli_dom::native::{DomHost, NativeDom, NativeNodeId, Node, NodeData, NodeType};
use xmlparser::{ElementEnd, Token, Tokenizer};

use crate::live_target::ParserStreamHtmlTreeSinkTarget;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const MATHML_NAMESPACE: &str = "http://www.w3.org/1998/Math/MathML";
const SOURCE_CONTAINER_ID: &str = "webkit-xml-viewer-source-xml";
const DEFAULT_HEADER: &str = "This XML file does not appear to have any style information associated with it. The document tree is shown below.";
const XSLT_HEADER: &str = "This document cannot be formatted as intended. It uses XSLT, which the browser does not support. You might be able to install a browser extension that allows you to view it.";
const VIEWER_STYLE: &str = concat!(
    "#webkit-xml-viewer-source-xml { display: none; }\n",
    ".folder > .hidden { display: none; }\n",
    ".pretty-print { white-space: pre; font-family: monospace; }\n",
    ".opened { margin-left: 1em; }\n",
);

pub(super) fn transform_document_to_xml_tree_view(document: NativeDom) -> NativeDom {
    if !should_transform(&document) {
        return document;
    }

    let mut host = DomHost::from_dom(document);
    let document = host.document_handle();
    let source_children = host
        .child_handles(document)
        .filter(|handle| {
            host.node(*handle)
                .is_some_and(|node| node.node_type() != NodeType::DocumentType)
        })
        .collect::<Vec<_>>();
    let all_document_children = host.child_handles(document).collect::<Vec<_>>();

    let html = create_html_element(&mut host, "html");
    let head = create_html_element(&mut host, "head");
    let style = create_html_element(&mut host, "style");
    set_attribute(&mut host, style, "id", "xml-viewer-style");
    append_text(&mut host, style, VIEWER_STYLE);
    append(&mut host, head, style);
    append(&mut host, html, head);

    let body = create_html_element(&mut host, "body");
    let source_container = create_html_element(&mut host, "div");
    set_attribute(&mut host, source_container, "id", SOURCE_CONTAINER_ID);
    append(&mut host, body, source_container);

    let header = create_html_element(&mut host, "div");
    set_attribute(&mut host, header, "class", "header");
    let header_span = create_html_element(&mut host, "span");
    let header_text = if first_rendered_source_node_is_xml_stylesheet(&host, &source_children) {
        XSLT_HEADER
    } else {
        DEFAULT_HEADER
    };
    append_text(&mut host, header_span, header_text);
    append(&mut host, header, header_span);
    let line_break = create_html_element(&mut host, "br");
    append(&mut host, header, line_break);
    append(&mut host, body, header);

    let pretty_print = create_html_element(&mut host, "div");
    set_attribute(&mut host, pretty_print, "class", "pretty-print");
    build_pretty_print(&mut host, pretty_print, &source_children);
    append(&mut host, body, pretty_print);
    append(&mut host, html, body);

    for child in all_document_children {
        if !host.remove_child(document, child) {
            continue;
        }
        if host
            .node(child)
            .is_some_and(|node| node.node_type() != NodeType::DocumentType)
        {
            append(&mut host, source_container, child);
        }
    }
    append(&mut host, document, html);
    host.into_dom()
}

/// Applies the XML viewer conversion to the parser's current DOM owner.
///
/// The snapshot is used only to decide and plan the Chromium-compatible tree
/// shape. Every structural change is replayed through the parser target, so a
/// bootstrapped renderer keeps one live `Document`, stable node identities for
/// the source XML subtree, and the normal mutation/runtime bookkeeping.
pub(super) fn transform_parser_target_to_xml_tree_view(
    target: &mut ParserStreamHtmlTreeSinkTarget,
) {
    let Some(source_document) = target.snapshot_current_parser_document() else {
        return;
    };
    if !should_transform(&source_document) {
        return;
    }

    let parser_document = target.parser_root_node_id();
    if source_document.document_node_id() != parser_document {
        return;
    }
    let original_handles = source_document
        .nodes()
        .map(Node::id)
        .collect::<HashSet<_>>();
    let original_document_children = source_document
        .child_ids(parser_document)
        .collect::<Vec<_>>();
    let transformed_document = transform_document_to_xml_tree_view(source_document);
    let desired_document_children = transformed_document
        .child_ids(transformed_document.document_node_id())
        .collect::<Vec<_>>();

    for child in original_document_children {
        target.remove_existing_node(parser_document, child);
    }

    let mut materialized_handles = HashMap::new();
    for desired_child in desired_document_children {
        let child = materialize_transformed_viewer_node(
            target,
            &transformed_document,
            &original_handles,
            &mut materialized_handles,
            desired_child,
        );
        target.append_existing_node(parser_document, child);
    }
}

fn materialize_transformed_viewer_node(
    target: &mut ParserStreamHtmlTreeSinkTarget,
    transformed: &NativeDom,
    original_handles: &HashSet<NativeNodeId>,
    materialized_handles: &mut HashMap<NativeNodeId, NativeNodeId>,
    source_handle: NativeNodeId,
) -> NativeNodeId {
    if original_handles.contains(&source_handle) {
        return source_handle;
    }
    if let Some(handle) = materialized_handles.get(&source_handle) {
        return *handle;
    }

    let data = transformed
        .node(source_handle)
        .unwrap_or_else(|| panic!("XML viewer plan references missing node {source_handle:?}"))
        .data()
        .clone();
    let materialized = match data {
        NodeData::Element(element) => {
            let handle = target.create_element_without_attributes(
                element.local_name().to_owned(),
                element.namespace().to_owned(),
                element.prefix().map(str::to_owned),
            );
            target.add_attrs_if_missing_for_parser(handle, element.attributes().to_vec());
            handle
        }
        NodeData::Text(text) => target.create_text_node_for_document(
            target.parser_owner_document_node_id(),
            text.data().to_owned(),
        ),
        NodeData::CDataSection(cdata) => target.create_cdata_section_node(cdata.data().to_owned()),
        NodeData::Comment(comment) => target.create_comment_node(comment.data().to_owned()),
        NodeData::ProcessingInstruction(instruction) => target.create_processing_instruction_node(
            instruction.target().to_owned(),
            instruction.data().to_owned(),
        ),
        NodeData::DocumentType(document_type) => target.create_document_type_node(
            document_type.name().to_owned(),
            document_type.public_id().to_owned(),
            document_type.system_id().to_owned(),
        ),
        NodeData::Document(_) | NodeData::DocumentFragment(_) => {
            panic!("XML viewer plan cannot materialize a document container node")
        }
    };
    materialized_handles.insert(source_handle, materialized);

    let children = transformed.child_ids(source_handle).collect::<Vec<_>>();
    for child in children {
        let child = materialize_transformed_viewer_node(
            target,
            transformed,
            original_handles,
            materialized_handles,
            child,
        );
        target.append_existing_node(materialized, child);
    }
    materialized
}

fn should_transform(document: &NativeDom) -> bool {
    if !document.parse_errors().is_empty() || document.document_element_handle().is_none() {
        return false;
    }

    if document.nodes().any(|node| {
        node.as_element().is_some_and(|element| {
            matches!(
                element.namespace(),
                HTML_NAMESPACE | SVG_NAMESPACE | MATHML_NAMESPACE
            )
        })
    }) {
        return false;
    }

    !document
        .child_ids(document.document_node_id())
        .any(|handle| {
            document
                .node(handle)
                .and_then(|node| node.as_processing_instruction())
                .is_some_and(|instruction| {
                    instruction.target() == "xml-stylesheet"
                        && processing_instruction_is_css(instruction.data())
                })
        })
}

fn processing_instruction_is_css(data: &str) -> bool {
    let Some(attributes) = parse_processing_instruction_attributes(data) else {
        return false;
    };
    attributes
        .iter()
        .find(|(name, _)| name == "type")
        .is_none_or(|(_, value)| value.is_empty() || value == "text/css")
}

fn parse_processing_instruction_attributes(data: &str) -> Option<Vec<(String, String)>> {
    let wrapped = format!("<xml-stylesheet {data}/>");
    let mut attributes = Vec::new();
    let mut saw_start = false;
    let mut saw_end = false;
    for token in Tokenizer::from(wrapped.as_str()) {
        match token.ok()? {
            Token::ElementStart { prefix, local, .. }
                if prefix.as_str().is_empty() && local.as_str() == "xml-stylesheet" =>
            {
                saw_start = true;
            }
            Token::Attribute {
                prefix,
                local,
                value,
                ..
            } if prefix.as_str().is_empty() => {
                attributes.push((local.as_str().to_owned(), value.as_str().to_owned()));
            }
            Token::ElementEnd {
                end: ElementEnd::Empty,
                ..
            } => saw_end = true,
            _ => return None,
        }
    }
    (saw_start && saw_end).then_some(attributes)
}

fn first_rendered_source_node_is_xml_stylesheet(
    host: &DomHost,
    source_children: &[NativeNodeId],
) -> bool {
    source_children.first().is_some_and(|handle| {
        host.node(*handle)
            .and_then(|node| node.as_processing_instruction())
            .is_some_and(|instruction| instruction.target() == "xml-stylesheet")
    })
}

enum PrettyTask {
    Node {
        parent: NativeNodeId,
        source: NativeNodeId,
    },
}

fn build_pretty_print(
    host: &mut DomHost,
    pretty_print: NativeNodeId,
    source_children: &[NativeNodeId],
) {
    let mut folder_index = 0usize;
    let mut tasks = source_children
        .iter()
        .rev()
        .copied()
        .map(|source| PrettyTask::Node {
            parent: pretty_print,
            source,
        })
        .collect::<Vec<_>>();
    while let Some(PrettyTask::Node { parent, source }) = tasks.pop() {
        let Some(node) = host.node(source) else {
            continue;
        };
        match node.data() {
            NodeData::Element(element) => {
                let name = element.node_name();
                let attributes = element
                    .attributes()
                    .iter()
                    .map(|attribute| (attribute.name(), attribute.value().to_owned()))
                    .collect::<Vec<_>>();
                let children = host.child_handles(source).collect::<Vec<_>>();
                if children.is_empty() {
                    let line = create_line(host);
                    append_tag(host, line, &name, &attributes, false, true);
                    append(host, parent, line);
                    continue;
                }
                if children.len() == 1
                    && host
                        .node(children[0])
                        .is_some_and(|child| child.node_type() == NodeType::Text)
                {
                    let text = host
                        .node(children[0])
                        .and_then(|child| child.node_value())
                        .unwrap_or_default()
                        .to_owned();
                    let line = create_line(host);
                    append_tag(host, line, &name, &attributes, false, false);
                    let text_span = create_text_span(host, &text);
                    append(host, line, text_span);
                    append_tag(host, line, &name, &[], true, false);
                    append(host, parent, line);
                    continue;
                }

                let folder = create_html_element(host, "div");
                set_attribute(host, folder, "class", "folder");
                set_attribute(host, folder, "id", &format!("folder{folder_index}"));
                folder_index += 1;

                let start = create_line(host);
                let button = create_html_element(host, "span");
                set_attribute(host, button, "class", "folder-button fold");
                append(host, start, button);
                append_tag(host, start, &name, &attributes, false, false);
                append(host, folder, start);

                let opened = create_html_element(host, "div");
                set_attribute(host, opened, "class", "opened");
                append(host, folder, opened);

                let folded = create_text_span(host, "...");
                set_attribute(host, folded, "class", "folded hidden");
                append(host, folder, folded);

                let end = create_line(host);
                append_tag(host, end, &name, &[], true, false);
                append(host, folder, end);
                append(host, parent, folder);

                tasks.extend(children.into_iter().rev().map(|source| PrettyTask::Node {
                    parent: opened,
                    source,
                }));
            }
            NodeData::Text(text) => {
                let text = text.data().to_owned();
                let span = create_text_span(host, &text);
                append(host, parent, span);
            }
            NodeData::CDataSection(cdata) => {
                let cdata = cdata.data().to_owned();
                let line = create_line(host);
                let span = create_text_span(host, &format!("<![CDATA[ {cdata} ]]>"));
                append(host, line, span);
                append(host, parent, line);
            }
            NodeData::Comment(comment) => {
                let comment = comment.data().to_owned();
                let line = create_line(host);
                let span = create_html_element(host, "span");
                set_attribute(host, span, "class", "comment html-comment");
                append_text(host, span, &format!("<!-- {comment} -->"));
                append(host, line, span);
                append(host, parent, line);
            }
            NodeData::ProcessingInstruction(instruction) => {
                let target = instruction.target().to_owned();
                let data = instruction.data().to_owned();
                let line = create_line(host);
                let span = create_html_element(host, "span");
                set_attribute(host, span, "class", "comment html-comment");
                append_text(host, span, &format!("<?{target} {data}?>"));
                append(host, line, span);
                append(host, parent, line);
            }
            NodeData::Document(_) | NodeData::DocumentType(_) | NodeData::DocumentFragment(_) => {}
        }
    }
}

fn append_tag(
    host: &mut DomHost,
    parent: NativeNodeId,
    name: &str,
    attributes: &[(String, String)],
    closing: bool,
    empty: bool,
) {
    let tag = create_html_element(host, "span");
    set_attribute(host, tag, "class", "html-tag");
    append_text(
        host,
        tag,
        &format!("<{}{name}", if closing { "/" } else { "" }),
    );
    if !closing {
        for (name, value) in attributes {
            let attribute = create_html_element(host, "span");
            set_attribute(host, attribute, "class", "html-attribute");
            append_text(host, attribute, " ");
            let attribute_name = create_html_element(host, "span");
            set_attribute(host, attribute_name, "class", "html-attribute-name");
            append_text(host, attribute_name, name);
            append(host, attribute, attribute_name);
            append_text(host, attribute, "=\"");
            let attribute_value = create_html_element(host, "span");
            set_attribute(host, attribute_value, "class", "html-attribute-value");
            append_text(host, attribute_value, value);
            append(host, attribute, attribute_value);
            append_text(host, attribute, "\"");
            append(host, tag, attribute);
        }
    }
    append_text(host, tag, if empty { "/>" } else { ">" });
    append(host, parent, tag);
}

fn create_line(host: &mut DomHost) -> NativeNodeId {
    let line = create_html_element(host, "div");
    set_attribute(host, line, "class", "line");
    line
}

fn create_text_span(host: &mut DomHost, value: &str) -> NativeNodeId {
    let span = create_html_element(host, "span");
    append_text(host, span, value);
    span
}

fn create_html_element(host: &mut DomHost, name: &str) -> NativeNodeId {
    host.create_element_ns(Some(HTML_NAMESPACE), name)
        .expect("static XML viewer HTML qualified name must be valid")
}

fn set_attribute(host: &mut DomHost, element: NativeNodeId, name: &str, value: &str) {
    let _ = host.set_attribute(element, name, value);
}

fn append_text(host: &mut DomHost, parent: NativeNodeId, text: &str) {
    let text = host.create_text_node(text);
    append(host, parent, text);
}

fn append(host: &mut DomHost, parent: NativeNodeId, child: NativeNodeId) {
    assert!(host.append_child(parent, child));
}

#[cfg(test)]
mod tests {
    use super::{HTML_NAMESPACE, SOURCE_CONTAINER_ID};
    use crate::XmlParser;
    use moli_dom::native::{NativeDom, NativeNodeId, Node};
    use url::Url;

    fn parse_top_level(source: &str) -> NativeDom {
        XmlParser.parse_top_level_document(
            Url::parse("https://example.test/document.xml").unwrap(),
            source.to_owned(),
        )
    }

    fn document_element(document: &NativeDom) -> NativeNodeId {
        document
            .document_element_handle()
            .expect("document element")
    }

    fn find_element(document: &NativeDom, predicate: impl Fn(&Node) -> bool) -> NativeNodeId {
        document
            .nodes()
            .find(|node| node.is_element() && predicate(node))
            .map(Node::id)
            .expect("matching element")
    }

    #[test]
    fn top_level_unstyled_xml_uses_chromium_viewer_shape_and_preserves_source_nodes() {
        let document = parse_top_level(concat!(
            "<?xml version='1.0'?>",
            "<!DOCTYPE semantic-root>",
            "<semantic-root attr='value'><semantic-child>xml-ready</semantic-child></semantic-root>"
        ));
        let root = document_element(&document);
        let root_element = document.node(root).and_then(Node::as_element).unwrap();
        assert_eq!(root_element.local_name(), "html");
        assert_eq!(root_element.namespace(), HTML_NAMESPACE);
        assert!(
            document
                .node(document.document_node_id())
                .and_then(Node::as_document)
                .is_some_and(|document| !document.is_html_document())
        );
        assert!(
            document
                .child_ids(document.document_node_id())
                .all(|handle| document
                    .node(handle)
                    .is_none_or(|node| node.as_document_type().is_none()))
        );

        let source_container = find_element(&document, |node| {
            node.as_element()
                .and_then(|element| element.attribute("id"))
                == Some(SOURCE_CONTAINER_ID)
        });
        let source_root = document
            .child_ids(source_container)
            .find(|handle| document.node(*handle).is_some_and(Node::is_element))
            .expect("source root");
        assert_eq!(
            document
                .node(source_root)
                .and_then(Node::as_element)
                .map(|element| (element.local_name(), element.namespace())),
            Some(("semantic-root", ""))
        );
        assert_eq!(
            document
                .node(source_root)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("attr")),
            Some("value")
        );

        let pretty_print = find_element(&document, |node| {
            node.as_element()
                .and_then(|element| element.attribute("class"))
                == Some("pretty-print")
        });
        let pretty_text = document
            .node(pretty_print)
            .map(|node| node.text_content(&document))
            .unwrap();
        assert!(pretty_text.contains("<semantic-root attr=\"value\">"));
        assert!(pretty_text.contains("xml-ready"));
    }

    #[test]
    fn raw_xml_parser_keeps_the_source_root() {
        let document = XmlParser.parse(
            Url::parse("https://example.test/document.xml").unwrap(),
            "<semantic-root/>".to_owned(),
        );
        assert_eq!(
            document
                .node(document_element(&document))
                .and_then(Node::as_element)
                .map(|element| element.local_name()),
            Some("semantic-root")
        );
    }

    #[test]
    fn top_level_viewer_accepts_chromium_xml_source_boundaries() {
        for source in [
            "\u{feff}<root/>",
            "<?xml version='1.0' encoding='UTF-8'?><root/>",
            "<?xml version='1.0' encoding='UTF-16'?><root/>",
            "<?xml version='1.1'?><root/>",
            "<p:root xmlns:p='urn:example'/>",
        ] {
            let document = parse_top_level(source);
            assert!(
                document.nodes().any(|node| {
                    node.as_element()
                        .and_then(|element| element.attribute("id"))
                        == Some(SOURCE_CONTAINER_ID)
                }),
                "missing viewer for Chromium-valid source {source:?}; parse errors: {:?}",
                document.parse_errors()
            );
        }
    }

    #[test]
    fn unclosed_top_level_xml_is_recorded_in_primary_parser_errors() {
        let document = parse_top_level("<root>");

        assert!(
            document
                .parse_errors()
                .iter()
                .any(|error| error.contains("unclosed XML element")),
            "parse errors: {:?}",
            document.parse_errors()
        );
        assert!(document.nodes().all(|node| {
            node.as_element()
                .and_then(|element| element.attribute("id"))
                != Some(SOURCE_CONTAINER_ID)
        }));
    }

    #[test]
    fn top_level_viewer_eligibility_matches_chromium_boundaries() {
        for source in [
            "<html xmlns='http://www.w3.org/1999/xhtml'><body/></html>",
            "<svg xmlns='http://www.w3.org/2000/svg'/>",
            "<math xmlns='http://www.w3.org/1998/Math/MathML'/>",
            "<?xml-stylesheet href='style.css'?><root/>",
            "<?xml-stylesheet type='text/css' href='style.css'?><root/>",
            "<root>",
        ] {
            let document = parse_top_level(source);
            let root = document.node(document_element(&document)).unwrap();
            assert!(
                root.as_element()
                    .is_none_or(|element| element.attribute("id") != Some(SOURCE_CONTAINER_ID))
            );
            assert!(
                document.nodes().all(|node| {
                    node.as_element()
                        .and_then(|element| element.attribute("id"))
                        != Some(SOURCE_CONTAINER_ID)
                }),
                "unexpected viewer for {source:?}"
            );
        }

        for source in [
            "<root/>",
            "<root xmlns='urn:example'/>",
            "<?xml-stylesheet type='application/json' href='data.json'?><root/>",
            "<?xml-stylesheet alternate href='style.css'?><root/>",
            "<root><?xml-stylesheet type='text/css' href='style.css'?></root>",
        ] {
            let document = parse_top_level(source);
            assert!(
                document.nodes().any(|node| {
                    node.as_element()
                        .and_then(|element| element.attribute("id"))
                        == Some(SOURCE_CONTAINER_ID)
                }),
                "missing viewer for {source:?}"
            );
        }
    }
}
