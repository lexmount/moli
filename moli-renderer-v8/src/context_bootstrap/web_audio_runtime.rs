use super::media_queries::dispatch_simple_event_target_event;
use super::*;
use crate::native_bridge::throw_dom_exception;
use crate::util::{
    call_object_method, get_private_value, object_string_property, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use moli_webapi::web_audio::biquad;
use moli_webapi::web_audio::buffer;
use moli_webapi::web_audio::channel_nodes;
use moli_webapi::web_audio::convolver;
use moli_webapi::web_audio::format;
use moli_webapi::web_audio::gain;

pub(in crate::context_bootstrap) use convolver::constructor as convolver_constructor;
pub(in crate::context_bootstrap) use iir::constructor as iir_constructor;
use moli_webapi::web_audio::graph;
use moli_webapi::web_audio::iir;
use moli_webapi::web_audio::listener;
mod media_element_source;
mod offline_context;
use moli_webapi::web_audio::oscillator;
use moli_webapi::web_audio::param_nodes;
use moli_webapi::web_audio::periodic_wave;
use moli_webapi::web_audio::source;
use moli_webapi::web_audio::wave_shaper;

pub(in crate::context_bootstrap) use offline_context::constructor as offline_audio_context_constructor_callback;
pub(in crate::context_bootstrap) use oscillator::constructor as oscillator_constructor;
pub(in crate::context_bootstrap) use periodic_wave::constructor as periodic_wave_constructor;
pub(in crate::context_bootstrap) use wave_shaper::constructor as wave_shaper_constructor;

pub(in crate::context_bootstrap) use buffer::constructor as audio_buffer_constructor;
pub(in crate::context_bootstrap) use channel_nodes::{
    channel_merger_constructor, channel_splitter_constructor,
};
pub(in crate::context_bootstrap) use param_nodes::{delay_constructor, stereo_panner_constructor};
pub(in crate::context_bootstrap) use source::{
    audio_buffer_source_constructor_callback, constant_source_constructor_callback,
};

pub(in crate::context_bootstrap) use gain::constructor as gain_node_constructor_callback;
pub(in crate::context_bootstrap) use media_element_source::constructor as media_element_source_constructor_callback;

use moli_webapi::web_audio::*;

const AUDIO_CONTEXT_MODULES_SLOT: &str = "__moliAudioContextModules";
const AUDIO_CONTEXT_MODULE_LIST_SLOT: &str = "__moliAudioContextModuleList";
const AUDIO_CONTEXT_PROCESSORS_SLOT: &str = "__moliAudioContextProcessors";
const AUDIO_CONTEXT_WORKLET_SLOT: &str = "__moliAudioContextWorklet";
const AUDIO_WORKLET_CONTEXT_SLOT: &str = "__moliAudioWorkletContext";
const AUDIO_WORKLET_MODULE_CONTEXT_SLOT: &str = "__moliAudioWorkletModuleContext";
const AUDIO_WORKLET_MODULE_WORKER_SLOT: &str = "__moliAudioWorkletModuleWorker";
const AUDIO_WORKLET_MODULE_RESOLVERS_SLOT: &str = "__moliAudioWorkletModuleResolvers";
const AUDIO_WORKLET_MODULE_ERROR_SLOT: &str = "__moliAudioWorkletModuleError";
const AUDIO_WORKLET_MODULE_LOADED_SLOT: &str = "__moliAudioWorkletModuleLoaded";
const AUDIO_WORKLET_MODULE_SETTLED_SLOT: &str = "__moliAudioWorkletModuleSettled";
const AUDIO_WORKLET_CALLBACK_MODULE_SLOT: &str = "__moliAudioWorkletCallbackModule";
const OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT: &str = "__moliOfflineAudioCompleteContext";
const OFFLINE_AUDIO_COMPLETE_BUFFER_SLOT: &str = "__moliOfflineAudioCompleteBuffer";

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
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),
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
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
    #[webapi(slot = AUDIO_WORKLET_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioWorkletNode)]
struct AudioWorkletNodeObjectDeclaration<'scope> {
    #[webapi(data_property)]
    port: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct AudioWorkletModuleStateDeclaration<'scope> {
    #[webapi(slot = AUDIO_WORKLET_MODULE_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_WORKER_SLOT)]
    worker: v8::Local<'scope, v8::Object>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_RESOLVERS_SLOT)]
    resolvers: v8::Local<'scope, v8::Array>,
    #[webapi(slot = AUDIO_WORKLET_MODULE_LOADED_SLOT)]
    loaded: bool,
    #[webapi(slot = AUDIO_WORKLET_MODULE_SETTLED_SLOT)]
    settled: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct AudioWorkletWorkerCallbackDataDeclaration<'scope> {
    #[webapi(slot = AUDIO_WORKLET_CALLBACK_MODULE_SLOT)]
    module_state: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct AudioWorkletBlobOptionsDeclaration {
    #[webapi(data_property = "type")]
    kind: &'static str,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct AudioWorkletWorkerOptionsDeclaration {
    #[webapi(data_property = "type")]
    kind: &'static str,
    credentials: &'static str,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
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
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),
    #[webapi(data_property)]
    state: &'static str,
    #[webapi(data_property)]
    destination: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct OfflineAudioCompletePayloadDeclaration<'scope> {
    #[webapi(slot = OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(slot = OFFLINE_AUDIO_COMPLETE_BUFFER_SLOT)]
    rendered_buffer: v8::Local<'scope, v8::Object>,
}

