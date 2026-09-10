use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(unbranded, interface = "Document")]
struct DetachedDocumentStateAccessorsDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = detached_document_implementation_getter
    )]
    implementation: (),
    #[webapi(accessor_property, enumerable, getter = detached_document_fonts_getter)]
    fonts: (),
}

#[derive(WebApiObject)]
#[webapi(unbranded, interface = "HTMLDocument")]
struct DetachedHtmlDocumentCreationMethodsDeclaration {
    #[webapi(method, callback = detached_create_html_element_method_callback)]
    create_element: (),

    #[webapi(
        method = "createElementNS",
        callback = detached_create_html_element_ns_method_callback
    )]
    create_element_ns: (),

    #[webapi(
        method = "createCDATASection",
        callback = detached_create_cdata_section_html_method_callback
    )]
    create_cdata_section: (),

    #[webapi(method, callback = detached_html_document_write_method_callback)]
    write: (),
}

#[derive(WebApiObject)]
#[webapi(unbranded, interface = "XMLDocument")]
struct DetachedXmlDocumentCreationMethodsDeclaration {
    #[webapi(method, callback = detached_create_xml_element_method_callback)]
    create_element: (),

    #[webapi(
        method = "createElementNS",
        callback = detached_create_xml_element_ns_method_callback
    )]
    create_element_ns: (),

    #[webapi(
        method = "createCDATASection",
        callback = detached_create_cdata_section_method_callback
    )]
    create_cdata_section: (),
}

#[derive(WebApiObject)]
#[webapi(unbranded, interface = "Document")]
struct DetachedDocumentCommonMethodsDeclaration {
    #[webapi(method, callback = detached_create_attribute_method_callback)]
    create_attribute: (),

    #[webapi(
        method = "createAttributeNS",
        callback = detached_create_attribute_ns_method_callback
    )]
    create_attribute_ns: (),

    #[webapi(method, callback = detached_create_text_node_method_callback)]
    create_text_node: (),

    #[webapi(method, callback = detached_create_comment_method_callback)]
    create_comment: (),

    #[webapi(method, callback = detached_create_document_fragment_method_callback)]
    create_document_fragment: (),

    #[webapi(method, callback = detached_create_processing_instruction_method_callback)]
    create_processing_instruction: (),

    #[webapi(method, callback = detached_import_node_method_callback)]
    import_node: (),

    #[webapi(method, callback = detached_adopt_node_method_callback)]
    adopt_node: (),

    #[webapi(
        method = "execCommand",
        callback = crate::native_bridge::document::node_document_exec_command_callback
    )]
    exec_command: (),

    #[webapi(
        method = "queryCommandEnabled",
        length = 1,
        callback = crate::native_bridge::document::node_document_query_command_enabled_callback
    )]
    query_command_enabled: (),

    #[webapi(
        method = "queryCommandIndeterm",
        length = 1,
        callback = crate::native_bridge::document::node_document_query_command_indeterm_callback
    )]
    query_command_indeterm: (),

    #[webapi(
        method = "queryCommandState",
        length = 1,
        callback = crate::native_bridge::document::node_document_query_command_state_callback
    )]
    query_command_state: (),

    #[webapi(
        method = "queryCommandSupported",
        length = 1,
        callback = crate::native_bridge::document::node_document_query_command_supported_callback
    )]
    query_command_supported: (),

    #[webapi(
        method = "queryCommandValue",
        length = 1,
        callback = crate::native_bridge::document::node_document_query_command_value_callback
    )]
    query_command_value: (),

    #[webapi(method, callback = detached_get_element_by_id_method_callback)]
    get_element_by_id: (),

    #[webapi(method, callback = detached_get_elements_by_tag_name_method_callback)]
    get_elements_by_tag_name: (),

    #[webapi(
        method = "getElementsByTagNameNS",
        callback = detached_get_elements_by_tag_name_ns_method_callback
    )]
    get_elements_by_tag_name_ns: (),

    #[webapi(method, callback = detached_get_elements_by_class_name_method_callback)]
    get_elements_by_class_name: (),

    #[webapi(method, callback = detached_get_elements_by_name_method_callback)]
    get_elements_by_name: (),

    #[webapi(method, callback = detached_query_selector_method_callback)]
    query_selector: (),

    #[webapi(method, callback = detached_query_selector_all_method_callback)]
    query_selector_all: (),

    #[webapi(method, callback = detached_get_selection_method_callback)]
    get_selection: (),

    #[webapi(method, callback = detached_create_node_iterator_method_callback)]
    create_node_iterator: (),
}

