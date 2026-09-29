use crate::webidl;

use super::super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Text")]
struct DetachedTextConstructorBridgeArgs {
    #[webidl(required, converter = "raw")]
    data: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Comment")]
struct DetachedCommentConstructorBridgeArgs {
    #[webidl(required, converter = "raw")]
    data: webidl::DomString16,
}

pub(in crate::native_bridge) fn bridge_create_detached_document_fragment_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(document) = object_property_as_object(scope, global, "document") else {
        rv.set_null();
        return;
    };
    match build_detached_document_fragment_object(scope, document) {
        Some(fragment) => rv.set(fragment.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_detached_text_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(document) = object_property_as_object(scope, global, "document") else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedTextConstructorBridgeArgs>(scope, &args) else {
        return;
    };
    match build_detached_text_object(
        scope,
        document,
        moli_dom::native::DomStringValue::from_utf16(parsed.data.0.as_slice()),
    ) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_create_detached_comment_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(document) = object_property_as_object(scope, global, "document") else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedCommentConstructorBridgeArgs>(scope, &args)
    else {
        return;
    };
    match build_detached_comment_object(
        scope,
        document,
        moli_dom::native::DomStringValue::from_utf16(parsed.data.0.as_slice()),
    ) {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}
