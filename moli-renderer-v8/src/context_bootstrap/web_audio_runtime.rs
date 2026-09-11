use super::media_queries::{
    dispatch_simple_event_target_event, simple_event_target_add_event_listener_callback,
    simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback,
};
use super::*;
use crate::native_bridge::throw_dom_exception;
use crate::util::{
    array_push_value, call_object_method, get_private_value, object_string_property,
    set_private_value, set_symbol_to_string_tag,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod audio_param;
mod biquad;
mod graph;

use audio_param::{audio_param, detune_param};

const AUDIO_CONTEXT_LISTENERS_SLOT: &str = "__moliAudioContextListeners";
const AUDIO_CONTEXT_MODULES_SLOT: &str = "__moliAudioContextModules";
const AUDIO_CONTEXT_MODULE_LIST_SLOT: &str = "__moliAudioContextModuleList";
const AUDIO_CONTEXT_PROCESSORS_SLOT: &str = "__moliAudioContextProcessors";
const AUDIO_WORKLET_CONTEXT_SLOT: &str = "__moliAudioWorkletContext";
const AUDIO_WORKLET_MODULE_CONTEXT_SLOT: &str = "__moliAudioWorkletModuleContext";
const AUDIO_WORKLET_MODULE_WORKER_SLOT: &str = "__moliAudioWorkletModuleWorker";
const AUDIO_WORKLET_MODULE_PROMISE_SLOT: &str = "__moliAudioWorkletModulePromise";
const AUDIO_WORKLET_MODULE_RESOLVER_SLOT: &str = "__moliAudioWorkletModuleResolver";
const AUDIO_WORKLET_MODULE_LOADED_SLOT: &str = "__moliAudioWorkletModuleLoaded";
const AUDIO_WORKLET_MODULE_SETTLED_SLOT: &str = "__moliAudioWorkletModuleSettled";
const AUDIO_WORKLET_CALLBACK_MODULE_SLOT: &str = "__moliAudioWorkletCallbackModule";
const OFFLINE_AUDIO_LENGTH_SLOT: &str = "__moliOfflineAudioLength";
const OFFLINE_AUDIO_SAMPLE_RATE_SLOT: &str = "__moliOfflineAudioSampleRate";
const OFFLINE_AUDIO_CHANNEL_COUNT_SLOT: &str = "__moliOfflineAudioChannelCount";
const OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT: &str = "__moliOfflineAudioCompleteContext";
const OFFLINE_AUDIO_COMPLETE_BUFFER_SLOT: &str = "__moliOfflineAudioCompleteBuffer";
const OFFLINE_AUDIO_COMPRESSORS_SLOT: &str = "__moliOfflineAudioCompressors";
const DYNAMICS_COMPRESSOR_REDUCTION_SLOT: &str = "__moliDynamicsCompressorReduction";
const ANALYSER_FFT_SIZE_SLOT: &str = "__moliAnalyserFftSize";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioContext)]
struct AudioContextObjectDeclaration<'scope> {
    #[webapi(data_property = "currentTime")]
    current_time: f64,
    #[webapi(data_property = "sampleRate")]
    sample_rate: f64,
    #[webapi(data_property)]
    state: &'static str,
    #[webapi(data_property)]
    destination: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "audioWorklet")]
    audio_worklet: v8::Local<'scope, v8::Object>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = AUDIO_CONTEXT_LISTENERS_SLOT)]
    event_target_slot: (),
    #[webapi(slot = AUDIO_CONTEXT_MODULES_SLOT)]
    modules: v8::Local<'scope, v8::Object>,
    #[webapi(slot = AUDIO_CONTEXT_MODULE_LIST_SLOT)]
    module_list: v8::Local<'scope, v8::Array>,
    #[webapi(slot = AUDIO_CONTEXT_PROCESSORS_SLOT)]
    processors: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioWorklet)]
struct AudioWorkletObjectDeclaration<'scope> {
    #[webapi(slot = AUDIO_WORKLET_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(method = "addModule", length = 1, callback = audio_worklet_add_module_callback)]
    add_module: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioWorkletNode)]
struct AudioWorkletNodeObjectDeclaration<'scope> {
    #[webapi(data_property)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    port: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct AudioWorkletModuleStateDeclaration<'scope> {
    #[webapi(slot = AUDIO_WORKLET_MODULE_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_WORKER_SLOT)]
    worker: v8::Local<'scope, v8::Object>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_PROMISE_SLOT)]
    promise: v8::Local<'scope, v8::Promise>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_RESOLVER_SLOT)]
    resolver: v8::Local<'scope, v8::PromiseResolver>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_LOADED_SLOT)]
    loaded: bool,
    #[webapi(slot = AUDIO_WORKLET_MODULE_SETTLED_SLOT)]
    settled: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct AudioWorkletWorkerCallbackDataDeclaration<'scope> {
    #[webapi(slot = AUDIO_WORKLET_CALLBACK_MODULE_SLOT)]
    module_state: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct AudioWorkletBlobOptionsDeclaration {
    #[webapi(data_property = "type")]
    kind: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct AudioWorkletWorkerOptionsDeclaration {
    #[webapi(data_property = "type")]
    kind: &'static str,
    credentials: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct AudioWorkletProcessorConstructMessageDeclaration<'scope> {
    #[webapi(data_property = "__moliAudioWorkletType")]
    message_type: &'static str,
    name: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::OfflineAudioContext)]
