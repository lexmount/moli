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

mod analyser;
mod audio_buffer;
mod audio_param;
mod backend;
mod biquad;
mod compressor;
mod graph;
mod oscillator;
mod oscillator_dsp;
mod wavetable;

pub(in crate::context_bootstrap) use audio_buffer::build_constructor_template as build_audio_buffer_constructor_template;
use backend::{Node, NodeKind, State};

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
const OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT: &str = "__moliOfflineAudioCompleteContext";
const OFFLINE_AUDIO_COMPLETE_RESOLVER_SLOT: &str = "__moliOfflineAudioCompleteResolver";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioContext)]
struct AudioContextObjectDeclaration<'scope> {
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
#[webapi(plain)]
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
    #[webapi(data_property)]
    length: f64,
    #[webapi(data_property = "sampleRate")]
    sample_rate: f64,
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
    #[webapi(data_property, readonly)]
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
    #[webapi(data_property, readonly)]
    threshold: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    knee: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    ratio: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    attack: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    release: v8::Local<'scope, v8::Object>,
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
#[webapi(plain)]
struct OfflineAudioCompletePayloadDeclaration<'scope> {
    #[webapi(slot = OFFLINE_AUDIO_COMPLETE_CONTEXT_SLOT)]
    context: v8::Local<'scope, v8::Object>,
    #[webapi(slot = OFFLINE_AUDIO_COMPLETE_RESOLVER_SLOT)]
    resolver: v8::Local<'scope, v8::Object>,
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
#[webapi(interface = web_api_interfaces::AudioDestinationNode)]
struct AudioDestinationNodeObjectDeclaration {}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OfflineAudioContext")]
struct OfflineAudioContextConstructorArgs {
    #[webidl(required)]
    channel_count: u32,
    #[webidl(required)]
    length: u32,
    #[webidl(required)]
    sample_rate: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioParam.setValueAtTime")]
struct AudioParamSetValueAtTimeArgs {
    #[webidl(required)]
    value: f64,
    #[webidl(required)]
    start_time: f64,
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
    #[webapi(accessor_property, getter = audio_context_current_time)]
    current_time: (),

    #[webapi(method = "createBuffer", length = 3, callback = audio_buffer::create_buffer)]
    create_buffer: (),

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
        "OscillatorNode" => oscillator::install(scope, template),
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
        "AnalyserNode" => analyser::install(scope, template),

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
    backend::initialize(scope, context, State::Context(backend::Context::realtime()));
    let destination = audio_destination_node(scope, context);
    let modules = new_web_audio_map_object(scope);
    let module_list = v8::Array::new(scope, 0);
    let processors = new_web_audio_map_object(scope);
    let audio_worklet = AudioWorkletObjectDeclaration::new(context)
        .bind(scope)
        .expect("AudioWorklet declaration should bind");
    set_symbol_to_string_tag(scope, audio_worklet, "AudioWorklet");

    AudioContextObjectDeclaration::new(
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
    web_api_interfaces::AudioContext::is_instance(scope, object)
}

fn audio_context_current_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(state) = backend::get(scope, args.this())
        && let State::Context(context) = &*state.borrow()
    {
        rv.set(v8::Number::new(scope, context.current_time()).into());
        return;
    }
    throw_type_error(scope, "Illegal invocation: expected a BaseAudioContext.");
}

fn audio_context_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let context = args.this();
    if !is_audio_context_object(scope, context) {
        throw_type_error(scope, "Illegal invocation: expected an AudioContext.");
        return;
    }
    if let Some(state) = backend::get(scope, context)
        && let State::Context(backend::Context::Realtime(native)) = &*state.borrow()
    {
        native.close_sync();
    }
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
    backend::initialize(scope, node, State::ModuleWorklet);
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
    if let Some(state) = backend::get(scope, args.this())
        && let State::Node(Node::Compressor { reduction, .. }) = &*state.borrow()
    {
        let value = f32::from_bits(reduction.load(std::sync::atomic::Ordering::Relaxed));
        rv.set(v8::Number::new(scope, f64::from(value)).into());
        return;
    }
    throw_type_error(
        scope,
        "Illegal invocation: expected a DynamicsCompressorNode.",
    );
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

    let channel_count = parsed.channel_count;
    let length = parsed.length;
    let sample_rate = f64::from(parsed.sample_rate as f32);
    if !sample_rate.is_finite() {
        throw_type_error(
            scope,
            "OfflineAudioContext sampleRate must be a finite float.",
        );
        return;
    }
    if !(1..=32).contains(&channel_count)
        || !(1..=i32::MAX as u32).contains(&length)
        || !(3000.0..=768000.0).contains(&sample_rate)
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Failed to construct 'OfflineAudioContext': invalid channel count, length, or sample rate.",
        );
        return;
    }

    let context = args.this();
    backend::initialize(
        scope,
        context,
        State::Context(backend::Context::offline(
            channel_count as usize,
            length as usize,
            sample_rate as f32,
        )),
    );
    let destination = audio_destination_node(scope, context);
    OfflineAudioContextObjectDeclaration::new(
        f64::from(length),
        sample_rate,
        OFFLINE_AUDIO_LISTENERS_SLOT,
        "suspended",
        destination,
    )
    .initialize(scope, context)
    .expect("OfflineAudioContext declaration should initialize object");
    rv.set(context.into());
}

