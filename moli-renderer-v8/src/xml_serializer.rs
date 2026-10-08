use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::{
    dom::native::{Attribute, DomHost, DomStringValue, Element, NativeNodeId, NodeType},
    native_bridge::NativeNodeReference,
    web_api_interfaces, webidl,
};

use super::util::{throw_type_error, v8_string_from_utf16_units};

const VOID_HTML: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "menuitem", "meta", "param", "source", "track", "wbr",
];
const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

pub(super) fn xml_serializer_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'XMLSerializer': Please use the 'new' operator.",
        );
        return;
    }
    rv.set(args.this().into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLSerializer.serializeToString")]
struct SerializeToStringArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    root: v8::Local<'s, v8::Object>,
}

pub(super) fn xml_serializer_serialize_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SerializeToStringArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::from_object(scope, parsed.root) else {
        throw_type_error(scope, "The Node has no native identity.");
        return;
    };
    let serialized = match node {
        NativeNodeReference::Tree { node, .. } => {
            // SAFETY: native identity resolves the node's owning context host;
            // serialization invokes no author JavaScript and retains no host.
            serialize_native_handle_utf16(unsafe { &*node.runtime_ptr }.dom_host(), node.handle)
        }
        NativeNodeReference::Attr(attr) => {
            let mut markup = XmlMarkup::default();
            markup.escaped_value(&attr.value(scope), true);
            markup.units
        }
    };
    if let Some(serialized) = v8_string_from_utf16_units(scope, &serialized) {
        rv.set(serialized.into());
    }
}

#[derive(Clone, Debug)]
struct NamespaceContext {
    default_namespace: DomStringValue,
    // A prefix has one effective binding in this element's scope. Iteration
    // order retains the preference for the most recently declared binding.
    prefixes: IndexMap<String, DomStringValue>,
}

impl Default for NamespaceContext {
    fn default() -> Self {
        Self {
            default_namespace: DomStringValue::default(),
            prefixes: IndexMap::from([("xml".to_owned(), XML_NAMESPACE.into())]),
        }
    }
}

impl NamespaceContext {
    fn add_prefix(&mut self, namespace: impl Into<DomStringValue>, prefix: &str) {
        self.prefixes.shift_remove(prefix);
        self.prefixes.insert(prefix.to_owned(), namespace.into());
    }

    fn preferred_prefix(&self, namespace: &str, preferred: Option<&str>) -> Option<String> {
        let namespace = DomStringValue::from(namespace);
        if let Some(preferred) = preferred
            && self.contains_prefix(&namespace, preferred)
        {
            return Some(preferred.to_owned());
        }
        self.prefixes
            .iter()
            .rev()
            .find_map(|(prefix, bound)| (bound == &namespace).then(|| prefix.clone()))
    }

    fn contains_prefix(&self, namespace: &DomStringValue, prefix: &str) -> bool {
        self.prefixes.get(prefix) == Some(namespace)
    }

    fn prefix_in_use(&self, prefix: &str) -> bool {
        self.prefixes.contains_key(prefix)
    }
}

/// One UTF-16 output buffer for both XML web surfaces and UTF-8 body consumers.
/// Escaping changes only XML delimiters; all other code units remain intact.
struct XmlMarkup {
    units: Vec<u16>,
    next_generated_prefix: usize,
    require_well_formed: bool,
}

impl Default for XmlMarkup {
    fn default() -> Self {
        Self {
            units: Vec::new(),
            next_generated_prefix: 1,
            require_well_formed: false,
        }
    }
}

impl XmlMarkup {
    fn ensure_xml_chars(&self, value: &DomStringValue) -> Result<(), XmlSerializationError> {
        if !self.require_well_formed {
            return Ok(());
        }
        let valid = match value.as_str() {
            Some(value) => value.chars().all(is_xml_char),
            None => char::decode_utf16(value.utf16_units().iter().copied())
                .all(|character| character.is_ok_and(is_xml_char)),
        };
        if valid {
            Ok(())
        } else {
            Err(XmlSerializationError)
        }
    }

    fn push_str(&mut self, value: &str) {
        self.units.extend(value.encode_utf16());
    }

    fn push_value(&mut self, value: &DomStringValue) {
        value.append_utf16_units_to(&mut self.units);
    }

    fn push_doctype_id(&mut self, value: &DomStringValue) {
        let quote = if value.as_str_lossy().contains('"') {
            0x27
        } else {
            0x22
        };
        self.units.push(quote);
        self.push_value(value);
        self.units.push(quote);
    }

    fn escaped_units(&mut self, units: impl Iterator<Item = u16>, attribute: bool) {
        for unit in units {
            let replacement = match unit {
                0x26 => Some("&amp;"),
                0x3c => Some("&lt;"),
                0x3e => Some("&gt;"),
                0x22 if attribute => Some("&quot;"),
                0x09 if attribute => Some("&#9;"),
                0x0a if attribute => Some("&#10;"),
                0x0d if attribute => Some("&#13;"),
                _ => None,
            };
            if let Some(replacement) = replacement {
                self.push_str(replacement);
            } else {
                self.units.push(unit);
            }
        }
    }