struct OfflineAudioContextObjectDeclaration<'scope> {
    #[webapi(data_property = "currentTime")]
    current_time: f64,
    #[webapi(data_property)]
    length: f64,
    #[webapi(data_property = "sampleRate")]
    sample_rate: f64,
    #[webapi(slot = OFFLINE_AUDIO_LENGTH_SLOT)]
    internal_length: f64,
    #[webapi(slot = OFFLINE_AUDIO_SAMPLE_RATE_SLOT)]
    internal_sample_rate: f64,
    #[webapi(slot = OFFLINE_AUDIO_CHANNEL_COUNT_SLOT)]
    channel_count: f64,
    #[webapi(slot = OFFLINE_AUDIO_COMPRESSORS_SLOT)]
    compressors: v8::Local<'scope, v8::Array>,
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT)]
    event_target_slot: &'static str,
    #[webapi(data_property)]
    state: &'static str,
    #[webapi(data_property)]
    destination: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "oncomplete", init = "null")]
    oncomplete: (),
    #[webapi(method, enumerable, callback = simple_event_target_add_event_listener_callback)]
    add_event_listener: (),
    #[webapi(
        method,
        enumerable,
        callback = simple_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),
    #[webapi(method, enumerable, callback = simple_event_target_dispatch_event_callback)]
    dispatch_event: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::OscillatorNode)]
struct OscillatorNodeObjectDeclaration<'scope> {
    #[webapi(data_property = "type")]
    kind: &'static str,
    #[webapi(data_property)]
    frequency: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    detune: v8::Local<'scope, v8::Object>,
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
    #[webapi(method, length = 1, callback = oscillator_start_callback)]
    start: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DynamicsCompressorNode)]
struct DynamicsCompressorNodeObjectDeclaration<'scope> {
    #[webapi(data_property)]
    threshold: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    knee: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    ratio: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    attack: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    release: v8::Local<'scope, v8::Object>,
    #[webapi(slot = DYNAMICS_COMPRESSOR_REDUCTION_SLOT)]
    reduction: f64,
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DynamicsCompressorNode, enumerable)]
struct DynamicsCompressorNodePrototypeDeclaration {
    #[webapi(accessor_property, getter = dynamics_compressor_reduction_getter_callback)]
    reduction: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AnalyserNode)]
struct AnalyserNodeObjectDeclaration {
    #[webapi(slot = ANALYSER_FFT_SIZE_SLOT)]
    fft_size: f64,
    #[webapi(data_property = "minDecibels")]
    min_decibels: f64,
    #[webapi(data_property = "maxDecibels")]
    max_decibels: f64,
    #[webapi(data_property = "smoothingTimeConstant")]
    smoothing_time_constant: f64,
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AnalyserNode, enumerable)]
struct AnalyserNodePrototypeDeclaration {
    #[webapi(accessor_property = "fftSize", getter = analyser_fft_size_getter_callback, setter = analyser_fft_size_setter_callback)]
    fft_size: (),
    #[webapi(accessor_property = "frequencyBinCount", getter = analyser_frequency_bin_count_getter_callback)]
    frequency_bin_count: (),
    #[webapi(method, length = 1, callback = analyser_get_float_frequency_data_callback)]
    get_float_frequency_data: (),
    #[webapi(method, length = 1, callback = analyser_get_float_time_domain_data_callback)]
    get_float_time_domain_data: (),
    #[webapi(method, length = 1, callback = analyser_get_byte_frequency_data_callback)]
    get_byte_frequency_data: (),
    #[webapi(method, length = 1, callback = analyser_get_byte_time_domain_data_callback)]
    get_byte_time_domain_data: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct OfflineAudioCompletePayloadDeclaration<'scope> {
    #[webapi(slot = OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(slot = OFFLINE_AUDIO_COMPLETE_BUFFER_SLOT)]
    rendered_buffer: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::OfflineAudioCompletionEvent,
    prototype = "Object",)]
struct OfflineAudioCompletionEventDeclaration<'scope> {
    #[webapi(data_property = "type")]
    event_type: &'static str,
    #[webapi(data_property = "renderedBuffer")]
    rendered_buffer: v8::Local<'scope, v8::Object>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioDestinationNode, allow_empty)]
struct AudioDestinationNodeObjectDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioBuffer)]
struct AudioBufferObjectDeclaration<'scope> {
    #[webapi(data_property)]
    length: f64,
    #[webapi(data_property = "sampleRate")]
    sample_rate: f64,
    #[webapi(data_property)]
    duration: f64,
    #[webapi(slot = OFFLINE_AUDIO_BUFFER_SLOT)]
    channel_data: v8::Local<'scope, v8::Object>,
}

// Captured from the Chromium-on-Linux baseline we use for Zhihu probe parity.
// If that browser profile changes, update this together with the audio probe
// assertions in `script_vm/tests.rs`.
const TARGET_AUDIO_FINGERPRINT_SUM: f64 = 124.04347527516074;
const SYNTHETIC_COMPRESSOR_REDUCTION: f64 = -82.26815795898438;
const SYNTHETIC_ANALYSER_FREQUENCY_BINS: &[f64] = &[
    -90.25955200195312,
    -90.22233581542969,
    -90.11856842041016,
    -89.96821594238281,
    -89.79446411132812,
    -89.61327362060547,
    -89.42459106445312,
    -89.20698547363281,
    -88.91907501220703,
    -88.51146697998047,
    -87.94953918457031,
    -87.2354736328125,
    -86.41204833984375,
    -85.54572296142578,
    -84.70391082763672,
    -83.94123077392578,
    -83.29612731933594,
    -82.79357147216797,
    -82.4489974975586,
    -82.27188110351562,
    -82.26815795898438,
    -82.4417953491211,
    -82.79552459716797,
    -83.33106231689453,
    -84.04853820800781,
    -84.94512939453125,
    -86.0123062133789,
    -87.2312240600586,
    -88.56591796875,
    -89.95575714111328,
    -91.31378936767578,
    -92.54295349121094,
    -93.5753173828125,
    -94.40994262695312,
    -95.10946655273438,
    -95.75737762451172,
    -96.4164810180664,
    -97.1112060546875,
    -97.82959747314453,
    -98.53618621826172,
    -99.19161987304688,
    -99.77345275878906,
    -100.28733825683594,
    -100.76123046875,
    -101.2282485961914,
    -101.71035766601562,
    -102.21070098876953,
    -102.71531677246094,
    -103.20209503173828,
    -103.65292358398438,
    -104.06330108642578,
    -104.44380950927734,
    -104.81304168701172,
    -105.1873779296875,
    -105.57337951660156,
    -105.9659423828125,
    -106.35223388671875,
    -106.71916961669922,
    -107.06056213378906,
    -107.38026428222656,
    -107.68936920166016,
    -107.99988555908203,
    -108.31855773925781,
    -108.6440658569336,
];

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OfflineAudioContext")]
struct OfflineAudioContextConstructorArgs {
    #[webidl(required)]
    channel_count: f64,
    #[webidl(required)]
    length: f64,
    #[webidl(required)]
    sample_rate: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBuffer.getChannelData")]
struct AudioBufferGetChannelDataArgs {
    #[webidl(required)]
    channel: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioParam.setValueAtTime")]
struct AudioParamSetValueAtTimeArgs {
    #[webidl(required)]
    value: f64,
    #[webidl(required)]
    start_time: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AnalyserNode.fftSize")]
struct AnalyserFftSizeArgs {
    #[webidl(required)]
    value: u32,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::AudioContext,
    constructor_callback = audio_context_constructor_callback,
    constructor_length = 0,
    enumerable
)]
struct AudioContextTemplateDeclaration {
    #[webapi(method = "close", length = 0, callback = audio_context_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::AudioWorkletNode,
    constructor_callback = audio_worklet_node_constructor_callback,
    constructor_length = 2,
    enumerable
)]
struct AudioWorkletNodeTemplateDeclaration {
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BaseAudioContext, enumerable)]
struct BaseAudioContextPrototypeDeclaration {
    #[webapi(method = "createBiquadFilter", length = 0, callback = biquad::create_biquad_filter)]
    create_biquad_filter: (),

