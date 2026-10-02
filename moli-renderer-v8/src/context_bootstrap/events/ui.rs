use super::{event_backing, reinitialize_event_object};
use crate::util::v8_string_from_utf16_units;
use crate::webidl;
use moli_webapi_declare::WebApiObject;
use webidl::WebIdlDictionarySerialize;

fn nullable_window_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Value> {
    window
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

/// UIEventInit members, converted after EventInit in dictionary order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "UIEventInit", snapshot)]
pub(super) struct UiEventInit<'s> {
    #[webidl(inherit)]
    base: super::EventInit,
    #[webidl(default = 0)]
    detail: i32,
    #[webidl(nullable, interface = crate::web_api_interfaces::InputDeviceCapabilities)]
    source_capabilities: Option<v8::Local<'s, v8::Object>>,
    #[webidl(nullable, interface = crate::web_api_interfaces::Window, brand_check = crate::context_bootstrap::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
}

impl<'s> UiEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }

    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("UIEvent state should initialize");
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyUiEventState<'s> {
    view: v8::Local<'s, v8::Value>,
    detail: i32,
}

pub(in crate::context_bootstrap) fn initialize_legacy_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    view: Option<v8::Local<'s, v8::Object>>,
    detail: i32,
) {
    let view = nullable_window_value(scope, view);
    LegacyUiEventState::new(view, detail)
        .initialize(scope, event)
        .expect("legacy UIEvent state should initialize");
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CompositionEventInit", snapshot)]
pub(super) struct CompositionEventInit<'s> {
    #[webidl(inherit)]
    ui: UiEventInit<'s>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    data: webidl::DomString16,
}

impl Default for CompositionEventInit<'_> {
    fn default() -> Self {
        Self { ui: UiEventInit::default(), data: webidl::DomString16(Vec::new()) }
    }
}

impl CompositionEventInit<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }
}

pub(super) fn initialize_composition_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: CompositionEventInit<'s>,
) {
    init.initialize_snapshot(scope, event).expect("CompositionEvent state should initialize");
}

pub(super) fn parse_ui_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<UiEventInit<'s>> {
    parse_ui_dictionary(scope, args, "UIEvent")
}

pub(super) fn parse_composition_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<CompositionEventInit<'s>> {
    parse_ui_dictionary(scope, args, "CompositionEvent")
}

fn parse_ui_dictionary<'s, T: Default + webidl::WebIdlDictionary<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor: &'static str,
) -> Option<T> {
    let parsed = webidl::dictionary_arg(args, 1, webidl::Context::argument(constructor, 2))
        .and_then(|object| match object {
            Some(object) => webidl::parse_dictionary_object(scope, object),
            None => Ok(T::default()),
        });
    match parsed {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CompositionEventData<'s> {
    data: v8::Local<'s, v8::String>,
}

fn initialize_composition_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    data: &[u16],
) {
    // Both CompositionEventInit.data and initCompositionEvent's dataArg are
    // DOMString inputs. Retain their UTF-16 code units without a Rust String.
    let data = v8_string_from_utf16_units(scope, data).expect("CompositionEvent data");
    CompositionEventData::new(data)
        .initialize(scope, event)
        .expect("CompositionEvent data should initialize");
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "UIEvent.initUIEvent")]
struct InitUiEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(nullable, interface = crate::web_api_interfaces::Window, brand_check = crate::context_bootstrap::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = 0)]
    detail: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CompositionEvent.initCompositionEvent")]
struct InitCompositionEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(nullable, interface = crate::web_api_interfaces::Window, brand_check = crate::context_bootstrap::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    data: webidl::DomString16,
}

pub(in crate::context_bootstrap) fn ui_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InitUiEventArgs>(scope, &args) else {
        return;
    };
    let event = event_backing(scope, args.this());
    if reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        initialize_legacy_ui_event(scope, event, parsed.view, parsed.detail);
    }
}

pub(in crate::context_bootstrap) fn composition_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InitCompositionEventArgs>(scope, &args) else {
        return;
    };
    let event = event_backing(scope, args.this());
    if reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        initialize_legacy_ui_event(scope, event, parsed.view, 0);
        initialize_composition_data(scope, event, &parsed.data.0);
    }
}
