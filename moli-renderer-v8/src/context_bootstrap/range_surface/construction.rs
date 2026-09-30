use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StaticRange")]
struct StaticRangeConstructorArgs<'s> {
    #[webidl(required, name = "init")]
    init: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "StaticRangeInit")]
struct StaticRangeInitMembers<'s> {
    #[webidl(required, name = "startContainer", with = static_range_node_member)]
    start_container: v8::Local<'s, v8::Object>,
    #[webidl(required, name = "startOffset")]
    start_offset: u32,
    #[webidl(required, name = "endContainer", with = static_range_node_member)]
    end_container: v8::Local<'s, v8::Object>,
    #[webidl(required, name = "endOffset")]
    end_offset: u32,
}

pub(super) fn range_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Range': Please use the 'new' operator.",
        );
        return;
    }
    let Some(document) = current_document_object(scope) else {
        rv.set(args.this().into());
        return;
    };
    initialize_range_object(scope, args.this(), document);
    rv.set(args.this().into());
}

pub(super) fn static_range_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'StaticRange': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<StaticRangeConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let init =
        match webidl::parse_dictionary_object::<StaticRangeInitMembers<'s>>(scope, parsed.init) {
            Ok(init) => init,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
    if static_range_endpoint_type_is_invalid(scope, init.start_container)
        || static_range_endpoint_type_is_invalid(scope, init.end_container)
    {
        throw_named_dom_exception(
            scope,
            "InvalidNodeTypeError",
            "StaticRange endpoints cannot be Attr or DocumentType nodes.",
        );
        return;
    }
    initialize_static_range_object(
        scope,
        args.this(),
        init.start_container,
        init.start_offset,
        init.end_container,
        init.end_offset,
    );
    rv.set(args.this().into());
}

pub(super) fn document_create_range_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let document = args.this();
    let relevant_context = crate::native_bridge::node_relevant_context(scope, document)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    let Some(range) = new_range_for_document(scope, document) else {
        rv.set_undefined();
        return;
    };
    rv.set(range.into());
}

fn static_range_node_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let context = webidl::Context::member("StaticRangeInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Err(webidl::WebIdlError::missing_required(context));
    };
    if value.is_undefined() {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    let Ok(node) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(webidl::WebIdlError::custom_message(
            "StaticRangeInit member is not a Node.",
        ));
    };
    if object_number_property(scope, node, "nodeType").is_none() {
        return Err(webidl::WebIdlError::custom_message(
            "StaticRangeInit member is not a Node.",
        ));
    }
    Ok(node)
}

fn static_range_endpoint_type_is_invalid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(
        object_number_property(scope, node, "nodeType").map(|node_type| node_type as u32),
        Some(2 | 10)
    )
}

/// `Range.prototype.detach()` — per WHATWG DOM, the steps are to do nothing.
/// Kept for legacy compatibility; many WPT tests still call it.
pub(super) fn range_detach_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}