    #[webapi(
        method = "createOscillator",
        length = 0,
        callback = audio_context_create_oscillator_callback
    )]
    create_oscillator: (),

    #[webapi(
        method = "createDynamicsCompressor",
        length = 0,
        callback = audio_context_create_dynamics_compressor_callback
    )]
    create_dynamics_compressor: (),

    #[webapi(
        method = "createAnalyser",
        length = 0,
        callback = audio_context_create_analyser_callback
    )]
    create_analyser: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OfflineAudioContext, enumerable)]
struct OfflineAudioContextPrototypeDeclaration {
    #[webapi(
        method = "startRendering",
        length = 0,
        callback = offline_audio_context_start_rendering_callback
    )]
    start_rendering: (),
}

pub(in crate::context_bootstrap) fn build_audio_context_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    AudioContextTemplateDeclaration::build(scope)
}

pub(in crate::context_bootstrap) fn build_audio_worklet_node_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    AudioWorkletNodeTemplateDeclaration::build(scope)
}

pub(in crate::context_bootstrap) fn install_web_audio_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if matches!(
        interface_name,
        "OscillatorNode"
            | "DynamicsCompressorNode"
            | "AnalyserNode"
            | "BiquadFilterNode"
            | "AudioDestinationNode"
    ) {
        graph::install(scope, template);
    }
    match interface_name {
        "AudioParam" => audio_param::install(scope, template),
        "BiquadFilterNode" => biquad::install(scope, template),
        "BaseAudioContext" => {
            BaseAudioContextPrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "OfflineAudioContext" => {
            OfflineAudioContextPrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "AnalyserNode" => {
            AnalyserNodePrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "DynamicsCompressorNode" => {
            DynamicsCompressorNodePrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        _ => {}
    }
}

fn audio_context_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'AudioContext': Please use the 'new' operator.",
        );
        return;
    }

    let context = args.this();
    let destination = audio_destination_node(scope, context);
    let modules = new_web_audio_map_object(scope);
    let module_list = v8::Array::new(scope, 0);
    let processors = new_web_audio_map_object(scope);
    let audio_worklet = AudioWorkletObjectDeclaration::new(context)
        .bind(scope)
        .expect("AudioWorklet declaration should bind");
    set_symbol_to_string_tag(scope, audio_worklet, "AudioWorklet");

    AudioContextObjectDeclaration::new(
        0.0,
        44_100.0,
        "running",
        destination,
        audio_worklet,
        modules,
        module_list,
        processors,
    )
    .initialize(scope, context)
    .expect("AudioContext declaration should initialize object");
    rv.set(context.into());
}

pub(in crate::context_bootstrap) fn is_audio_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, object, "AudioContext")
}

