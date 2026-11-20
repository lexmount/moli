use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use moli_time::{TimerId, TimerScheduler};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::*;
use crate::{
    abort_signal_route::register_weak_algorithm,
    common_runtime::{CommonRuntimeBinding, RuntimeExecutionContext},
    native_bridge::{WindowExecutionContextBinding, WindowOperationReceiver},
    script_continuation::scheduling_state,
    util::{
        context_host_ptr_from_global_bridge, get_private_value, global_constructor_prototype,
        set_private_value, v8str,
    },
    web_api_interfaces,
};

const SCHEDULER_SLOT: &str = "__moliScheduler";
const SCHEDULER_ID_SLOT: &str = "__moliSchedulerId";
const SCHEDULER_WINDOW_SLOT: &str = "__moliSchedulerWindow";

static NEXT_SCHEDULER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::Scheduler)]
struct SchedulerObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Scheduler, enumerable)]
struct SchedulerPrototype {
    #[webapi(method, length = 1, callback = post_task, returns_promise)]
    post_task: (),
    #[webapi(method = "yield", length = 0, callback = yield_task, returns_promise)]
    yield_task: (),
}

#[derive(Default, WebApiObject)]
#[webapi(fragment = web_api_interfaces::WorkerGlobalScope)]
pub(crate) struct WorkerSchedulerProperty {
    #[webapi(accessor_property, enumerable, getter = worker_scheduler_getter, setter = worker_scheduler_setter)]
    scheduler: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SchedulerPostTaskOptions")]
struct PostTaskOptions<'s> {
    #[webidl(default = 0, converter = "enforce_range_unsigned_long_long")]
    delay: u64,
    #[webidl(converter = "enum")]
    priority: Option<TaskPriority>,
    #[webidl(interface = web_api_interfaces::AbortSignal)]
    signal: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Scheduler.postTask")]
struct PostTaskArgs<'s> {
    #[webidl(required, converter = "callback_function")]
    callback: WebIdlCallbackFunction,
    #[webidl(index = 1, dictionary)]
    options: PostTaskOptions<'s>,
}

#[derive(Clone, Default)]
pub(crate) struct WorkerScheduledTasks(Rc<RefCell<TimerScheduler<ScheduledTask>>>);

impl WorkerScheduledTasks {
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.0.borrow().next_deadline()
    }

    pub(crate) fn has_ready(&self) -> bool {
        self.next_deadline()
            .is_some_and(|deadline| deadline <= Instant::now())
    }

    pub(crate) fn take_ready(&self) -> Option<ScheduledTask> {
        let mut queue = self.0.borrow_mut();
        let timer = queue.take_next_ready_ranked(Instant::now(), |task| Some(task.rank()))?;
        queue.finish_running(timer.id);
        Some(timer.payload)
    }
}

enum TaskOwner {
    Window {
        host: *mut PageScriptHost,
        binding: WindowExecutionContextBinding,
    },
    Worker(WorkerScheduledTasks),
}

impl TaskOwner {
    fn resolve<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        scheduler: v8::Local<'s, v8::Object>,
        context: v8::Local<'s, v8::Context>,
    ) -> Option<Self> {
        let (runtime, owner) = CommonRuntimeBinding::from_context(context)?;
        match owner {
            RuntimeExecutionContext::Window(host) => {
                let identity = get_private_value(scope, scheduler, SCHEDULER_WINDOW_SLOT)?
                    .to_rust_string_lossy(scope);
                let identity =
                    WindowExecutionContextIdentity::deserialize_from_internal_slot(&identity)?;
                let binding = WindowExecutionContextBinding::new(
                    identity.owner(),
                    identity.dispatch_scope(),
                    identity.realm_token(),
                    v8::Global::new(scope, context),
                );
                binding
                    .is_current(unsafe { &*host })
                    .then_some(Self::Window { host, binding })
            }
            RuntimeExecutionContext::Worker => {
                let scope = &mut v8::ContextScope::new(scope, context);
                runtime.callback_execution_context(scope)?;
                scope
                    .get_slot::<WorkerScheduledTasks>()
                    .cloned()
                    .map(Self::Worker)
            }
        }
    }

    fn queue(self, task: ScheduledTask, delay: u64) -> TimerId {
        match self {
            Self::Window { host, binding } => {
                unsafe { &mut *host }.queue_prioritized_task(binding, task, delay)
            }
            Self::Worker(queue) => queue
                .0
                .borrow_mut()
                .schedule_after(task, delay, Instant::now()),
        }
    }
}

