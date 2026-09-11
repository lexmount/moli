use super::events::{event_is_dispatching, initialize_event_object};
use super::*;
use crate::util::{context_host_ptr_from_window_object, throw_type_error};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Event.initEvent")]
struct InitEventArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomEvent.initCustomEvent")]
struct InitCustomEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initCustomEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    detail: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StorageEvent.initStorageEvent")]
struct InitStorageEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initStorageEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, nullable)]
    key: Option<String>,
    #[webidl(name = "oldValue", index = 4, nullable)]
    old_value: Option<String>,
    #[webidl(name = "newValue", index = 5, nullable)]
    new_value: Option<String>,
    #[webidl(default = "", index = 6, converter = "usv_string")]
    url: String,
    #[webidl(index = 7, converter = "raw", nullable)]
    storage_area: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "UIEvent.initUIEvent")]
struct InitUiEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initUIEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    view: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = 0, index = 4)]
    detail: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MouseEvent.initMouseEvent")]
struct InitMouseEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initMouseEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    view: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = 0, index = 4)]
    detail: i32,
    #[webidl(default = 0, index = 5)]
    screen_x: i32,
    #[webidl(default = 0, index = 6)]
    screen_y: i32,
    #[webidl(default = 0, index = 7)]
    client_x: i32,
    #[webidl(default = 0, index = 8)]
    client_y: i32,
    #[webidl(default = false, index = 9)]
    ctrl_key: bool,
    #[webidl(default = false, index = 10)]
    alt_key: bool,
    #[webidl(default = false, index = 11)]
    shift_key: bool,
    #[webidl(default = false, index = 12)]
    meta_key: bool,
    #[webidl(default = 0, index = 13)]
    button: i32,
    #[webidl(index = 14, converter = "raw")]
    related_target: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "KeyboardEvent.initKeyboardEvent")]
struct InitKeyboardEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initKeyboardEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    view: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "", index = 4)]
    key: String,
    #[webidl(default = 0, index = 5)]
    location: i32,
    #[webidl(default = "", index = 6)]
    _modifiers_list: String,
    #[webidl(default = false, index = 7)]
    repeat: bool,
    #[webidl(default = "", index = 8)]
    _locale: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextEvent.initTextEvent")]
struct InitTextEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initTextEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    view: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "undefined", index = 4)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CompositionEvent.initCompositionEvent")]
struct InitCompositionEventArgs<'s> {
    #[webidl(
        required,
        missing_message = "Failed to execute 'initCompositionEvent': 1 argument required."
    )]
    event_type: String,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    view: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "", index = 4)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "KeyboardEvent.getModifierState")]
struct KeyboardEventGetModifierStateArgs {
    #[webidl(required)]
    key_arg: String,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct LegacyUiEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: i32,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct LegacyTextEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
    data: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct LegacyMouseEventBaseInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: i32,
    screen_x: i32,
    screen_y: i32,
    client_x: i32,
    client_y: i32,
    #[webapi(constructor_default = client_x)]
    x: i32,
    #[webapi(constructor_default = client_y)]
    y: i32,
    #[webapi(constructor_default = client_x)]
    page_x: i32,
    #[webapi(constructor_default = client_y)]
    page_y: i32,
    button: i32,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct LegacyMouseEventTailInitDeclaration<'scope> {
    #[webapi(constructor_default = 0)]
    buttons: i32,
    ctrl_key: bool,
    alt_key: bool,
    shift_key: bool,
    meta_key: bool,
    related_target: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct LegacyKeyboardEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    #[webapi(constructor_default = 0)]
    detail: i32,
    key: v8::Local<'scope, v8::String>,
    code: v8::Local<'scope, v8::String>,
    location: i32,
    repeat: bool,
    #[webapi(constructor_default = false)]
    is_composing: bool,
    #[webapi(constructor_default = false)]
    ctrl_key: bool,
    #[webapi(constructor_default = false)]
    shift_key: bool,
    #[webapi(constructor_default = false)]
    alt_key: bool,
    #[webapi(constructor_default = false)]
    meta_key: bool,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct LegacyCustomEventInitDeclaration<'scope> {
    detail: v8::Local<'scope, v8::Value>,
}

fn legacy_event_view_or_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Value> {
    view.filter(|value| !value.is_null_or_undefined())
        .unwrap_or_else(|| scope.get_current_context().global(scope).into())
}

fn legacy_text_event_view_or_null<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(view) = view.filter(|value| !value.is_null_or_undefined()) else {
        return Some(v8::null(scope).into());
    };
    let Ok(window) = v8::Local::<v8::Object>::try_from(view) else {
        throw_type_error(
            scope,
            "Failed to execute 'initTextEvent': parameter 4 is not of type 'Window'.",
        );
        return None;
    };
    if context_host_ptr_from_window_object(scope, window).is_none() {
        throw_type_error(
            scope,
            "Failed to execute 'initTextEvent': parameter 4 is not of type 'Window'.",
        );
        return None;
    }
    Some(view)
}