// Captured from the Chromium-on-Linux baseline we use for Zhihu probe parity.
// If that browser profile changes, update this together with the audio probe
// assertions in `script_vm/tests.rs`.
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
    #[webapi(method = "createMediaElementSource", length = 1, callback = media_element_source::create)]
    create_media_element_source: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::AudioWorkletNode,
    constructor_callback = audio_worklet_node_constructor_callback,
    constructor_length = 2,
    enumerable
)]
struct AudioWorkletNodeTemplateDeclaration {
    #[webapi(
        accessor_property = "onprocessorerror",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onprocessorerror"),
        setter_data = v8str(scope, "onprocessorerror"),
        enumerable
    )]
    onprocessorerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BaseAudioContext, enumerable)]
struct BaseAudioContextPrototypeDeclaration {
    #[webapi(accessor_property, getter = listener::get, enumerable)]
    listener: (),

    #[webapi(
        accessor_property = "onstatechange",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onstatechange"),
        setter_data = v8str(scope, "onstatechange"),
        enumerable
    )]
    onstatechange: (),

    #[webapi(accessor_property, getter = offline_context::render_quantum_size_getter)]
    render_quantum_size: (),
    #[webapi(method, length = 3, callback = buffer::create)]
    create_buffer: (),
    #[webapi(method, length = 0, callback = wave_shaper::create)]
    create_wave_shaper: (),
    #[webapi(method, length = 0, callback = convolver::create)]
    create_convolver: (),
    #[webapi(method = "createIIRFilter", length = 2, callback = iir::create)]
    create_iir_filter: (),
    #[webapi(method, length = 2, callback = periodic_wave::create)]
    create_periodic_wave: (),
    #[webapi(method, length = 0, callback = channel_nodes::create_channel_merger)]
    create_channel_merger: (),
    #[webapi(method, length = 0, callback = channel_nodes::create_channel_splitter)]
    create_channel_splitter: (),
    #[webapi(method, length = 0, callback = param_nodes::create_delay)]
    create_delay: (),
    #[webapi(method, length = 0, callback = param_nodes::create_stereo_panner)]
    create_stereo_panner: (),
    #[webapi(method, length = 0, callback = source::create_buffer_source)]
    create_buffer_source: (),
    #[webapi(method, length = 0, callback = source::create_constant_source)]
    create_constant_source: (),
    #[webapi(method = "createBiquadFilter", length = 0, callback = biquad::create_biquad_filter)]
    create_biquad_filter: (),

    #[webapi(method = "createGain", length = 0, callback = gain::create)]
    create_gain: (),

    #[webapi(
        method = "createOscillator",
        length = 0,
        callback = oscillator::create
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
        accessor_property = "oncomplete",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "oncomplete"),
        setter_data = v8str(scope, "oncomplete"),
        enumerable
    )]
    oncomplete: (),

    #[webapi(
        method = "startRendering",
        returns_promise,
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
    moli_webapi::web_audio::install(scope, template, interface_name);
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Worklet" => WorkletPrototypeDeclaration::initialize_prototype_template(scope, prototype),
        "BaseAudioContext" => {
            BaseAudioContextPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "OfflineAudioContext" => {
            OfflineAudioContextPrototypeDeclaration::initialize_prototype_template(scope, prototype)
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

    AudioContextObjectDeclaration::new(
        0.0,
        44_100.0,
        "running",
        destination,
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
    web_api_interfaces::AudioContext::is_instance(scope, object)
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
                let error = new_dom_exception_value(
                    scope,
                    "AudioWorklet module loading was aborted.",
                    "AbortError",
                );
                settle_audio_worklet_module(scope, module_state, Err(error));
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

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Worklet, enumerable)]
struct WorkletPrototypeDeclaration {
    #[webapi(method = "addModule", length = 1, returns_promise, callback = audio_worklet_add_module_callback)]
    add_module: (),
}

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "BaseAudioContext", enumerable, receiver = web_api_interfaces::BaseAudioContext::is_instance)]
struct BaseAudioContextSecureAttributes {
    #[webapi(accessor_property = "audioWorklet", getter = audio_worklet_getter)]
    audio_worklet: (),
}

