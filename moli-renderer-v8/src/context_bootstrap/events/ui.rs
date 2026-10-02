use super::{event_backing, reinitialize_event_object};
use crate::context_bootstrap::is_window_receiver;
use crate::util::v8_string_from_utf16_units;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

pub(super) struct WindowReference<'s>(v8::Local<'s, v8::Object>);

impl<'s> webidl::WebIdlConverter<'s> for WindowReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(window) = v8::Local::<v8::Object>::try_from(value)
            && is_window_receiver(scope, window)
        {
            return Ok(Self(window));
        }
        Err(webidl::WebIdlError::cannot_convert(context, "Window"))
    }
}

fn nullable_window_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<WindowReference<'s>>,
) -> v8::Local<'s, v8::Value> {
    window
        .map(|window| window.0.into())
        .unwrap_or_else(|| v8::null(scope).into())
}

/// Own UIEventInit members, including legacy which, in dictionary order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "UIEventInit")]
struct UiEventInitMembers<'s> {
    #[webidl(default = 0)]
    detail: i32,
    #[webidl(nullable, converter = "raw")]
    view: Option<WindowReference<'s>>,
    #[webidl(default = 0)]
    which: u32,
}

#[derive(Default)]
pub(super) struct UiEventInit<'s> {
    flags: (bool, bool, bool),
    members: UiEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for UiEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        // Read inherited members before own members. Derived dictionaries
        // delegate here so each observable getter runs once, base-first.
        Ok(Self {
            flags: super::init::parse_event_init(scope, Some(object))?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> UiEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.flags
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let view = nullable_window_value(scope, self.members.view);
        UiEventState::new(view, self.members.detail, self.members.which)
            .initialize(scope, event)
            .expect("UIEvent state should initialize");
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CompositionEventInit")]
struct CompositionEventInitMembers {
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    data: webidl::DomString16,
}

impl Default for CompositionEventInitMembers {
    fn default() -> Self {
        Self {
            data: webidl::DomString16(Vec::new()),
        }
    }
}

#[derive(Default)]
pub(super) struct CompositionEventInit<'s> {
    ui: UiEventInit<'s>,
    members: CompositionEventInitMembers,
}

impl<'s> webidl::WebIdlDictionary<'s> for CompositionEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        Ok(Self {
            ui: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl CompositionEventInit<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }
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
struct UiEventState<'s> {
    view: v8::Local<'s, v8::Value>,
    detail: i32,
    which: u32,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CompositionEventData<'s> {
    data: v8::Local<'s, v8::String>,
}

pub(super) fn initialize_composition_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: CompositionEventInit<'s>,
) {
    init.ui.initialize(scope, event);
    initialize_composition_data(scope, event, &init.members.data.0);
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
    #[webidl(nullable, converter = "raw")]
    view: Option<WindowReference<'s>>,
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
    #[webidl(nullable, converter = "raw")]
    view: Option<WindowReference<'s>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    data: webidl::DomString16,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyUiEventState<'s> {
    view: v8::Local<'s, v8::Value>,
    detail: i32,
}

pub(super) fn initialize_legacy_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    view: Option<WindowReference<'s>>,
    detail: i32,
) {
    let view = nullable_window_value(scope, view);
    LegacyUiEventState::new(view, detail)
        .initialize(scope, event)
        .expect("legacy UIEvent state should initialize");
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