pub(super) fn event_init_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitEventArgs>(scope, &args) else {
        return;
    };
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
}

pub(super) fn ui_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitUiEventArgs>(scope, &args) else {
        return;
    };
    let view = legacy_event_view_or_global(scope, parsed.view);
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    LegacyUiEventInitDeclaration::new(view, parsed.detail)
        .initialize(scope, event)
        .expect("legacy UIEvent init declaration should initialize");
}

pub(super) fn text_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitTextEventArgs>(scope, &args) else {
        return;
    };
    let Some(view) = legacy_text_event_view_or_null(scope, parsed.view) else {
        return;
    };
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    let data = v8_string(scope, &parsed.data).expect("text event data");
    LegacyTextEventInitDeclaration::new(view, 0.0, data)
        .initialize(scope, event)
        .expect("legacy TextEvent init declaration should initialize");
}

pub(super) fn mouse_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitMouseEventArgs>(scope, &args) else {
        return;
    };
    let view = legacy_event_view_or_global(scope, parsed.view);
    let related_target = parsed
        .related_target
        .filter(|value| !value.is_null_or_undefined())
        .unwrap_or_else(|| v8::null(scope).into());
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    LegacyMouseEventBaseInitDeclaration::new(
        view,
        parsed.detail,
        parsed.screen_x,
        parsed.screen_y,
        parsed.client_x,
        parsed.client_y,
        parsed.button,
    )
    .initialize(scope, event)
    .expect("legacy MouseEvent base init declaration should initialize");
    LegacyMouseEventTailInitDeclaration::new(
        parsed.ctrl_key,
        parsed.alt_key,
        parsed.shift_key,
        parsed.meta_key,
        related_target,
    )
    .initialize(scope, event)
    .expect("legacy MouseEvent tail init declaration should initialize");
}

pub(super) fn keyboard_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitKeyboardEventArgs>(scope, &args) else {
        return;
    };
    let view = legacy_event_view_or_global(scope, parsed.view);
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    let key = v8_string(scope, &parsed.key).unwrap_or_else(|| v8str(scope, ""));
    let code = v8str(scope, "");
    LegacyKeyboardEventInitDeclaration::new(view, key, code, parsed.location, parsed.repeat)
        .initialize(scope, event)
        .expect("legacy KeyboardEvent init declaration should initialize");
}

pub(super) fn composition_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitCompositionEventArgs>(scope, &args) else {
        return;
    };
    let view = legacy_event_view_or_global(scope, parsed.view);
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    let data = v8_string(scope, &parsed.data).expect("composition event data");
    LegacyTextEventInitDeclaration::new(view, 0.0, data)
        .initialize(scope, event)
        .expect("legacy CompositionEvent init declaration should initialize");
}

pub(super) fn custom_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitCustomEventArgs>(scope, &args) else {
        return;
    };
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    let detail = parsed.detail.unwrap_or_else(|| v8::null(scope).into());
    LegacyCustomEventInitDeclaration::new(detail)
        .initialize(scope, event)
        .expect("legacy CustomEvent init declaration should initialize");
}

pub(super) fn storage_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_is_dispatching(scope, event) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<InitStorageEventArgs>(scope, &args) else {
        return;
    };
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.bubbles,
        parsed.cancelable,
    );
    super::events::define_storage_event_properties(
        scope,
        event,
        parsed.key.as_deref(),
        parsed.old_value.as_deref(),
        parsed.new_value.as_deref(),
        &parsed.url,
        parsed.storage_area,
    );
}

pub(super) fn keyboard_event_get_modifier_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<KeyboardEventGetModifierStateArgs>(scope, &args) else {
        return;
    };
    let event = args.this();
    let key_name = match parsed.key_arg.as_str() {
        "Alt" | "AltGraph" => "altKey",
        "Control" => "ctrlKey",
        "Shift" => "shiftKey",
        "Meta" => "metaKey",
        "Accel" => {
            let ctrl = event
                .get(scope, v8str(scope, "ctrlKey").into())
                .is_some_and(|value| value.boolean_value(scope));
            let meta = event
                .get(scope, v8str(scope, "metaKey").into())
                .is_some_and(|value| value.boolean_value(scope));
            rv.set(v8::Boolean::new(scope, ctrl || meta).into());
            return;
        }
        _ => {
            rv.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };
    let value = event
        .get(scope, v8str(scope, key_name).into())
        .is_some_and(|value| value.boolean_value(scope));
    rv.set(v8::Boolean::new(scope, value).into());
}
