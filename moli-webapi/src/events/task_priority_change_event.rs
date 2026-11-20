use moli_v8_util::{throw_type_error, v8str};
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;
use webidl::WebIdlEnumSnapshot;

use super::{
    EventInit, define_event_property, event_private_value, initialize_event_object_with_type,
    initialize_event_wrapper, new_event_state, set_event_private_value,
};
use crate::interfaces as web_api_interfaces;

const PREVIOUS_PRIORITY_SLOT: &str = "__moliTaskPriorityChangeEventPreviousPriority";

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, webidl::WebIdlEnum)]
#[webidl(name = "TaskPriority")]
pub enum TaskPriority {
    Background,
    #[default]
    #[webidl(token = "user-visible")]
    UserVisible,
    #[webidl(token = "user-blocking")]
    UserBlocking,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TaskPriorityChangeEvent, enumerable)]
struct TaskPriorityChangeEventPrototype {
    #[webapi(accessor_property, getter = previous_priority)]
    previous_priority: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TaskPriorityChangeEventInit")]
struct TaskPriorityChangeEventInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, converter = "enum")]
    previous_priority: TaskPriority,
}

pub fn constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
        web_api_interfaces::TaskPriorityChangeEvent,
        constructor
    ))
    .length(2)
    .build(scope)
}

pub fn install_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    TaskPriorityChangeEventPrototype::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn previous_priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = event_private_value(scope, args.this(), PREVIOUS_PRIORITY_SLOT) {
        rv.set(value);
    }
}

fn event_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: v8::Local<'s, v8::String>,
    init: EventInit,
    priority: TaskPriority,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = new_event_state(scope);
    initialize_event_object_with_type(scope, state, event_type, init.bubbles, init.cancelable);
    define_event_property(
        scope,
        state,
        "composed",
        v8::Boolean::new(scope, init.composed).into(),
    );
    set_event_private_value(
        scope,
        state,
        PREVIOUS_PRIORITY_SLOT,
        v8str(scope, priority.snapshot_token()).into(),
    );
    web_api_interfaces::initialize(scope, state, "TaskPriorityChangeEvent").ok()?;
    Some(state)
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "TaskPriorityChangeEvent requires the new operator.");
        return;
    }
    let Some(event_type) =
        super::init::event_type_argument(scope, &args, "TaskPriorityChangeEvent")
    else {
        return;
    };
    let Some(init) = super::init::parse_event_dictionary::<TaskPriorityChangeEventInit>(
        scope,
        &args,
        "TaskPriorityChangeEvent",
    ) else {
        return;
    };
    let Some(state) = event_state(scope, event_type, init.base, init.previous_priority) else {
        return;
    };
    if initialize_event_wrapper(scope, args.this(), state).is_some() {
        rv.set(args.this().into());
    }
}

pub fn new_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: TaskPriority,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = event_state(
        scope,
        v8str(scope, "prioritychange"),
        EventInit::default(),
        previous,
    )?;
    super::new_event_wrapper(scope, state)
}
