//! Constructible event payloads that do not require an animation or recording backend.

use super::{
    define_event_property, event_private_value, initialize_event_object_with_type,
    initialize_event_wrapper, new_event_state,
};
use crate::context_bootstrap::events::EventInit;
use crate::{
    util::{new_null_prototype_object, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;
use webidl::WebIdlDictionarySerialize;

const RENDERED_BUFFER_SLOT: &str = "__moliAudioCompletionRenderedBuffer";
const INPUT_BUFFER_SLOT: &str = "__moliAudioProcessingInputBuffer";
const OUTPUT_BUFFER_SLOT: &str = "__moliAudioProcessingOutputBuffer";
const PLAYBACK_TIME_SLOT: &str = "__moliAudioProcessingPlaybackTime";

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) enum AudioEventKind {
    Processing,
    Completion,
}
impl AudioEventKind {
    fn name(self) -> &'static str {
        match self {
            Self::Processing => "AudioProcessingEvent",
            Self::Completion => "OfflineAudioCompletionEvent",
        }
    }
    fn length(self) -> i32 {
        2
    }
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioProcessingEvent, enumerable, receiver)]
struct AudioProcessingEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, INPUT_BUFFER_SLOT))]
    input_buffer: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, OUTPUT_BUFFER_SLOT))]
    output_buffer: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, PLAYBACK_TIME_SLOT))]
    playback_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OfflineAudioCompletionEvent, enumerable, receiver)]
struct OfflineAudioCompletionEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, RENDERED_BUFFER_SLOT))]
    rendered_buffer: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioProcessingEventInit", snapshot)]
struct AudioProcessingEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::AudioBuffer)]
    #[webidl(snapshot_slot = INPUT_BUFFER_SLOT)]
    input_buffer: v8::Local<'s, v8::Object>,
    #[webidl(required, interface = web_api_interfaces::AudioBuffer)]
    #[webidl(snapshot_slot = OUTPUT_BUFFER_SLOT)]
    output_buffer: v8::Local<'s, v8::Object>,
    #[webidl(required, converter = "double")]
    #[webidl(snapshot_slot = PLAYBACK_TIME_SLOT)]
    playback_time: f64,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "OfflineAudioCompletionEventInit", snapshot)]
struct OfflineAudioCompletionEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::AudioBuffer)]
    #[webidl(snapshot_slot = RENDERED_BUFFER_SLOT)]
    rendered_buffer: v8::Local<'s, v8::Object>,
}

fn payload_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let key = args.data().to_rust_string_lossy(scope);
    if let Some(value) = event_private_value(scope, args.this(), &key) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap) fn build_audio_event_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: AudioEventKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = v8::FunctionTemplate::builder(audio_event_constructor)
        .data(v8::Integer::new(scope, kind as i32).into())
        .length(2)
        .build(scope);
    let prototype = template.prototype_template(scope);
    match kind {
        AudioEventKind::Processing => {
            AudioProcessingEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        AudioEventKind::Completion => {
            OfflineAudioCompletionEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
    }
    template
}
fn audio_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let kind = match args.data().int32_value(scope) {
        Some(0) => AudioEventKind::Processing,
        Some(1) => AudioEventKind::Completion,
        _ => return,
    };
    if !args.is_construct_call() {
        throw_type_error(scope, &format!("{} requires 'new'.", kind.name()));
        return;
    }
    if args.length() < kind.length() {
        throw_type_error(
            scope,
            &format!("{} requires {} arguments.", kind.name(), kind.length()),
        );
        return;
    }
    let Some(event_type) = args.get(0).to_string(scope) else {
        return;
    };
    let state = new_event_state(scope);
    let result = (|| -> Result<(), webidl::WebIdlError> {
        let dictionary =
            webidl::dictionary_arg(&args, 1, webidl::Context::argument(kind.name(), 2))?
                .unwrap_or_else(|| new_null_prototype_object(scope));
        let (bubbles, cancelable, composed) = match kind {
            AudioEventKind::Processing => {
                let parsed =
                    webidl::parse_dictionary_object::<AudioProcessingEventInit>(scope, dictionary)?;
                parsed
                    .initialize_snapshot(scope, state)
                    .expect("audio event state");
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            AudioEventKind::Completion => {
                let parsed = webidl::parse_dictionary_object::<OfflineAudioCompletionEventInit>(
                    scope, dictionary,
                )?;
                parsed
                    .initialize_snapshot(scope, state)
                    .expect("audio event state");
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
        };
        initialize_event_object_with_type(scope, state, event_type, bubbles, cancelable);
        define_event_property(
            scope,
            state,
            "composed",
            v8::Boolean::new(scope, composed).into(),
        );
        Ok(())
    })();
    if let Err(error) = result {
        webidl::throw_error(scope, &error);
        return;
    }
    web_api_interfaces::initialize(scope, state, kind.name())
        .expect("value event brand should initialize");
    if initialize_event_wrapper(scope, args.this(), state).is_some() {
        rv.set(args.this().into());
    }
}

/// Create a trusted native completion payload without consulting public
/// constructors or converting a synthetic author-visible dictionary.
pub(in crate::context_bootstrap) fn new_offline_audio_completion_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rendered_buffer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype =
        crate::context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_prototype(
            scope,
            "OfflineAudioCompletionEvent",
        )
        .ok()?;
    let state = new_event_state(scope);
    super::initialize_event_object(scope, state, "complete", false, false);
    OfflineAudioCompletionEventInit {
        base: EventInit::default(),
        rendered_buffer,
    }
    .initialize_snapshot(scope, state)?;
    web_api_interfaces::initialize(scope, state, "OfflineAudioCompletionEvent").ok()?;
    let event = v8::Object::new(scope);
    if event.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    initialize_event_wrapper(scope, event, state)?;
    super::mark_event_trusted(scope, event);
    Some(event)
}
