use super::super::{XML_NAMESPACE, XMLNS_NAMESPACE};
use std::collections::HashMap;
use xmlparser::{
    ElementEnd as XmlElementEnd, Token as XmlTokenizerToken, Tokenizer as XmlTokenizer,
};

struct XmlPrepassError {
    detail: String,
}
impl XmlPrepassError {
    fn tree(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
    fn namespace(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

pub(super) fn validate(input: &str) -> Result<(), super::XmlFragmentParseError> {
    match xml_element_stack_error(input) {
        Some(error) => Err(super::XmlFragmentParseError(error.detail)),
        None => Ok(()),
    }
}

type XmlNamespaceBindings = HashMap<String, Vec<String>>;

fn xml_element_stack_error(xml: &str) -> Option<XmlPrepassError> {
    let mut pending_start_name = None;
    let mut pending_used_prefixes = Vec::new();
    let mut pending_declared_prefixes = Vec::new();
    let mut pending_attributes = Vec::new();
    let mut open_elements = Vec::new();
    let mut active_prefixes = HashMap::from([("xml".to_owned(), vec![XML_NAMESPACE.to_owned()])]);

    for token in XmlTokenizer::from(xml) {
        match token {
            Ok(XmlTokenizerToken::ElementStart { prefix, local, .. }) => {
                let prefix = prefix.as_str();
                if !prefix.is_empty() {
                    pending_used_prefixes.push(prefix.to_owned());
                }
                pending_start_name = Some(qualified_xml_name(prefix, local.as_str()));
            }
            Ok(XmlTokenizerToken::Attribute {
                prefix,
                local,
                value,
                ..
            }) => {
                let prefix = prefix.as_str();
                if prefix == "xmlns" || (prefix.is_empty() && local.as_str() == "xmlns") {
                    let Some(namespace) = xml_namespace_attribute_value(value.as_str()) else {
                        return Some(XmlPrepassError::namespace("Invalid XML namespace value"));
                    };
                    let declared_prefix = if prefix == "xmlns" {
                        local.as_str()
                    } else {
                        ""
                    };
                    if !xml_namespace_declaration_is_valid(declared_prefix, &namespace) {
                        return Some(XmlPrepassError::namespace(
                            "Invalid XML namespace declaration",
                        ));
                    }
                    pending_declared_prefixes.push((declared_prefix.to_owned(), namespace));
                } else if !prefix.is_empty() {
                    pending_used_prefixes.push(prefix.to_owned());
                    pending_attributes.push((prefix.to_owned(), local.as_str().to_owned()));
                }
            }
            Ok(XmlTokenizerToken::ElementEnd { end, .. }) => match end {
                end @ (XmlElementEnd::Open | XmlElementEnd::Empty) => {
                    let is_open = matches!(end, XmlElementEnd::Open);
                    let Some(name) = pending_start_name.take() else {
                        return Some(XmlPrepassError::tree(if is_open {
                            "XML start tag is missing a name"
                        } else {
                            "XML empty-element tag is missing a name"
                        }));
                    };
                    for (prefix, namespace) in &pending_declared_prefixes {
                        active_prefixes
                            .entry(prefix.clone())
                            .or_default()
                            .push(namespace.clone());
                    }
                    if let Some(error) = xml_start_namespace_error(
                        &active_prefixes,
                        &pending_used_prefixes,
                        &pending_attributes,
                    ) {
                        return Some(error);
                    }
                    pending_used_prefixes.clear();
                    pending_attributes.clear();
                    if is_open {
                        let prefixes = pending_declared_prefixes
                            .drain(..)
                            .map(|(prefix, _)| prefix)
                            .collect::<Vec<_>>();
                        open_elements.push((name, prefixes));
                    } else {
                        for (prefix, _) in pending_declared_prefixes.drain(..) {
                            pop_xml_namespace_binding(&mut active_prefixes, &prefix);
                        }
                    }
                }
                XmlElementEnd::Close(prefix, local) => {
                    let closing_name = qualified_xml_name(prefix.as_str(), local.as_str());
                    let Some((open_name, declared_prefixes)) = open_elements.pop() else {
                        return Some(XmlPrepassError::tree(format!(
                            "XML end tag </{closing_name}> has no matching start tag"
                        )));
                    };
                    if open_name != closing_name {
                        return Some(XmlPrepassError::tree(format!(
                            "XML end tag </{closing_name}> does not match <{open_name}>"
                        )));
                    }
                    for prefix in declared_prefixes {
                        pop_xml_namespace_binding(&mut active_prefixes, &prefix);
                    }
                }
            },
            Ok(_) => {}
            Err(error) => return Some(XmlPrepassError::tree(error.to_string())),
        }
    }

    if let Some(name) = pending_start_name {
        Some(XmlPrepassError::tree(format!(
            "XML start tag <{name}> is incomplete"
        )))
    } else {
        open_elements
            .last()
            .map(|(name, _)| XmlPrepassError::tree(format!("XML element <{name}> is not closed")))
    }
}

fn xml_namespace_declaration_is_valid(prefix: &str, namespace: &str) -> bool {
    match prefix {
        "xmlns" => false,
        "xml" => namespace == XML_NAMESPACE,
        _ => {
            namespace != XML_NAMESPACE
                && namespace != XMLNS_NAMESPACE
                && (prefix.is_empty() || !namespace.is_empty())
        }
    }
}

fn xml_start_namespace_error(
    namespaces: &XmlNamespaceBindings,
    used_prefixes: &[String],
    attributes: &[(String, String)],
) -> Option<XmlPrepassError> {
    for prefix in used_prefixes {
        if namespaces
            .get(prefix)
            .and_then(|values| values.last())
            .is_none_or(String::is_empty)
        {
            return Some(XmlPrepassError::namespace(format!(
                "XML namespace prefix `{prefix}` is not declared"
            )));
        }
    }
    // xml5ever drops attributes with duplicate expanded names without an
    // error. Validate before that recovery loses their original identities.
    let mut expanded_names = std::collections::HashSet::new();
    for (prefix, local) in attributes {
        let namespace = namespaces
            .get(prefix)
            .and_then(|values| values.last())
            .map(String::as_str)
            .unwrap_or("");
        if !expanded_names.insert((namespace, local)) {
            return Some(XmlPrepassError::namespace(format!(
                "Duplicate XML attribute {{{namespace}}}{local}"
            )));
        }
    }
    None
}

fn pop_xml_namespace_binding(namespaces: &mut XmlNamespaceBindings, prefix: &str) {
    if let Some(values) = namespaces.get_mut(prefix) {
        values.pop();
        if values.is_empty() {
            namespaces.remove(prefix);
        }
    }
}

fn xml_namespace_attribute_value(mut input: &str) -> Option<String> {
    let mut value = String::new();
    while let Some(character) = input.chars().next() {
        if character == '&' {
            let mut stream = xmlparser::Stream::from(input);
            let xmlparser::Reference::Char(character) = stream.consume_reference().ok()? else {
                return None;
            };
            value.push(character);
            input = &input[stream.pos()..];
        } else {
            input = &input[character.len_utf8()..];
            match character {
                '\r' => {
                    input = input.strip_prefix('\n').unwrap_or(input);
                    value.push(' ');
                }
                '\n' | '\t' => value.push(' '),
                _ => value.push(character),
            }
        }
    }
    Some(value)
}

fn qualified_xml_name(prefix: &str, local_name: &str) -> String {
    if prefix.is_empty() {
        local_name.to_owned()
    } else {
        format!("{prefix}:{local_name}")
    }
}
