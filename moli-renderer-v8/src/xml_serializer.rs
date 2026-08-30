use std::collections::HashMap;

use crate::{
    dom::native::{DomHost, NativeNodeId, NodeType},
    native_bridge::node_runtime_and_handle_from_object_or_detached,
};

use super::util::{throw_type_error, v8_string, v8str};

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
    prefixes: HashMap<String, String>,
}

impl Default for NamespaceContext {
    fn default() -> Self {
        Self {
            default_namespace: String::new(),
            prefixes: HashMap::from([
                ("xml".to_owned(), XML_NAMESPACE.to_owned()),
                ("xmlns".to_owned(), XMLNS_NAMESPACE.to_owned()),
            ]),
        }
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

    for attribute in element.attributes() {
        if attribute.namespace() != XMLNS_NAMESPACE {
            continue;
        }
        if attribute.prefix() == Some("xmlns") {
            namespace_context.prefixes.insert(
                attribute.local_name().to_owned(),
                attribute.value().to_owned(),
            );
        } else if attribute.local_name() == "xmlns" {
            namespace_context.default_namespace = attribute.value().to_owned();
        }
    }

    let mut serialized_attributes = Vec::<String>::new();
    let element_prefix = if let Some(prefix) = original_prefix {
        if namespace_context.prefixes.get(prefix).map(String::as_str) != Some(namespace) {
            serialized_attributes.push(format!(" xmlns:{prefix}=\"{}\"", escape_attr(namespace)));
            namespace_context
                .prefixes
                .insert(prefix.to_owned(), namespace.to_owned());
        }
        Some(prefix.to_owned())
    } else if namespace_context.default_namespace != namespace {
        serialized_attributes.push(format!(" xmlns=\"{}\"", escape_attr(namespace)));
        namespace_context.default_namespace = namespace.to_owned();
        None
    } else {
        None
    };

    for attribute in element.attributes() {
        if attribute.namespace() == XMLNS_NAMESPACE {
            serialized_attributes.push(format!(
                " {}=\"{}\"",
                attribute.name(),
                escape_attr(attribute.value())
            ));
            continue;
        }

        let attribute_name = if attribute.namespace().is_empty() {
            attribute.name()
        } else if let Some(prefix) = attribute.prefix().filter(|prefix| !prefix.is_empty()) {
            if prefix != "xml"
                && namespace_context.prefixes.get(prefix).map(String::as_str)
                    != Some(attribute.namespace())
            {
                serialized_attributes.push(format!(
                    " xmlns:{prefix}=\"{}\"",
                    escape_attr(attribute.namespace())
                ));
                namespace_context
                    .prefixes
                    .insert(prefix.to_owned(), attribute.namespace().to_owned());
            }
            format!("{prefix}:{}", attribute.local_name())
        } else {
            let prefix = next_available_namespace_prefix(&namespace_context, next_generated_prefix);
            serialized_attributes.push(format!(
                " xmlns:{prefix}=\"{}\"",
                escape_attr(attribute.namespace())
            ));
            namespace_context
                .prefixes
                .insert(prefix.clone(), attribute.namespace().to_owned());
            format!("{prefix}:{}", attribute.local_name())
        };
        serialized_attributes.push(format!(
            " {attribute_name}=\"{}\"",
            escape_attr(attribute.value())
        ));
    }

    let tag = element_prefix
        .map(|prefix| format!("{prefix}:{local_name}"))
        .unwrap_or_else(|| local_name.to_owned());
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

fn next_available_namespace_prefix(
    namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
) -> String {
    loop {
        let prefix = format!("ns{next_generated_prefix}");
        *next_generated_prefix += 1;
        if !namespace_context.prefixes.contains_key(&prefix) {
            return prefix;
        }
    }
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
    use super::{escape_attr, escape_text};

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
}
