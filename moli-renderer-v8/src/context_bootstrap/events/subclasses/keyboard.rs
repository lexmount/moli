use super::*;
use crate::context_bootstrap::is_window_receiver;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

struct WindowReference<'s>(v8::Local<'s, v8::Object>);

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

/// UIEventInit members, including the legacy which member, in dictionary order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "KeyboardEventInit")]
struct UiEventInitMembers<'s> {
    #[webidl(default = 0)]
    detail: i32,
    #[webidl(nullable, converter = "raw")]
    view: Option<WindowReference<'s>>,
    #[webidl(default = 0)]
    which: u32,
}

/// Own KeyboardEventInit members, including the legacy code members.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "KeyboardEventInit")]
struct KeyboardEventInitMembers {
    #[webidl(default = 0)]
    char_code: u32,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    code: webidl::DomString16,
    #[webidl(default = false)]
    is_composing: bool,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    key: webidl::DomString16,
    #[webidl(default = 0)]
    key_code: u32,
    #[webidl(default = 0)]
    location: u32,
    #[webidl(default = false)]
    repeat: bool,
}

impl Default for KeyboardEventInitMembers {
    fn default() -> Self {
        Self {
            char_code: 0,
            code: webidl::DomString16(Vec::new()),
            is_composing: false,
            key: webidl::DomString16(Vec::new()),
            key_code: 0,
            location: 0,
            repeat: false,
        }
    }
}

#[derive(Default)]
pub(super) struct KeyboardEventInit<'s> {
    flags: (bool, bool, bool),
    ui: UiEventInitMembers<'s>,
    modifiers: super::super::modifiers::EventModifierInitMembers,
    keyboard: KeyboardEventInitMembers,
}

impl KeyboardEventInit<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.flags
    }
}

pub(super) fn parse_keyboard_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<KeyboardEventInit<'s>> {
    let parsed = webidl::dictionary_arg(args, 1, webidl::Context::argument("KeyboardEvent", 2))
        .and_then(|object| {
            let Some(object) = object else {
                return Ok(KeyboardEventInit::default());
            };
            // Parse the inheritance chain base-first. Each group's members are
            // read once in lexical order, before initializing any event state.
            Ok(KeyboardEventInit {
                flags: super::super::init::parse_event_init(scope, Some(object))?,
                ui: webidl::parse_dictionary_object(scope, object)?,
                modifiers: webidl::parse_dictionary_object(scope, object)?,
                keyboard: webidl::parse_dictionary_object(scope, object)?,
            })
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
struct KeyboardEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    detail: i32,
    key: v8::Local<'scope, v8::String>,
    code: v8::Local<'scope, v8::String>,
    location: u32,
    char_code: u32,
    key_code: u32,
    which: u32,
    repeat: bool,
    is_composing: bool,
}

pub(super) fn initialize_keyboard_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    parsed: KeyboardEventInit<'s>,
) {
    let view = parsed
        .ui
        .view
        .map(|window| window.0.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let key = v8_string_from_utf16_units(scope, &parsed.keyboard.key.0).expect("KeyboardEvent key");
    let code =
        v8_string_from_utf16_units(scope, &parsed.keyboard.code.0).expect("KeyboardEvent code");
    parsed.modifiers.initialize(scope, event);
    KeyboardEventInitDeclaration::new(
        view,
        parsed.ui.detail,
        key,
        code,
        parsed.keyboard.location,
        parsed.keyboard.char_code,
        parsed.keyboard.key_code,
        parsed.ui.which,
        parsed.keyboard.repeat,
        parsed.keyboard.is_composing,
    )
    .initialize(scope, event)
    .expect("KeyboardEvent init declaration should initialize");
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "KeyboardEvent.initKeyboardEvent")]
struct InitKeyboardEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(nullable, converter = "raw")]
    view: Option<WindowReference<'s>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    key: webidl::DomString16,
    #[webidl(default = 0)]
    location: u32,
    #[webidl(default = false)]
    ctrl_key: bool,
    #[webidl(default = false)]
    alt_key: bool,
    #[webidl(default = false)]
    shift_key: bool,
    #[webidl(default = false)]
    meta_key: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyKeyboardEventInitDeclaration<'scope> {
    view: v8::Local<'scope, v8::Value>,
    #[webapi(constructor_default = 0)]
    detail: i32,
    key: v8::Local<'scope, v8::String>,
    location: u32,
    #[webapi(constructor_default = false)]
    repeat: bool,
}

pub(in crate::context_bootstrap) fn keyboard_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InitKeyboardEventArgs>(scope, &args) else {
        return;
    };
    let event = event_backing(scope, args.this());
    if !reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        return;
    }
    let view = parsed
        .view
        .map(|window| window.0.into())
        .unwrap_or_else(|| v8::null(scope).into());
    let key = v8_string_from_utf16_units(scope, &parsed.key.0).expect("KeyboardEvent key");
    super::super::modifiers::initialize_legacy_event_modifiers(
        scope,
        event,
        parsed.ctrl_key,
        parsed.alt_key,
        parsed.shift_key,
        parsed.meta_key,
    );
    LegacyKeyboardEventInitDeclaration::new(view, key, parsed.location)
        .initialize(scope, event)
        .expect("legacy KeyboardEvent init declaration should initialize");
}