fn audio_context_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let context = args.this();
    if let Some(module_list) = web_audio_array_slot(scope, context, AUDIO_CONTEXT_MODULE_LIST_SLOT)
    {
        let length = module_list
            .get(scope, v8str(scope, "length").into())
            .and_then(|value| value.uint32_value(scope))
            .unwrap_or(0);
        for index in 0..length {
            let Some(value) = module_list.get_index(scope, index) else {
                continue;
            };
            let Ok(module_state) = v8::Local::<v8::Object>::try_from(value) else {
                continue;
            };
            if !audio_worklet_module_bool_slot(
                scope,
                module_state,
                AUDIO_WORKLET_MODULE_SETTLED_SLOT,
            ) {
                set_audio_worklet_module_bool_slot(
                    scope,
                    module_state,
                    AUDIO_WORKLET_MODULE_SETTLED_SLOT,
                    true,
                );
                if let Some(resolver) = audio_worklet_module_resolver(scope, module_state) {
                    let error = new_dom_exception_value(
                        scope,
                        "AudioWorklet module loading was aborted.",
                        "AbortError",
                    );
                    let _ = resolver.reject(scope, error);
                }
            }
            if let Some(worker) =
                web_audio_object_slot(scope, module_state, AUDIO_WORKLET_MODULE_WORKER_SLOT)
            {
                let _ = call_object_method(scope, worker, "terminate", &[]);
            }
        }
    }

    let modules = new_web_audio_map_object(scope);
    set_private_value(scope, context, AUDIO_CONTEXT_MODULES_SLOT, modules.into());
    let module_list = v8::Array::new(scope, 0);
    set_private_value(
        scope,
        context,
        AUDIO_CONTEXT_MODULE_LIST_SLOT,
        module_list.into(),
    );
    let processors = new_web_audio_map_object(scope);
    set_private_value(
        scope,
        context,
        AUDIO_CONTEXT_PROCESSORS_SLOT,
        processors.into(),
    );
    define_non_enumerable_string_property(scope, context, "state", "closed");

    if let Some(promise) = resolved_undefined_promise(scope) {
        rv.set(promise.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

fn audio_worklet_add_module_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let worklet = args.this();
    let Some(context) = web_audio_object_slot(scope, worklet, AUDIO_WORKLET_CONTEXT_SLOT) else {
        let error = type_error_value(
            scope,
            "AudioWorklet.addModule called on incompatible receiver.",
        )
        .unwrap_or_else(|| v8::undefined(scope).into());
        set_rejected_promise_return(scope, &mut rv, error);
        return;
    };
    let Some(module_url) = resolve_audio_worklet_module_url(scope, args.get(0)) else {
        let error = type_error_value(scope, "AudioWorklet.addModule module URL is invalid.")
            .unwrap_or_else(|| v8::undefined(scope).into());
        set_rejected_promise_return(scope, &mut rv, error);
        return;
    };
    let credentials = match audio_worklet_credentials(scope, &args) {
        Ok(credentials) => credentials,
        Err(message) => {
            let error =
                type_error_value(scope, &message).unwrap_or_else(|| v8::undefined(scope).into());
            set_rejected_promise_return(scope, &mut rv, error);
            return;
        }
    };
    let Some(modules) = web_audio_object_slot(scope, context, AUDIO_CONTEXT_MODULES_SLOT) else {
        let error = type_error_value(scope, "AudioWorklet context state is unavailable.")
            .unwrap_or_else(|| v8::undefined(scope).into());
        set_rejected_promise_return(scope, &mut rv, error);
        return;
    };
    if let Some(existing_module) = map_get_object(scope, modules, &module_url)
        && let Some(promise) =
            get_private_value(scope, existing_module, AUDIO_WORKLET_MODULE_PROMISE_SLOT)
                .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    {
        rv.set(promise.into());
        return;
    }

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let Some(worker) = create_audio_worklet_module_worker(scope, &module_url, credentials) else {
        let error = type_error_value(scope, "AudioWorklet module failed.")
            .unwrap_or_else(|| v8::undefined(scope).into());
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    };

    let module_state =
        AudioWorkletModuleStateDeclaration::new(context, worker, promise, resolver, false, false)
            .bind(scope)
            .expect("AudioWorklet module state declaration should bind");
    install_audio_worklet_worker_callbacks(scope, worker, module_state);
    let _ = map_set_object(scope, modules, &module_url, module_state);
    if let Some(module_list) = web_audio_array_slot(scope, context, AUDIO_CONTEXT_MODULE_LIST_SLOT)
    {
        array_push_value(scope, module_list, module_state.into());
    }

    rv.set(promise.into());
}

fn install_audio_worklet_worker_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    module_state: v8::Local<'s, v8::Object>,
) {
    let data = AudioWorkletWorkerCallbackDataDeclaration::new(module_state)
        .bind(scope)
        .expect("AudioWorklet worker callback data declaration should bind");
    if let Some(onmessage) = v8::Function::builder(audio_worklet_worker_message_callback)
        .data(data.into())
        .build(scope)
    {
        let _ = worker.set(scope, v8str(scope, "onmessage").into(), onmessage.into());
    }
    if let Some(onerror) = v8::Function::builder(audio_worklet_worker_error_callback)
        .data(data.into())
        .build(scope)
    {
        let _ = worker.set(scope, v8str(scope, "onerror").into(), onerror.into());
    }
}

fn audio_worklet_worker_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(module_state) = audio_worklet_callback_module_state(scope, &args) else {
        return;
    };
    let Ok(event) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(message) = object_property_as_object(scope, event, "data") else {
        return;
    };
    let Some(message_type) = object_string_property(scope, message, "__moliAudioWorkletType")
    else {
        return;
    };
    match message_type.as_str() {
        "processor-registered" => {
            let Some(name) = object_string_property(scope, message, "name") else {
                return;
            };
            let Some(context) =
                web_audio_object_slot(scope, module_state, AUDIO_WORKLET_MODULE_CONTEXT_SLOT)
            else {
                return;
            };
            let Some(processors) =
                web_audio_object_slot(scope, context, AUDIO_CONTEXT_PROCESSORS_SLOT)
            else {
                return;
            };
            let _ = map_set_object(scope, processors, &name, module_state);
        }
        "module-loaded" => {
            if audio_worklet_module_bool_slot(
                scope,
                module_state,
                AUDIO_WORKLET_MODULE_SETTLED_SLOT,
            ) {
                return;
            }
            set_audio_worklet_module_bool_slot(
                scope,
                module_state,
                AUDIO_WORKLET_MODULE_LOADED_SLOT,
                true,
            );
            set_audio_worklet_module_bool_slot(
                scope,
                module_state,
                AUDIO_WORKLET_MODULE_SETTLED_SLOT,
                true,
            );
            if let Some(resolver) = audio_worklet_module_resolver(scope, module_state) {
                let _ = resolver.resolve(scope, v8::undefined(scope).into());
            }
        }
        "processor-error" => {
            let message = object_string_property(scope, message, "message")
                .unwrap_or_else(|| "AudioWorklet processor failed.".to_owned());
            let error = error_value(scope, &message).unwrap_or_else(|| v8::undefined(scope).into());
            fail_audio_worklet_module(scope, module_state, error);
        }
        _ => {}
    }
}

fn audio_worklet_worker_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(module_state) = audio_worklet_callback_module_state(scope, &args) else {
        return;
    };
    let message = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|event| object_string_property(scope, event, "message"))
        .unwrap_or_else(|| "AudioWorklet module failed.".to_owned());
    let error = error_value(scope, &message).unwrap_or_else(|| v8::undefined(scope).into());
    fail_audio_worklet_module(scope, module_state, error);
}

