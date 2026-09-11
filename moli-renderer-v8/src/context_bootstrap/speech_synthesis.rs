use super::{
    SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT,
    simple_object_event_set_ordered_handler,
};
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
    throw_type_error,
};
use crate::web_api_interfaces;
use anyhow::Result;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, v8};

const SPEECH_SYNTHESIS_PENDING_SLOT: &str = "__lmSpeechSynthesisPending";
const SPEECH_SYNTHESIS_SPEAKING_SLOT: &str = "__lmSpeechSynthesisSpeaking";
const SPEECH_SYNTHESIS_PAUSED_SLOT: &str = "__lmSpeechSynthesisPaused";
const SPEECH_SYNTHESIS_LISTENERS_SLOT: &str = "__lmSpeechSynthesisListeners";
const SPEECH_SYNTHESIS_ONVOICESCHANGED_SLOT: &str = "__lmSpeechSynthesisOnvoiceschanged";

const SPEECH_UTTERANCE_TEXT_SLOT: &str = "__lmSpeechSynthesisUtteranceText";
const SPEECH_UTTERANCE_LANG_SLOT: &str = "__lmSpeechSynthesisUtteranceLang";
const SPEECH_UTTERANCE_VOICE_SLOT: &str = "__lmSpeechSynthesisUtteranceVoice";
const SPEECH_UTTERANCE_VOLUME_SLOT: &str = "__lmSpeechSynthesisUtteranceVolume";
const SPEECH_UTTERANCE_RATE_SLOT: &str = "__lmSpeechSynthesisUtteranceRate";
const SPEECH_UTTERANCE_PITCH_SLOT: &str = "__lmSpeechSynthesisUtterancePitch";
const SPEECH_UTTERANCE_LISTENERS_SLOT: &str = "__lmSpeechSynthesisUtteranceListeners";
const SPEECH_UTTERANCE_ONSTART_SLOT: &str = "__lmSpeechSynthesisUtteranceOnstart";
const SPEECH_UTTERANCE_ONEND_SLOT: &str = "__lmSpeechSynthesisUtteranceOnend";
const SPEECH_UTTERANCE_ONERROR_SLOT: &str = "__lmSpeechSynthesisUtteranceOnerror";
const SPEECH_UTTERANCE_ONPAUSE_SLOT: &str = "__lmSpeechSynthesisUtteranceOnpause";
const SPEECH_UTTERANCE_ONRESUME_SLOT: &str = "__lmSpeechSynthesisUtteranceOnresume";
const SPEECH_UTTERANCE_ONMARK_SLOT: &str = "__lmSpeechSynthesisUtteranceOnmark";
const SPEECH_UTTERANCE_ONBOUNDARY_SLOT: &str = "__lmSpeechSynthesisUtteranceOnboundary";

const SPEECH_VOICE_URI_SLOT: &str = "__lmSpeechSynthesisVoiceUri";
const SPEECH_VOICE_NAME_SLOT: &str = "__lmSpeechSynthesisVoiceName";
const SPEECH_VOICE_LANG_SLOT: &str = "__lmSpeechSynthesisVoiceLang";
const SPEECH_VOICE_LOCAL_SERVICE_SLOT: &str = "__lmSpeechSynthesisVoiceLocalService";
const SPEECH_VOICE_DEFAULT_SLOT: &str = "__lmSpeechSynthesisVoiceDefault";

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::SpeechSynthesis)]
struct SpeechSynthesisObjectDeclaration {
    #[webapi(slot = SPEECH_SYNTHESIS_PENDING_SLOT, init = false)]
    pending: (),

    #[webapi(slot = SPEECH_SYNTHESIS_SPEAKING_SLOT, init = false)]
    speaking: (),

    #[webapi(slot = SPEECH_SYNTHESIS_PAUSED_SLOT, init = false)]
    paused: (),

    #[webapi(slot = SPEECH_SYNTHESIS_ONVOICESCHANGED_SLOT, init = "null")]
    onvoiceschanged: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = SPEECH_SYNTHESIS_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesis, enumerable)]