pub(crate) fn scheduler_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    SchedulerPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
}

fn scheduler_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    window: Option<WindowExecutionContextIdentity>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(cached) = get_private_value(scope, global, SCHEDULER_SLOT) {
        return v8::Local::<v8::Object>::try_from(cached).ok();
    }
    let prototype = global_constructor_prototype(scope, "Scheduler")?;
    let object = SchedulerObject { prototype }.bind(scope).ok()?;
    let id = NEXT_SCHEDULER_ID.fetch_add(1, Ordering::Relaxed);
    let id = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, object, SCHEDULER_ID_SLOT, id.into());
    if let Some(window) = window {
        let identity = v8::String::new(scope, &window.serialize_for_internal_slot())?;
        set_private_value(scope, object, SCHEDULER_WINDOW_SLOT, identity.into());
    }
    set_private_value(scope, global, SCHEDULER_SLOT, object.into());
    Some(object)
}

fn scheduler_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<SchedulerId> {
    let id = get_private_value(scope, object, SCHEDULER_ID_SLOT)?;
    let id = v8::Local::<v8::BigInt>::try_from(id).ok()?;
    Some(SchedulerId(id.u64_value().0))
}

fn window_scheduler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let host = unsafe { &*context_host_ptr_from_global_bridge(scope)? };
    let binding = WindowOperationReceiver::capture_after_access_check(scope, receiver, host)
        .resolve_live_binding(host)?;
    let identity = binding.resolve_identity(host)?;
    let context = binding.context(scope);
    let receiver = moli_webapi_declare::web_api_object_target(scope, receiver)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    scheduler_object(scope, receiver, Some(identity))
}

pub(crate) fn window_scheduler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(scheduler) = window_scheduler(scope, args.this()) {
        rv.set(scheduler.into());
    }
}

fn worker_scheduler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(scheduler) = scheduler_object(scope, args.this(), None) {
        rv.set(scheduler.into());
    }
}

fn worker_scheduler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let key = v8str(scope, "scheduler");
    let _ = args
        .this()
        .create_data_property(scope, key.into(), args.get(0));
}

pub(crate) fn with_idle_scheduling_state<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    run: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let Some(scheduler) =
        window_scheduler(scope, receiver).and_then(|object| scheduler_id(scope, object))
    else {
        return run(scope);
    };
    with_scheduling_state(
        scope,
        SchedulingState {
            scheduler,
            priority: PrioritySource::Fixed(TaskPriority::Background),
        },
        None,
        run,
    )
}

fn post_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let scheduler = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("the Scheduler receiver was checked");
    let Some(parsed) = webidl::parse_args::<PostTaskArgs>(scope, &args) else {
        return;
    };
    let priority = parsed
        .options
        .priority
        .map(PrioritySource::Fixed)
        .or_else(|| {
            let signal = parsed.options.signal?;
            super::task_signal::priority_cell(scope, signal).map(PrioritySource::Signal)
        })
        .unwrap_or(PrioritySource::Fixed(TaskPriority::UserVisible));
    if let Some(promise) = schedule(
        scope,
        scheduler,
        Some(parsed.callback),
        priority,
        parsed.options.signal,
        parsed.options.delay,
    ) {
        rv.set(promise.into());
    }
}