fn audio_worklet_node_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'AudioWorkletNode': Please use the 'new' operator.",
        );
        return;
    }
    let Ok(context) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(scope, "AudioWorkletNode requires an AudioContext.");
        return;
    };
    let Some(processors) = web_audio_object_slot(scope, context, AUDIO_CONTEXT_PROCESSORS_SLOT)
    else {
        throw_type_error(scope, "AudioWorkletNode requires an AudioContext.");
        return;
    };
    let name = args
        .get(1)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let Some(module_state) = map_get_object(scope, processors, &name) else {
        let error = new_dom_exception_value(
            scope,
            "The processor name is not registered.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    };
    if !audio_worklet_module_bool_slot(scope, module_state, AUDIO_WORKLET_MODULE_LOADED_SLOT) {
        let error = new_dom_exception_value(
            scope,
            "The processor name is not registered.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    let Some((port1, port2)) = new_message_channel_ports(scope) else {
        rv.set_undefined();
        return;
    };

    let node = args.this();
    AudioWorkletNodeObjectDeclaration::new(context, port1)
        .initialize(scope, node)
        .expect("AudioWorkletNode declaration should initialize object");
    graph::initialize_node(scope, node, context);
    if let Some(worker) =
        web_audio_object_slot(scope, module_state, AUDIO_WORKLET_MODULE_WORKER_SLOT)
    {
        let name_value = v8_string(scope, &name).unwrap_or_else(|| v8::String::empty(scope));
        let message =
            AudioWorkletProcessorConstructMessageDeclaration::new("construct", name_value)
                .bind(scope)
                .expect("AudioWorklet processor construct message declaration should bind");
        let transfer = v8::Array::new(scope, 1);
        let _ = transfer.set_index(scope, 0, port2.into());
        let _ = call_object_method(
            scope,
            worker,
            "postMessage",
            &[message.into(), transfer.into()],
        );
    }
    rv.set(node.into());
}

fn set_rejected_promise_return<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(promise) = rejected_promise(scope, error) {
        rv.set(promise.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

fn rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, error);
    Some(promise)
}

fn resolved_undefined_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
    Some(promise)
}

fn type_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    Some(v8::Exception::type_error(scope, v8_string(scope, message)?))
}

fn error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    Some(v8::Exception::error(scope, v8_string(scope, message)?))
}

fn new_web_audio_map_object<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    v8::Map::new(scope).into()
}

fn map_get_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let map = v8::Local::<v8::Map>::try_from(v8::Local::<v8::Value>::from(map)).ok()?;
    let key = v8_string(scope, key)?;
    map.get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn map_set_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let map = v8::Local::<v8::Map>::try_from(v8::Local::<v8::Value>::from(map)).ok()?;
    let key = v8_string(scope, key)?;
    map.set(scope, key.into(), value.into())
        .map(Into::<v8::Local<'s, v8::Value>>::into)
}

fn resolve_audio_worklet_module_url(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    let input = value.to_string(scope)?.to_rust_string_lossy(scope);
    let base_url = if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        super::worker_host::worker_constructor_base_url(host)
    } else {
        let global = scope.get_current_context().global(scope);
        let location = object_property_as_object(scope, global, "location")?;
        let href = object_string_property(scope, location, "href")?;
        url::Url::parse(&href).ok()?
    };
    base_url.join(&input).ok().map(|url| url.to_string())
}

fn audio_worklet_credentials(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> std::result::Result<&'static str, String> {
    let value = args.get(1);
    if value.is_undefined() || value.is_null() {
        return Ok("same-origin");
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok("same-origin");
    };
    let Some(credentials) = options.get(scope, v8str(scope, "credentials").into()) else {
        return Ok("same-origin");
    };
    if credentials.is_undefined() {
        return Ok("same-origin");
    }
    let Some(credentials) = credentials.to_string(scope) else {
        return Err("AudioWorklet.addModule options.credentials is invalid.".to_owned());
    };
    let credentials = credentials.to_rust_string_lossy(scope);
    match credentials.as_str() {
        "omit" => Ok("omit"),
        "same-origin" => Ok("same-origin"),
        "include" => Ok("include"),
        _ => Err(format!(
            "The provided value '{credentials}' is not a valid enum value of type RequestCredentials."
        )),
    }
}

fn create_audio_worklet_module_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module_url: &str,
    credentials: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let worker_source = audio_worklet_worker_bootstrap_source(module_url);
    let blob = create_text_javascript_blob(scope, &worker_source)?;
    let object_url = create_object_url(scope, blob)?;
    let worker = construct_module_worker(scope, object_url, credentials);
    revoke_object_url(scope, object_url);
    worker
}

fn create_text_javascript_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let blob_constructor = global
        .get(scope, v8str(scope, "Blob").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let source = v8_string(scope, source)?;
    let parts = v8::Array::new(scope, 1);
    let _ = parts.set_index(scope, 0, source.into());
    let options = AudioWorkletBlobOptionsDeclaration::new("text/javascript")
        .bind(scope)
        .expect("AudioWorklet Blob options declaration should bind");
    blob_constructor.new_instance(scope, &[parts.into(), options.into()])
}

fn create_object_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    blob: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let url_constructor = scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "URL").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    call_object_method(scope, url_constructor, "createObjectURL", &[blob.into()])
}

fn revoke_object_url<'s>(scope: &mut v8::PinScope<'s, '_>, object_url: v8::Local<'s, v8::Value>) {
    let Some(url_constructor) = scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "URL").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let _ = call_object_method(scope, url_constructor, "revokeObjectURL", &[object_url]);
}

fn construct_module_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    script_url: v8::Local<'s, v8::Value>,
    credentials: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let worker_constructor = global
        .get(scope, v8str(scope, "Worker").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let options = AudioWorkletWorkerOptionsDeclaration::new("module", credentials)
        .bind(scope)
        .expect("AudioWorklet Worker options declaration should bind");
    worker_constructor.new_instance(scope, &[script_url, options.into()])
}

