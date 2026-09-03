use super::super::navigation_activation::{
    navigation_activation_value, navigation_current_entry_value, navigation_transition_value,
};
use super::super::navigation_entry::{
    history_length_value, history_scroll_restoration_value, history_state_value,
    set_history_scroll_restoration,
};
use super::*;
use moli_webapi_declare::WebApiObject;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum ScrollRestoration {
    Auto,
    Manual,
}

impl ScrollRestoration {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "History")]
struct HistoryPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = history_length_getter_function, enumerable)]
    length: (),

    #[webapi(accessor_property, getter = history_state_getter_function, enumerable)]
    state: (),

    #[webapi(
        accessor_property = "scrollRestoration",
        getter = history_scroll_restoration_getter_function,
        setter = history_scroll_restoration_setter_function,
        enumerable
    )]
    scroll_restoration: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Navigation")]
struct NavigationPrototypeAccessorsDeclaration {
    #[webapi(accessor_property = "canGoBack", getter = navigation_can_go_back_getter_function, enumerable)]
    can_go_back: (),

    #[webapi(
        accessor_property = "canGoForward",
        getter = navigation_can_go_forward_getter_function,
        enumerable
    )]
    can_go_forward: (),

    #[webapi(
        accessor_property = "currentEntry",
        getter = navigation_current_entry_getter_function,
        enumerable
    )]
    current_entry: (),

    #[webapi(accessor_property, getter = navigation_activation_getter_function, enumerable)]
    activation: (),

    #[webapi(accessor_property, getter = navigation_transition_getter_function, enumerable)]
    transition: (),
}

pub(super) fn install_history_prototype_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) {
    HistoryPrototypeAccessorsDeclaration::default()
        .initialize(scope, prototype)
        .expect("History prototype accessors declaration should initialize");
}

pub(super) fn install_navigation_prototype_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) {
    NavigationPrototypeAccessorsDeclaration::default()
        .initialize(scope, prototype)
        .expect("Navigation prototype accessors declaration should initialize");
}

fn navigation_can_go_back_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = runtime_window_owner(scope, args.this());
    if !navigation_document_is_active(scope, owner) {
        rv.set_bool(false);
        return;
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        rv.set_bool(false);
        return;
    }
    if let Some((current, _)) = pending_child_navigation_position(scope, owner) {
        rv.set_bool(current > 0);
        return;
    }
    let can_go_back = window_history_for_holder(scope, owner)
        .is_some_and(|history| pending_or_current_navigation_entry_index(scope, history) > 0);
    rv.set_bool(can_go_back);
}

fn navigation_can_go_forward_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = runtime_window_owner(scope, args.this());
    if !navigation_document_is_active(scope, owner) {
        rv.set_bool(false);
        return;
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        rv.set_bool(false);
        return;
    }
    if let Some((current, len)) = pending_child_navigation_position(scope, owner) {
        rv.set_bool(current + 1 < len);
        return;
    }
    let can_go_forward = window_history_for_holder(scope, owner).is_some_and(|history| {
        let current = pending_or_current_navigation_entry_index(scope, history);
        current + 1 < navigation_entries_len(scope, history)
    });
    rv.set_bool(can_go_forward);
}

fn pending_child_navigation_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<(u32, u32)> {
    let handle = child_browsing_context_handle_for_runtime_owner(scope, owner)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.pending_child_browsing_context_navigation_position(handle)
}

fn history_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if history_owner_if_fully_active(scope, args.this()).is_none() {
        return;
    }
    let value = history_length_value(scope, args.this())
        .unwrap_or_else(|| v8::Number::new(scope, 0.0).into());
    rv.set(value);
}

fn history_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if history_owner_if_fully_active(scope, args.this()).is_none() {
        return;
    }
    let value = history_state_value(scope, args.this());
    rv.set(value);
}

fn navigation_current_entry_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let owner = runtime_window_owner(scope, args.this());
    if !navigation_document_is_active(scope, owner)
        && !navigation_error_event_active(scope, args.this())
    {
        rv.set(v8::null(scope).into());
        return;
    }
    if navigation_document_has_opaque_origin(scope, owner) {
        rv.set(v8::null(scope).into());
        return;
    }
    rv.set(
        navigation_current_entry_value(scope, args.this())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn navigation_activation_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(
        navigation_activation_value(scope, args.this()).unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn navigation_transition_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(
        navigation_transition_value(scope, args.this()).unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn history_scroll_restoration_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if history_owner_if_fully_active(scope, args.this()).is_none() {
        return;
    }
    let value = history_scroll_restoration_value(scope, args.this())
        .unwrap_or_else(|| v8str(scope, "auto").into());
    rv.set(value);
}

fn history_scroll_restoration_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(next_value) = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Some(next_value) = ScrollRestoration::parse(&next_value) else {
        return;
    };
    if history_owner_if_fully_active(scope, args.this()).is_none() {
        return;
    }
    set_history_scroll_restoration(scope, args.this(), next_value.label());
}

#[cfg(test)]
mod tests {
    use super::ScrollRestoration;

    #[test]
    fn scroll_restoration_parses_standard_history_tokens() {
        assert_eq!(
            ScrollRestoration::parse("auto"),
            Some(ScrollRestoration::Auto)
        );
        assert_eq!(
            ScrollRestoration::parse("manual"),
            Some(ScrollRestoration::Manual)
        );
        assert_eq!(ScrollRestoration::parse("Auto"), None);
        assert_eq!(ScrollRestoration::parse("invalid"), None);
    }

    #[test]
    fn scroll_restoration_labels_use_web_exposed_tokens() {
        assert_eq!(ScrollRestoration::Auto.label(), "auto");
        assert_eq!(ScrollRestoration::Manual.label(), "manual");
    }
}
