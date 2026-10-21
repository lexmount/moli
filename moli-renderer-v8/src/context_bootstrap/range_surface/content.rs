use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.insertNode")]
struct RangeInsertNodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    node: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.createContextualFragment")]
struct RangeCreateContextualFragmentArgs<'s> {
    #[webidl(required)]
    markup: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.surroundContents")]
struct RangeSurroundContentsArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    new_parent: v8::Local<'s, v8::Object>,
}

pub(super) fn range_clone_contents_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(fragment) = range_clone_contents(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    rv.set(fragment.into());
}

pub(super) fn range_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = range_string_contents(scope, args.this()).unwrap_or_default();
    let Some(text) = v8_string(scope, &value) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    rv.set(text.into());
}

pub(super) fn range_insert_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeInsertNodeArgs<'s>>(scope, &args) else {
        return;
    };
    let new_node = parsed.node;
    let Some(start_container) =
        range_boundary_container_object(scope, args.this(), RangeBoundarySide::Start)
    else {
        rv.set_undefined();
        return;
    };
    let start_offset = range_boundary_offset(scope, args.this(), RangeBoundarySide::Start) as u32;
    let _ =
        range_insert_node_at_boundary(scope, args.this(), start_container, start_offset, new_node);
    rv.set_undefined();
}

pub(super) fn range_create_contextual_fragment_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeCreateContextualFragmentArgs<'s>>(scope, &args)
    else {
        return;
    };
    let requirements = context_host_ptr_from_global_bridge(scope)
        .map(|host_ptr| unsafe { &*host_ptr }.trusted_types_for_script_requirements(scope))
        .unwrap_or_default();
    let Some(markup) = crate::context_bootstrap::trusted_html_string_or_throw(
        scope,
        parsed.markup,
        requirements,
        "Range createContextualFragment",
        "createContextualFragment",
    ) else {
        return;
    };
    let Some(fragment) = create_contextual_fragment_internal(scope, args.this(), &markup) else {
        rv.set_undefined();
        return;
    };
    rv.set(fragment.into());
}

pub(super) fn range_delete_contents_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = range_delete_contents(scope, args.this());
    rv.set_undefined();
}

pub(super) fn range_extract_contents_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(fragment) = range_extract_contents(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    rv.set(fragment.into());
}

pub(super) fn range_surround_contents_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeSurroundContentsArgs<'s>>(scope, &args) else {
        return;
    };
    let _ = range_surround_contents(scope, args.this(), parsed.new_parent);
    rv.set_undefined();
}
