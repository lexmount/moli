use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StaticRange")]
struct StaticRangeConstructorArgs<'s> {
    #[webidl(required, dictionary)]
    init: StaticRangeInitMembers<'s>,
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
    let init = parsed.init;
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
    let Some(range) = new_range_for_document(scope, document) else {
        rv.set_undefined();
        return;
    };
    rv.set(range.into());
}

fn static_range_endpoint_type_is_invalid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(range_node_type(scope, node), Some(2 | 10))
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
