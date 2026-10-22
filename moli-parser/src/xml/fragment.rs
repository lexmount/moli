use std::{collections::BTreeMap, error::Error, fmt};

use moli_dom::native::{DomHost, DomStringValue, NativeDom, NativeNodeId, Node};

use super::{XML_NAMESPACE, XMLNS_NAMESPACE, XmlParser};

#[derive(Debug)]
pub struct XmlFragmentParseError(String);

impl fmt::Display for XmlFragmentParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for XmlFragmentParseError {}

/// A validated XML fragment. The context wrapper is private and is never
/// imported into the receiving Document.
#[derive(Debug)]
pub struct ParsedXmlFragment {
    document: NativeDom,
    children: Vec<NativeNodeId>,
}

impl ParsedXmlFragment {
    pub fn document(&self) -> &NativeDom {
        &self.document
    }

    pub fn children(&self) -> &[NativeNodeId] {
        &self.children
    }
}

impl XmlParser {
    /// Parse an XML fragment without mutating its context Document. The XML
    /// Standard's context wrapper carries namespace bindings, never its DTD.
    pub fn parse_fragment(
        &self,
        host: &DomHost,
        context: NativeNodeId,
        markup: &DomStringValue,
    ) -> Result<ParsedXmlFragment, XmlFragmentParseError> {
        let element = host
            .node(context)
            .and_then(Node::as_element)
            .ok_or_else(|| XmlFragmentParseError("XML context must be an Element.".into()))?;
        let document = host
            .owner_document_handle(context)
            .and_then(|document| host.node(document))
            .and_then(Node::as_document)
            .ok_or_else(|| XmlFragmentParseError("XML context has no Document.".into()))?;
        let markup = markup.as_str().ok_or_else(|| {
            XmlFragmentParseError("XML markup contains an unpaired surrogate.".into())
        })?;
        let name = match element.prefix().filter(|prefix| !prefix.is_empty()) {
            Some(prefix) => format!("{prefix}:{}", element.local_name()),
            None => element.local_name().to_owned(),
        };
        let mut input = String::new();
        input.push('<');
        input.push_str(&name);
        for (prefix, value) in namespaces_in_scope(host, context) {
            let value = value.as_str().ok_or_else(|| {
                XmlFragmentParseError("XML namespace contains an unpaired surrogate.".into())
            })?;
            if prefix.is_some() && value.is_empty() {
                // An empty declaration masks ancestors, but lookupNamespaceURI
                // returns null, so this prefix is not declared in the wrapper.
                continue;
            }
            input.push_str(" xmlns");
            if let Some(prefix) = prefix {
                input.push(':');
                input.push_str(&prefix);
            }
            input.push_str("=\"");
            append_namespace_value(&mut input, value);
            input.push('"');
        }
        input.push('>');
        input.push_str(markup);
        input.push_str("</");
        input.push_str(&name);
        input.push('>');

        let parsed = self.parse(document.url().clone(), input);
        if let Some(error) = parsed.parse_errors().first() {
            return Err(XmlFragmentParseError(error.clone()));
        }
        let mut parsed = DomHost::from_dom(parsed);
        let root = {
            let mut document_children = parsed.child_handles(parsed.document_handle());
            let root = document_children
                .next()
                .filter(|root| parsed.node(*root).is_some_and(Node::is_element))
                .ok_or_else(|| {
                    XmlFragmentParseError("XML fragment has no context element.".into())
                })?;
            // A context name containing "--" can let a trailing comment swallow
            // the generated end tag. XML well-formedness alone does not catch it.
            if document_children.next().is_some() {
                return Err(XmlFragmentParseError(
                    "XML fragment escapes its context element.".into(),
                ));
            }
            root
        };
        let content = parsed
            .node(root)
            .and_then(Node::as_element)
            .and_then(|element| element.template_contents())
            .unwrap_or(root);
        let children = parsed.child_handles(content).collect();
        let scripts = parsed
            .nodes()
            .iter()
            .filter(|node| node.is_script_element())
            .map(Node::id)
            .collect::<Vec<_>>();
        for script in scripts {
            // XML fragments stay inert even when subsequently adopted into
            // an HTML Document, including scripts inside template contents.
            parsed.set_script_already_started(script, true);
        }
        Ok(ParsedXmlFragment {
            document: parsed.into_dom(),
            children,
        })
    }
}

