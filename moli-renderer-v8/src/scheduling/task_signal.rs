use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use moli_webapi::{
    dom_exception::throw_dom_exception_value,
    events::{self, task_priority_change_event},
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl::WebIdlEnumSnapshot;

use super::TaskPriority;
use crate::{
    context_bootstrap::{
        abort_signal_events::{onprioritychange_getter, onprioritychange_setter},
        dispatch_simple_event_target_event,
    },
    native_bridge::abort,
    util::{
        context_host_ptr_from_global_bridge, get_private_value, global_constructor_prototype,
        set_private_value, throw_type_error, v8str,
    },
    web_api_interfaces, webidl,
    worker::abort as worker_abort,
};

const PRIORITY_STATE_SLOT: &str = "__moliTaskSignalPriorityState";

enum PriorityOrigin {
    Controller,
    Fixed,
    Dependent(v8::Weak<v8::Object>),
}

struct PriorityState {
    value: Rc<Cell<TaskPriority>>,
    changing: Cell<bool>,
    origin: PriorityOrigin,
    dependents: RefCell<Vec<v8::Weak<v8::Object>>>,
}

type SignalPriorities = Rc<RefCell<HashMap<usize, Weak<PriorityState>>>>;

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::TaskSignal)]
struct TaskSignalObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TaskControllerInit")]
struct ControllerInit {
    #[webidl(converter = "enum", default = TaskPriority::UserVisible)]
    priority: TaskPriority,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TaskController")]
struct ControllerArgs {
    #[webidl(dictionary)]
    init: ControllerInit,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TaskController.setPriority")]
struct SetPriorityArgs {
    #[webidl(required, converter = "enum")]
    priority: TaskPriority,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TaskSignalAnyInit")]
struct TaskSignalAnyInit<'s> {
    #[webidl(converter = "raw")]
    priority: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TaskSignal.any")]
struct AnyArgs<'s> {
    #[webidl(required, sequence, interface = web_api_interfaces::AbortSignal)]
    signals: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(index = 1, dictionary)]
    init: TaskSignalAnyInit<'s>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TaskController, enumerable)]
struct TaskControllerPrototype {
    #[webapi(method, length = 1, callback = set_priority)]
    set_priority: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TaskSignal, enumerable)]
struct TaskSignalBindings {
    #[webapi(static_method, length = 1, callback = any)]
    any: (),
    #[webapi(accessor_property, getter = priority)]
    priority: (),
    #[webapi(accessor_property, getter = onprioritychange_getter, setter = onprioritychange_setter)]
    onprioritychange: (),
}

pub(crate) fn task_signal_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "TaskController" => {
            TaskControllerPrototype::initialize_prototype_template(scope, prototype)
        }
        "TaskSignal" => {
            TaskSignalBindings::initialize_template(scope, template);
            TaskSignalBindings::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

fn init_priority(
    scope: &mut v8::PinScope<'_, '_>,
    signal: v8::Local<'_, v8::Object>,
    priority: TaskPriority,
    origin: PriorityOrigin,
) {
    let registry = if let Some(registry) = scope.get_slot::<SignalPriorities>() {
        Rc::clone(registry)
    } else {
        let registry = Rc::new(RefCell::new(HashMap::new()));
        scope.set_slot(Rc::clone(&registry));
        registry
    };
    let state = Rc::new(PriorityState {
        value: Rc::new(Cell::new(priority)),
        changing: Cell::new(false),
        origin,
        dependents: RefCell::new(Vec::new()),
    });
    let key = Rc::as_ptr(&state) as usize;
    registry.borrow_mut().insert(key, Rc::downgrade(&state));
    let external = v8::External::new(scope, key as *mut std::ffi::c_void);
    set_private_value(scope, signal, PRIORITY_STATE_SLOT, external.into());
    crate::v8_finalizer::track_context_owned_external_finalizer(scope, external, move || {
        registry.borrow_mut().remove(&key);
        drop(state);
    });
}

fn priority_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> Option<Rc<PriorityState>> {
    let signal = moli_webapi_declare::web_api_object_target(scope, signal)?;
    let key = get_private_value(scope, signal, PRIORITY_STATE_SLOT)?;
    let key = v8::Local::<v8::External>::try_from(key).ok()?.value() as usize;
    scope
        .get_slot::<SignalPriorities>()?
        .borrow()
        .get(&key)
        .and_then(Weak::upgrade)
}

pub(super) fn priority_cell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) -> Option<Rc<Cell<TaskPriority>>> {
    priority_state(scope, signal).map(|state| Rc::clone(&state.value))
}

fn new_signal_object<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "TaskSignal")?;
    TaskSignalObject { prototype }.bind(scope).ok()
}

