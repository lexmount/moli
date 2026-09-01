use std::collections::HashMap;

use crate::{
    dom::native::{DomHost, Element, NativeNodeId, NodeType},
    native_bridge::node_runtime_and_handle_from_object_or_detached,
};

use super::util::{throw_type_error, v8_string, v8str};

const VOID_HTML: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "menuitem", "meta", "param", "source", "track", "wbr",
];
const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
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

pub(super) fn xml_serializer_serialize_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let serialized =
        serialize_native_value(scope, value).unwrap_or_else(|| serialize_value(scope, value));
    if let Some(serialized) = v8_string(scope, &serialized) {
        rv.set(serialized.into());
    } else {
        rv.set(v8::String::empty(scope).into());
    }
}

#[derive(Clone, Debug)]
struct NamespaceContext {
    default_namespace: String,
    prefixes: HashMap<String, Vec<String>>,
}

impl Default for NamespaceContext {
    fn default() -> Self {
        Self {
            default_namespace: String::new(),
            prefixes: HashMap::from([(XML_NAMESPACE.to_owned(), vec!["xml".to_owned()])]),
        }
    }
}

impl NamespaceContext {
    fn add_prefix(&mut self, namespace: &str, prefix: &str) {
        self.prefixes
            .entry(namespace.to_owned())
            .or_default()
            .push(prefix.to_owned());
    }

    fn preferred_prefix(&self, namespace: &str, preferred: Option<&str>) -> Option<String> {
        let candidates = self.prefixes.get(namespace)?;
        if let Some(preferred) = preferred
            && candidates.iter().any(|candidate| candidate == preferred)
        {
            return Some(preferred.to_owned());
        }
        candidates.last().cloned()
    }

    fn contains_prefix(&self, namespace: &str, prefix: &str) -> bool {
        self.prefixes
            .get(namespace)
            .is_some_and(|prefixes| prefixes.iter().any(|candidate| candidate == prefix))
    }
}

fn serialize_native_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, object).ok()?;
    // SAFETY: the node bridge only returns the context host installed for this
    // live V8 callback, and serialization holds no reference past the callback.
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    Some(serialize_native_handle(dom_host, handle))
}

pub(crate) fn serialize_native_handle(dom_host: &DomHost, handle: NativeNodeId) -> String {
    let mut next_generated_prefix = 1;
    serialize_native_node(
        dom_host,
        handle,
        &NamespaceContext::default(),
        &mut next_generated_prefix,
    )
}

pub(crate) fn serialize_native_inner_html(
    dom_host: &DomHost,
    handle: NativeNodeId,
) -> Option<String> {
    let child_container = dom_host
        .node(handle)?
        .as_element()
        .and_then(Element::template_contents)
        .unwrap_or(handle);
    let mut next_generated_prefix = 1;
    Some(serialize_native_children(
        dom_host,
        child_container,
        &NamespaceContext::default(),
        &mut next_generated_prefix,
    ))
}

fn serialize_native_node(
    dom_host: &DomHost,
    handle: NativeNodeId,
    namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
) -> String {
    let Some(node) = dom_host.node(handle) else {
        return String::new();
    };
    match node.node_type() {
        NodeType::Element => {
            serialize_native_element(dom_host, handle, namespace_context, next_generated_prefix)
        }
        NodeType::Text => escape_text(node.data_value().unwrap_or_default()),
        NodeType::CDataSection => {
            format!("<![CDATA[{}]]>", node.data_value().unwrap_or_default())
        }
        NodeType::ProcessingInstruction => {
            let target = node.target().unwrap_or_default();
            let data = node.data_value().unwrap_or_default();
            if data.is_empty() {
                format!("<?{target}?>")
            } else {
                format!("<?{target} {data}?>")
            }
        }
        NodeType::Comment => format!("<!--{}-->", node.data_value().unwrap_or_default()),
        NodeType::Document | NodeType::DocumentFragment => {
            serialize_native_children(dom_host, handle, namespace_context, next_generated_prefix)
        }
        NodeType::DocumentType => {
            let Some(doctype) = node.as_document_type() else {
                return String::new();
            };
            if !doctype.public_id().is_empty() {
                format!(
                    "<!DOCTYPE {} PUBLIC \"{}\" \"{}\">",
                    doctype.name(),
                    doctype.public_id(),
                    doctype.system_id()
                )
            } else if !doctype.system_id().is_empty() {
                format!(
                    "<!DOCTYPE {} SYSTEM \"{}\">",
                    doctype.name(),
                    doctype.system_id()
                )
            } else {
                format!("<!DOCTYPE {}>", doctype.name())
            }
        }
    }
}

