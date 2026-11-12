use crate::{
    dom::native::{DomHost, NativeNodeId},
    native_bridge::node_runtime_and_handle_from_object_or_detached,
    web_api_interfaces, webidl,
};

use super::util::{throw_type_error, v8_string, v8str};

const VOID_HTML: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input",
    "keygen", "link", "menuitem", "meta", "param", "source", "track", "wbr",
];

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
    let value = parsed.root.into();
    let serialized =
        serialize_native_value(scope, value).unwrap_or_else(|| serialize_value(scope, value));
    if let Some(serialized) = v8_string(scope, &serialized) {
        rv.set(serialized.into());
    } else {
        rv.set(v8::String::empty(scope).into());
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
    dom_host.serialize_xml(handle)
}

pub(crate) fn serialize_native_inner_html(
    dom_host: &DomHost,
    handle: NativeNodeId,
) -> Option<String> {
    dom_host.serialize_xml_children(handle)
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

use crate::util::object_defined_string_property as string_property;

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
    use super::{escape_attr, escape_text, serialize_native_handle};

    const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";
    use crate::dom::native::{DomHost, NativeDom};

    fn xml_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_xml(
            url::Url::parse("https://xml-serializer.test/").unwrap(),
        ))
    }

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
            "<root xmlns=\"u1\"><child xmlns=\"\" xmlns=\"FAIL\"/><sibling/></root>"
        );

        let empty = host.create_element_with_parts(None, None, "empty");
        assert!(host.set_attribute_ns(empty, Some(XMLNS_NAMESPACE), None, "xmlns", ""));
        assert!(host.set_attribute_ns(empty, Some(XMLNS_NAMESPACE), Some("xmlns"), "p", ""));
        assert_eq!(
            serialize_native_handle(&host, empty),
            "<empty xmlns:p=\"\"/>"
        );
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
    }
}