fn namespaces_in_scope(
    host: &DomHost,
    context: NativeNodeId,
) -> BTreeMap<Option<String>, DomStringValue> {
    let mut namespaces = BTreeMap::from([(Some("xml".into()), XML_NAMESPACE.into())]);
    let mut current = Some(context);
    while let Some(node) = current.and_then(|handle| host.node(handle)) {
        let Some(element) = node.as_element() else {
            break;
        };
        if !element.namespace().is_empty() {
            namespaces
                .entry(
                    element
                        .prefix()
                        .filter(|prefix| !prefix.is_empty())
                        .map(str::to_owned),
                )
                .or_insert_with(|| element.namespace().into());
        }
        for attribute in element.attributes() {
            if attribute.namespace() != XMLNS_NAMESPACE {
                continue;
            }
            let prefix = match attribute.prefix().filter(|prefix| !prefix.is_empty()) {
                Some("xmlns") if attribute.local_name() != "xmlns" => {
                    Some(attribute.local_name().to_owned())
                }
                None if attribute.local_name() == "xmlns" => None,
                _ => continue,
            };
            // Keep an empty declaration: it masks an ancestor binding.
            namespaces.entry(prefix).or_insert_with(|| {
                element
                    .attribute_ns_utf16_units(XMLNS_NAMESPACE, attribute.local_name())
                    .map(DomStringValue::from_utf16)
                    .unwrap_or_else(|| attribute.value().into())
            });
        }
        current = node.parent_node();
    }
    namespaces
}