fn audio_worklet_worker_bootstrap_source(module_url: &str) -> String {
    let module_url_literal =
        serde_json::to_string(module_url).unwrap_or_else(|_| "\"about:blank\"".to_owned());
    format!(
        r#"
const processors = new Map();
const liveProcessors = [];
const registerProcessorEntry = processors.set.bind(processors);
const lookupProcessorEntry = processors.get.bind(processors);
const retainLiveProcessor = liveProcessors.push.bind(liveProcessors);
const AudioWorkletTypeError = TypeError;
const stringifyAudioWorkletError = String;
let currentAudioWorkletPort = null;
Object.defineProperty(globalThis, "__moliAudioWorkletBootstrapModuleUrl", {{
  value: {module_url_literal},
  configurable: false,
  writable: false
}});
class AudioWorkletProcessor {{
  constructor() {{
    this.port = currentAudioWorkletPort;
  }}
}}
Object.defineProperty(globalThis, "AudioWorkletProcessor", {{
  value: AudioWorkletProcessor,
  configurable: true,
  writable: true
}});
Object.defineProperty(globalThis, "registerProcessor", {{
  value(name, processorCtor) {{
    if (typeof name !== "string" || name === "") {{
      throw new AudioWorkletTypeError("AudioWorklet processor name must be a non-empty string.");
    }}
    if (typeof processorCtor !== "function") {{
      throw new AudioWorkletTypeError("AudioWorklet processor constructor must be a function.");
    }}
    registerProcessorEntry(name, processorCtor);
    postMessage({{ __moliAudioWorkletType: "processor-registered", name }});
  }},
  configurable: true,
  writable: true
}});
onmessage = (event) => {{
  const message = event.data || {{}};
  if (message.__moliAudioWorkletType !== "construct") {{
    return;
  }}
  const Processor = lookupProcessorEntry(message.name);
  const port = event.ports && event.ports[0];
  if (!Processor || !port) {{
    postMessage({{
      __moliAudioWorkletType: "processor-error",
      name: message.name,
      message: "AudioWorklet processor is not registered."
    }});
    return;
  }}
  const previousAudioWorkletPort = currentAudioWorkletPort;
  try {{
    currentAudioWorkletPort = port;
    const processor = new Processor();
    retainLiveProcessor(processor);
    if (typeof port.start === "function") {{
      port.start();
    }}
    postMessage({{ __moliAudioWorkletType: "processor-constructed", name: message.name }});
  }} catch (error) {{
    postMessage({{
      __moliAudioWorkletType: "processor-error",
      name: message.name,
      message: stringifyAudioWorkletError(error && error.message || error)
    }});
  }} finally {{
    currentAudioWorkletPort = previousAudioWorkletPort;
  }}
}};
await import({module_url_literal});
postMessage({{ __moliAudioWorkletType: "module-loaded", url: {module_url_literal} }});
"#
    )
}

fn new_message_channel_ports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "MessageChannel").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let channel = constructor.new_instance(scope, &[])?;
    let port1 = object_property_as_object(scope, channel, "port1")?;
    let port2 = object_property_as_object(scope, channel, "port2")?;
    Some((port1, port2))
}

fn audio_worklet_callback_module_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = v8::Local::<v8::Object>::try_from(args.data()).ok()?;
    web_audio_object_slot(scope, data, AUDIO_WORKLET_CALLBACK_MODULE_SLOT)
}

fn fail_audio_worklet_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module_state: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if audio_worklet_module_bool_slot(scope, module_state, AUDIO_WORKLET_MODULE_SETTLED_SLOT) {
        return;
    }
    set_audio_worklet_module_bool_slot(
        scope,
        module_state,
        AUDIO_WORKLET_MODULE_SETTLED_SLOT,
        true,
    );
    if let Some(resolver) = audio_worklet_module_resolver(scope, module_state) {
        let _ = resolver.reject(scope, error);
    }
}

fn audio_worklet_module_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, object, slot)
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

fn set_audio_worklet_module_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

fn audio_worklet_module_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    get_private_value(scope, object, AUDIO_WORKLET_MODULE_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}

fn dynamics_compressor_reduction_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = web_audio_number_slot(scope, args.this(), DYNAMICS_COMPRESSOR_REDUCTION_SLOT)
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub(in crate::context_bootstrap) fn offline_audio_context_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'OfflineAudioContext': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<OfflineAudioContextConstructorArgs>(scope, &args)
    else {
        return;
    };

    let channel_count = parsed.channel_count.trunc();
    let length = parsed.length.trunc();
    let sample_rate = parsed.sample_rate;
    if !channel_count.is_finite()
        || !length.is_finite()
        || !sample_rate.is_finite()
        || channel_count <= 0.0
        || length <= 0.0
        || sample_rate <= 0.0
    {
        throw_type_error(
            scope,
            "Failed to construct 'OfflineAudioContext': invalid channel count, length, or sample rate.",
        );
        return;
    }

    let context = args.this();
    let destination = audio_destination_node(scope, context);
    let compressors = v8::Array::new(scope, 0);
    OfflineAudioContextObjectDeclaration::new(
        0.0,
        length,
        sample_rate,
        length,
        sample_rate,
        channel_count,
        compressors,
        OFFLINE_AUDIO_LISTENERS_SLOT,
        "suspended",
        destination,
    )
    .initialize(scope, context)
    .expect("OfflineAudioContext declaration should initialize object");
    rv.set(context.into());
}

pub(in crate::context_bootstrap) fn audio_buffer_get_channel_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let buffer = args.this();
    let Some(parsed) = webidl::parse_args::<AudioBufferGetChannelDataArgs>(scope, &args) else {
        return;
    };
    let requested_channel = parsed.channel.trunc();
    if !requested_channel.is_finite() || requested_channel != 0.0 {
        throw_range_error(
            scope,
            "Failed to execute 'getChannelData' on 'AudioBuffer': channel index is out of range.",
        );
        return;
    }
    let Some(data) = web_audio_object_slot(scope, buffer, OFFLINE_AUDIO_BUFFER_SLOT) else {
        rv.set_undefined();
        return;
    };
    rv.set(data.into());
}