fn yield_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let scheduler = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("the Scheduler receiver was checked");
    let id = scheduler_id(scope, scheduler);
    let (priority, signal) = scheduling_state(scope)
        .filter(|(state, _)| Some(state.scheduler) == id)
        .map_or(
            (PrioritySource::Fixed(TaskPriority::UserVisible), None),
            |(state, signal)| (state.priority, signal),
        );
    if let Some(promise) = schedule(scope, scheduler, None, priority, signal, 0) {
        rv.set(promise.into());
    }
}

fn schedule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scheduler: v8::Local<'s, v8::Object>,
    callback: Option<WebIdlCallbackFunction>,
    priority: PrioritySource,
    signal: Option<v8::Local<'s, v8::Object>>,
    delay: u64,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let id = scheduler_id(scope, scheduler);
    let context = scheduler.get_creation_context(scope);
    let owner = context.and_then(|context| TaskOwner::resolve(scope, scheduler, context));
    let (Some(scheduler), Some(context), Some(owner)) = (id, context, owner) else {
        let reason = moli_webapi::dom_exception::new_dom_exception_value(
            scope,
            "The scheduler's global is no longer active.",
            "NotSupportedError",
        );
        let _ = resolver.reject(scope, reason);
        return Some(promise);
    };
    let body = match (callback, &owner) {
        (Some(callback), TaskOwner::Window { host, .. }) => TaskBody::Window(Box::new(
            WindowWebIdlCallbackFunction::new(scope, unsafe { &**host }, callback),
        )),
        (Some(callback), TaskOwner::Worker(_)) => TaskBody::Worker {
            callback,
            initiator: CallbackInitiator::capture(scope),
        },
        (None, _) => TaskBody::Continuation,
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let abort_signal = signal.and_then(|signal| ResolvedAbortSignal::resolve(scope, signal));
    if let Some(signal) = abort_signal.filter(|signal| signal.is_aborted(scope)) {
        let reason = signal
            .reason(scope)
            .unwrap_or_else(|| v8::undefined(scope).into());
        let _ = resolver.reject(scope, reason);
        return Some(promise);
    }
    let registration = if let Some(signal) = abort_signal {
        let empty = v8::undefined(scope).into();
        let running = v8::Boolean::new(scope, false).into();
        let data = v8::Array::new_with_elements(scope, &[resolver.into(), empty, running]);
        let algorithm = v8::Function::builder(abort_task)
            .data(data.into())
            .build(scope)?;
        Some((signal, algorithm, data))
    } else {
        None
    };
    let task = ScheduledTask {
        state: SchedulingState {
            scheduler,
            priority,
        },
        context: v8::Global::new(scope, context),
        body,
        resolver: v8::Global::new(scope, resolver),
        abort: registration.map(|(signal, algorithm, data)| AbortRegistration {
            signal: v8::Global::new(scope, signal.value()),
            algorithm: AbortAlgorithm::Strong(v8::Global::new(scope, algorithm)),
            data: v8::Global::new(scope, data),
        }),
    };
    let id = owner.queue(task, delay);
    if let Some((signal, algorithm, data)) = registration {
        let id = v8::Integer::new_from_unsigned(scope, id.get());
        let _ = data.set_index(scope, ABORT_QUEUE_ID_INDEX, id.into());
        register_weak_algorithm(scope, signal, algorithm);
    }
    Some(promise)
}

fn abort_task(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).expect("scheduled abort data");
    let Some(id) = data
        .get_index(scope, ABORT_QUEUE_ID_INDEX)
        .and_then(|value| value.uint32_value(scope))
        .and_then(TimerId::new)
    else {
        return;
    };
    let running = data
        .get_index(scope, ABORT_RUNNING_INDEX)
        .is_some_and(|value| value.is_true());
    let active = if running {
        true
    } else if let Some(host) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }.cancel_prioritized_task(id)
    } else {
        scope
            .get_slot::<WorkerScheduledTasks>()
            .is_some_and(|queue| queue.0.borrow_mut().remove_pending(id))
    };
    if active
        && let Some(resolver) = data
            .get_index(scope, ABORT_RESOLVER_INDEX)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        // The first element is the resolver installed by schedule().
        let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
        let _ = resolver.reject(scope, args.get(0));
    }
}
