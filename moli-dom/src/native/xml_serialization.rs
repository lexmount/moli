use std::collections::{HashMap, HashSet};

use super::{DomHost, Element, NativeNodeId, NodeType};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";
const VOID_HTML: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "menuitem", "meta", "param", "source", "track", "wbr",
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

#[derive(Clone, Debug)]
struct NamespaceContext {
    default_namespace: String,
    // Candidate order survives rebinding, but only the current binding can
    // authorize reuse. Each element clones both maps to keep sibling scopes apart.
    prefixes: HashMap<String, Vec<String>>,
    bindings: HashMap<String, String>,
}

impl Default for NamespaceContext {
    fn default() -> Self {
        Self {
            default_namespace: String::new(),
            prefixes: HashMap::from([(XML_NAMESPACE.to_owned(), vec!["xml".to_owned()])]),
            bindings: HashMap::from([("xml".to_owned(), XML_NAMESPACE.to_owned())]),
        }
    }
}

impl NamespaceContext {
    fn add_prefix(&mut self, namespace: &str, prefix: &str) {
        self.bindings
            .insert(prefix.to_owned(), namespace.to_owned());
        self.prefixes
            .entry(namespace.to_owned())
            .or_default()
            .push(prefix.to_owned());
    }

    fn preferred_prefix(&self, namespace: &str, preferred: Option<&str>) -> Option<String> {
        let candidates = self.prefixes.get(namespace)?;
        if let Some(preferred) = preferred
            && self.contains_prefix(namespace, preferred)
        {
            return Some(preferred.to_owned());
        }
        candidates
            .iter()
            .rev()
            .find(|prefix| self.contains_prefix(namespace, prefix))
            .cloned()
    }

    fn contains_prefix(&self, namespace: &str, prefix: &str) -> bool {
        self.bindings
            .get(prefix)
            .is_some_and(|binding| binding == namespace)
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
        serialize(self, node, usize::MAX, false, false)
            .expect("unbounded XML output")
            .xml
    }

    pub fn serialize_xml_children(&self, node: NativeNodeId) -> Option<String> {
        let container = self
            .node(node)?
            .as_element()
            .and_then(Element::template_contents)
            .unwrap_or(node);
        Some(
            serialize(self, container, usize::MAX, false, true)
                .expect("unbounded XML output")
                .xml,
        )
    }

    /// Captures XML and every emitted element's source within a shared budget.
    /// No rendering policy or caller callback runs inside this traversal.
    pub fn xml_snapshot_with_limit(
        &self,
        node: NativeNodeId,
        max_bytes: usize,
    ) -> Result<XmlSnapshot, XmlSerializationLimitExceeded> {
        serialize(self, node, max_bytes, true, false)
    }
}

