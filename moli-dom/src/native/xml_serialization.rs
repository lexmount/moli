use std::collections::BTreeMap;

use super::{DomHost, NativeNodeId, NodeType};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";
const VOID_HTML: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "meta", "param", "source", "track", "wbr",
];

/// The source of one emitted XML element. The parent is an index into the same
/// snapshot's element list, including elements in other namespaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmlElementSource {
    pub node: NativeNodeId,
    pub parent: Option<usize>,
}

/// XML and its source mapping, produced by the same traversal.
#[derive(Debug)]
pub struct XmlSnapshot {
    pub xml: String,
    pub elements: Vec<XmlElementSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmlSerializationLimitExceeded {
    pub max_bytes: usize,
}

impl std::fmt::Display for XmlSerializationLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "serialized XML snapshot exceeds the {}-byte output limit",
            self.max_bytes
        )
    }
}

impl std::error::Error for XmlSerializationLimitExceeded {}

#[derive(Clone)]
struct NamespaceContext {
    default_namespace: String,
    prefixes: BTreeMap<String, String>,
}

impl Default for NamespaceContext {
    fn default() -> Self {
        Self {
            default_namespace: String::new(),
            prefixes: BTreeMap::from([
                ("xml".to_owned(), XML_NAMESPACE.to_owned()),
                ("xmlns".to_owned(), XMLNS_NAMESPACE.to_owned()),
            ]),
        }
    }
}

impl NamespaceContext {
    fn prefix(
        &mut self,
        preferred: Option<&str>,
        namespace: &str,
        next: &mut usize,
        can_rebind: bool,
    ) -> (String, bool) {
        let preferred = preferred.filter(|prefix| !prefix.is_empty());
        if let Some(prefix) = preferred
            && self.prefixes.get(prefix).map(String::as_str) == Some(namespace)
        {
            return (prefix.to_owned(), false);
        }
        if let Some((prefix, _)) = self
            .prefixes
            .iter()
            .find(|(prefix, value)| prefix.as_str() != "xmlns" && value.as_str() == namespace)
        {
            return (prefix.clone(), false);
        }
        let prefix = match preferred {
            Some(prefix) if can_rebind || !self.prefixes.contains_key(prefix) => prefix.to_owned(),
            _ => loop {
                let prefix = format!("ns{next}");
                *next += 1;
                if !self.prefixes.contains_key(&prefix) {
                    break prefix;
                }
            },
        };
        self.prefixes.insert(prefix.clone(), namespace.to_owned());
        (prefix, true)
    }
}

struct Output {
    xml: String,
    max_bytes: usize,
    source_bytes: usize,
}

impl Output {
    fn push(&mut self, value: &str) -> Result<(), XmlSerializationLimitExceeded> {
        if value.len()
            > self
                .max_bytes
                .saturating_sub(self.xml.len())
                .saturating_sub(self.source_bytes)
        {
            return Err(XmlSerializationLimitExceeded {
                max_bytes: self.max_bytes,
            });
        }
        self.xml.push_str(value);
        Ok(())
    }

    fn escaped(
        &mut self,
        value: &str,
        attribute: bool,
    ) -> Result<(), XmlSerializationLimitExceeded> {
        for ch in value.chars() {
            match ch {
                '&' => self.push("&amp;")?,
                '<' => self.push("&lt;")?,
                '>' => self.push("&gt;")?,
                '"' if attribute => self.push("&quot;")?,
                '\t' if attribute => self.push("&#9;")?,
                '\n' if attribute => self.push("&#10;")?,
                '\r' if attribute => self.push("&#13;")?,
                _ => self.push(ch.encode_utf8(&mut [0; 4]))?,
            }
        }
        Ok(())
    }

    fn attribute(&mut self, name: &str, value: &str) -> Result<(), XmlSerializationLimitExceeded> {
        self.push(" ")?;
        self.push(name)?;
        self.push("=\"")?;
        self.escaped(value, true)?;
        self.push("\"")
    }

    fn source(&mut self) -> Result<(), XmlSerializationLimitExceeded> {
        // Include spare vector capacity in the snapshot's admission budget.
        let bytes = 2 * std::mem::size_of::<XmlElementSource>();
        if bytes
            > self
                .max_bytes
                .saturating_sub(self.xml.len())
                .saturating_sub(self.source_bytes)
        {
            return Err(XmlSerializationLimitExceeded {
                max_bytes: self.max_bytes,
            });
        }
        self.source_bytes += bytes;
        Ok(())
    }
}