struct SpeechSynthesisPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = speech_synthesis_state_getter,
        data = callback_data_index_value(scope, 0)
    )]
    pending: (),

    #[webapi(
        accessor_property,
        getter = speech_synthesis_state_getter,
        data = callback_data_index_value(scope, 1)
    )]
    speaking: (),

    #[webapi(
        accessor_property,
        getter = speech_synthesis_state_getter,
        data = callback_data_index_value(scope, 2)
    )]
    paused: (),

    #[webapi(method, length = 1, callback = speech_synthesis_speak_callback)]
    speak: (),

    #[webapi(method, length = 0, callback = speech_synthesis_cancel_callback)]
    cancel: (),

    #[webapi(method, length = 0, callback = speech_synthesis_pause_callback)]
    pause: (),

    #[webapi(method, length = 0, callback = speech_synthesis_resume_callback)]
    resume: (),

    #[webapi(method = "getVoices", length = 0, callback = speech_synthesis_get_voices_callback)]
    get_voices: (),

    #[webapi(
        accessor_property,
        getter = speech_synthesis_onvoiceschanged_getter,
        setter = speech_synthesis_onvoiceschanged_setter
    )]
    onvoiceschanged: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisUtterance)]
struct SpeechSynthesisUtteranceObjectDeclaration<'scope> {
    #[webapi(slot = SPEECH_UTTERANCE_TEXT_SLOT)]
    text: v8::Local<'scope, v8::String>,

    #[webapi(slot = SPEECH_UTTERANCE_LANG_SLOT)]
    lang: v8::Local<'scope, v8::String>,

    #[webapi(slot = SPEECH_UTTERANCE_VOICE_SLOT, init = "null")]
    voice: (),

    #[webapi(slot = SPEECH_UTTERANCE_VOLUME_SLOT)]
    volume: f64,

    #[webapi(slot = SPEECH_UTTERANCE_RATE_SLOT)]
    rate: f64,

    #[webapi(slot = SPEECH_UTTERANCE_PITCH_SLOT)]
    pitch: f64,

    #[webapi(slot = SPEECH_UTTERANCE_ONSTART_SLOT, init = "null")]
    onstart: (),

    #[webapi(slot = SPEECH_UTTERANCE_ONEND_SLOT, init = "null")]
    onend: (),

    #[webapi(slot = SPEECH_UTTERANCE_ONERROR_SLOT, init = "null")]
    onerror: (),

    #[webapi(slot = SPEECH_UTTERANCE_ONPAUSE_SLOT, init = "null")]
    onpause: (),

    #[webapi(slot = SPEECH_UTTERANCE_ONRESUME_SLOT, init = "null")]
    onresume: (),

    #[webapi(slot = SPEECH_UTTERANCE_ONMARK_SLOT, init = "null")]
    onmark: (),

    #[webapi(slot = SPEECH_UTTERANCE_ONBOUNDARY_SLOT, init = "null")]
    onboundary: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = SPEECH_UTTERANCE_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisUtterance, enumerable)]
struct SpeechSynthesisUtterancePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = speech_utterance_string_getter,
        setter = speech_utterance_string_setter,
        data = callback_data_index_value(scope, 0),
        setter_data = callback_data_index_value(scope, 0)
    )]
    text: (),

    #[webapi(
        accessor_property,
        getter = speech_utterance_string_getter,
        setter = speech_utterance_string_setter,
        data = callback_data_index_value(scope, 1),
        setter_data = callback_data_index_value(scope, 1)
    )]
    lang: (),

    #[webapi(
        accessor_property,
        getter = speech_utterance_voice_getter,
        setter = speech_utterance_voice_setter
    )]
    voice: (),

    #[webapi(
        accessor_property,
        getter = speech_utterance_float_getter,
        setter = speech_utterance_float_setter,
        data = callback_data_index_value(scope, 0),
        setter_data = callback_data_index_value(scope, 0)
    )]
    volume: (),

    #[webapi(
        accessor_property,
        getter = speech_utterance_float_getter,
        setter = speech_utterance_float_setter,
        data = callback_data_index_value(scope, 1),
        setter_data = callback_data_index_value(scope, 1)
    )]
    rate: (),

    #[webapi(
        accessor_property,
        getter = speech_utterance_float_getter,
        setter = speech_utterance_float_setter,
        data = callback_data_index_value(scope, 2),
        setter_data = callback_data_index_value(scope, 2)
    )]
    pitch: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 0), setter_data = callback_data_index_value(scope, 0))]
    onstart: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 1), setter_data = callback_data_index_value(scope, 1))]
    onend: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 2), setter_data = callback_data_index_value(scope, 2))]
    onerror: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 3), setter_data = callback_data_index_value(scope, 3))]
    onpause: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 4), setter_data = callback_data_index_value(scope, 4))]
    onresume: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 5), setter_data = callback_data_index_value(scope, 5))]
    onmark: (),

    #[webapi(accessor_property, getter = speech_utterance_event_handler_getter, setter = speech_utterance_event_handler_setter, data = callback_data_index_value(scope, 6), setter_data = callback_data_index_value(scope, 6))]
    onboundary: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisVoice, enumerable)]
struct SpeechSynthesisVoicePrototypeDeclaration {
    #[webapi(accessor_property = "voiceURI", getter = speech_voice_getter, data = callback_data_index_value(scope, 0))]
    voice_uri: (),

    #[webapi(accessor_property, getter = speech_voice_getter, data = callback_data_index_value(scope, 1))]
    name: (),

    #[webapi(accessor_property, getter = speech_voice_getter, data = callback_data_index_value(scope, 2))]
    lang: (),

    #[webapi(accessor_property = "localService", getter = speech_voice_getter, data = callback_data_index_value(scope, 3))]
    local_service: (),

    #[webapi(accessor_property = "default", getter = speech_voice_getter, data = callback_data_index_value(scope, 4))]
    is_default: (),
}

#[derive(Clone, Copy)]
struct SpeechUtteranceEventHandler {
    event_type: &'static str,
    slot: &'static str,
}

#[derive(Clone, Copy)]
struct SpeechUtteranceFloatAttribute {
    slot: &'static str,
    minimum: f64,
    maximum: f64,
}

const SPEECH_SYNTHESIS_STATE_SLOTS: &[&str] = &[
    SPEECH_SYNTHESIS_PENDING_SLOT,
    SPEECH_SYNTHESIS_SPEAKING_SLOT,
    SPEECH_SYNTHESIS_PAUSED_SLOT,
];

const SPEECH_UTTERANCE_STRING_SLOTS: &[&str] =
    &[SPEECH_UTTERANCE_TEXT_SLOT, SPEECH_UTTERANCE_LANG_SLOT];

const SPEECH_UTTERANCE_FLOAT_ATTRIBUTES: &[SpeechUtteranceFloatAttribute] = &[
    SpeechUtteranceFloatAttribute {
        slot: SPEECH_UTTERANCE_VOLUME_SLOT,
        minimum: 0.0,
        maximum: 1.0,
    },
    SpeechUtteranceFloatAttribute {
        slot: SPEECH_UTTERANCE_RATE_SLOT,
        minimum: 0.1,
        maximum: 10.0,
    },
    SpeechUtteranceFloatAttribute {
        slot: SPEECH_UTTERANCE_PITCH_SLOT,
        minimum: 0.0,
        maximum: 2.0,
    },
];

const SPEECH_UTTERANCE_EVENT_HANDLERS: &[SpeechUtteranceEventHandler] = &[
    SpeechUtteranceEventHandler {
        event_type: "start",
        slot: SPEECH_UTTERANCE_ONSTART_SLOT,
    },
    SpeechUtteranceEventHandler {
        event_type: "end",
        slot: SPEECH_UTTERANCE_ONEND_SLOT,
    },
    SpeechUtteranceEventHandler {
        event_type: "error",
        slot: SPEECH_UTTERANCE_ONERROR_SLOT,
    },
    SpeechUtteranceEventHandler {
        event_type: "pause",
        slot: SPEECH_UTTERANCE_ONPAUSE_SLOT,
    },
    SpeechUtteranceEventHandler {
        event_type: "resume",
        slot: SPEECH_UTTERANCE_ONRESUME_SLOT,
    },
    SpeechUtteranceEventHandler {
        event_type: "mark",
        slot: SPEECH_UTTERANCE_ONMARK_SLOT,
    },
    SpeechUtteranceEventHandler {
        event_type: "boundary",
        slot: SPEECH_UTTERANCE_ONBOUNDARY_SLOT,
    },
];