pub(super) fn install_detached_document_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    html_document_prototype: v8::Local<'s, v8::Object>,
    xml_document_prototype: v8::Local<'s, v8::Object>,
    plain_document_prototype: Option<v8::Local<'s, v8::Object>>,
) {
    install_detached_document_creation_methods(
        scope,
        html_document_prototype,
        xml_document_prototype,
        plain_document_prototype,
    );
    install_detached_document_state_accessors(
        scope,
        html_document_prototype,
        xml_document_prototype,
        plain_document_prototype,
    );
    let document_prototypes: Vec<v8::Local<'s, v8::Object>> = [
        Some(html_document_prototype),
        Some(xml_document_prototype),
        plain_document_prototype,
    ]
    .into_iter()
    .flatten()
    .collect();
    install_detached_document_common_methods(scope, &document_prototypes);
}

fn install_detached_document_creation_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    html_document_prototype: v8::Local<'s, v8::Object>,
    xml_document_prototype: v8::Local<'s, v8::Object>,
    plain_document_prototype: Option<v8::Local<'s, v8::Object>>,
) {
    DetachedHtmlDocumentCreationMethodsDeclaration::new()
        .initialize(scope, html_document_prototype)
        .expect("detached HTMLDocument creation method declaration should initialize");
    DetachedXmlDocumentCreationMethodsDeclaration::new()
        .initialize(scope, xml_document_prototype)
        .expect("detached XMLDocument creation method declaration should initialize");
    if let Some(plain_doc_proto) = plain_document_prototype {
        DetachedXmlDocumentCreationMethodsDeclaration::new()
            .initialize(scope, plain_doc_proto)
            .expect("detached Document creation method declaration should initialize");
    }
}

fn detached_html_document_write_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let mut html = String::new();
    for index in 0..args.length() {
        if let Some(chunk) = callback_arg_string(scope, &args, index) {
            html.push_str(&chunk);
        }
    }
    let Some(body_value) = args.this().get(scope, v8str(scope, "body").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(body) = v8::Local::<v8::Object>::try_from(body_value) else {
        rv.set_undefined();
        return;
    };
    if let Some(value) = v8_string(scope, &html) {
        let _ = body.set(scope, v8str(scope, "innerHTML").into(), value.into());
    }
    rv.set_undefined();
}

fn install_detached_document_state_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    html_document_prototype: v8::Local<'s, v8::Object>,
    xml_document_prototype: v8::Local<'s, v8::Object>,
    plain_document_prototype: Option<v8::Local<'s, v8::Object>>,
) {
    for prototype in [
        Some(html_document_prototype),
        Some(xml_document_prototype),
        plain_document_prototype,
    ]
    .into_iter()
    .flatten()
    {
        DetachedDocumentStateAccessorsDeclaration::new()
            .initialize(scope, prototype)
            .expect("detached Document state accessor declaration should initialize");
    }
}

fn install_detached_document_common_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_prototypes: &[v8::Local<'s, v8::Object>],
) {
    for prototype in document_prototypes {
        DetachedDocumentCommonMethodsDeclaration::new()
            .initialize(scope, *prototype)
            .expect("detached Document common method declaration should initialize");
    }
}

fn detached_get_selection_method_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(default_view) = args.this().get(scope, v8str(scope, "defaultView").into()) else {
        return;
    };
    if default_view.is_null_or_undefined() {
        rv.set(v8::null(scope).into());
        return;
    }
    let Ok(window) = v8::Local::<v8::Object>::try_from(default_view) else {
        rv.set(v8::null(scope).into());
        return;
    };
    match crate::context_bootstrap::selection_value_for_window(scope, window) {
        Some(selection) => rv.set(selection.into()),
        None => rv.set(v8::null(scope).into()),
    }
}