pub(crate) fn task_controller_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "TaskController requires the new operator.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ControllerArgs>(scope, &args) else {
        return;
    };
    let Some(signal) = new_signal_object(scope) else {
        return;
    };
    let controller = args.this();
    let initialized = if context_host_ptr_from_global_bridge(scope).is_some() {
        abort::initialize_controller_signal(scope, controller, signal)
    } else {
        worker_abort::initialize_worker_controller_signal(scope, controller, signal)
    };
    if initialized.is_some() {
        init_priority(
            scope,
            signal,
            parsed.init.priority,
            PriorityOrigin::Controller,
        );
        rv.set(controller.into());
    }
}

fn set_priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let controller = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("the TaskController receiver was checked");
    let Some(parsed) = webidl::parse_args::<SetPriorityArgs>(scope, &args) else {
        return;
    };
    let Some(signal) = abort::controller_signal(scope, controller)
        .or_else(|| worker_abort::worker_controller_signal(scope, controller))
    else {
        return;
    };
    let Some(state) = priority_state(scope, signal) else {
        return;
    };
    if state.changing.get() {
        throw_dom_exception_value(
            scope,
            "A priority change is already being dispatched.",
            "NotAllowedError",
        );
        return;
    }
    if state.value.get() == parsed.priority {
        return;
    }
    state.changing.set(true);
    dispatch_priority_change(scope, signal, &state, parsed.priority);
    let mut signals = Vec::new();
    state.dependents.borrow_mut().retain(|dependent| {
        if let Some(signal) = dependent.to_local(scope) {
            signals.push(signal);
            true
        } else {
            false
        }
    });
    for signal in signals {
        if let Some(state) = priority_state(scope, signal) {
            dispatch_priority_change(scope, signal, &state, parsed.priority);
        }
    }
    state.changing.set(false);
}

fn dispatch_priority_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
    state: &PriorityState,
    priority: TaskPriority,
) {
    let previous = state.value.replace(priority);
    if previous == priority {
        return;
    }
    let Some(context) = signal.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(event) = task_priority_change_event::new_event(scope, previous) {
        events::state::mark_event_trusted(scope, event);
        dispatch_simple_event_target_event(scope, signal, "prioritychange", event);
    }
}

fn priority<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(state) = priority_state(scope, args.this()) {
        rv.set(v8str(scope, state.value.get().snapshot_token()).into());
    }
}

fn any<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AnyArgs>(scope, &args) else {
        return;
    };
    let (priority, source) = match parsed.init.priority {
        None => (TaskPriority::UserVisible, None),
        Some(value) => {
            let signal = v8::Local::<v8::Object>::try_from(value).ok();
            if let Some((signal, state)) =
                signal.and_then(|signal| Some((signal, priority_state(scope, signal)?)))
            {
                let source = match &state.origin {
                    PriorityOrigin::Controller => Some(signal),
                    PriorityOrigin::Fixed => None,
                    PriorityOrigin::Dependent(source) => source.to_local(scope),
                };
                (state.value.get(), source)
            } else {
                let priority = match webidl::convert::<webidl::EnumValue<TaskPriority>>(
                    scope,
                    value,
                    webidl::Context::member("TaskSignalAnyInit", "priority"),
                ) {
                    Ok(webidl::EnumValue(priority)) => priority,
                    Err(error) => {
                        webidl::throw_error(scope, &error);
                        return;
                    }
                };
                (priority, None)
            }
        }
    };
    let Some(signal) = new_signal_object(scope) else {
        return;
    };
    let initialized = if context_host_ptr_from_global_bridge(scope).is_some() {
        abort::initialize_dependent_abort_signal(scope, signal, &parsed.signals)
    } else {
        worker_abort::initialize_worker_dependent_abort_signal(scope, signal, &parsed.signals)
    };
    if initialized.is_none() {
        return;
    }
    let origin = match source.and_then(|source| Some((source, priority_state(scope, source)?))) {
        Some((source, root)) => {
            let mut dependents = root.dependents.borrow_mut();
            dependents.retain(|dependent| !dependent.is_empty());
            dependents.push(v8::Weak::new(scope, signal));
            PriorityOrigin::Dependent(v8::Weak::new(scope, source))
        }
        None => PriorityOrigin::Fixed,
    };
    init_priority(scope, signal, priority, origin);
    rv.set(signal.into());
}