const SPEECH_VOICE_ATTRIBUTE_SLOTS: &[&str] = &[
    SPEECH_VOICE_URI_SLOT,
    SPEECH_VOICE_NAME_SLOT,
    SPEECH_VOICE_LANG_SLOT,
    SPEECH_VOICE_LOCAL_SERVICE_SLOT,
    SPEECH_VOICE_DEFAULT_SLOT,
];

pub(in crate::context_bootstrap) fn install_speech_synthesis_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "SpeechSynthesis" => {
            SpeechSynthesisPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "SpeechSynthesisUtterance" => {
            SpeechSynthesisUtterancePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "SpeechSynthesisVoice" => {
            SpeechSynthesisVoicePrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn build_window_speech_synthesis<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    Ok(SpeechSynthesisObjectDeclaration::default().bind(scope)?)
}

pub(in crate::context_bootstrap) fn speech_synthesis_utterance_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'SpeechSynthesisUtterance': Please use the 'new' operator.",
        );
        return;
    }

    let text = if args.length() == 0 || args.get(0).is_undefined() {
        v8::String::empty(scope)
    } else {
        let Some(text) = args.get(0).to_string(scope) else {
            return;
        };
        text
    };
    let lang = v8::String::empty(scope);
    let utterance = args.this();
    let declaration = SpeechSynthesisUtteranceObjectDeclaration {
        text,
        lang,
        voice: (),
        volume: 1.0,
        rate: 1.0,
        pitch: 1.0,
        onstart: (),
        onend: (),
        onerror: (),
        onpause: (),
        onresume: (),
        onmark: (),
        onboundary: (),
        event_target_slot: (),
        ordered_handlers: (),
    };
    if declaration.initialize(scope, utterance).is_err() {
        rv.set_undefined();
        return;
    }
    rv.set(utterance.into());
}

fn speech_synthesis_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        SPEECH_SYNTHESIS_STATE_SLOTS,
        "SpeechSynthesis state slots",
    ) else {
        rv.set_bool(false);
        return;
    };
    let value = get_private_value(scope, args.this(), slot)
        .unwrap_or_else(|| v8::Boolean::new(scope, false).into());
    rv.set(value);
}

fn speech_synthesis_speak_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'speak' on 'SpeechSynthesis': 1 argument required, but only 0 present.",
        );
        return;
    }
    let utterance = args.get(0);
    let branded = v8::Local::<v8::Object>::try_from(utterance)
        .ok()
        .is_some_and(|object| {
            web_api_interfaces::SpeechSynthesisUtterance::is_instance(scope, object)
        });
    if !branded {
        throw_type_error(
            scope,
            "Failed to execute 'speak' on 'SpeechSynthesis': parameter 1 is not of type 'SpeechSynthesisUtterance'.",
        );
        return;
    }

    // This browser intentionally has no audio playback backend. Chromium under
    // Xvfb with an empty voice list likewise keeps all three queue state flags
    // false when speech cannot be started. The observable API remains usable
    // without pretending to synthesize audio.
    rv.set_undefined();
}

fn speech_synthesis_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    rv.set_undefined();
}

fn speech_synthesis_pause_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    rv.set_undefined();
}

fn speech_synthesis_resume_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    rv.set_undefined();
}

fn speech_synthesis_get_voices_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    rv.set(v8::Array::new(scope, 0).into());
}