pub(super) fn finalize_base_audio_context_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    if super::runtime_state::window_realm_secure_context_available(scope, global) {
        BaseAudioContextSecureAttributes::default()
            .initialize(scope, prototype)
            .map_err(|error| anyhow!(error))?;
    }
    Ok(())
}

fn audio_worklet_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let context = args.this();
    if let Some(worklet) = web_audio_object_slot(scope, context, AUDIO_CONTEXT_WORKLET_SLOT) {
        rv.set(worklet.into());
        return;
    }
    let Some(realm) = context.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Ok(prototype) = ensure_intrinsic_interface_prototype(scope, "AudioWorklet") else {
        return;
    };
    let worklet = AudioWorkletObjectDeclaration::new(prototype, context)
        .bind(scope)
        .expect("AudioWorklet declaration should bind");
    set_private_value(scope, context, AUDIO_CONTEXT_WORKLET_SLOT, worklet.into());
    rv.set(worklet.into());
}

use moli_webapi::worker_options::WorkletAddModuleArgs;

fn audio_worklet_add_module_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WorkletAddModuleArgs>(scope, &args) else {
        return;
    };
    let worklet = args.this();
    let Some(context) = web_audio_object_slot(scope, worklet, AUDIO_WORKLET_CONTEXT_SLOT) else {
        throw_type_error(scope, "AudioWorklet context state is unavailable.");
        return;
    };
    // Argument conversion failures use the method's realm. The operation's
    // Promise, URL resolution and workers use the Worklet's relevant realm.
    let Some(realm) = worklet.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some(module_url) = resolve_audio_worklet_module_url(scope, &parsed.module_url) else {
        let error = new_dom_exception_value(
            scope,
            "AudioWorklet.addModule module URL is invalid.",
            "SyntaxError",
        );
        let _ = resolver.reject(scope, error);
        return;
    };
    let Some(modules) = web_audio_object_slot(scope, context, AUDIO_CONTEXT_MODULES_SLOT) else {
        let error = v8::Exception::type_error(
            scope,
            v8str(scope, "AudioWorklet context state is unavailable."),
        );
        let _ = resolver.reject(scope, error);
        return;
    };
    if let Some(module_state) = map_get_object(scope, modules, &module_url) {
        if audio_worklet_module_bool_slot(scope, module_state, AUDIO_WORKLET_MODULE_SETTLED_SLOT) {
            if audio_worklet_module_bool_slot(scope, module_state, AUDIO_WORKLET_MODULE_LOADED_SLOT)
            {
                let _ = resolver.resolve(scope, v8::undefined(scope).into());
            } else if let Some(error) =
                get_private_value(scope, module_state, AUDIO_WORKLET_MODULE_ERROR_SLOT)
            {
                let _ = resolver.reject(scope, error);
            }
        } else if let Some(resolvers) =
            web_audio_array_slot(scope, module_state, AUDIO_WORKLET_MODULE_RESOLVERS_SLOT)
        {
            let key = v8_string(scope, &resolvers.length().to_string()).expect("array index");
            let _ = resolvers.create_data_property(scope, key.into(), resolver.into());
        }
        return;
    }
    let Some(worker) =
        create_audio_worklet_module_worker(scope, &module_url, parsed.options.credentials.0.into())
    else {
        let error = type_error_value(scope, "AudioWorklet module failed.")
            .unwrap_or_else(|| v8::undefined(scope).into());
        let _ = resolver.reject(scope, error);
        return;
    };
    let resolvers = v8::Array::new_with_elements(scope, &[resolver.into()]);
    let module_state =
        AudioWorkletModuleStateDeclaration::new(context, worker, resolvers, false, false)
            .bind(scope)
            .expect("AudioWorklet module state declaration should bind");
    install_audio_worklet_worker_callbacks(scope, worker, module_state);
    let _ = map_set_object(scope, modules, &module_url, module_state);
    if let Some(module_list) = web_audio_array_slot(scope, context, AUDIO_CONTEXT_MODULE_LIST_SLOT)
    {
        let key = v8_string(scope, &module_list.length().to_string()).expect("array index");
        let _ = module_list.create_data_property(scope, key.into(), module_state.into());
    }
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
        "module-loaded" => settle_audio_worklet_module(scope, module_state, Ok(())),
        "processor-error" => {
            let message = object_string_property(scope, message, "message")
                .unwrap_or_else(|| "AudioWorklet processor failed.".to_owned());
            let error = error_value(scope, &message).unwrap_or_else(|| v8::undefined(scope).into());
            settle_audio_worklet_module(scope, module_state, Err(error));
        }
        _ => {}
    }
}

