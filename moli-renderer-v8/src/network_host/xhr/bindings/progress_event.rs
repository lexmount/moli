use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use webidl::WebIdlDictionarySerialize;

const PROGRESS_EVENT_LENGTH_COMPUTABLE_SLOT: &str = "__lmProgressEventLengthComputable";
const PROGRESS_EVENT_LOADED_SLOT: &str = "__lmProgressEventLoaded";
const PROGRESS_EVENT_TOTAL_SLOT: &str = "__lmProgressEventTotal";

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ProgressEventInit", snapshot)]
struct ProgressEventInitArgs {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    #[webidl(snapshot_slot = PROGRESS_EVENT_LENGTH_COMPUTABLE_SLOT)]
    length_computable: bool,
    #[webidl(default = 0.0)]
    #[webidl(snapshot_slot = PROGRESS_EVENT_LOADED_SLOT)]
    loaded: f64,
    #[webidl(default = 0.0)]
    #[webidl(snapshot_slot = PROGRESS_EVENT_TOTAL_SLOT)]
    total: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ProgressEvent")]
struct ProgressEventConstructorArgs {
    #[webidl(
        required,
        missing_message = "Failed to construct 'ProgressEvent': 1 argument required, but only 0 present."
    )]
    event_type: String,
    #[webidl(dictionary)]
    init: ProgressEventInitArgs,
}

pub(in crate::network_host::xhr) fn construct_native_progress_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    length_computable: bool,
    loaded: f64,
    total: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = crate::context_bootstrap::new_event_state(scope);
    crate::context_bootstrap::initialize_event_object(scope, state, event_type, false, false);
    ProgressEventInitArgs {
        length_computable,
        loaded,
        total,
        ..Default::default()
    }
    .initialize_snapshot(scope, state)?;
    web_api_interfaces::ProgressEvent::DESCRIPTOR
        .initialize(scope, state)
        .ok()?;
    let event = crate::context_bootstrap::new_event_wrapper(scope, state)?;
    crate::context_bootstrap::mark_event_trusted(scope, event);
    Some(event)
}

pub(crate) fn progress_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ProgressEvent': Please use the 'new' operator.",
        );
        return;
    }
    let wrapper = args.this();
    let obj = crate::context_bootstrap::new_event_state(scope);
    let Some(parsed) = webidl::parse_args::<ProgressEventConstructorArgs>(scope, &args) else {
        return;
    };
    crate::context_bootstrap::initialize_event_object(
        scope,
        obj,
        &parsed.event_type,
        parsed.init.bubbles,
        parsed.init.cancelable,
    );
    parsed
        .init
        .initialize_snapshot(scope, obj)
        .expect("ProgressEvent state");
    web_api_interfaces::initialize(scope, obj, "ProgressEvent").expect("ProgressEvent brand");
    if crate::context_bootstrap::initialize_event_wrapper(scope, wrapper, obj).is_some() {
        rv.set(wrapper.into());
    }
}

fn progress_event_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = crate::context_bootstrap::event_private_value(scope, receiver, slot)
        .unwrap_or_else(|| v8::undefined(scope).into());
    if value.is_undefined() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(value);
}

pub(crate) fn progress_event_length_computable_function_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    progress_event_slot_getter(
        scope,
        args.this(),
        PROGRESS_EVENT_LENGTH_COMPUTABLE_SLOT,
        rv,
    );
}

pub(crate) fn progress_event_loaded_function_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    progress_event_slot_getter(scope, args.this(), PROGRESS_EVENT_LOADED_SLOT, rv);
}

pub(crate) fn progress_event_total_function_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    progress_event_slot_getter(scope, args.this(), PROGRESS_EVENT_TOTAL_SLOT, rv);
}
