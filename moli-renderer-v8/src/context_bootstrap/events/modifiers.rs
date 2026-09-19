use super::*;
use crate::webidl;

const EVENT_MODIFIERS_SLOT: &str = "__moliEventModifiers";
const MODIFIER_KEYS: [&str; 14] = [
    "Alt",
    "Control",
    "Meta",
    "AltGraph",
    "CapsLock",
    "Fn",
    "FnLock",
    "Hyper",
    "NumLock",
    "ScrollLock",
    "Super",
    "Symbol",
    "SymbolLock",
    "Shift",
];

/// EventModifierInit members, in Web IDL dictionary order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "EventModifierInit")]
struct EventModifierInitMembers {
    #[webidl(default = false)]
    alt_key: bool,
    #[webidl(default = false)]
    ctrl_key: bool,
    #[webidl(default = false)]
    meta_key: bool,
    #[webidl(default = false)]
    modifier_alt_graph: bool,
    #[webidl(default = false)]
    modifier_caps_lock: bool,
    #[webidl(default = false)]
    modifier_fn: bool,
    #[webidl(default = false)]
    modifier_fn_lock: bool,
    #[webidl(default = false)]
    modifier_hyper: bool,
    #[webidl(default = false)]
    modifier_num_lock: bool,
    #[webidl(default = false)]
    modifier_scroll_lock: bool,
    #[webidl(default = false)]
    modifier_super: bool,
    #[webidl(default = false)]
    modifier_symbol: bool,
    #[webidl(default = false)]
    modifier_symbol_lock: bool,
    #[webidl(default = false)]
    shift_key: bool,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct EventModifierProperties {
    alt_key: bool,
    ctrl_key: bool,
    meta_key: bool,
    shift_key: bool,
}

impl EventModifierInitMembers {
    fn initialize<'s>(&self, scope: &mut v8::PinScope<'s, '_>, event: v8::Local<'s, v8::Object>) {
        let values = [
            self.alt_key,
            self.ctrl_key,
            self.meta_key,
            self.modifier_alt_graph,
            self.modifier_caps_lock,
            self.modifier_fn,
            self.modifier_fn_lock,
            self.modifier_hyper,
            self.modifier_num_lock,
            self.modifier_scroll_lock,
            self.modifier_super,
            self.modifier_symbol,
            self.modifier_symbol_lock,
            self.shift_key,
        ];
        let bits = values
            .into_iter()
            .enumerate()
            .fold(0, |bits, (index, active)| {
                bits | (u32::from(active) << index)
            });
        set_private_value(
            scope,
            event,
            EVENT_MODIFIERS_SLOT,
            v8::Integer::new_from_unsigned(scope, bits).into(),
        );
        EventModifierProperties::new(self.alt_key, self.ctrl_key, self.meta_key, self.shift_key)
            .initialize(scope, event)
            .expect("event modifier properties should initialize");
    }
}

pub(super) fn initialize_event_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let modifiers = match init {
        Some(init) => {
            match webidl::parse_dictionary_object::<EventModifierInitMembers>(scope, init) {
                Ok(modifiers) => modifiers,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return false;
                }
            }
        }
        None => EventModifierInitMembers::default(),
    };
    modifiers.initialize(scope, event);
    true
}

pub(in crate::context_bootstrap) fn initialize_legacy_event_modifiers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    ctrl_key: bool,
    alt_key: bool,
    shift_key: bool,
    meta_key: bool,
) {
    EventModifierInitMembers {
        ctrl_key,
        alt_key,
        shift_key,
        meta_key,
        ..Default::default()
    }
    .initialize(scope, event);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "getModifierState")]
struct GetModifierStateArgs {
    #[webidl(required)]
    key_arg: String,
}

pub(in crate::context_bootstrap) fn event_get_modifier_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<GetModifierStateArgs>(scope, &args) else {
        return;
    };
    // Retain the legacy accelerator alias for the native platform.
    let key = match parsed.key_arg.as_str() {
        "Accel" if cfg!(target_os = "macos") => "Meta",
        "Accel" => "Control",
        key => key,
    };
    let bits = get_private_value(scope, args.this(), EVENT_MODIFIERS_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or_default();
    let active = MODIFIER_KEYS
        .iter()
        .position(|name| *name == key)
        .is_some_and(|index| bits & (1 << index) != 0);
    rv.set(v8::Boolean::new(scope, active).into());
}
