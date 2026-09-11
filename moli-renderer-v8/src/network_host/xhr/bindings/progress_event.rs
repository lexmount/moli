use super::*;
use crate::util::get_private_value;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

const PROGRESS_EVENT_LENGTH_COMPUTABLE_SLOT: &str = "__lmProgressEventLengthComputable";
const PROGRESS_EVENT_LOADED_SLOT: &str = "__lmProgressEventLoaded";
const PROGRESS_EVENT_TOTAL_SLOT: &str = "__lmProgressEventTotal";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ProgressEvent)]
struct ProgressEventStateDeclaration {
    #[webapi(slot = PROGRESS_EVENT_LENGTH_COMPUTABLE_SLOT)]
    length_computable: bool,
    #[webapi(slot = PROGRESS_EVENT_LOADED_SLOT)]
    loaded: f64,
    #[webapi(slot = PROGRESS_EVENT_TOTAL_SLOT)]
    total: f64,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ProgressEventInit")]
struct ProgressEventInitArgs {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    length_computable: bool,
    #[webidl(default = 0.0)]
    loaded: f64,
    #[webidl(default = 0.0)]
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
    #[webidl(default = ProgressEventInitArgs::default(), with = progress_event_init_arg)]
    init: ProgressEventInitArgs,
}

fn progress_event_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ProgressEventInitArgs, webidl::WebIdlError> {
    let context = webidl::Context::argument("ProgressEvent", (index + 1) as usize);
    if args.length() <= index {
        return Ok(ProgressEventInitArgs::default());
    }
    let value = args.get(index);
    webidl::parse_dictionary::<ProgressEventInitArgs>(scope, value, context)
        .map(|value| value.unwrap_or_default())
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
    let obj = args.this();
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
    ProgressEventStateDeclaration::new(
        parsed.init.length_computable,
        parsed.init.loaded,
        parsed.init.total,
    )
    .initialize(scope, obj)
    .expect("ProgressEvent state declaration should initialize");
    rv.set(obj.into());
}

fn progress_event_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        get_private_value(scope, receiver, slot).unwrap_or_else(|| v8::undefined(scope).into());
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
