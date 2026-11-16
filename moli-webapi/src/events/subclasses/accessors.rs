use super::*;

pub fn event_subclass_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<EventSubclassKind> {
    event_private_value(scope, event, EVENT_SUBCLASS_KIND_SLOT)
        .and_then(|value| v8::Local::<v8::Integer>::try_from(value).ok())
        .and_then(|value| i32::try_from(value.value()).ok())
        .and_then(EventSubclassKind::from_i32)
}

pub fn new_uninitialized_before_unload_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = moli_v8_util::global_constructor_prototype(scope, "BeforeUnloadEvent")?;
    let wrapper = v8::Object::new(scope);
    if wrapper.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    let event = new_event_state(scope);
    initialize_event_object(scope, event, "", false, false);
    set_event_initialized(scope, event, false);
    set_private_value(
        scope,
        event,
        BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT,
        v8str(scope, "").into(),
    );
    web_api_interfaces::initialize(scope, event, "BeforeUnloadEvent").ok()?;
    initialize_event_wrapper(scope, wrapper, event)?;
    Some(wrapper)
}

pub(super) fn before_unload_event_return_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) =
        event_private_value(scope, args.this(), BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT)
    {
        rv.set(value);
    }
}

pub(super) fn before_unload_event_return_value_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    set_event_private_value(
        scope,
        args.this(),
        BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT,
        value.into(),
    );
}

pub(super) fn close_event_was_clean_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = event_private_value(scope, args.this(), CLOSE_EVENT_WAS_CLEAN_SLOT)
        .unwrap_or_else(|| v8::Boolean::new(scope, false).into());
    rv.set(value);
}

pub(super) fn close_event_code_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = event_private_value(scope, args.this(), CLOSE_EVENT_CODE_SLOT)
        .unwrap_or_else(|| v8::Number::new(scope, 0.0).into());
    rv.set(value);
}

pub(super) fn close_event_reason_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = event_private_value(scope, args.this(), CLOSE_EVENT_REASON_SLOT)
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

pub(super) fn track_event_track_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = event_private_value(scope, args.this(), TRACK_EVENT_TRACK_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(super) fn clipboard_event_clipboard_data_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::ClipboardEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = event_private_value(scope, args.this(), CLIPBOARD_EVENT_CLIPBOARD_DATA_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(super) fn clipboard_change_event_types_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::ClipboardChangeEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(value) = event_private_value(scope, args.this(), CLIPBOARD_CHANGE_EVENT_TYPES_SLOT)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

pub(super) fn clipboard_change_event_change_id_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::ClipboardChangeEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(value) =
        event_private_value(scope, args.this(), CLIPBOARD_CHANGE_EVENT_CHANGE_ID_SLOT)
    else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}