    fn escaped_value(&mut self, value: &DomStringValue, attribute: bool) {
        if let Some(value) = value.as_str() {
            self.escaped_units(value.encode_utf16(), attribute);
        } else {
            self.escaped_units(value.utf16_units().iter().copied(), attribute);
        }
    }

    fn namespace_declaration(
        &mut self,
        prefix: Option<&str>,
        namespace: &str,
    ) -> Result<(), XmlSerializationError> {
        if self.require_well_formed && !namespace.chars().all(is_xml_char) {
            return Err(XmlSerializationError);
        }
        self.push_str(" xmlns");
        if let Some(prefix) = prefix {
            self.push_str(":");
            self.push_str(prefix);
        }
        self.push_str("=\"");
        self.escaped_units(namespace.encode_utf16(), true);
        self.push_str("\"");
        Ok(())
    }

    fn generate_namespace_prefix(
        &mut self,
        namespace_context: &mut NamespaceContext,
        namespace: &str,
    ) -> String {
        loop {
            let prefix = format!("ns{}", self.next_generated_prefix);
            self.next_generated_prefix += 1;
            if !namespace_context.prefix_in_use(&prefix) {
                namespace_context.add_prefix(namespace, &prefix);
                return prefix;
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct XmlSerializationError;

fn is_xml_char(character: char) -> bool {
    matches!(character as u32, 0x9 | 0xa | 0xd | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

fn is_xml_name_start(character: char) -> bool {
    matches!(character as u32,
        0x3a | 0x41..=0x5a | 0x5f | 0x61..=0x7a | 0xc0..=0xd6 | 0xd8..=0xf6
        | 0xf8..=0x2ff | 0x370..=0x37d | 0x37f..=0x1fff | 0x200c..=0x200d
        | 0x2070..=0x218f | 0x2c00..=0x2fef | 0x3001..=0xd7ff | 0xf900..=0xfdcf
        | 0xfdf0..=0xfffd | 0x10000..=0xeffff)
}

fn is_xml_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(is_xml_name_start)
        && characters.all(|character| {
            is_xml_name_start(character)
                || matches!(character as u32, 0x2d | 0x2e | 0x30..=0x39 | 0xb7 | 0x300..=0x36f | 0x203f..=0x2040)
        })
}

fn is_xml_pubid_char(unit: u16) -> bool {
    matches!(unit, 0x20 | 0xd | 0xa | 0x61..=0x7a | 0x41..=0x5a | 0x30..=0x39)
        || b"-'()+,./:=?;!*#@$_%"
            .iter()
            .any(|byte| unit == u16::from(*byte))
}

pub(crate) fn serialize_native_handle_utf16(dom_host: &DomHost, handle: NativeNodeId) -> Vec<u16> {
    serialize_native_handle_with_mode(dom_host, handle, false)
        .expect("permissive XML serialization does not validate well-formedness")
}

pub(crate) fn serialize_native_handle_well_formed(
    dom_host: &DomHost,
    handle: NativeNodeId,
) -> Result<Vec<u16>, XmlSerializationError> {
    serialize_native_handle_with_mode(dom_host, handle, true)
}

fn serialize_native_handle_with_mode(
    dom_host: &DomHost,
    handle: NativeNodeId,
    require_well_formed: bool,
) -> Result<Vec<u16>, XmlSerializationError> {
    let mut markup = XmlMarkup {
        require_well_formed,
        ..XmlMarkup::default()
    };
    serialize_native_node(dom_host, handle, &NamespaceContext::default(), &mut markup)?;
    Ok(markup.units)
}

/// UTF-8 consumers, such as XMLHttpRequest's Document body, encode the completed
/// serialization as scalar text. Web DOMString getters use the original units.
pub(crate) fn serialize_native_handle(dom_host: &DomHost, handle: NativeNodeId) -> String {
    String::from_utf16_lossy(&serialize_native_handle_utf16(dom_host, handle))
}

pub(crate) fn serialize_native_inner_html(
    dom_host: &DomHost,
    handle: NativeNodeId,
) -> Result<Option<Vec<u16>>, XmlSerializationError> {
    let Some(node) = dom_host.node(handle) else {
        return Ok(None);
    };
    let child_container = node
        .as_element()
        .and_then(Element::template_contents)
        .unwrap_or(handle);
    let mut markup = XmlMarkup {
        require_well_formed: true,
        ..XmlMarkup::default()
    };
    serialize_native_children(
        dom_host,
        child_container,
        &NamespaceContext::default(),
        &mut markup,
    )?;
    Ok(Some(markup.units))
}

fn serialize_native_node(
    dom_host: &DomHost,
    handle: NativeNodeId,
    namespace_context: &NamespaceContext,
    markup: &mut XmlMarkup,
) -> Result<(), XmlSerializationError> {
    let Some(node) = dom_host.node(handle) else {
        return Ok(());
    };
    match node.node_type() {
        NodeType::Element => serialize_native_element(dom_host, handle, namespace_context, markup)?,
        NodeType::Text => {
            if let Some(value) = node.character_data_value() {
                markup.ensure_xml_chars(value)?;
                markup.escaped_value(value, false);
            }
        }
        NodeType::CDataSection | NodeType::Comment | NodeType::ProcessingInstruction => {
            // The CDATASection serialization algorithm does not apply the
            // require-well-formed checks used by Text, Comment and PI nodes.
            if node.node_type() != NodeType::CDataSection {
                if let Some(value) = node.character_data_value() {
                    markup.ensure_xml_chars(value)?;
                    if markup.require_well_formed {
                        let data = value.as_str_lossy();
                        let invalid = match node.node_type() {
                            NodeType::Comment => data.contains("--") || data.ends_with('-'),
                            _ => data.contains("?>"),
                        };
                        if invalid {
                            return Err(XmlSerializationError);
                        }
                    }
                }
                if node.node_type() == NodeType::ProcessingInstruction
                    && markup.require_well_formed
                    && node.target().is_some_and(|target| {
                        target.contains(':') || target.eq_ignore_ascii_case("xml")
                    })
                {
                    return Err(XmlSerializationError);
                }
            }
            let (open, close) = match node.node_type() {
                NodeType::CDataSection => ("<![CDATA[", "]]>"),
                NodeType::Comment => ("<!--", "-->"),
                _ => ("<?", "?>"),
            };
            markup.push_str(open);
            if node.node_type() == NodeType::ProcessingInstruction {
                markup.push_str(node.target().unwrap_or_default());
                markup.push_str(" ");
            }
            if let Some(value) = node.character_data_value() {
                value.append_utf16_units_to(&mut markup.units);
            }
            markup.push_str(close);
        }
        NodeType::Document | NodeType::DocumentFragment => {
            if node.node_type() == NodeType::Document
                && markup.require_well_formed
                && !dom_host
                    .child_handles(handle)
                    .any(|child| dom_host.node(child).is_some_and(|node| node.is_element()))
            {
                return Err(XmlSerializationError);
            }
            serialize_native_children(dom_host, handle, namespace_context, markup)?;
        }
        NodeType::DocumentType => {
            let Some(doctype) = node.as_document_type() else {
                return Ok(());
            };
            if markup.require_well_formed {
                if !doctype
                    .public_id_value()
                    .utf16_units()
                    .iter()
                    .copied()
                    .all(is_xml_pubid_char)
                    || (doctype.system_id().contains('"') && doctype.system_id().contains('\''))
                {
                    return Err(XmlSerializationError);
                }
                markup.ensure_xml_chars(doctype.system_id_value())?;
            }
            markup.push_str("<!DOCTYPE ");
            markup.push_value(doctype.name_value());
            if !doctype.public_id().is_empty() {
                markup.push_str(" PUBLIC ");
                markup.push_doctype_id(doctype.public_id_value());
            } else if !doctype.system_id().is_empty() {
                markup.push_str(" SYSTEM");
            }
            if !doctype.system_id().is_empty() {
                markup.push_str(" ");
                markup.push_doctype_id(doctype.system_id_value());
            }
            markup.push_str(">");
        }
    }
    Ok(())
}

fn serialize_native_children(
    dom_host: &DomHost,
    handle: NativeNodeId,
    namespace_context: &NamespaceContext,
    markup: &mut XmlMarkup,
) -> Result<(), XmlSerializationError> {
    for child in dom_host.child_handles(handle) {
        serialize_native_node(dom_host, child, namespace_context, markup)?;
    }
    Ok(())
}

fn serialize_native_element(
    dom_host: &DomHost,
    handle: NativeNodeId,
    parent_namespace_context: &NamespaceContext,
    markup: &mut XmlMarkup,
) -> Result<(), XmlSerializationError> {
    let Some(element) = dom_host.node(handle).and_then(|node| node.as_element()) else {
        return Ok(());
    };
    let namespace = element.namespace();
    let original_prefix = element.prefix().filter(|prefix| !prefix.is_empty());
    let local_name = element.local_name();
    if markup.require_well_formed && (local_name.contains(':') || !is_xml_name(local_name)) {
        return Err(XmlSerializationError);
    }
    let mut namespace_context = parent_namespace_context.clone();
    let (local_default_namespace, local_prefixes) =
        record_namespace_information(element, &mut namespace_context);
    let mut inherited_namespace = parent_namespace_context.default_namespace.clone();
    let mut ignore_namespace_definition_attribute = false;
    let mut declaration = None;

    let tag = if inherited_namespace.as_str() == Some(namespace) {
        if local_default_namespace.is_some()
            && !local_prefixes.values().any(DomStringValue::is_empty)
        {
            ignore_namespace_definition_attribute = true;
        }
        if namespace == XML_NAMESPACE {
            format!("xml:{local_name}")
        } else {
            local_name.to_owned()
        }
    } else {
        if markup.require_well_formed && original_prefix == Some("xmlns") {
            return Err(XmlSerializationError);
        }
        let candidate_prefix = namespace_context.preferred_prefix(namespace, original_prefix);
        if let Some(candidate_prefix) = candidate_prefix {
            if let Some(local_default_namespace) = local_default_namespace
                .as_ref()
                .filter(|namespace| namespace.as_str() != Some(XML_NAMESPACE))
            {
                inherited_namespace = local_default_namespace.clone();
            }
            format!("{candidate_prefix}:{local_name}")
        } else if let Some(original_prefix) = original_prefix {
            let prefix = if local_prefixes.contains_key(original_prefix) {
                markup.generate_namespace_prefix(&mut namespace_context, namespace)
            } else {
                namespace_context.add_prefix(namespace, original_prefix);
                original_prefix.to_owned()
            };
            declaration = Some((Some(prefix.clone()), namespace));
            if let Some(local_default_namespace) = local_default_namespace.as_ref() {
                inherited_namespace = local_default_namespace.clone();
            }
            format!("{prefix}:{local_name}")
        } else if local_default_namespace
            .as_ref()
            .and_then(DomStringValue::as_str)
            != Some(namespace)
        {
            ignore_namespace_definition_attribute = true;
            inherited_namespace = namespace.into();
            declaration = Some((None, namespace));
            local_name.to_owned()
        } else {
            inherited_namespace = namespace.into();
            local_name.to_owned()
        }
    };

    markup.push_str("<");
    markup.push_str(&tag);
    if let Some((prefix, namespace)) = declaration {
        markup.namespace_declaration(prefix.as_deref(), namespace)?;
    }
    serialize_native_attributes(
        element,
        &mut namespace_context,
        &local_prefixes,
        ignore_namespace_definition_attribute,
        markup,
    )?;
    namespace_context.default_namespace = inherited_namespace;
    let child_handle = element.template_contents().unwrap_or(handle);
    let has_children = dom_host.child_handles(child_handle).next().is_some();
    if !has_children && namespace == HTML_NAMESPACE && VOID_HTML.contains(&local_name) {
        markup.push_str(" />");
        return Ok(());
    }
    if !has_children && namespace != HTML_NAMESPACE {
        markup.push_str("/>");
        return Ok(());
    }
    markup.push_str(">");
    serialize_native_children(dom_host, child_handle, &namespace_context, markup)?;
    markup.push_str("</");
    markup.push_str(&tag);
    markup.push_str(">");
    Ok(())
}

fn attribute_value(element: &Element, attribute: &Attribute) -> DomStringValue {
    element
        .attribute_value_utf16_units(attribute)
        .map_or_else(|| attribute.value().into(), DomStringValue::from_utf16)
}

fn record_namespace_information(
    element: &Element,
    namespace_context: &mut NamespaceContext,
) -> (Option<DomStringValue>, HashMap<String, DomStringValue>) {
    let mut local_default_namespace = None;
    let mut local_prefixes = HashMap::new();
    for attribute in element.attributes() {
        if attribute.namespace() != XMLNS_NAMESPACE {
            continue;
        }
        // A spelling such as setAttribute("xmlns", value) does not give the
        // attribute native namespace identity or change descendant bindings.
        let Some(attribute_prefix) = attribute.prefix() else {
            local_default_namespace = Some(attribute_value(element, attribute));
            continue;
        };
        if attribute_prefix.is_empty() {
            local_default_namespace = Some(attribute_value(element, attribute));
            continue;
        }

        let prefix = attribute.local_name();
        let namespace = attribute_value(element, attribute);
        if namespace.as_str() == Some(XML_NAMESPACE)
            || namespace_context.contains_prefix(&namespace, prefix)
        {
            continue;
        }
        namespace_context.add_prefix(&namespace, prefix);
        local_prefixes.insert(prefix.to_owned(), namespace);
    }
    (local_default_namespace, local_prefixes)
}

fn serialize_native_attributes(
    element: &Element,
    namespace_context: &mut NamespaceContext,
    local_prefixes: &HashMap<String, DomStringValue>,
    ignore_namespace_definition_attribute: bool,
    markup: &mut XmlMarkup,
) -> Result<(), XmlSerializationError> {
    let mut local_names = HashSet::new();
    for attribute in element.attributes() {
        let attribute_namespace = attribute.namespace();
        if markup.require_well_formed
            && !local_names.insert((attribute_namespace, attribute.local_name()))
        {
            return Err(XmlSerializationError);
        }
        let mut candidate_prefix = (!attribute_namespace.is_empty())
            .then(|| namespace_context.preferred_prefix(attribute_namespace, attribute.prefix()))
            .flatten();

        if attribute_namespace == XMLNS_NAMESPACE {
            let value = attribute_value(element, attribute);
            if value.as_str() == Some(XML_NAMESPACE)
                || (attribute.prefix().is_none() && ignore_namespace_definition_attribute)
            {
                continue;
            }
            if attribute.prefix().is_some() {
                let local_namespace = local_prefixes.get(attribute.local_name());
                if local_namespace.is_none()
                    || (local_namespace.is_some_and(|namespace| namespace != &value)
                        && namespace_context.contains_prefix(&value, attribute.local_name()))
                {
                    continue;
                }
            }
            if markup.require_well_formed
                && (value.as_str() == Some(XMLNS_NAMESPACE)
                    || (attribute.prefix().is_some() && value.is_empty()))
            {
                return Err(XmlSerializationError);
            }
            if attribute.prefix() == Some("xmlns") {
                candidate_prefix = Some("xmlns".to_owned());
            }
        } else if !attribute_namespace.is_empty() && candidate_prefix.is_none() {
            let prefix = match attribute
                .prefix()
                .filter(|prefix| !prefix.is_empty() && !namespace_context.prefix_in_use(prefix))
            {
                Some(prefix) => {
                    namespace_context.add_prefix(attribute_namespace, prefix);
                    prefix.to_owned()
                }
                None => markup.generate_namespace_prefix(namespace_context, attribute_namespace),
            };
            markup.namespace_declaration(Some(&prefix), attribute_namespace)?;
            candidate_prefix = Some(prefix);
        }

        if markup.require_well_formed {
            if attribute.local_name().contains(':')
                || !is_xml_name(attribute.local_name())
                || (attribute.local_name() == "xmlns" && attribute_namespace.is_empty())
            {
                return Err(XmlSerializationError);
            }
            markup.ensure_xml_chars(&attribute_value(element, attribute))?;
        }
        markup.push_str(" ");
        if let Some(prefix) = candidate_prefix {
            markup.push_str(&prefix);
            markup.push_str(":");
        }
        markup.push_str(attribute.local_name());
        markup.push_str("=\"");
        if let Some(units) = element.attribute_value_utf16_units(attribute) {
            markup.escaped_units(units.iter().copied(), true);
        } else {
            markup.escaped_units(attribute.value().encode_utf16(), true);
        }
        markup.push_str("\"");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::dom::native::{DomHost, DomStringValue, NativeDom};

    use super::{
        XMLNS_NAMESPACE, XmlMarkup, serialize_native_handle, serialize_native_handle_utf16,
        serialize_native_handle_well_formed,
    };

    fn xml_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_xml(
            url::Url::parse("https://xml-serializer.test/").unwrap(),
        ))
    }

    #[test]
    fn checked_xml_serialization_validates_each_native_node_without_replacing_utf16() {
        let mut host = xml_host();
        for (units, valid) in [
            (vec![0x9, 0xa, 0xd, 0x20, 0xd7ff, 0xe000, 0xfffd], true),
            (vec![0xd800, 0xdc00, 0xdbff, 0xdfff], true),
            (vec![0x7f, 0x85, 0xfdd0, 0xd83f, 0xdffe], true),
            (vec![0x0], false),
            (vec![0xc], false),
            (vec![0x1f], false),
            (vec![0xfffe], false),
            (vec![0xffff], false),
            (vec![0xd800], false),
            (vec![0xdc00], false),
            (vec![0xdc00, 0xd800], false),
        ] {
            let value = DomStringValue::from_utf16(&units);
            let text = host.create_text_node(value.clone());
            let comment = host.create_comment(value.clone());
            let pi = host.create_processing_instruction("target", value.clone());
            for node in [text, comment, pi] {
                let original = serialize_native_handle_utf16(&host, node);
                let checked = serialize_native_handle_well_formed(&host, node);
                assert_eq!(checked.is_ok(), valid, "{units:x?}");
                if valid {
                    assert_eq!(checked.unwrap(), original);
                }
                assert_eq!(serialize_native_handle_utf16(&host, node), original);
            }
            let cdata = host.create_cdata_section(value);
            assert_eq!(
                serialize_native_handle_well_formed(&host, cdata).unwrap(),
                serialize_native_handle_utf16(&host, cdata),
            );
        }
        let root = host.create_element_with_parts(None, None, "root");
        let high = host.create_text_node(DomStringValue::from_utf16(&[0xd83d]));
        let low = host.create_text_node(DomStringValue::from_utf16(&[0xde00]));
        assert!(host.append_child(root, high));
        assert!(host.append_child(root, low));
        assert!(serialize_native_handle_well_formed(&host, root).is_err());
        assert_eq!(serialize_native_handle(&host, root), "<root>😀</root>");

        let document = host.document_handle();
        assert!(serialize_native_handle_well_formed(&host, document).is_err());
        assert!(host.append_child(document, root));
        assert!(host.set_text_content(high, "valid"));
        assert!(host.set_text_content(low, ""));
        assert_eq!(
            String::from_utf16(&serialize_native_handle_well_formed(&host, document).unwrap())
                .unwrap(),
            "<root>valid</root>",
        );
    }

    #[test]
    fn checked_xml_serialization_applies_xml_name_and_doctype_productions() {
        let mut host = xml_host();
        // Only attributes in the XMLNS namespace can be skipped as default
        // namespace declarations. An ordinary xmlns attribute is invalid.
        for value in ["", "urn:ignored", "urn:\u{fffd}"] {
            let root = host.create_element_with_parts(None, None, "root");
            assert!(host.set_attribute(root, "xmlns", value));
            assert!(serialize_native_handle_well_formed(&host, root).is_err());
            assert_eq!(
                serialize_native_handle(&host, root),
                format!("<root xmlns=\"{value}\"/>"),
            );
        }
        for (name, valid) in [
            ("root", true),
            ("_a-9.\u{b7}\u{300}\u{203f}", true),
            ("\u{370}", true),
            ("\u{200c}", true),
            ("\u{10000}", true),
            ("\u{effff}", true),
            ("", false),
            ("1root", false),
            ("a:b", false),
            ("\u{37e}", false),
            ("\u{f0000}", false),
        ] {
            let root = host.create_element_with_parts(None, None, name);
            assert_eq!(
                serialize_native_handle_well_formed(&host, root).is_ok(),
                valid,
                "{name:?}"
            );
        }
        for (public, system, valid) in [
            ("AZaz09 -'()+,./:=?;!*#@$_%\r\n", "identifier", true),
            ("", "a\"b", true),
            ("", "a'b", true),
            ("", "😀", true),
            ("a\"b", "", false),
            ("\t", "", false),
            ("é", "", false),
            ("", "a\"'b", false),
            ("", "\0", false),
        ] {
            let doctype = host.create_document_type("root", public, system);
            let original = serialize_native_handle_utf16(&host, doctype);
            let checked = serialize_native_handle_well_formed(&host, doctype);
            assert_eq!(
                checked.is_ok(),
                valid,
                "public={public:?}, system={system:?}"
            );
            if valid {
                assert_eq!(checked.unwrap(), original);
            }
            assert_eq!(serialize_native_handle_utf16(&host, doctype), original);
        }
    }

    #[test]
    fn xml_serialization_retains_code_units_until_the_utf8_body_boundary() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(None, None, "root");
        let units = [0xd800, 0x26, 0x3c, 0x3e, 0x22, 0x09, 0x0a, 0x0d, 0xdc00];
        let value = DomStringValue::from_utf16(&units);
        assert!(host.set_attribute_utf16_units(root, "v", value.as_str_lossy(), units.to_vec()));
        let text = host.create_text_node(value.clone());
        let comment = host.create_comment(value.clone());
        let cdata = host.create_cdata_section(value.clone());
        let pi = host.create_processing_instruction("target", value);
        let empty_pi = host.create_processing_instruction("empty", "");
        for child in [text, comment, cdata, pi, empty_pi] {
            assert!(host.append_child(root, child));
        }
        let mut expected = "<root v=\"".encode_utf16().collect::<Vec<_>>();
        expected.push(0xd800);
        expected.extend("&amp;&lt;&gt;&quot;&#9;&#10;&#13;".encode_utf16());
        expected.push(0xdc00);
        expected.extend("\">".encode_utf16());
        expected.push(0xd800);
        expected.extend("&amp;&lt;&gt;\"\t\n\r".encode_utf16());
        expected.push(0xdc00);
        for (open, close) in [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?target ", "?>")] {
            expected.extend(open.encode_utf16());
            expected.extend(units);
            expected.extend(close.encode_utf16());
        }
        expected.extend("<?empty ?></root>".encode_utf16());
        assert_eq!(serialize_native_handle_utf16(&host, root), expected);
        assert_eq!(
            serialize_native_handle(&host, root),
            String::from_utf16_lossy(&expected)
        );

        let pair = host.create_element_with_parts(None, None, "pair");
        let high = host.create_text_node(DomStringValue::from_utf16(&[0xd83d]));
        let low = host.create_text_node(DomStringValue::from_utf16(&[0xde00]));
        assert!(host.append_child(pair, high));
        assert!(host.append_child(pair, low));
        assert_eq!(serialize_native_handle(&host, pair), "<pair>😀</pair>");
    }

    #[test]
    fn xml_namespace_prefixes_distinguish_unpaired_units_from_replacement_characters() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(None, None, "root");
        for (prefix, unit) in [("p", 0xd800), ("q", 0xd801)] {
            let mut units = "urn:".encode_utf16().collect::<Vec<_>>();
            units.push(unit);
            host.set_attribute_ns_utf16_units_mutation_outcome(
                root,
                Some(XMLNS_NAMESPACE),
                Some("xmlns"),
                prefix,
                "urn:�",
                units,
            );
        }
        assert!(host.set_attribute_ns(root, Some("urn:�"), None, "value", "x"));
        let mut expected = "<root xmlns:p=\"urn:".encode_utf16().collect::<Vec<_>>();
        expected.push(0xd800);
        expected.extend("\" xmlns:q=\"urn:".encode_utf16());
        expected.push(0xd801);
        expected.extend("\" xmlns:ns1=\"urn:�\" ns1:value=\"x\"/>".encode_utf16());
        assert_eq!(serialize_native_handle_utf16(&host, root), expected);
    }

    #[test]
    fn xml_serializer_escapes_text_with_html_escape_crate() {
        let mut markup = XmlMarkup::default();
        markup.escaped_units("a > b && a < c".encode_utf16(), false);
        assert_eq!(
            String::from_utf16(&markup.units).unwrap(),
            "a &gt; b &amp;&amp; a &lt; c"
        );
    }

    #[test]
    fn xml_serializer_escapes_double_quoted_attributes() {
        let mut markup = XmlMarkup::default();
        markup.escaped_units("a \"quoted\" > b && a < c".encode_utf16(), true);
        assert_eq!(
            String::from_utf16(&markup.units).unwrap(),
            "a &quot;quoted&quot; &gt; b &amp;&amp; a &lt; c"
        );
    }

    #[test]
    fn xml_serializer_reuses_the_nearest_namespace_prefix() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(None, None, "root");
        let child = host.create_element_with_parts(None, None, "child");
        let child2 = host.create_element_with_parts(Some("u1"), None, "child2");
        let grandchild = host.create_element_with_parts(Some("u1"), None, "grandchild");
        assert!(host.set_attribute_ns(root, Some(XMLNS_NAMESPACE), Some("xmlns"), "p1", "u1"));
        assert!(host.set_attribute_ns(child, Some(XMLNS_NAMESPACE), Some("xmlns"), "p2", "u1"));
        assert!(host.set_attribute_ns(child2, Some("u1"), None, "name", "v"));
        assert!(host.append_child(root, child));
        assert!(host.append_child(child, child2));
        assert!(host.append_child(child2, grandchild));

        assert_eq!(
            serialize_native_handle(&host, root),
            concat!(
                "<root xmlns:p1=\"u1\"><child xmlns:p2=\"u1\">",
                "<p2:child2 p2:name=\"v\"><p2:grandchild/>",
                "</p2:child2></child></root>"
            )
        );
    }

    #[test]
    fn xml_serializer_generates_prefixes_for_local_conflicts() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(Some("uri1"), Some("p"), "root");
        assert!(host.set_attribute_ns(root, Some(XMLNS_NAMESPACE), Some("xmlns"), "p", "uri2"));
        assert!(host.set_attribute_ns(root, Some("uri3"), Some("p"), "name", "v"));

        assert_eq!(
            serialize_native_handle(&host, root),
            concat!(
                "<ns1:root xmlns:ns1=\"uri1\" xmlns:p=\"uri2\" ",
                "xmlns:ns2=\"uri3\" ns2:name=\"v\"/>"
            )
        );
    }

