//! Speech event construction is independent of the speech synthesis backend.

use super::{
    EventInit, define_event_property, event_private_value, initialize_event_object_with_type,
    initialize_event_wrapper, new_event_state,
};
use crate::{
    util::{new_null_prototype_object, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;
use webidl::WebIdlDictionarySerialize;

const UTTERANCE: &str = "__moliSpeechEventUtterance";
const CHAR_INDEX: &str = "__moliSpeechEventCharIndex";
const CHAR_LENGTH: &str = "__moliSpeechEventCharLength";
const ELAPSED_TIME: &str = "__moliSpeechEventElapsedTime";
const NAME: &str = "__moliSpeechEventName";
const ERROR: &str = "__moliSpeechEventError";

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) enum SpeechEventKind {
    Synthesis,
    Error,
}

impl SpeechEventKind {
    fn name(self) -> &'static str {
        match self {
            Self::Synthesis => "SpeechSynthesisEvent",
            Self::Error => "SpeechSynthesisErrorEvent",
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "SpeechSynthesisErrorCode", rename_all = "kebab-case")]
enum SpeechSynthesisErrorCode {
    Canceled,
    Interrupted,
    AudioBusy,
    AudioHardware,
    Network,
    SynthesisUnavailable,
    SynthesisFailed,
    LanguageUnavailable,
    VoiceUnavailable,
    TextTooLong,
    InvalidArgument,
    NotAllowed,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SpeechSynthesisEventInit", snapshot)]
struct SpeechSynthesisEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::SpeechSynthesisUtterance)]
    #[webidl(snapshot_slot = UTTERANCE)]
    utterance: v8::Local<'s, v8::Object>,
    #[webidl(default = 0)]
    #[webidl(snapshot_slot = CHAR_INDEX)]
    char_index: u32,
    #[webidl(default = 0)]
    #[webidl(snapshot_slot = CHAR_LENGTH)]
    char_length: u32,
    #[webidl(default = 0.0)]
    #[webidl(snapshot_slot = ELAPSED_TIME)]
    elapsed_time: f32,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    #[webidl(snapshot_slot = NAME)]
    name: webidl::DomString16,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SpeechSynthesisErrorEventInit", snapshot)]
struct SpeechSynthesisErrorEventInit<'s> {
    #[webidl(inherit)]
    base: SpeechSynthesisEventInit<'s>,
    #[webidl(required, converter = "enum")]
    #[webidl(snapshot_slot = ERROR)]
    error: SpeechSynthesisErrorCode,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisEvent, enumerable, receiver)]
struct SpeechSynthesisEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, UTTERANCE))]
    utterance: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, CHAR_INDEX))]
    char_index: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, CHAR_LENGTH))]
    char_length: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, ELAPSED_TIME))]
    elapsed_time: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, NAME))]
    name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisErrorEvent, enumerable, receiver)]
struct SpeechSynthesisErrorEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, ERROR))]
    error: (),
}

fn payload_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let key = args.data().to_rust_string_lossy(scope);
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("speech-event binding validated its receiver");
    if let Some(value) = event_private_value(scope, receiver, &key) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "SpeechSynthesisEvent" => {
            SpeechSynthesisEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "SpeechSynthesisErrorEvent" => {
            SpeechSynthesisErrorEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn build_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: SpeechEventKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    v8::FunctionTemplate::builder(constructor)
        .data(v8::Boolean::new(scope, matches!(kind, SpeechEventKind::Error)).into())
        .length(2)
        .build(scope)
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let kind = if args.data().is_true() {
        SpeechEventKind::Error
    } else {
        SpeechEventKind::Synthesis
    };
    if !args.is_construct_call() {
        throw_type_error(scope, &format!("{} requires 'new'.", kind.name()));
        return;
    }
    if args.length() < 2 {
        throw_type_error(scope, &format!("{} requires 2 arguments.", kind.name()));
        return;
    }
    let Some(event_type) = args.get(0).to_string(scope) else {
        return;
    };
    let state = new_event_state(scope);
    let parsed = (|| {
        let dictionary =
            webidl::dictionary_arg(&args, 1, webidl::Context::argument(kind.name(), 2))?
                .unwrap_or_else(|| new_null_prototype_object(scope));
        match kind {
            SpeechEventKind::Synthesis => {
                webidl::parse_dictionary_object::<SpeechSynthesisEventInit>(scope, dictionary).map(
                    |init| {
                        init.initialize_snapshot(scope, state)
                            .expect("speech event state");
                        init.base
                    },
                )
            }
            SpeechEventKind::Error => {
                webidl::parse_dictionary_object::<SpeechSynthesisErrorEventInit>(scope, dictionary)
                    .map(|init| {
                        init.initialize_snapshot(scope, state)
                            .expect("speech error event state");
                        init.base.base
                    })
            }
        }
    })();
    let init = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    initialize_event_object_with_type(scope, state, event_type, init.bubbles, init.cancelable);
    define_event_property(
        scope,
        state,
        "composed",
        v8::Boolean::new(scope, init.composed).into(),
    );
    web_api_interfaces::initialize(scope, state, kind.name())
        .expect("speech event brand should initialize");
    if initialize_event_wrapper(scope, args.this(), state).is_some() {
        rv.set(args.this().into());
    }
}
