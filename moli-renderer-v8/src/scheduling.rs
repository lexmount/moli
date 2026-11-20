//! Scheduling state and task execution shared by Window and Worker.

mod bindings;
mod task_signal;

pub(crate) use bindings::*;
pub(crate) use moli_webapi::events::task_priority_change_event::TaskPriority;
pub(crate) use task_signal::{task_controller_constructor, task_signal_template};

use std::{cell::Cell, rc::Rc};

use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    abort_signal_route::{AbortAlgorithm, ResolvedAbortSignal},
    native_bridge::{PageScriptHost, WindowExecutionContextIdentity},
    resource_initiator::CallbackInitiator,
    script_continuation::with_scheduling_state,
    webidl,
    webidl_callback_execution::{PreparedWebIdlCallbackExecution, WebIdlCallbackExecutionOutcome},
    window_webidl_callback::WindowWebIdlCallbackFunction,
};

#[derive(Clone)]
enum PrioritySource {
    Fixed(TaskPriority),
    Signal(Rc<Cell<TaskPriority>>),
}

impl PrioritySource {
    fn priority(&self) -> TaskPriority {
        match self {
            Self::Fixed(priority) => *priority,
            Self::Signal(priority) => priority.get(),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct SchedulerId(u64);

#[derive(Clone)]
pub(crate) struct SchedulingState {
    scheduler: SchedulerId,
    priority: PrioritySource,
}

enum TaskBody {
    Window(Box<WindowWebIdlCallbackFunction>),
    Worker {
        callback: WebIdlCallbackFunction,
        initiator: CallbackInitiator,
    },
    Continuation,
}

pub(crate) struct ScheduledTask {
    state: SchedulingState,
    context: v8::Global<v8::Context>,
    body: TaskBody,
    resolver: v8::Global<v8::PromiseResolver>,
    abort: Option<AbortRegistration>,
}

struct AbortRegistration {
    signal: v8::Global<v8::Object>,
    algorithm: AbortAlgorithm,
    data: v8::Global<v8::Array>,
}

const ABORT_RESOLVER_INDEX: u32 = 0;
const ABORT_QUEUE_ID_INDEX: u32 = 1;
const ABORT_RUNNING_INDEX: u32 = 2;

impl ScheduledTask {
    pub(crate) fn rank(&self) -> (TaskPriority, bool) {
        (
            self.state.priority.priority(),
            matches!(self.body, TaskBody::Continuation),
        )
    }

    pub(crate) fn context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.context)
    }

    pub(crate) fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        match &self.body {
            TaskBody::Window(callback) => callback.relevant_identity(),
            _ => None,
        }
    }

    pub(crate) fn dispatch(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host: Option<*mut PageScriptHost>,
    ) {
        let abort = self.abort.as_ref().map(|registration| {
            let signal = v8::Local::new(scope, &registration.signal);
            let algorithm = registration
                .algorithm
                .prepare(scope)
                .expect("a queued task owns its abort algorithm");
            let data = v8::Local::new(scope, &registration.data);
            let running = v8::Boolean::new(scope, true);
            let _ = data.set_index(scope, ABORT_RUNNING_INDEX, running.into());
            (signal, algorithm, data)
        });
        let resolver = v8::Local::new(scope, &self.resolver);
        with_scheduling_state(
            scope,
            self.state.clone(),
            abort.map(|(signal, ..)| signal),
            |scope| {
                let callback = match &self.body {
                    TaskBody::Window(callback) => {
                        host.and_then(|host| callback.prepare_execution(scope, host, None))
                    }
                    TaskBody::Worker {
                        callback,
                        initiator,
                    } => {
                        let context = self.context(scope);
                        let callback = callback.prepare(scope);
                        PreparedWebIdlCallbackExecution::worker(scope, callback, context)
                            .map(|callback| callback.with_resource_initiator(initiator.clone()))
                    }
                    TaskBody::Continuation => {
                        let value = v8::undefined(scope).into();
                        let _ = resolver.resolve(scope, value);
                        return;
                    }
                };
                let Some(callback) = callback else {
                    return;
                };
                let receiver = v8::undefined(scope).into();
                let outcome = callback.invoke_with(
                    scope,
                    receiver,
                    &[],
                    |scope, function, receiver, arguments| {
                        v8::tc_scope!(let scope, scope);
                        if let Some(value) = crate::util::call_script_visible_function(
                            scope,
                            function,
                            receiver,
                            arguments,
                            "Scheduler callback",
                        ) {
                            Ok(value)
                        } else {
                            Err(scope.exception())
                        }
                    },
                );
                match outcome {
                    WebIdlCallbackExecutionOutcome::Returned(value) => {
                        let _ = resolver.resolve(scope, value);
                    }
                    WebIdlCallbackExecutionOutcome::Failed(Some(exception)) => {
                        let _ = resolver.reject(scope, exception);
                    }
                    WebIdlCallbackExecutionOutcome::Failed(None)
                    | WebIdlCallbackExecutionOutcome::Unavailable => {}
                }
            },
        );
        if let Some((signal, algorithm, data)) = abort {
            let running = v8::Boolean::new(scope, false);
            let _ = data.set_index(scope, ABORT_RUNNING_INDEX, running.into());
            if let Some(resolved) = ResolvedAbortSignal::resolve(scope, signal) {
                resolved.unregister_algorithm(scope, algorithm);
            }
        }
    }
}