fn serialize_native_children(
    dom_host: &DomHost,
    handle: NativeNodeId,
    namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
) -> String {
    dom_host
        .child_handles(handle)
        .map(|child| {
            serialize_native_node(dom_host, child, namespace_context, next_generated_prefix)
        })
        .collect::<Vec<_>>()
        .join("")
}

fn serialize_native_element(
    dom_host: &DomHost,
    handle: NativeNodeId,
    parent_namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
) -> String {
    let Some(element) = dom_host.node(handle).and_then(|node| node.as_element()) else {
        return String::new();
    };
    let namespace = element.namespace();
    let original_prefix = element.prefix().filter(|prefix| !prefix.is_empty());
    let local_name = element.local_name();
    let mut namespace_context = parent_namespace_context.clone();
    let (local_default_namespace, local_prefixes) =
        record_namespace_information(element, &mut namespace_context);
    let mut inherited_namespace = parent_namespace_context.default_namespace.clone();
    let mut ignore_namespace_definition_attribute = false;
    let mut serialized_attributes = Vec::<String>::new();

    let tag = if inherited_namespace == namespace {
        if local_default_namespace.is_some()
            && !local_prefixes
                .values()
                .any(|namespace| namespace.is_empty())
        {
            ignore_namespace_definition_attribute = true;
        }
        if namespace == XML_NAMESPACE {
            format!("xml:{local_name}")
        } else {
            local_name.to_owned()
        }
    } else {
        let candidate_prefix = namespace_context.preferred_prefix(namespace, original_prefix);
        if let Some(candidate_prefix) = candidate_prefix {
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
            serialized_attributes.push(format!(" xmlns:{prefix}=\"{}\"", escape_attr(namespace)));
            if let Some(local_default_namespace) = local_default_namespace.as_deref() {
                inherited_namespace = local_default_namespace.to_owned();
            }
            format!("{prefix}:{local_name}")
        } else if local_default_namespace.as_deref() != Some(namespace) {
            ignore_namespace_definition_attribute = true;
            inherited_namespace = namespace.to_owned();
            serialized_attributes.push(format!(" xmlns=\"{}\"", escape_attr(namespace)));
            local_name.to_owned()
        } else {
            inherited_namespace = namespace.to_owned();
            local_name.to_owned()
        }
    };

    serialized_attributes.extend(serialize_native_attributes(
        element,
        &mut namespace_context,
        next_generated_prefix,
        &local_prefixes,
        ignore_namespace_definition_attribute,
    ));
    namespace_context.default_namespace = inherited_namespace;
    let child_handle = element.template_contents().unwrap_or(handle);
    let has_children = dom_host.child_handles(child_handle).next().is_some();
    let open = format!("<{tag}{}", serialized_attributes.join(""));
    if !has_children && namespace == HTML_NAMESPACE && VOID_HTML.contains(&local_name) {
        return format!("{open} />");
    }
    if !has_children && namespace != HTML_NAMESPACE {
        return format!("{open}/>");
    }
    let open = format!("{open}>");
    format!(
        "{open}{}</{tag}>",
        serialize_native_children(
            dom_host,
            child_handle,
            &namespace_context,
            next_generated_prefix,
        )
    )
}