fn require_base_audio_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if web_api_interfaces::BaseAudioContext::is_instance(scope, object) {
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
    let native = backend::create_node(scope, args.this(), NodeKind::Oscillator);
    let frequency = audio_param::wrap(scope, native.param("frequency"));
    let detune = audio_param::wrap(scope, native.param("detune"));
    let node = OscillatorNodeObjectDeclaration::new(frequency, detune)
        .bind(scope)
        .expect("OscillatorNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    graph::initialize_source(scope, node);
    backend::initialize(scope, node, State::Node(native));
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
    let native = backend::create_node(scope, args.this(), NodeKind::Compressor);
    let threshold = audio_param::wrap(scope, native.param("threshold"));
    let knee = audio_param::wrap(scope, native.param("knee"));
    let ratio = audio_param::wrap(scope, native.param("ratio"));
    let attack = audio_param::wrap(scope, native.param("attack"));
    let release = audio_param::wrap(scope, native.param("release"));
    let node =
        DynamicsCompressorNodeObjectDeclaration::new(threshold, knee, ratio, attack, release)
            .bind(scope)
            .expect("DynamicsCompressorNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    backend::initialize(scope, node, State::Node(native));
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
    let node = analyser::create(scope, args.this());
    rv.set(node.into());
}

fn offline_audio_context_start_rendering_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let context = args.this();
    let Some(state) = backend::get(scope, context) else {
        throw_type_error(
            scope,
            "Illegal invocation: expected an OfflineAudioContext.",
        );
        return;
    };
    {
        let mut state = state.borrow_mut();
        let State::Context(backend::Context::Offline { rendered, .. }) = &mut *state else {
            throw_type_error(
                scope,
                "Illegal invocation: expected an OfflineAudioContext.",
            );
            return;
        };
        if *rendered {
            let error = new_dom_exception_value(
                scope,
                "Offline rendering has already started.",
                "InvalidStateError",
            );
            set_rejected_promise_return(scope, &mut rv, error);
            return;
        }
        *rendered = true;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let payload = OfflineAudioCompletePayloadDeclaration::new(context, resolver.into())
        .bind(scope)
        .expect("OfflineAudio payload should bind");
    let Some(callback) = v8::Function::builder(offline_audio_context_complete_microtask_callback)
        .data(payload.into())
        .build(scope)
    else {
        return;
    };
    // Defer work and observable results until after the initiating JS call.
    // Both PCM and analyser history are produced by the same native graph.
    scope.enqueue_microtask(callback);
    define_non_enumerable_string_property(scope, context, "state", "running");
    rv.set(resolver.get_promise(scope).into());
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
    let Some(resolver) =
        web_audio_object_slot(scope, payload, OFFLINE_AUDIO_COMPLETE_RESOLVER_SLOT)
    else {
        return;
    };
    // Only this callback's native payload can provide this private slot.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let state = backend::get(scope, context).expect("rendering context must retain its backend");
    let rendered = {
        let mut state = state.borrow_mut();
        let State::Context(backend::Context::Offline { context, .. }) = &mut *state else {
            unreachable!()
        };
        context.start_rendering_sync()
    };
    let Some(buffer) = audio_buffer::new_buffer(
        scope,
        rendered.number_of_channels() as u32,
        rendered.length() as u32,
        f64::from(rendered.sample_rate()),
    ) else {
        return;
    };
    for channel in 0..rendered.number_of_channels() {
        if audio_buffer::write_channel(
            scope,
            buffer,
            channel as u32,
            rendered.get_channel_data(channel),
        )
        .is_none()
        {
            return;
        }
    }
    define_non_enumerable_string_property(scope, context, "state", "closed");
    let event = OfflineAudioCompletionEventDeclaration::new("complete", buffer)
        .bind(scope)
        .expect("OfflineAudio completion event should bind");
    let _ = dispatch_simple_event_target_event(
        scope,
        context,
        OFFLINE_AUDIO_LISTENERS_SLOT,
        "complete",
        event,
    );
    let _ = resolver.resolve(scope, buffer.into());
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

fn oscillator_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    graph::start_source(scope, &args);
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
    let native = backend::create_node(scope, context, NodeKind::Destination);
    backend::initialize(scope, node, State::Node(native));
    node
}