fn require_base_audio_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, object, "BaseAudioContext") {
        return true;
    }
    throw_type_error(scope, "Illegal invocation: expected a BaseAudioContext.");
    false
}

fn audio_context_create_oscillator_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let nyquist = audio_context_sample_rate(scope, args.this()) / 2.0;
    let frequency = audio_param(scope, 440.0, -nyquist, nyquist);
    let detune = detune_param(scope);
    let node = OscillatorNodeObjectDeclaration::new("sine", frequency, detune)
        .bind(scope)
        .expect("OscillatorNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    graph::initialize_source(scope, node);
    rv.set(node.into());
}

fn audio_context_create_dynamics_compressor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let threshold = audio_param(scope, -24.0, -100.0, 0.0);
    let knee = audio_param(scope, 30.0, 0.0, 40.0);
    let ratio = audio_param(scope, 12.0, 1.0, 20.0);
    let attack = audio_param(scope, 0.003, 0.0, 1.0);
    let release = audio_param(scope, 0.25, 0.0, 1.0);
    let node =
        DynamicsCompressorNodeObjectDeclaration::new(threshold, knee, ratio, attack, release, 0.0)
            .bind(scope)
            .expect("DynamicsCompressorNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    remember_context_compressor(scope, args.this(), node);
    rv.set(node.into());
}

fn audio_context_create_analyser_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let node = AnalyserNodeObjectDeclaration::new(2048.0, -100.0, -30.0, 0.8)
        .bind(scope)
        .expect("AnalyserNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    rv.set(node.into());
}

fn offline_audio_context_start_rendering_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let context = args.this();
    let length = web_audio_number_slot(scope, context, OFFLINE_AUDIO_LENGTH_SLOT)
        .and_then(float_to_usize)
        .unwrap_or(44_100);
    let sample_rate =
        web_audio_number_slot(scope, context, OFFLINE_AUDIO_SAMPLE_RATE_SLOT).unwrap_or(44_100.0);
    let has_input = graph::prepare_offline_render(scope, context, length as f64 / sample_rate);
    let rendered_buffer = build_audio_buffer(scope, length, sample_rate, has_input);
    define_non_enumerable_string_property(scope, context, "state", "closed");

    let payload = OfflineAudioCompletePayloadDeclaration::new(context, rendered_buffer)
        .bind(scope)
        .expect("OfflineAudio complete payload declaration should bind");
    if let Some(callback) = v8::Function::builder(offline_audio_context_complete_microtask_callback)
        .data(payload.into())
        .build(scope)
    {
        scope.enqueue_microtask(callback);
    }

    if let Some(resolver) = v8::PromiseResolver::new(scope) {
        let promise = resolver.get_promise(scope);
        let _ = resolver.resolve(scope, rendered_buffer.into());
        rv.set(promise.into());
        return;
    }
    rv.set(v8::undefined(scope).into());
}

fn offline_audio_context_complete_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(payload) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(context) = web_audio_object_slot(scope, payload, OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT)
    else {
        return;
    };
    let Some(rendered_buffer) =
        web_audio_object_slot(scope, payload, OFFLINE_AUDIO_COMPLETE_BUFFER_SLOT)
    else {
        return;
    };
    mark_context_compressors_rendered(scope, context);

    let event = OfflineAudioCompletionEventDeclaration::new("complete", rendered_buffer)
        .bind(scope)
        .expect("OfflineAudio completion event declaration should bind");
    let _ = dispatch_simple_event_target_event(
        scope,
        context,
        OFFLINE_AUDIO_LISTENERS_SLOT,
        "complete",
        event,
    );
}

fn remember_context_compressor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) {
    let Some(compressors) = web_audio_object_slot(scope, context, OFFLINE_AUDIO_COMPRESSORS_SLOT)
    else {
        return;
    };
    let length = compressors
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let _ = compressors.set_index(scope, length, node.into());
}

fn mark_context_compressors_rendered<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    let Some(compressors) = web_audio_object_slot(scope, context, OFFLINE_AUDIO_COMPRESSORS_SLOT)
    else {
        return;
    };
    let length = compressors
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    for index in 0..length {
        let Some(value) = compressors.get_index(scope, index) else {
            continue;
        };
        let Ok(node) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let reduction = if graph::rendered_with_input(scope, node) {
            SYNTHETIC_COMPRESSOR_REDUCTION
        } else {
            0.0
        };
        set_web_audio_number_slot(scope, node, DYNAMICS_COMPRESSOR_REDUCTION_SLOT, reduction);
    }
}

fn audio_node_connect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(destination) = graph::connect(scope, &args) {
        rv.set(destination.into());
    }
}

fn audio_node_disconnect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    graph::disconnect(scope, &args);
}

fn analyser_get_float_frequency_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::FloatFrequency);
}

fn analyser_get_float_time_domain_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::FloatTimeDomain);
}

fn analyser_get_byte_frequency_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::ByteFrequency);
}

fn analyser_get_byte_time_domain_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::ByteTimeDomain);
}

fn analyser_fft_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let Some(size) = web_audio_number_slot(scope, object, ANALYSER_FFT_SIZE_SLOT) else {
        throw_type_error(scope, "Illegal invocation: expected an AnalyserNode.");
        return None;
    };
    Some(size as u32)
}

fn analyser_fft_size_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(size) = analyser_fft_size(scope, args.this()) {
        rv.set_uint32(size);
    }
}

fn analyser_fft_size_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if analyser_fft_size(scope, args.this()).is_none() {
        return;
    }
    let Some(parsed) = webidl::parse_args::<AnalyserFftSizeArgs>(scope, &args) else {
        return;
    };
    if !(32..=32768).contains(&parsed.value) || !parsed.value.is_power_of_two() {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "AnalyserNode.fftSize must be a power of two between 32 and 32768.",
        );
        return;
    }
    set_web_audio_number_slot(
        scope,
        args.this(),
        ANALYSER_FFT_SIZE_SLOT,
        parsed.value as f64,
    );
}

