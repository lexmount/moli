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

impl NamespaceContext {
    fn preferred_prefix(&self, namespace: &str, preferred: Option<&str>) -> Option<String> {
        if let Some(prefix) = preferred
            && self.prefixes.get(prefix).map(String::as_str) == Some(namespace)
        {
            return Some(prefix.to_owned());
        }
        if let Some(prefix) = self
            .prefixes
            .iter()
            .filter(|(prefix, bound)| prefix.as_str() != "xmlns" && bound.as_str() == namespace)
            .map(|(prefix, _)| prefix)
            .min()
        {
            return Some(prefix.clone());
        }
        preferred
            .filter(|prefix| !self.prefixes.contains_key(*prefix))
            .map(str::to_owned)
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
    let mut output = String::new();
    let mut next_generated_prefix = 1;
    serialize_native_node(
        dom_host,
        handle,
        &NamespaceContext::default(),
        &mut next_generated_prefix,
        &mut output,
    );
    output
}

fn serialize_native_node(
    dom_host: &DomHost,
    handle: NativeNodeId,
    namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
    output: &mut String,
) {
    let Some(node) = dom_host.node(handle) else {
        return;
    };
    match node.node_type() {
        NodeType::Element => serialize_native_element(
            dom_host,
            handle,
            namespace_context,
            next_generated_prefix,
            output,
        ),
        NodeType::Text => write_escaped_text(output, node.data_value().unwrap_or_default()),
        NodeType::CDataSection => {
            output.push_str("<![CDATA[");
            output.push_str(node.data_value().unwrap_or_default());
            output.push_str("]]>");
        }
        NodeType::ProcessingInstruction => write_processing_instruction(
            output,
            node.target().unwrap_or_default(),
            node.data_value().unwrap_or_default(),
        ),
        NodeType::Comment => {
            output.push_str("<!--");
            output.push_str(node.data_value().unwrap_or_default());
            output.push_str("-->");
        }
        NodeType::Document | NodeType::DocumentFragment => serialize_native_children(
            dom_host,
            handle,
            namespace_context,
            next_generated_prefix,
            output,
        ),
        NodeType::DocumentType => {
            if let Some(doctype) = node.as_document_type() {
                write_document_type(
                    output,
                    doctype.name(),
                    doctype.public_id(),
                    doctype.system_id(),
                );
            }
        }
    }
}

fn serialize_native_children(
    dom_host: &DomHost,
    handle: NativeNodeId,
    namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
    output: &mut String,
) {
    for child in dom_host.child_handles(handle) {
        serialize_native_node(
            dom_host,
            child,
            namespace_context,
            next_generated_prefix,
            output,
        );
    }
}

fn serialize_native_element(
    dom_host: &DomHost,
    handle: NativeNodeId,
    parent_namespace_context: &NamespaceContext,
    next_generated_prefix: &mut usize,
    output: &mut String,
) {
    let Some(element) = dom_host.node(handle).and_then(|node| node.as_element()) else {
        return;
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
    let ignore_default_declaration =
        original_prefix.is_none() && namespace_context.default_namespace != namespace;
    let element_prefix = original_prefix.map(|preferred| {
        namespace_context
            .preferred_prefix(namespace, Some(preferred))
            .filter(|prefix| {
                namespace_context
                    .prefixes
                    .get(prefix)
                    .is_some_and(|bound| bound == namespace)
            })
            .unwrap_or_else(|| {
                let declaration = format!("xmlns:{preferred}");
                if element
                    .attributes()
                    .iter()
                    .any(|attribute| attribute.name_matches(&declaration))
                {
                    next_available_namespace_prefix(&namespace_context, next_generated_prefix)
                } else {
                    preferred.to_owned()
                }
            })
    });
    let tag = element_prefix
        .as_ref()
        .map(|prefix| format!("{prefix}:{local_name}"))
        .unwrap_or_else(|| local_name.to_owned());
    output.push('<');
    output.push_str(&tag);
    if let Some(prefix) = element_prefix {
        if namespace_context.prefixes.get(&prefix).map(String::as_str) != Some(namespace) {
            write_attribute(output, &format!("xmlns:{prefix}"), namespace);
            namespace_context
                .prefixes
                .insert(prefix, namespace.to_owned());
        }
    } else if namespace_context.default_namespace != namespace {
        write_attribute(output, "xmlns", namespace);
        namespace_context.default_namespace = namespace.to_owned();
    }
    for attribute in element.attributes() {
        if attribute.namespace() == XMLNS_NAMESPACE {
            if ignore_default_declaration
                && attribute.prefix().is_none()
                && attribute.local_name() == "xmlns"
            {
                continue;
            }
            write_attribute(output, &attribute.name(), attribute.value());
            continue;
        }
        let attribute_name = if attribute.namespace().is_empty() {
            attribute.name()
        } else {
            let prefix = namespace_context
                .preferred_prefix(
                    attribute.namespace(),
                    attribute.prefix().filter(|prefix| !prefix.is_empty()),
                )
                .unwrap_or_else(|| {
                    next_available_namespace_prefix(&namespace_context, next_generated_prefix)
                });
            if namespace_context.prefixes.get(&prefix).map(String::as_str)
                != Some(attribute.namespace())
            {
                write_attribute(output, &format!("xmlns:{prefix}"), attribute.namespace());
                namespace_context
                    .prefixes
                    .insert(prefix.clone(), attribute.namespace().to_owned());
            }
            format!("{prefix}:{}", attribute.local_name())
        };
        write_attribute(output, &attribute_name, attribute.value());
    }
    let child_handle = element.template_contents().unwrap_or(handle);
    let has_children = dom_host.child_handles(child_handle).next().is_some();
    if !has_children && namespace == HTML_NAMESPACE && VOID_HTML.contains(&local_name) {
        output.push_str(" />");
        return;
    }
    if !has_children && namespace != HTML_NAMESPACE {
        output.push_str("/>");
        return;
    }
    output.push('>');
    serialize_native_children(
        dom_host,
        child_handle,
        &namespace_context,
        next_generated_prefix,
        output,
    );
    output.push_str("</");
    output.push_str(&tag);
    output.push('>');
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
    let mut output = String::new();
    serialize_value_into(scope, value, &mut output);
    output
}

fn serialize_value_into(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    output: &mut String,
) {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return;
    };
    match node_type(scope, object) {
        1 => serialize_element(scope, object, output),
        2 => write_escaped_attr(
            output,
            &string_property(scope, object, "value").unwrap_or_default(),
        ),
        3 => write_escaped_text(
            output,
            &string_property(scope, object, "data").unwrap_or_default(),
        ),
        7 => write_processing_instruction(
            output,
            &string_property(scope, object, "target").unwrap_or_default(),
            &string_property(scope, object, "data").unwrap_or_default(),
        ),
        8 => {
            output.push_str("<!--");
            output.push_str(&string_property(scope, object, "data").unwrap_or_default());
            output.push_str("-->");
        }
        9 | 11 => serialize_children(scope, object, output),
        10 => {
            let name = string_property(scope, object, "name")
                .or_else(|| string_property(scope, object, "nodeName"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let public_id = string_property(scope, object, "publicId").unwrap_or_default();
            let system_id = string_property(scope, object, "systemId").unwrap_or_default();
            write_document_type(output, &name, &public_id, &system_id);
        }
        _ => {}
    }
}

fn serialize_element(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    output: &mut String,
) {
    let tag = string_property(scope, object, "tagName")
        .or_else(|| string_property(scope, object, "nodeName"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    output.push('<');
    output.push_str(&tag);
    for name in attribute_names(scope, object) {
        let value = attribute_value(scope, object, &name).unwrap_or_default();
        write_attribute(output, &name, &value);
    }
    output.push('>');
    if VOID_HTML.contains(&tag.as_str()) {
        return;
    }
    serialize_children(scope, object, output);
    output.push_str("</");
    output.push_str(&tag);
    output.push('>');
}

fn serialize_children(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    output: &mut String,
) {
    for value in child_values(scope, object) {
        serialize_value_into(scope, value, output);
    }
}

fn write_processing_instruction(output: &mut String, target: &str, data: &str) {
    output.push_str("<?");
    output.push_str(target);
    if !data.is_empty() {
        output.push(' ');
        output.push_str(data);
    }
    output.push_str("?>");
}

fn write_document_type(output: &mut String, name: &str, public_id: &str, system_id: &str) {
    output.push_str("<!DOCTYPE ");
    output.push_str(name);
    if !public_id.is_empty() {
        output.push_str(" PUBLIC \"");
        output.push_str(public_id);
        output.push_str("\" \"");
        output.push_str(system_id);
        output.push('"');
    } else if !system_id.is_empty() {
        output.push_str(" SYSTEM \"");
        output.push_str(system_id);
        output.push('"');
    }
    output.push('>');
}

fn write_attribute(output: &mut String, name: &str, value: &str) {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    write_escaped_attr(output, value);
    output.push('"');
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

fn write_escaped_text(output: &mut String, value: &str) {
    v_xmlescape::escape_string(value, output);
}

fn write_escaped_attr(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '"' => output.push_str("&quot;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\t' => output.push_str("&#9;"),
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            _ => output.push(character),
        }
    }
}

#[cfg(test)]
fn escape_text(value: &str) -> String {
    let mut output = String::new();
    write_escaped_text(&mut output, value);
    output
}

#[cfg(test)]
fn escape_attr(value: &str) -> String {
    let mut output = String::new();
    write_escaped_attr(&mut output, value);
    output
}

#[cfg(test)]
mod tests {
    use super::{escape_attr, escape_text};

    #[test]
    fn xml_serializer_escapes_text_markup_and_preserves_other_characters() {
        assert_eq!(
            escape_text("a > b && a < c"),
            "a &gt; b &amp;&amp; a &lt; c"
        );
        assert_eq!(
            escape_text("\"café\" / '尾' &amp; < >\t\n\r"),
            "\"café\" / '尾' &amp;amp; &lt; &gt;\t\n\r"
        );
    }

    #[test]
    fn xml_serializer_escapes_double_quoted_attributes() {
        assert_eq!(
            escape_attr("a \"quoted\" > b && a < c"),
            "a &quot;quoted&quot; &gt; b &amp;&amp; a &lt; c"
        );
        assert_eq!(
            escape_attr("\t\n\rcafé<&\"'尾\n\u{a0}&#10;"),
            "&#9;&#10;&#13;café&lt;&amp;&quot;'尾&#10;\u{a0}&amp;#10;"
        );
    }
}