fn append_namespace_value(output: &mut String, value: &str) {
    for character in value.chars() {
        output.push_str(match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\t' => "&#9;",
            '\n' => "&#10;",
            '\r' => "&#13;",
            _ => {
                output.push(character);
                continue;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_dom::native::NodeType;
    use url::Url;

    fn context(source: &str) -> (DomHost, NativeNodeId) {
        let document = XmlParser.parse(
            Url::parse("https://xml-fragment.test/context.xml").unwrap(),
            source.into(),
        );
        assert!(
            document.parse_errors().is_empty(),
            "{:?}",
            document.parse_errors()
        );
        let host = DomHost::from_dom(document);
        let context = host
            .nodes()
            .iter()
            .find(|node| node.local_name() == Some("Context"))
            .map(Node::id)
            .unwrap_or_else(|| host.dom().document_element_node_id().unwrap());
        (host, context)
    }

    #[test]
    fn xml_fragments_preserve_namespaces_case_and_xml_node_kinds() {
        let (host, context) = context(
            "<Root xmlns='urn:outer'><p:Parent xmlns:p='urn:local'><Context/></p:Parent></Root>",
        );
        let parsed = XmlParser
            .parse_fragment(
                &host,
                context,
                &"<Mixed p:Token='v'/><p:Item/>tail<![CDATA[a<b]]><!--end--><?target data?>".into(),
            )
            .unwrap();
        let nodes = parsed
            .children()
            .iter()
            .map(|handle| parsed.document().node(*handle).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.node_type())
                .collect::<Vec<_>>(),
            [
                NodeType::Element,
                NodeType::Element,
                NodeType::Text,
                NodeType::CDataSection,
                NodeType::Comment,
                NodeType::ProcessingInstruction
            ]
        );
        assert_eq!(nodes[0].local_name(), Some("Mixed"));
        assert_eq!(nodes[0].namespace(), Some("urn:outer"));
        assert_eq!(
            nodes[0]
                .as_element()
                .unwrap()
                .attribute_ns("urn:local", "Token"),
            Some("v")
        );
        assert_eq!(nodes[1].namespace(), Some("urn:local"));
        assert_eq!(nodes[1].prefix(), Some("p"));
        assert_eq!(nodes[3].node_value(), Some("a<b"));
    }

    #[test]
    fn xml_fragments_reject_grammar_namespace_and_context_escape_errors() {
        let (host, context) = context("<Context xmlns:p='urn:p'/>");
        let failures = [
            "<Item>",
            "<A></B>",
            "&unknown;",
            "<Item x='1' x='2'/>",
            "<Item xmlns:q='urn:p' p:x='1' q:x='2'/>",
            "<Item x=value/>",
            "<?xml version='1.0'?>",
            "<!DOCTYPE Item><Item/>",
            "\0",
            "\u{b}",
            "\u{fffe}",
            "&#xD800;",
            "</Context><!--escape--><Context>",
            "<missing:Item/>",
            "<p:Item xmlns:p=''/>",
            "<Item xmlns:xml='urn:wrong'/>",
            "<Item xmlns:xmlns='urn:wrong'/>",
            "<Item xmlns:p=''/>",
            "<Item xmlns:q='http://www.w3.org/XML/1998/namespace'/>",
            "<Item xmlns='http://www.w3.org/XML/1998/namespace'/>",
            "<Item xmlns:q='http://www.w3.org/2000/xmlns/'/>",
            "<Item xmlns='http://www.w3.org/2000/xmlns/'/>",
        ]
        .into_iter()
        .filter(|markup| {
            XmlParser
                .parse_fragment(&host, context, &(*markup).into())
                .is_ok()
        })
        .collect::<Vec<_>>();
        assert!(failures.is_empty(), "accepted malformed XML: {failures:?}");
        for units in [vec![0xd800], vec![0xdc00]] {
            assert!(
                XmlParser
                    .parse_fragment(&host, context, &DomStringValue::from_utf16(&units))
                    .is_err()
            );
        }
        let (host, context) = self::context("<foo--/>");
        assert!(
            XmlParser
                .parse_fragment(&host, context, &"x</foo--><!--y".into())
                .is_err()
        );
    }

    #[test]
    fn xml_fragments_escape_and_mask_native_namespace_declarations() {
        let (mut host, context) = context(
            "<Root xmlns='urn:outer' xmlns:p='urn:parent'><Context xmlns='' xmlns:p='urn:a&amp;b&quot;c&lt;d&gt;e'/></Root>",
        );
        let parsed = XmlParser
            .parse_fragment(&host, context, &"<Item/><p:Item/>".into())
            .unwrap();
        assert_eq!(
            parsed
                .document()
                .node(parsed.children()[0])
                .unwrap()
                .as_element()
                .unwrap()
                .namespace(),
            ""
        );
        assert_eq!(
            parsed
                .document()
                .node(parsed.children()[1])
                .unwrap()
                .namespace(),
            Some("urn:a&b\"c<d>e")
        );
        host.set_attribute_ns(
            context,
            Some(XMLNS_NAMESPACE),
            Some("xmlns"),
            "p",
            "urn:new",
        );
        let parsed = XmlParser
            .parse_fragment(&host, context, &"<p:Item/>".into())
            .unwrap();
        assert_eq!(
            parsed
                .document()
                .node(parsed.children()[0])
                .unwrap()
                .namespace(),
            Some("urn:new")
        );
        host.set_attribute_ns(context, Some(XMLNS_NAMESPACE), Some("xmlns"), "p", "");
        assert!(
            XmlParser
                .parse_fragment(&host, context, &"<Item/>".into())
                .is_ok()
        );
        assert!(
            XmlParser
                .parse_fragment(&host, context, &"<p:Item/>".into())
                .is_err()
        );
    }

    #[test]
    fn xml_fragments_use_only_predefined_entities_and_keep_scripts_inert() {
        let (host, context) =
            context("<!DOCTYPE Root [<!ENTITY inherited 'secret'>]><Root><Context/></Root>");
        assert!(
            XmlParser
                .parse_fragment(&host, context, &"&inherited;".into())
                .is_err()
        );
        let parsed = XmlParser.parse_fragment(&host, context, &"&lt;&amp;&quot;&apos;&gt;&#x1F98A;<script xmlns='http://www.w3.org/1999/xhtml'>1</script><template xmlns='http://www.w3.org/1999/xhtml'><script>2</script></template>".into()).unwrap();
        assert_eq!(
            parsed
                .document()
                .node(parsed.children()[0])
                .unwrap()
                .node_value(),
            Some("<&\"'>🦊")
        );
        let scripts = parsed
            .document()
            .nodes()
            .filter(|node| node.is_script_element())
            .collect::<Vec<_>>();
        assert_eq!(scripts.len(), 2);
        assert!(
            scripts
                .iter()
                .all(|node| node.as_element().unwrap().script_already_started())
        );
    }

    #[test]
    fn xml_fragments_validate_expanded_names_and_restore_namespace_bindings() {
        let (mut host, context) = context("<Context xmlns:p='urn:parent'/>");
        let duplicate = "<Item xmlns:p='urn:a&amp;b' xmlns:q='urn:a&#38;b' p:x='1' q:x='2'/>";
        assert!(
            XmlParser
                .parse_fragment(&host, context, &duplicate.into())
                .is_err()
        );
        let markup = "<Item xmlns:p='urn:child'><p:Child/></Item><p:Sibling/><Item xmlns:p='urn:other'/><p:Last/>";
        let parsed = XmlParser
            .parse_fragment(&host, context, &markup.into())
            .unwrap();
        for index in [1, 3] {
            assert_eq!(
                parsed
                    .document()
                    .node(parsed.children()[index])
                    .unwrap()
                    .namespace(),
                Some("urn:parent")
            );
        }
        host.set_attribute_ns(
            context,
            Some(XMLNS_NAMESPACE),
            Some("xmlns"),
            "p",
            "urn:a\tb\nc\rd",
        );
        let parsed = XmlParser
            .parse_fragment(&host, context, &"<p:Item/>".into())
            .unwrap();
        assert_eq!(
            parsed
                .document()
                .node(parsed.children()[0])
                .unwrap()
                .namespace(),
            Some("urn:a\tb\nc\rd")
        );
        host.set_attribute_utf16_units(context, "xmlns:p", "\u{fffd}", vec![0xd800]);
        assert!(
            XmlParser
                .parse_fragment(&host, context, &"<p:Item/>".into())
                .is_err()
        );
    }
}