enum Frame {
    Node {
        node: NativeNodeId,
        context: NamespaceContext,
        parent: Option<usize>,
    },
    Children {
        next: Option<NativeNodeId>,
        context: NamespaceContext,
        parent: Option<usize>,
    },
    Close(String),
}

impl DomHost {
    /// Serializes XML using the same namespace rules as a bounded snapshot.
    pub fn serialize_xml(&self, node: NativeNodeId) -> String {
        serialize(self, node, usize::MAX, false)
            .expect("unbounded XML output")
            .xml
    }

    /// Captures XML and every emitted element's source within a shared budget.
    /// No rendering policy or caller callback runs inside this traversal.
    pub fn xml_snapshot_with_limit(
        &self,
        node: NativeNodeId,
        max_bytes: usize,
    ) -> Result<XmlSnapshot, XmlSerializationLimitExceeded> {
        serialize(self, node, max_bytes, true)
    }
}

fn serialize(
    host: &DomHost,
    root: NativeNodeId,
    max_bytes: usize,
    record_sources: bool,
) -> Result<XmlSnapshot, XmlSerializationLimitExceeded> {
    let mut out = Output {
        xml: String::new(),
        max_bytes,
        source_bytes: 0,
    };
    let mut elements = Vec::new();
    let mut next_prefix = 1;
    let mut stack = vec![Frame::Node {
        node: root,
        context: NamespaceContext::default(),
        parent: None,
    }];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Close(tag) => {
                out.push("</")?;
                out.push(&tag)?;
                out.push(">")?;
            }
            Frame::Children {
                next: Some(node),
                context,
                parent,
            } => {
                if let Some(next) = host.dom().next_sibling(node) {
                    stack.push(Frame::Children {
                        next: Some(next),
                        context: context.clone(),
                        parent,
                    });
                }
                stack.push(Frame::Node {
                    node,
                    context,
                    parent,
                });
            }
            Frame::Children { next: None, .. } => {}
            Frame::Node {
                node: handle,
                mut context,
                parent,
            } => {
                let Some(node) = host.node(handle) else {
                    continue;
                };
                match node.node_type() {
                    NodeType::Element => {
                        let Some(element) = node.as_element() else {
                            continue;
                        };
                        // Blink also recognizes null-namespace `xmlns` attributes
                        // created by setAttribute in HTML documents. Treat them as
                        // declarations so the snapshot never duplicates xmlns.
                        for attribute in element.attributes() {
                            if attribute.prefix().is_none() && attribute.local_name() == "xmlns" {
                                context.default_namespace = attribute.value().to_owned();
                            } else if attribute.namespace() == XMLNS_NAMESPACE {
                                context.prefixes.insert(
                                    attribute.local_name().to_owned(),
                                    attribute.value().to_owned(),
                                );
                            } else if let Some(prefix) = attribute.name().strip_prefix("xmlns:") {
                                context
                                    .prefixes
                                    .insert(prefix.to_owned(), attribute.value().to_owned());
                            }
                        }
                        let namespace = element.namespace();
                        let (prefix, declaration) =
                            match element.prefix().filter(|prefix| !prefix.is_empty()) {
                                Some(prefix) => {
                                    let declaration_name = format!("xmlns:{prefix}");
                                    let local_declaration =
                                        element.attributes().iter().any(|attribute| {
                                            attribute.name() == declaration_name
                                                || (attribute.namespace() == XMLNS_NAMESPACE
                                                    && attribute.local_name() == prefix)
                                        });
                                    let (prefix, declaration) = context.prefix(
                                        Some(prefix),
                                        namespace,
                                        &mut next_prefix,
                                        !local_declaration,
                                    );
                                    (Some(prefix), declaration)
                                }
                                None => (None, false),
                            };
                        let tag = prefix
                            .as_ref()
                            .map(|prefix| format!("{prefix}:{}", element.local_name()))
                            .unwrap_or_else(|| element.local_name().to_owned());
                        out.push("<")?;
                        out.push(&tag)?;
                        if let Some(prefix) = &prefix {
                            if declaration {
                                out.attribute(&format!("xmlns:{prefix}"), namespace)?;
                            }
                        } else if context.default_namespace != namespace {
                            out.attribute("xmlns", namespace)?;
                            context.default_namespace = namespace.to_owned();
                        }
                        for attribute in element.attributes() {
                            if prefix.is_none()
                                && attribute.prefix().is_none()
                                && attribute.local_name() == "xmlns"
                                && attribute.value() != namespace
                            {
                                // Drop a declaration inconsistent with the element,
                                // rather than emitting two conflicting xmlns values.
                                continue;
                            }
                            let name = if attribute.namespace().is_empty() {
                                attribute.name()
                            } else if attribute.namespace() == XMLNS_NAMESPACE {
                                if attribute.prefix().is_none() && attribute.local_name() == "xmlns"
                                {
                                    "xmlns".to_owned()
                                } else {
                                    format!("xmlns:{}", attribute.local_name())
                                }
                            } else {
                                let (prefix, declaration) = context.prefix(
                                    attribute.prefix(),
                                    attribute.namespace(),
                                    &mut next_prefix,
                                    false,
                                );
                                if declaration {
                                    out.attribute(
                                        &format!("xmlns:{prefix}"),
                                        attribute.namespace(),
                                    )?;
                                }
                                format!("{prefix}:{}", attribute.local_name())
                            };
                            out.attribute(&name, attribute.value())?;
                        }
                        let source = if record_sources {
                            out.source()?;
                            let index = elements.len();
                            elements.push(XmlElementSource {
                                node: handle,
                                parent,
                            });
                            Some(index)
                        } else {
                            None
                        };
                        let children = element.template_contents().unwrap_or(handle);
                        let first = host.dom().first_child(children);
                        if first.is_none()
                            && namespace == HTML_NAMESPACE
                            && VOID_HTML.contains(&element.local_name())
                        {
                            out.push(" />")?;
                        } else if first.is_none() && namespace != HTML_NAMESPACE {
                            out.push("/>")?;
                        } else {
                            out.push(">")?;
                            stack.push(Frame::Close(tag));
                            stack.push(Frame::Children {
                                next: first,
                                context,
                                parent: source,
                            });
                        }
                    }
                    NodeType::Text => out.escaped(node.data_value().unwrap_or_default(), false)?,
                    NodeType::CDataSection => {
                        out.push("<![CDATA[")?;
                        out.push(node.data_value().unwrap_or_default())?;
                        out.push("]]>")?;
                    }
                    NodeType::Comment => {
                        out.push("<!--")?;
                        out.push(node.data_value().unwrap_or_default())?;
                        out.push("-->")?;
                    }
                    NodeType::ProcessingInstruction => {
                        out.push("<?")?;
                        out.push(node.target().unwrap_or_default())?;
                        let data = node.data_value().unwrap_or_default();
                        if !data.is_empty() {
                            out.push(" ")?;
                            out.push(data)?;
                        }
                        out.push("?>")?;
                    }
                    NodeType::Document | NodeType::DocumentFragment => {
                        stack.push(Frame::Children {
                            next: host.dom().first_child(handle),
                            context,
                            parent,
                        })
                    }
                    NodeType::DocumentType => {
                        let Some(doctype) = node.as_document_type() else {
                            continue;
                        };
                        out.push("<!DOCTYPE ")?;
                        out.push(doctype.name())?;
                        if !doctype.public_id().is_empty() {
                            out.push(" PUBLIC \"")?;
                            out.push(doctype.public_id())?;
                            out.push("\" \"")?;
                            out.push(doctype.system_id())?;
                            out.push("\"")?;
                        } else if !doctype.system_id().is_empty() {
                            out.push(" SYSTEM \"")?;
                            out.push(doctype.system_id())?;
                            out.push("\"")?;
                        }
                        out.push(">")?;
                    }
                }
            }
        }
    }
    Ok(XmlSnapshot {
        xml: out.xml,
        elements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;

    #[test]
    fn xml_namespace_declarations_are_unique_and_conflicting_prefixes_are_repaired() {
        for authored_namespace in ["http://www.w3.org/2000/svg", "urn:conflict"] {
            let mut dom = NativeDom::new_html("https://example.com/".parse().unwrap());
            let svg = dom
                .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
                .unwrap();
            assert!(dom.set_attribute(svg, "xmlns", authored_namespace));
            let host = DomHost::from_dom(dom);
            let snapshot = host.xml_snapshot_with_limit(svg, 1024).unwrap();
            assert_eq!(snapshot.xml, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
            assert_eq!(
                host.get_attribute(svg, "xmlns").as_deref(),
                Some(authored_namespace)
            );
        }
        let mut dom = NativeDom::new_html("https://example.com/".parse().unwrap());
        let svg = dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "p:svg")
            .unwrap();
        assert!(dom.set_attribute_ns(svg, Some(XMLNS_NAMESPACE), Some("xmlns"), "p", "urn:kept"));
        assert!(dom.set_attribute_ns(svg, Some("urn:attribute"), Some("p"), "value", "ok"));
        let host = DomHost::from_dom(dom);
        let snapshot = host.xml_snapshot_with_limit(svg, 1024).unwrap();
        assert_eq!(
            snapshot.xml,
            concat!(
                "<ns1:svg xmlns:ns1=\"http://www.w3.org/2000/svg\" xmlns:p=\"urn:kept\" ",
                "xmlns:ns2=\"urn:attribute\" ns2:value=\"ok\"/>"
            )
        );
        assert_eq!(
            host.node(svg).unwrap().as_element().unwrap().prefix(),
            Some("p")
        );
    }

    #[test]
    fn deep_xml_is_iterative_and_source_records_share_the_budget() {
        let mut dom = NativeDom::new_html("https://example.com/".parse().unwrap());
        let root = dom.create_element_ns(Some("urn:tree"), "p:root").unwrap();
        assert!(dom.set_attribute_ns(root, Some("urn:attribute"), None, "label", "a&b"));
        let mut parent = root;
        for _ in 0..2048 {
            let child = dom.create_element_ns(Some("urn:tree"), "p:child").unwrap();
            assert!(dom.append_child(parent, child));
            parent = child;
        }
        let host = DomHost::from_dom(dom);
        let xml = host.serialize_xml(root);
        assert!(xml.starts_with(
            "<p:root xmlns:p=\"urn:tree\" xmlns:ns1=\"urn:attribute\" ns1:label=\"a&amp;b\">"
        ));
        assert!(host.xml_snapshot_with_limit(root, xml.len()).is_err());
        let snapshot = host.xml_snapshot_with_limit(root, 1024 * 1024).unwrap();
        assert_eq!(snapshot.xml, xml);
        assert_eq!(snapshot.elements.len(), 2049);
        assert_eq!(snapshot.elements[2048].parent, Some(2047));
        assert_eq!(host.get_attribute(root, "ns1:label"), None);
    }

    #[test]
    fn xml_snapshot_tracks_sources_and_parents_including_foreign_namespaces() {
        let mut dom = NativeDom::new_html("https://example.com/".parse().unwrap());
        let svg = dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
            .unwrap();
        let group = dom
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "g")
            .unwrap();
        let child = dom.create_element("img");
        assert!(dom.append_child(svg, group));
        assert!(dom.append_child(group, child));
        assert!(dom.set_attribute(group, "data-value", "\"&\n\u{a0}"));
        let host = DomHost::from_dom(dom);
        let snapshot = host.xml_snapshot_with_limit(svg, 1024).unwrap();
        assert_eq!(
            snapshot.elements,
            vec![
                XmlElementSource {
                    node: svg,
                    parent: None
                },
                XmlElementSource {
                    node: group,
                    parent: Some(0)
                },
                XmlElementSource {
                    node: child,
                    parent: Some(1)
                }
            ]
        );
        assert!(
            snapshot
                .xml
                .contains("xmlns=\"http://www.w3.org/2000/svg\"")
        );
        assert!(
            snapshot
                .xml
                .contains("xmlns=\"http://www.w3.org/1999/xhtml\"")
        );
        assert!(
            snapshot
                .xml
                .contains("data-value=\"&quot;&amp;&#10;\u{a0}\"")
        );
        assert!(!snapshot.xml.contains("&nbsp;"));
        assert_eq!(host.serialize_xml(svg), snapshot.xml);
        let budget = snapshot.xml.len()
            + snapshot.elements.len() * 2 * std::mem::size_of::<XmlElementSource>();
        assert!(host.xml_snapshot_with_limit(svg, budget).is_ok());
        assert_eq!(
            host.xml_snapshot_with_limit(svg, budget - 1)
                .unwrap_err()
                .max_bytes,
            budget - 1
        );
    }
}
