use super::modifiers::initialize_legacy_event_modifiers;
use super::state::{event_backing, reinitialize_event_object};
use super::ui::{initialize_legacy_ui_event, is_window_receiver};
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{throw_type_error, v8_string};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Event.initEvent")]
struct InitEventArgs {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'initEvent': 1 argument required."
    )]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MouseEvent.initMouseEvent")]
struct InitMouseEventArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'initMouseEvent': 1 argument required."
    )]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(
        index = 3,
        nullable,
        interface = web_api_interfaces::Window,
        brand_check = is_window_receiver
    )]
    view: Option<v8::Local<'s, v8::Object>>,
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
    button: i16,
    #[webidl(index = 14, nullable, interface = web_api_interfaces::EventTarget)]
    related_target: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextEvent.initTextEvent")]
struct InitTextEventArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'initTextEvent': 1 argument required."
    )]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    view: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "undefined", index = 4)]
    data: String,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyTextEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: f64,
    data: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyMouseEventBaseInitDeclaration {
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
    button: i16,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyMouseEventTailInitDeclaration<'scope> {
    #[webapi(constructor_default = 0)]
    buttons: i32,
    related_target: v8::Local<'scope, v8::Value>,
}

fn legacy_text_event_view_or_null<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: Option<v8::Local<'s, v8::Value>>,
    is_host_window: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
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
    if !is_host_window(scope, window) {
        throw_type_error(
            scope,
            "Failed to execute 'initTextEvent': parameter 4 is not of type 'Window'.",
        );
        return None;
    }
    Some(view)
}

pub fn event_init_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = event_backing(scope, args.this());
    let Some(parsed) = webidl::parse_args::<InitEventArgs>(scope, &args) else {
        return;
    };
    reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    );
}

pub fn initialize_legacy_text_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    is_host_window: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
) {
    let event = event_backing(scope, args.this());
    let Some(parsed) = webidl::parse_args::<InitTextEventArgs>(scope, &args) else {
        return;
    };
    let Some(view) = legacy_text_event_view_or_null(scope, parsed.view, is_host_window) else {
        return;
    };
    if !reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        return;
    }
    let data = v8_string(scope, &parsed.data).expect("text event data");
    LegacyTextEventInitDeclaration::new(view, 0.0, data)
        .initialize(scope, event)
        .expect("legacy TextEvent init declaration should initialize");
}

pub fn mouse_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = event_backing(scope, args.this());
    let Some(parsed) = webidl::parse_args::<InitMouseEventArgs>(scope, &args) else {
        return;
    };
    let related_target = parsed
        .related_target
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    if !reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        return;
    }
    initialize_legacy_ui_event(scope, event, parsed.view, parsed.detail);
    LegacyMouseEventBaseInitDeclaration::new(
        parsed.screen_x,
        parsed.screen_y,
        parsed.client_x,
        parsed.client_y,
        parsed.button,
    )
    .initialize(scope, event)
    .expect("legacy MouseEvent base init declaration should initialize");
    initialize_legacy_event_modifiers(
        scope,
        event,
        parsed.ctrl_key,
        parsed.alt_key,
        parsed.shift_key,
        parsed.meta_key,
    );
    LegacyMouseEventTailInitDeclaration::new(related_target)
        .initialize(scope, event)
        .expect("legacy MouseEvent tail init declaration should initialize");
}