fn record_namespace_information(
    element: &Element,
    namespace_context: &mut NamespaceContext,
) -> (Option<String>, HashMap<String, String>) {
    let mut local_default_namespace = None;
    let mut local_prefixes = HashMap::new();
    for attribute in element.attributes() {
        if attribute.prefix().is_none()
            && attribute.local_name() == "xmlns"
            && matches!(attribute.namespace(), "" | XMLNS_NAMESPACE)
        {
            local_default_namespace = Some(attribute.value().to_owned());
            continue;
        }
        if attribute.namespace() != XMLNS_NAMESPACE {
            continue;
        }
        let Some(attribute_prefix) = attribute.prefix() else {
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
    ignore_namespace_definition_attribute: bool,
) -> Vec<String> {
    let mut serialized = Vec::new();
    for attribute in element.attributes() {
        let attribute_namespace = attribute.namespace();
        let is_default_namespace_declaration = attribute.prefix().is_none()
            && attribute.local_name() == "xmlns"
            && matches!(attribute_namespace, "" | XMLNS_NAMESPACE);
        let mut candidate_prefix = (!attribute_namespace.is_empty())
            .then(|| namespace_context.preferred_prefix(attribute_namespace, attribute.prefix()))
            .flatten();

        if is_default_namespace_declaration && ignore_namespace_definition_attribute {
            continue;
        }
        if attribute_namespace == XMLNS_NAMESPACE {
            if attribute.value() == XML_NAMESPACE {
                continue;
            }
            if attribute.prefix().is_some() {
                let local_namespace = local_prefixes.get(attribute.local_name());
                if local_namespace.is_none()
                    || (local_namespace.is_some_and(|namespace| namespace != attribute.value())
                        && namespace_context
                            .contains_prefix(attribute.value(), attribute.local_name()))
                {
                    continue;
                }
            }
            if attribute.prefix() == Some("xmlns") {
                candidate_prefix = Some("xmlns".to_owned());
            }
        } else if attribute_namespace == XLINK_NAMESPACE
            && candidate_prefix.is_none()
            && let Some(prefix) = attribute.prefix().filter(|prefix| !prefix.is_empty())
        {
            // XML serialization preserves an explicitly supplied XLink prefix here;
            // unlike HTML serialization, it must not force the canonical `xlink` prefix.
            namespace_context.add_prefix(attribute_namespace, prefix);
            serialized.push(format!(
                " xmlns:{prefix}=\"{}\"",
                escape_attr(attribute_namespace)
            ));
            candidate_prefix = Some(prefix.to_owned());
        } else if !attribute_namespace.is_empty() && candidate_prefix.is_none() {
            let prefix = generate_namespace_prefix(
                namespace_context,
                attribute_namespace,
                next_generated_prefix,
            );
            serialized.push(format!(
                " xmlns:{prefix}=\"{}\"",
                escape_attr(attribute_namespace)
            ));
            candidate_prefix = Some(prefix);
        }

        let attribute_name = candidate_prefix
            .map(|prefix| format!("{prefix}:{}", attribute.local_name()))
            .unwrap_or_else(|| attribute.local_name().to_owned());
        serialized.push(format!(
            " {attribute_name}=\"{}\"",
            escape_attr(attribute.value())
        ));
    }
    serialized
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

fn serialize_value(scope: &mut v8::PinScope<'_, '_>, value: v8::Local<'_, v8::Value>) -> String {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return String::new();
    };
    match node_type(scope, object) {
        1 => serialize_element(scope, object),
        2 => escape_attr(&string_property(scope, object, "value").unwrap_or_default()),
        3 => escape_text(&string_property(scope, object, "data").unwrap_or_default()),
        7 => serialize_processing_instruction(scope, object),
        8 => format!(
            "<!--{}-->",
            string_property(scope, object, "data").unwrap_or_default()
        ),
        9 | 11 => serialize_children(scope, object),
        10 => serialize_document_type(scope, object),
        _ => String::new(),
    }
}

fn serialize_element(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> String {
    let tag = string_property(scope, object, "tagName")
        .or_else(|| string_property(scope, object, "nodeName"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let attrs = attribute_names(scope, object)
        .into_iter()
        .map(|name| {
            let value = attribute_value(scope, object, &name).unwrap_or_default();
            format!(" {}=\"{}\"", name, escape_attr(&value))
        })
        .collect::<Vec<_>>()
        .join("");
    let open = format!("<{tag}{attrs}>");
    if VOID_HTML.contains(&tag.as_str()) {
        return open;
    }
    format!("{open}{}{}</{tag}>", serialize_children(scope, object), "")
}

fn serialize_processing_instruction(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> String {
    let target = string_property(scope, object, "target").unwrap_or_default();
    let data = string_property(scope, object, "data").unwrap_or_default();
    if data.is_empty() {
        format!("<?{target}?>")
    } else {
        format!("<?{target} {data}?>")
    }
}

fn serialize_children(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> String {
    child_values(scope, object)
        .into_iter()
        .map(|value| serialize_value(scope, value))
        .collect::<Vec<_>>()
        .join("")
}

fn serialize_document_type(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> String {
    let name = string_property(scope, object, "name")
        .or_else(|| string_property(scope, object, "nodeName"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let public_id = string_property(scope, object, "publicId").unwrap_or_default();
    let system_id = string_property(scope, object, "systemId").unwrap_or_default();
    if !public_id.is_empty() {
        format!("<!DOCTYPE {name} PUBLIC \"{public_id}\" \"{system_id}\">")
    } else if !system_id.is_empty() {
        format!("<!DOCTYPE {name} SYSTEM \"{system_id}\">")
    } else {
        format!("<!DOCTYPE {name}>")
    }
}

fn node_type(scope: &mut v8::PinScope<'_, '_>, object: v8::Local<'_, v8::Object>) -> i32 {
    object
        .get(scope, v8str(scope, "nodeType").into())
        .and_then(|value| value.int32_value(scope))
        .unwrap_or(0)
}

fn string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<String> {
    let key = v8_string(scope, key)?;
    let value = object.get(scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn child_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<v8::Local<'s, v8::Value>> {
    let Some(children) = object.get(scope, v8str(scope, "childNodes").into()) else {
        return Vec::new();
    };
    if let Ok(array) = v8::Local::<v8::Array>::try_from(children) {
        let mut values = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            if let Some(value) = array.get_index(scope, index) {
                values.push(value);
            }
        }
        return values;
    }
    let Some(children_obj) = children.to_object(scope) else {
        return Vec::new();
    };
    let length = children_obj
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let mut values = Vec::with_capacity(length as usize);
    for index in 0..length {
        if let Some(value) = children_obj.get_index(scope, index) {
            values.push(value);
        }
    }
    values
}

fn attribute_names(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<String> {
    let Some(get_attribute_names) = object.get(scope, v8str(scope, "getAttributeNames").into())
    else {
        return Vec::new();
    };
    let Ok(get_attribute_names) = v8::Local::<v8::Function>::try_from(get_attribute_names) else {
        return Vec::new();
    };
    let Some(result) = get_attribute_names.call(scope, object.into(), &[]) else {
        return Vec::new();
    };
    let Ok(array) = v8::Local::<v8::Array>::try_from(result) else {
        return Vec::new();
    };
    let mut names = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        if let Some(value) = array
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
        {
            names.push(value.to_rust_string_lossy(scope));
        }
    }
    names
}

fn attribute_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Option<String> {
    let get_attribute = object.get(scope, v8str(scope, "getAttribute").into())?;
    let get_attribute = v8::Local::<v8::Function>::try_from(get_attribute).ok()?;
    let name = v8_string(scope, name)?;
    let value = get_attribute.call(scope, object.into(), &[name.into()])?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn escape_text(value: &str) -> String {
    html_escape::encode_text(value).into_owned()
}

fn escape_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\t' => escaped.push_str("&#9;"),
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use crate::dom::native::{DomHost, NativeDom};

    use super::{XMLNS_NAMESPACE, escape_attr, escape_text, serialize_native_handle};

    fn xml_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_xml(
            url::Url::parse("https://xml-serializer.test/").unwrap(),
        ))
    }

    #[test]
    fn xml_serializer_escapes_text_with_html_escape_crate() {
        assert_eq!(
            escape_text("a > b && a < c"),
            "a &gt; b &amp;&amp; a &lt; c"
        );
    }

    #[test]
    fn xml_serializer_escapes_double_quoted_attributes() {
        assert_eq!(
            escape_attr("a \"quoted\" > b && a < c"),
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
            "<root xmlns=\"u1\"><child xmlns=\"\"/><sibling/></root>"
        );

        let empty = host.create_element_with_parts(None, None, "empty");
        assert!(host.set_attribute_ns(empty, Some(XMLNS_NAMESPACE), None, "xmlns", ""));
        assert!(host.set_attribute_ns(empty, Some(XMLNS_NAMESPACE), Some("xmlns"), "p", ""));
        assert_eq!(
            serialize_native_handle(&host, empty),
            "<empty xmlns=\"\" xmlns:p=\"\"/>"
        );
    }
}