    #[test]
    fn xml_serializer_uses_effective_bindings_without_leaking_sibling_scope() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(None, None, "root");
        let child = host.create_element_with_parts(Some("urn:a"), None, "child");
        let sibling = host.create_element_with_parts(Some("urn:a"), None, "sibling");
        for prefix in ["p", "q"] {
            assert!(host.set_attribute_ns(
                root,
                Some(XMLNS_NAMESPACE),
                Some("xmlns"),
                prefix,
                "urn:a"
            ));
        }
        assert!(host.set_attribute_ns(child, Some(XMLNS_NAMESPACE), Some("xmlns"), "q", "urn:b"));
        assert!(host.set_attribute_ns(child, Some("urn:a"), None, "v", "value"));
        assert!(host.append_child(root, child));
        assert!(host.append_child(root, sibling));
        assert_eq!(
            serialize_native_handle(&host, root),
            concat!(
                "<root xmlns:p=\"urn:a\" xmlns:q=\"urn:a\">",
                "<p:child xmlns:q=\"urn:b\" p:v=\"value\"/><q:sibling/></root>"
            ),
        );
    }

    #[test]
    fn xml_serializer_preserves_unbound_attribute_prefixes_and_reserves_them() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(None, None, "root");
        assert!(host.set_attribute_ns(root, Some("urn:a"), Some("p"), "a", "one"));
        assert!(host.set_attribute_ns(root, Some("urn:b"), Some("p"), "b", "two"));
        assert!(host.set_attribute_ns(root, Some("urn:c"), Some("ns1"), "c", "three"));
        assert_eq!(
            serialize_native_handle(&host, root),
            concat!(
                "<root xmlns:p=\"urn:a\" p:a=\"one\" xmlns:ns1=\"urn:b\" ns1:b=\"two\" ",
                "xmlns:ns2=\"urn:c\" ns2:c=\"three\"/>"
            ),
        );
    }

    #[test]
    fn xml_serializer_generated_prefixes_skip_existing_and_synthesized_declarations() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(Some("urn:element"), Some("p"), "root");
        assert!(host.set_attribute_ns(
            root,
            Some(XMLNS_NAMESPACE),
            Some("xmlns"),
            "p",
            "urn:local"
        ));
        assert!(host.set_attribute_ns(
            root,
            Some(XMLNS_NAMESPACE),
            Some("xmlns"),
            "ns1",
            "urn:reserved"
        ));
        assert!(host.set_attribute_ns(root, Some("urn:attribute"), Some("ns2"), "v", "text"));
        assert_eq!(
            serialize_native_handle(&host, root),
            concat!(
                "<ns2:root xmlns:ns2=\"urn:element\" xmlns:p=\"urn:local\" ",
                "xmlns:ns1=\"urn:reserved\" xmlns:ns3=\"urn:attribute\" ns3:v=\"text\"/>"
            ),
        );
        assert_eq!(
            serialize_native_handle_well_formed(&host, root).unwrap(),
            serialize_native_handle_utf16(&host, root),
        );
    }

    #[test]
    fn xml_serializer_xlink_attributes_cannot_rebind_the_element_prefix() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(Some("urn:element"), Some("p"), "root");
        assert!(host.set_attribute_ns(
            root,
            Some("http://www.w3.org/1999/xlink"),
            Some("p"),
            "href",
            "target"
        ));
        assert_eq!(
            serialize_native_handle(&host, root),
            concat!(
                "<p:root xmlns:p=\"urn:element\" ",
                "xmlns:ns1=\"http://www.w3.org/1999/xlink\" ns1:href=\"target\"/>"
            ),
        );
    }

    #[test]
    fn xml_serializer_reconciles_default_namespace_declarations() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(Some("u1"), None, "root");
        let child = host.create_element_with_parts(None, None, "child");
        let sibling = host.create_element_with_parts(Some("u1"), None, "sibling");
        assert!(host.set_attribute_ns(root, Some(XMLNS_NAMESPACE), None, "xmlns", "u1"));
        assert!(host.set_attribute(child, "xmlns", "FAIL"));
        assert!(host.set_attribute_ns(sibling, Some(XMLNS_NAMESPACE), None, "xmlns", "FAIL"));
        assert!(host.append_child(root, child));
        assert!(host.append_child(root, sibling));

        assert_eq!(
            serialize_native_handle(&host, root),
            "<root xmlns=\"u1\"><child xmlns=\"\" xmlns=\"FAIL\"/><sibling/></root>"
        );

        let empty = host.create_element_with_parts(None, None, "empty");
        assert!(host.set_attribute_ns(empty, Some(XMLNS_NAMESPACE), None, "xmlns", ""));
        assert!(host.set_attribute_ns(empty, Some(XMLNS_NAMESPACE), Some("xmlns"), "p", ""));
        assert_eq!(
            serialize_native_handle(&host, empty),
            "<empty xmlns=\"\" xmlns:p=\"\"/>"
        );
    }

    #[test]
    fn xml_serializer_preserves_ordinary_xmlns_utf16_values() {
        let mut host = xml_host();
        for units in [vec![0xd800], vec![0xdc00], vec![0xd83d, 0xde00]] {
            let root = host.create_element_with_parts(None, None, "root");
            let value = DomStringValue::from_utf16(&units);
            assert!(host.set_attribute_utf16_units(
                root,
                "xmlns",
                value.as_str_lossy(),
                units.clone()
            ));
            let mut expected = "<root xmlns=\"".encode_utf16().collect::<Vec<_>>();
            expected.extend_from_slice(&units);
            expected.extend("\"/>".encode_utf16());
            assert_eq!(serialize_native_handle_utf16(&host, root), expected);
            assert!(serialize_native_handle_well_formed(&host, root).is_err());
        }
    }

    #[test]
    fn ordinary_xmlns_does_not_set_the_inherited_native_namespace() {
        let mut host = xml_host();
        let root = host.create_element_with_parts(Some("urn:element"), Some("p"), "root");
        let child = host.create_element_with_parts(None, None, "child");
        assert!(host.set_attribute(root, "xmlns", "urn:ordinary"));
        assert!(host.append_child(root, child));
        assert_eq!(
            serialize_native_handle(&host, root),
            "<p:root xmlns:p=\"urn:element\" xmlns=\"urn:ordinary\"><child/></p:root>",
        );
        assert!(serialize_native_handle_well_formed(&host, root).is_err());

        // A real default declaration sets the context for a null-namespace
        // child, which therefore needs a generated namespace reset.
        let declared = host.create_element_with_parts(Some("urn:element"), Some("p"), "root");
        let declared_child = host.create_element_with_parts(None, None, "child");
        assert!(host.set_attribute_ns(
            declared,
            Some(XMLNS_NAMESPACE),
            None,
            "xmlns",
            "urn:declared",
        ));
        assert!(host.append_child(declared, declared_child));
        assert_eq!(
            serialize_native_handle(&host, declared),
            "<p:root xmlns:p=\"urn:element\" xmlns=\"urn:declared\"><child xmlns=\"\"/></p:root>",
        );
        assert_eq!(
            serialize_native_handle_well_formed(&host, declared).unwrap(),
            serialize_native_handle_utf16(&host, declared),
        );
    }
}
