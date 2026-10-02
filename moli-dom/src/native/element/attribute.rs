use super::super::node::{NativeNodeId, NodeData};
use super::super::{DomStringValue, NativeDom};
use html5ever::{LocalName, Namespace, Prefix};

#[derive(Debug, Clone)]
pub struct Attribute {
    pub(super) local_name: LocalName,
    pub(super) namespace: Namespace,
    pub(super) prefix: Option<Prefix>,
    pub(super) value: Box<str>,
}

impl Attribute {
    const XLINK_NAMESPACE: &'static str = "http://www.w3.org/1999/xlink";
    const XML_NAMESPACE: &'static str = "http://www.w3.org/XML/1998/namespace";
    const XMLNS_NAMESPACE: &'static str = "http://www.w3.org/2000/xmlns/";

    pub fn new(
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        value: String,
    ) -> Self {
        Self {
            local_name: LocalName::from(local_name),
            namespace: Namespace::from(namespace),
            prefix: prefix.map(Prefix::from),
            value: value.into_boxed_str(),
        }
    }

    pub fn local_name(&self) -> &str {
        self.local_name.as_ref()
    }

    pub fn name(&self) -> String {
        match self.prefix.as_deref() {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}:{}", self.local_name),
            _ => self.local_name.to_string(),
        }
    }

    pub(crate) fn push_html_serialized_name(&self, mut push_str: impl FnMut(&str)) {
        let namespace: &str = self.namespace.as_ref();
        let local_name: &str = self.local_name.as_ref();
        let prefix = match namespace {
            Self::XML_NAMESPACE => Some("xml"),
            Self::XLINK_NAMESPACE => Some("xlink"),
            Self::XMLNS_NAMESPACE if self.prefix.is_none() && local_name != "xmlns" => {
                Some("xmlns")
            }
            _ => self.prefix.as_deref().filter(|prefix| !prefix.is_empty()),
        };
        if let Some(prefix) = prefix {
            push_str(prefix);
            push_str(":");
        }
        push_str(local_name);
    }

    pub fn name_matches(&self, name: &str) -> bool {
        match self.prefix.as_deref() {
            Some(prefix) if !prefix.is_empty() => {
                name.len() == prefix.len() + 1 + self.local_name.len()
                    && name.starts_with(prefix)
                    && name.as_bytes().get(prefix.len()) == Some(&b':')
                    && name[(prefix.len() + 1)..] == self.local_name
            }
            _ => self.local_name() == name,
        }
    }

    pub fn namespace(&self) -> &str {
        self.namespace.as_ref()
    }

    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_ref().map(AsRef::as_ref)
    }

    pub fn value(&self) -> &str {
        self.value.as_ref()
    }
}

pub(super) fn normalized_option_text_content(
    dom: &NativeDom,
    handle: NativeNodeId,
) -> DomStringValue {
    let mut out = Vec::new();
    let mut pending_space = false;
    let mut stack = vec![handle];
    while let Some(handle) = stack.pop() {
        let Some(node) = dom.node(handle) else {
            continue;
        };
        match node.data() {
            NodeData::Text(text) => {
                append_normalized_option_text(text.value(), &mut out, &mut pending_space)
            }
            NodeData::CDataSection(cdata) => {
                append_normalized_option_text(cdata.value(), &mut out, &mut pending_space)
            }
            NodeData::Element(element)
                if (element.local_name() == "script"
                    && matches!(
                        element.namespace(),
                        "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
                    ))
                    || element.is_html_element("img") => {}
            NodeData::Document(_) | NodeData::Element(_) | NodeData::DocumentFragment(_) => {
                stack.extend(dom.child_ids_reversed(handle));
            }
            NodeData::Comment(_)
            | NodeData::ProcessingInstruction(_)
            | NodeData::DocumentType(_) => {}
        }
    }
    DomStringValue::from_utf16(&out)
}

fn append_normalized_option_text(
    text: &DomStringValue,
    out: &mut Vec<u16>,
    pending_space: &mut bool,
) {
    for &unit in text.utf16_units().iter() {
        if matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20) {
            *pending_space = !out.is_empty();
        } else {
            if *pending_space {
                out.push(0x20);
                *pending_space = false;
            }
            out.push(unit);
        }
    }
}

pub(super) fn split_class_names(value: &str) -> Vec<&str> {
    value.split_ascii_whitespace().collect()
}