fn speech_synthesis_onvoiceschanged_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), SPEECH_SYNTHESIS_ONVOICESCHANGED_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn speech_synthesis_onvoiceschanged_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesis") {
        return;
    }
    set_event_handler(
        scope,
        args.this(),
        args.get(0),
        SPEECH_SYNTHESIS_LISTENERS_SLOT,
        "voiceschanged",
        SPEECH_SYNTHESIS_ONVOICESCHANGED_SLOT,
    );
}

fn speech_utterance_string_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        SPEECH_UTTERANCE_STRING_SLOTS,
        "SpeechSynthesisUtterance string slots",
    ) else {
        rv.set(v8::String::empty(scope).into());
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
    );
}

fn speech_utterance_string_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        SPEECH_UTTERANCE_STRING_SLOTS,
        "SpeechSynthesisUtterance string slots",
    ) else {
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    set_private_value(scope, args.this(), slot, value.into());
}

fn speech_utterance_voice_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), SPEECH_UTTERANCE_VOICE_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn speech_utterance_voice_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let value = args.get(0);
    if value.is_null_or_undefined() {
        set_private_value(
            scope,
            args.this(),
            SPEECH_UTTERANCE_VOICE_SLOT,
            v8::null(scope).into(),
        );
        return;
    }
    let valid = v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|object| web_api_interfaces::SpeechSynthesisVoice::is_instance(scope, object));
    if !valid {
        throw_type_error(
            scope,
            "Failed to set the 'voice' property on 'SpeechSynthesisUtterance': Failed to convert value to 'SpeechSynthesisVoice'.",
        );
        return;
    }
    set_private_value(scope, args.this(), SPEECH_UTTERANCE_VOICE_SLOT, value);
}

fn speech_utterance_float_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SPEECH_UTTERANCE_FLOAT_ATTRIBUTES,
        "SpeechSynthesisUtterance float attributes",
    ) else {
        rv.set(v8::Number::new(scope, 1.0).into());
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), attribute.slot)
            .unwrap_or_else(|| v8::Number::new(scope, 1.0).into()),
    );
}

fn speech_utterance_float_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SPEECH_UTTERANCE_FLOAT_ATTRIBUTES,
        "SpeechSynthesisUtterance float attributes",
    ) else {
        return;
    };
    let Some(number) = args.get(0).number_value(scope) else {
        return;
    };
    if !number.is_finite() {
        throw_type_error(
            scope,
            "Failed to set SpeechSynthesisUtterance float property: The provided float value is non-finite.",
        );
        return;
    }
    let number = number.clamp(attribute.minimum, attribute.maximum) as f32 as f64;
    set_private_value(
        scope,
        args.this(),
        attribute.slot,
        v8::Number::new(scope, number).into(),
    );
}

fn speech_utterance_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        SPEECH_UTTERANCE_EVENT_HANDLERS,
        "SpeechSynthesisUtterance event handlers",
    ) else {
        rv.set_null();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), handler.slot)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn speech_utterance_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisUtterance") {
        return;
    }
    let Some(handler) = callback_data_item(
        scope,
        &args,
        SPEECH_UTTERANCE_EVENT_HANDLERS,
        "SpeechSynthesisUtterance event handlers",
    ) else {
        return;
    };
    set_event_handler(
        scope,
        args.this(),
        args.get(0),
        SPEECH_UTTERANCE_LISTENERS_SLOT,
        handler.event_type,
        handler.slot,
    );
}

fn speech_voice_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_brand(scope, args.this(), "SpeechSynthesisVoice") {
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        SPEECH_VOICE_ATTRIBUTE_SLOTS,
        "SpeechSynthesisVoice attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn set_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    listeners_slot: &'static str,
    event_type: &'static str,
    handler_slot: &'static str,
) {
    let active = v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|callback| callback.is_callable());
    let stored = if active {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, target, handler_slot, stored);
    simple_object_event_set_ordered_handler(
        scope,
        target,
        listeners_slot,
        event_type,
        handler_slot,
        active,
    );
}

fn require_brand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    if web_api_interfaces::descriptor(slot)
        .is_some_and(|interface| interface.is_instance(scope, receiver))
    {
        return true;
    }
    throw_type_error(scope, "Illegal invocation");
    false
}