fn serialize(
    host: &DomHost,
    root: NativeNodeId,
    max_bytes: usize,
    record_sources: bool,
    children_only: bool,
) -> Result<XmlSnapshot, XmlSerializationLimitExceeded> {
    let mut out = Output {
        xml: String::new(),
        max_bytes,
        source_bytes: 0,
    };
    let mut elements = Vec::new();
    let mut next_prefix = 1;
    let mut stack = vec![if children_only {
        Frame::Children {
            next: host.dom().first_child(root),
            context: NamespaceContext::default(),
            parent: None,
        }
    } else {
        Frame::Node {
            node: root,
            context: NamespaceContext::default(),
            parent: None,
        }
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
                        let namespace = element.namespace();
                        out.push("<")?;
                        let tag = serialize_element_start(
                            element,
                            &mut context,
                            &mut next_prefix,
                            &mut out,
                        )?;
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

fn serialize_element_start(
    element: &Element,
    context: &mut NamespaceContext,
    next_generated_prefix: &mut usize,
    output: &mut Output,
) -> Result<String, XmlSerializationLimitExceeded> {
    let namespace = element.namespace();
    let original_prefix = element.prefix().filter(|prefix| !prefix.is_empty());
    let local_name = element.local_name();
    let mut namespace_context = context.clone();
    let (local_default_namespace, local_prefixes) =
        record_namespace_information(element, &mut namespace_context);
    // Declaration emission and prefix use are different: the element name and
    // earlier attributes must not have their bindings overwritten by a later
    // attribute, even when the prefix was originally inherited.
    let mut occupied_prefixes: HashSet<_> = local_prefixes.keys().cloned().collect();
    let mut inherited_namespace = context.default_namespace.clone();
    let mut ignore_namespace_definition_attribute = false;
    let mut serialized_attributes = Output {
        xml: String::new(),
        max_bytes: output
            .max_bytes
            .saturating_sub(output.xml.len())
            .saturating_sub(output.source_bytes),
        source_bytes: 0,
    };

    let tag = if inherited_namespace == namespace {
        if local_default_namespace.is_some() {
            ignore_namespace_definition_attribute = true;
        }
        if namespace == XML_NAMESPACE {
            occupied_prefixes.insert("xml".to_owned());
            format!("xml:{local_name}")
        } else {
            local_name.to_owned()
        }
    } else {
        let candidate_prefix = namespace_context.preferred_prefix(namespace, original_prefix);
        if let Some(candidate_prefix) = candidate_prefix {
            occupied_prefixes.insert(candidate_prefix.clone());
            if let Some(local_default_namespace) = local_default_namespace
                .as_deref()
                .filter(|namespace| *namespace != XML_NAMESPACE)
            {
                inherited_namespace = local_default_namespace.to_owned();
            }
            format!("{candidate_prefix}:{local_name}")
        } else if let Some(original_prefix) = original_prefix {
            let prefix = if local_prefixes.contains_key(original_prefix) {
                generate_namespace_prefix(&mut namespace_context, namespace, next_generated_prefix)
            } else {
                namespace_context.add_prefix(namespace, original_prefix);
                original_prefix.to_owned()
            };
            write_attribute(
                &mut serialized_attributes,
                &format!("xmlns:{prefix}"),
                namespace,
            )?;
            occupied_prefixes.insert(prefix.clone());
            if let Some(local_default_namespace) = local_default_namespace.as_deref() {
                inherited_namespace = local_default_namespace.to_owned();
            }
            format!("{prefix}:{local_name}")
        } else if local_default_namespace.as_deref() != Some(namespace) {
            ignore_namespace_definition_attribute = true;
            inherited_namespace = namespace.to_owned();
            write_attribute(&mut serialized_attributes, "xmlns", namespace)?;
            local_name.to_owned()
        } else {
            inherited_namespace = namespace.to_owned();
            local_name.to_owned()
        }
    };

    serialize_native_attributes(
        element,
        &mut namespace_context,
        next_generated_prefix,
        &local_prefixes,
        &mut occupied_prefixes,
        ignore_namespace_definition_attribute,
        &mut serialized_attributes,
    )?;
    namespace_context.default_namespace = inherited_namespace;
    output.push(&tag)?;
    output.push(&serialized_attributes.xml)?;
    *context = namespace_context;
    Ok(tag)
}

fn record_namespace_information(
    element: &Element,
    namespace_context: &mut NamespaceContext,
) -> (Option<String>, HashMap<String, String>) {
    let mut local_default_namespace = None;
    let mut local_prefixes = HashMap::new();
    for attribute in element.attributes() {
        if attribute.namespace() != XMLNS_NAMESPACE {
            continue;
        }
        // Only native XMLNS attributes declare namespaces; an ordinary
        // attribute spelled xmlns must not change descendant bindings.
        let Some(attribute_prefix) = attribute.prefix() else {
            local_default_namespace = Some(attribute.value().to_owned());
            continue;
        };
        if attribute_prefix.is_empty() {
            local_default_namespace = Some(attribute.value().to_owned());
            continue;
        }

        let prefix = attribute.local_name();
        let namespace = attribute.value();
        if namespace == XML_NAMESPACE || namespace_context.contains_prefix(namespace, prefix) {
            continue;
        }
        namespace_context.add_prefix(namespace, prefix);
        local_prefixes.insert(prefix.to_owned(), namespace.to_owned());
    }
    (local_default_namespace, local_prefixes)
}

fn serialize_native_attributes(
    element: &Element,
    namespace_context: &mut NamespaceContext,
    next_generated_prefix: &mut usize,
    local_prefixes: &HashMap<String, String>,
    occupied_prefixes: &mut HashSet<String>,
    ignore_namespace_definition_attribute: bool,
    output: &mut Output,
) -> Result<(), XmlSerializationLimitExceeded> {
    for attribute in element.attributes() {
        let attribute_namespace = attribute.namespace();
        let attribute_prefix = attribute.prefix().filter(|prefix| !prefix.is_empty());
        let mut candidate_prefix = (!attribute_namespace.is_empty())
            .then(|| namespace_context.preferred_prefix(attribute_namespace, attribute_prefix))
            .flatten();

        if attribute_namespace == XMLNS_NAMESPACE {
            if attribute.value() == XML_NAMESPACE
                || (attribute_prefix.is_none() && ignore_namespace_definition_attribute)
            {
                continue;
            }
            if attribute_prefix.is_some() {
                let local_namespace = local_prefixes.get(attribute.local_name());
                if local_namespace.is_none()
                    || (local_namespace.is_some_and(|namespace| namespace != attribute.value())
                        && namespace_context
                            .contains_prefix(attribute.value(), attribute.local_name()))
                {
                    continue;
                }
            }
            if attribute_prefix == Some("xmlns") {
                candidate_prefix = Some("xmlns".to_owned());
            }
        } else if !attribute_namespace.is_empty() && candidate_prefix.is_none() {
            let prefix = if let Some(prefix) =
                attribute_prefix.filter(|prefix| !occupied_prefixes.contains(*prefix))
            {
                namespace_context.add_prefix(attribute_namespace, prefix);
                prefix.to_owned()
            } else {
                generate_namespace_prefix(
                    namespace_context,
                    attribute_namespace,
                    next_generated_prefix,
                )
            };
            write_attribute(output, &format!("xmlns:{prefix}"), attribute_namespace)?;
            candidate_prefix = Some(prefix);
        }

        if attribute_namespace != XMLNS_NAMESPACE
            && let Some(prefix) = &candidate_prefix
        {
            occupied_prefixes.insert(prefix.clone());
        }

        let attribute_name = candidate_prefix
            .map(|prefix| format!("{prefix}:{}", attribute.local_name()))
            .unwrap_or_else(|| attribute.local_name().to_owned());
        write_attribute(output, &attribute_name, attribute.value())?;
    }
    Ok(())
}

fn generate_namespace_prefix(
    namespace_context: &mut NamespaceContext,
    namespace: &str,
    next_generated_prefix: &mut usize,
) -> String {
    let prefix = format!("ns{next_generated_prefix}");
    *next_generated_prefix += 1;
    namespace_context.add_prefix(namespace, &prefix);
    prefix
}

fn write_attribute(
    output: &mut Output,
    name: &str,
    value: &str,
) -> Result<(), XmlSerializationLimitExceeded> {
    output.attribute(name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;

    #[test]
    fn xml_namespace_declarations_are_unique_and_conflicting_prefixes_are_repaired() {
        for (prefix, authored_namespace) in [
            (None, "http://www.w3.org/2000/svg"),
            (None, "urn:conflict"),
            (Some(""), "http://www.w3.org/2000/svg"),
            (Some(""), "urn:conflict"),
        ] {
            let mut dom = NativeDom::new_html("https://example.com/".parse().unwrap());
            let svg = dom
                .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
                .unwrap();
            assert!(dom.set_attribute_ns(
                svg,
                Some(XMLNS_NAMESPACE),
                prefix,
                "xmlns",
                authored_namespace
            ));
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