fn audio_worklet_worker_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(module_state) = audio_worklet_callback_module_state(scope, &args) else {
        return;
    };
    let message = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|event| {
            crate::context_bootstrap::event_attribute(scope, event, "message")
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
        })
        .unwrap_or_else(|| "AudioWorklet module failed.".to_owned());
    let error = error_value(scope, &message).unwrap_or_else(|| v8::undefined(scope).into());
    settle_audio_worklet_module(scope, module_state, Err(error));
    // This internal worker failure is reported through addModule's Promise.
    // Cancel the handled error instead of reporting it again on Window.
    rv.set_bool(false);
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
    AudioWorkletNodeObjectDeclaration::new(port1)
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
    input: &str,
) -> Option<String> {
    use crate::native_bridge::OwnerDispatchScope;

    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    // SAFETY: the bridge owns the host for the lifetime of the callback.
    let host = unsafe { &*host_ptr };
    let global = scope.get_current_context().global(scope);
    let document = match super::navigation_window::runtime_window_dispatch_scope(scope, global)? {
        OwnerDispatchScope::Top => host.document_handle(),
        OwnerDispatchScope::Child(handle) => host.child_browsing_context_document_handle(handle)?,
        OwnerDispatchScope::LightweightPopup(id) => host.lightweight_popup_document_handle(id)?,
    };
    host.document_base_url_for_handle(document)
        .join(input)
        .ok()
        .map(|url| url.to_string())
}

fn create_audio_worklet_module_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module_url: &str,
    credentials: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let worker_source = audio_worklet_worker_bootstrap_source(module_url);
    let blob = create_text_javascript_blob(scope, &worker_source)?;
    let object_url = create_object_url(scope, blob)?;
    let worker = construct_module_worker(scope, object_url, credentials, module_url);
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
    module_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let worker_constructor = ensure_intrinsic_interface_constructor(scope, "Worker").ok()?;
    let options = AudioWorkletWorkerOptionsDeclaration::new("module", credentials)
        .bind(scope)
        .expect("AudioWorklet Worker options declaration should bind");
    // Keep the native Worklet settings private during dictionary conversion.
    let null_prototype = v8::null(scope);
    if options.set_prototype(scope, null_prototype.into()) != Some(true) {
        return None;
    }
    let native_module_url = v8_string(scope, module_url)?;
    set_private_value(
        scope,
        options,
        super::worker_host::AUDIO_WORKLET_WORKER_OPTION_SLOT,
        native_module_url.into(),
    );
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

fn settle_audio_worklet_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module_state: v8::Local<'s, v8::Object>,
    result: Result<(), v8::Local<'s, v8::Value>>,
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
    set_audio_worklet_module_bool_slot(
        scope,
        module_state,
        AUDIO_WORKLET_MODULE_LOADED_SLOT,
        result.is_ok(),
    );
    if let Err(error) = result {
        set_private_value(scope, module_state, AUDIO_WORKLET_MODULE_ERROR_SLOT, error);
    }
    let Some(resolvers) =
        web_audio_array_slot(scope, module_state, AUDIO_WORKLET_MODULE_RESOLVERS_SLOT)
    else {
        return;
    };
    // Drop the state's references before settling, and never invoke author
    // Promise.then / constructor properties to join a module response.
    set_private_value(
        scope,
        module_state,
        AUDIO_WORKLET_MODULE_RESOLVERS_SLOT,
        v8::undefined(scope).into(),
    );
    for index in 0..resolvers.length() {
        let Some(value) = resolvers.get_index(scope, index) else {
            continue;
        };
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        // Only native PromiseResolvers are written to this private array.
        let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) };
        match result {
            Ok(()) => {
                let _ = resolver.resolve(scope, v8::undefined(scope).into());
            }
            Err(error) => {
                let _ = resolver.reject(scope, error);
            }
        }
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
    let Some(has_input) =
        graph::prepare_offline_render(scope, context, length as f64 / sample_rate)
    else {
        return;
    };
    let channels = web_audio_number_slot(scope, context, OFFLINE_AUDIO_CHANNEL_COUNT_SLOT)
        .unwrap_or(1.0) as u32;
    let rendered_buffer = {
        let realm = context
            .get_creation_context(scope)
            .expect("Audio context should have a creation realm");
        let scope = &mut v8::ContextScope::new(scope, realm);
        build_audio_buffer(scope, length, sample_rate, channels, has_input)
    };
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

    let Some(realm) = context.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Some(event) = events::audio::new_offline_audio_completion_event(scope, rendered_buffer)
    else {
        return;
    };
    let _ = dispatch_simple_event_target_event(scope, context, "complete", event);
}

pub(crate) fn install_audio_runtime(scope: &mut v8::PinScope<'_, '_>) {
    moli_webapi::web_audio::install_runtime(
        scope,
        moli_webapi::web_audio::AudioRuntime {
            initialize_media_source: media_element_source::initialize_native_source,
        },
    );
}
