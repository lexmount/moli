use super::super::ui::initialize_legacy_ui_event;
use super::*;
use crate::webidl;
use moli_webapi_declare::WebApiObject;
use webidl::WebIdlDictionarySerialize;

/// Own KeyboardEventInit members, including the legacy code members.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "KeyboardEventInit", snapshot)]
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
    // Retain the existing KeyboardEvent legacy input without adding a member
    // to UIEventInit or reading it for unrelated UI event subclasses.
    #[webidl(default = 0)]
    which: u32,
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
            which: 0,
        }
    }
}

#[derive(Default)]
pub(super) struct KeyboardEventInit<'s> {
    modifiers: super::super::modifiers::EventModifierInit<'s>,
    keyboard: KeyboardEventInitMembers,
}

impl KeyboardEventInit<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.modifiers.event_flags()
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

pub(super) fn initialize_keyboard_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    parsed: KeyboardEventInit<'s>,
) {
    parsed.modifiers.initialize(scope, event);
    parsed
        .keyboard
        .initialize_snapshot(scope, event)
        .expect("KeyboardEvent state should initialize");
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
    #[webidl(nullable, interface = crate::web_api_interfaces::Window, brand_check = crate::context_bootstrap::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
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
    initialize_legacy_ui_event(scope, event, parsed.view, 0);
    let key = v8_string_from_utf16_units(scope, &parsed.key.0).expect("KeyboardEvent key");
    super::super::modifiers::initialize_legacy_event_modifiers(
        scope,
        event,
        parsed.ctrl_key,
        parsed.alt_key,
        parsed.shift_key,
        parsed.meta_key,
    );
    LegacyKeyboardEventInitDeclaration::new(key, parsed.location)
        .initialize(scope, event)
        .expect("legacy KeyboardEvent init declaration should initialize");
}
