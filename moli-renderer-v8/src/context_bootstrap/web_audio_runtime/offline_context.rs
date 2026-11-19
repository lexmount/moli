//! Bounded OfflineAudioContext constructor overloads and render-size metadata.

use super::*;
use moli_webapi::web_audio::offline_context_options::{
    RenderSizeCategory, RenderSizeHint, parse_offline_audio_context_options,
};

const RENDER_QUANTUM_SIZE: &str = "__moliAudioRenderQuantumSize";
const DEFAULT_QUANTUM: u32 = 128;

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "OfflineAudioContext requires new.");
        return;
    }
    let Some(options) = parse_offline_audio_context_options(scope, &args) else {
        return;
    };
    if !format::validate(
        scope,
        options.number_of_channels,
        options.length,
        options.sample_rate,
    ) {
        return;
    }
    let quantum = match options.render_size_hint {
        RenderSizeHint::Category(RenderSizeCategory::Default | RenderSizeCategory::Hardware) => {
            DEFAULT_QUANTUM
        }
        RenderSizeHint::Frames(frames) => {
            let maximum = (6.0 * f64::from(options.sample_rate)).floor() as u32;
            if !(1..=maximum).contains(&frames) {
                throw_dom_exception(
                    scope,
                    "NotSupportedError",
                    9,
                    "Unsupported audio render quantum size.",
                );
                return;
            }
            frames
        }
    };

    let context = args.this();
    let destination = audio_destination_node(scope, context);
    let compressors = v8::Array::new(scope, 0);
    let modules = new_web_audio_map_object(scope);
    let module_list = v8::Array::new(scope, 0);
    let processors = new_web_audio_map_object(scope);
    set_private_value(scope, context, AUDIO_CONTEXT_MODULES_SLOT, modules.into());
    set_private_value(
        scope,
        context,
        AUDIO_CONTEXT_MODULE_LIST_SLOT,
        module_list.into(),
    );
    set_private_value(
        scope,
        context,
        AUDIO_CONTEXT_PROCESSORS_SLOT,
        processors.into(),
    );
    set_web_audio_number_slot(scope, context, RENDER_QUANTUM_SIZE, f64::from(quantum));
    let length = f64::from(options.length);
    let sample_rate = f64::from(options.sample_rate);
    OfflineAudioContextObjectDeclaration::new(
        0.0,
        length,
        sample_rate,
        length,
        sample_rate,
        f64::from(options.number_of_channels),
        compressors,
        "suspended",
        destination,
    )
    .initialize(scope, context)
    .expect("OfflineAudioContext declaration should initialize object");
    rv.set(context.into());
}

pub(super) fn render_quantum_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let quantum = web_audio_number_slot(scope, args.this(), RENDER_QUANTUM_SIZE)
        .unwrap_or(f64::from(DEFAULT_QUANTUM));
    rv.set(v8::Number::new(scope, quantum).into());
}