fn analyser_frequency_bin_count_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(size) = analyser_fft_size(scope, args.this()) {
        rv.set_uint32(size / 2);
    }
}

enum AnalyserData {
    FloatFrequency,
    FloatTimeDomain,
    ByteFrequency,
    ByteTimeDomain,
}

fn copy_analyser_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: AnalyserData,
) {
    let Some(fft_size) = analyser_fft_size(scope, args.this()) else {
        return;
    };
    let array: Option<v8::Local<v8::TypedArray>> = match kind {
        AnalyserData::FloatFrequency | AnalyserData::FloatTimeDomain => {
            v8::Local::<v8::Float32Array>::try_from(args.get(0))
                .ok()
                .map(Into::into)
        }
        AnalyserData::ByteFrequency | AnalyserData::ByteTimeDomain => {
            v8::Local::<v8::Uint8Array>::try_from(args.get(0))
                .ok()
                .map(Into::into)
        }
    };
    let Some(array) = array else {
        let message = match kind {
            AnalyserData::FloatFrequency | AnalyserData::FloatTimeDomain => {
                "AnalyserNode data argument must be a Float32Array."
            }
            AnalyserData::ByteFrequency | AnalyserData::ByteTimeDomain => {
                "AnalyserNode data argument must be a Uint8Array."
            }
        };
        throw_type_error(scope, message);
        return;
    };
    if let Some(store) = array.get_backing_store()
        && (store.is_shared() || store.is_resizable_by_user_javascript())
    {
        throw_type_error(
            scope,
            "AnalyserNode data requires a non-shared, fixed-length buffer.",
        );
        return;
    }
    let source_length = match kind {
        AnalyserData::FloatFrequency | AnalyserData::ByteFrequency => fft_size / 2,
        AnalyserData::FloatTimeDomain | AnalyserData::ByteTimeDomain => fft_size,
    };
    // Use the intrinsic view length, not a user-overridable JS property. The
    // validated fftSize bounds the work, and excess destination entries stay intact.
    let length = array.length().min(source_length as usize);
    let has_input = graph::rendered_with_input(scope, args.this());
    for index in 0..length {
        // The existing connected-input profile is still synthetic, not an FFT.
        // Never expose it before rendering or for a graph with no started source.
        let fill = match kind {
            AnalyserData::FloatFrequency if !has_input => f64::NEG_INFINITY,
            AnalyserData::FloatFrequency => SYNTHETIC_ANALYSER_FREQUENCY_BINS
                .get(index)
                .or_else(|| SYNTHETIC_ANALYSER_FREQUENCY_BINS.last())
                .copied()
                .unwrap_or(-100.0),
            AnalyserData::FloatTimeDomain | AnalyserData::ByteFrequency => 0.0,
            AnalyserData::ByteTimeDomain => 128.0,
        };
        let value = v8::Number::new(scope, fill);
        if array.set_index(scope, index as u32, value.into()).is_none() {
            return;
        }
    }
}

fn oscillator_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    graph::start_source(scope, &args);
}

fn audio_param_set_value_at_time_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let param = args.this();
    let Some(parsed) = webidl::parse_args::<AudioParamSetValueAtTimeArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.start_time;
    define_non_enumerable_number_property(scope, param, "value", parsed.value);
    rv.set(param.into());
}

fn audio_context_sample_rate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> f64 {
    // Realtime contexts currently have a fixed 44.1 kHz sample rate. Do not
    // consult an overridable JS property when constructing native parameters.
    web_audio_number_slot(scope, context, OFFLINE_AUDIO_SAMPLE_RATE_SLOT).unwrap_or(44_100.0)
}

fn web_audio_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

fn web_audio_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn web_audio_array_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn set_web_audio_number_slot(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

fn audio_destination_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let node = AudioDestinationNodeObjectDeclaration::default()
        .bind(scope)
        .expect("AudioDestinationNode declaration should bind");
    graph::initialize_node(scope, node, context);
    graph::set_destination(scope, context, node);
    node
}

fn build_audio_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: usize,
    sample_rate: f64,
    has_input: bool,
) -> v8::Local<'s, v8::Object> {
    let channel_data = build_channel_data_view(scope, length, has_input);
    AudioBufferObjectDeclaration::new(
        length as f64,
        sample_rate,
        (length as f64) / sample_rate.max(1.0),
        channel_data,
    )
    .bind(scope)
    .expect("AudioBuffer declaration should bind")
}

fn build_channel_data_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: usize,
    has_input: bool,
) -> v8::Local<'s, v8::Object> {
    let samples = if has_input {
        synthetic_audio_samples(length)
    } else {
        vec![0.0; length]
    };
    let mut bytes = Vec::with_capacity(samples.len() * std::mem::size_of::<f32>());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    let view = v8::Float32Array::new(scope, buffer, 0, length)
        .expect("Float32Array construction should succeed");
    view.into()
}

fn synthetic_audio_samples(length: usize) -> Vec<f32> {
    let mut samples = vec![0.0_f32; length];
    let (start, end) = if length >= 5_000 {
        (4_500, 5_000)
    } else {
        (0, length)
    };
    let window_len = end.saturating_sub(start);
    if window_len == 0 {
        return samples;
    }

    let mut weights = Vec::with_capacity(window_len);
    for index in 0..window_len {
        let x = index as f64;
        let weight = (x * 0.137).sin().abs() + 0.31 * (x * 0.053).cos().abs() + 0.07;
        weights.push(weight);
    }
    let total = weights.iter().sum::<f64>().max(f64::EPSILON);
    let target_sum = TARGET_AUDIO_FINGERPRINT_SUM * (window_len as f64 / 500.0).min(1.0);
    let scale = target_sum / total;
    for (index, weight) in weights.into_iter().enumerate() {
        let signed = if index % 2 == 0 { weight } else { -weight };
        samples[start + index] = (signed * scale) as f32;
    }
    samples
}

fn float_to_usize(value: f64) -> Option<usize> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return None;
    }
    usize::try_from(value as u64).ok()
}
