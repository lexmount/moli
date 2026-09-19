use super::super::*;
use crate::custom_elements;
use crate::native_bridge::document::validate_registry_association_for_document;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createTextNode")]
struct DetachedDocumentCreateTextNodeArgs {
    #[webidl(index = 1, required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createComment")]
struct DetachedDocumentCreateCommentArgs {
    #[webidl(index = 1, required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createProcessingInstruction")]
struct DetachedDocumentCreateProcessingInstructionArgs {
    #[webidl(index = 1, required)]
    target: String,
    #[webidl(index = 2, required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createCDATASection")]
struct DocumentCreateCdataSectionArgs {
    #[webidl(required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createCDATASection")]
struct DetachedDocumentCreateCdataSectionArgs {
    #[webidl(index = 1, required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElement")]
struct DetachedDocumentCreateElementBridgeArgs {
    #[webidl(index = 1, required)]
    qualified_name: String,
    #[webidl(index = 2, required, nullable)]
    namespace_uri: Option<String>,
    #[webidl(index = 3, required)]
    document_kind: String,
    #[webidl(index = 4)]
    namespace_validation: Option<String>,
}

pub(in crate::native_bridge) fn bridge_detached_create_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentCreateTextNodeArgs>(scope, &args)
    else {
        return;
    };
    match build_detached_text_object(scope, document, &parsed.data) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_create_comment_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentCreateCommentArgs>(scope, &args) else {
        return;
    };
    match build_detached_comment_object(scope, document, &parsed.data) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_create_document_fragment_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    match build_detached_document_fragment_object(scope, document) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_create_processing_instruction_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) =
        webidl::parse_args::<DetachedDocumentCreateProcessingInstructionArgs>(scope, &args)
    else {
        return;
    };
    if !is_valid_pi_target(&parsed.target) {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    if parsed.data.contains("?>") {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    match build_detached_processing_instruction_object(
        scope,
        document,
        &parsed.target,
        &parsed.data,
    ) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_create_cdata_section_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentCreateCdataSectionArgs>(scope, &args)
    else {
        return;
    };
    if parsed.data.contains("]]>") {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    match build_detached_cdata_section_object(scope, document, &parsed.data) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_cdata_section_not_supported_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateCdataSectionArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.data;
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "This operation is not supported for HTML documents",
    );
}

pub(in crate::native_bridge) fn bridge_detached_create_element_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(document) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentCreateElementBridgeArgs>(scope, &args)
    else {
        return;
    };
    let mut namespace_uri = normalize_namespace(parsed.namespace_uri);
    let document_kind = parsed.document_kind;
    let validation = parsed.namespace_validation.as_deref();
    let validation_error = if validation == Some("qualified") {
        validate_qualified_element_name_and_namespace(
            namespace_uri.as_deref(),
            &parsed.qualified_name,
        )
        .err()
    } else if !validate_element_name(&parsed.qualified_name) {
        Some((
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        ))
    } else {
        None
    };
    if let Some((name, code, message)) = validation_error {
        throw_dom_exception(scope, name, code, message);
        return;
    }
    if namespace_uri.is_none()
        && validation != Some("qualified")
        && document_kind == "xml"
        && detached_document_content_type_value(scope, document) == "application/xhtml+xml"
    {
        namespace_uri = Some(XHTML_NS.to_owned());
    }
    let registry_association =
        custom_elements::registry_association_from_create_options_value(scope, args.get(5));
    let is_name = custom_elements::is_name_from_create_options_value(scope, args.get(5));
    if let Some(registry_association) = registry_association
        && let Some(runtime_ptr) = crate::util::context_host_ptr_from_global_bridge(scope)
        && let Some(document_handle) = detached_native_handle(scope, document)
        && !validate_registry_association_for_document(
            scope,
            runtime_ptr,
            document_handle,
            Some(registry_association),
        )
    {
        return;
    }
    match build_detached_element_object(
        scope,
        document,
        &parsed.qualified_name,
        namespace_uri,
        &document_kind,
        validation == Some("qualified"),
        is_name.as_deref(),
        registry_association,
    ) {
        Some(element) => rv.set(element.into()),
        None => rv.set_null(),
    }
}
