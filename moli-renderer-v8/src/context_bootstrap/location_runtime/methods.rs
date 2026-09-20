use super::helpers::{require_location_href_slot, set_return_string};
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Location.assign")]
struct LocationAssignArgs {
    #[webidl(required, converter = "usv_string")]
    url: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Location.replace")]
struct LocationReplaceArgs {
    #[webidl(required, converter = "usv_string")]
    url: String,
}

pub(super) fn location_assign_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !super::access::require_same_origin(scope, args.this()) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<LocationAssignArgs>(scope, &args) else {
        return;
    };
    if !super::access::require_entry_origin(scope, args.this())
        || require_location_href_slot(scope, args.this()).is_none()
    {
        return;
    }
    navigate_location_object(
        scope,
        args.this(),
        LocationNavigationKind::Assign,
        Some(parsed.url),
    );
}

pub(super) fn location_replace_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<LocationReplaceArgs>(scope, &args) else {
        return;
    };
    if require_location_href_slot(scope, args.this()).is_none() {
        return;
    }
    navigate_location_object_with_child_navigate_event(
        scope,
        args.this(),
        LocationNavigationKind::Replace,
        Some(parsed.url),
    );
}

pub(super) fn location_reload_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !super::access::require_same_origin(scope, args.this())
        || !super::access::require_entry_origin(scope, args.this())
        || require_location_href_slot(scope, args.this()).is_none()
    {
        return;
    }
    navigate_location_object_with_child_navigate_event(
        scope,
        args.this(),
        LocationNavigationKind::Reload,
        None,
    );
}

pub(super) fn location_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !super::access::require_same_origin(scope, args.this())
        || !super::access::require_entry_origin(scope, args.this())
    {
        return;
    }
    let Some(href) = super::helpers::location_url(scope, args.this()) else {
        return;
    };
    set_return_string(scope, &mut rv, &href);
}
