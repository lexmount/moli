//! Callback ownership for timers inside one worker run.
//!
//! A worker timer queue, its active timers, V8 context, and isolate all retire
//! together when that worker run stops. The queue therefore is already the
//! exact run residence. The shared executor validates its Context binding
//! and the existing termination flag immediately before invoking author code.
//!
//! Page-supplied Web IDL callbacks remain distinct from browser-created
//! functions used by AbortSignal, XHR, and compiled string-timer algorithms.

use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    exception_reporting::{V8ExceptionReport, build_event_handler_exception_report},
    webidl_callback_execution::{PreparedWebIdlCallbackExecution, WebIdlCallbackExecutionOutcome},
};

pub(super) struct WorkerTimerCallback {
    storage: WorkerTimerCallbackStorage,
}

enum WorkerTimerCallbackStorage {
    BrowserFunction {
        callback: v8::Global<v8::Function>,
        context: v8::Global<v8::Context>,
    },
    WebIdl {
        callback: WebIdlCallbackFunction,
        target_context: v8::Global<v8::Context>,
        kind: WorkerWebIdlCallbackKind,
    },
}

#[derive(Clone, Copy)]
enum WorkerWebIdlCallbackKind {
    Timer,
    AnimationFrame { timestamp: f64 },
}

pub(super) enum WorkerTimerCallbackOutcome {
    Returned,
    Threw(Box<V8ExceptionReport>),
}

impl WorkerTimerCallback {
    pub(super) fn browser_function(
        scope: &mut v8::PinScope<'_, '_>,
        callback: v8::Local<'_, v8::Function>,
    ) -> Self {
        Self {
            storage: WorkerTimerCallbackStorage::BrowserFunction {
                callback: v8::Global::new(scope, callback),
                context: v8::Global::new(scope, scope.get_current_context()),
            },
        }
    }

    pub(super) fn webidl_timer(
        scope: &mut v8::PinScope<'_, '_>,
        callback: WebIdlCallbackFunction,
    ) -> Self {
        Self {
            storage: WorkerTimerCallbackStorage::WebIdl {
                callback,
                target_context: v8::Global::new(scope, scope.get_current_context()),
                kind: WorkerWebIdlCallbackKind::Timer,
            },
        }
    }

    pub(super) fn webidl_animation_frame(
        scope: &mut v8::PinScope<'_, '_>,
        callback: WebIdlCallbackFunction,
        timestamp: f64,
    ) -> Self {
        Self {
            storage: WorkerTimerCallbackStorage::WebIdl {
                callback,
                target_context: v8::Global::new(scope, scope.get_current_context()),
                kind: WorkerWebIdlCallbackKind::AnimationFrame { timestamp },
            },
        }
    }

    pub(super) fn target_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::Context> {
        match &self.storage {
            WorkerTimerCallbackStorage::BrowserFunction { context, .. } => {
                v8::Local::new(scope, context)
            }
            WorkerTimerCallbackStorage::WebIdl { target_context, .. } => {
                v8::Local::new(scope, target_context)
            }
        }
    }

    pub(super) fn invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        extra_args: &[v8::Global<v8::Value>],
    ) -> WorkerTimerCallbackOutcome {
        match &self.storage {
            WorkerTimerCallbackStorage::BrowserFunction { callback, .. } => {
                let callback = v8::Local::new(scope, callback);
                let global = scope.get_current_context().global(scope);
                let arguments: Vec<_> = extra_args
                    .iter()
                    .map(|argument| v8::Local::new(scope, argument))
                    .collect();
                let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();
                if callback.call(&scope, global.into(), &arguments).is_some() {
                    WorkerTimerCallbackOutcome::Returned
                } else {
                    let exception = scope.exception();
                    let message = scope.message();
                    let stack_trace = scope.stack_trace();
                    WorkerTimerCallbackOutcome::Threw(Box::new(
                        build_event_handler_exception_report(
                            &mut scope,
                            exception,
                            message,
                            stack_trace,
                        ),
                    ))
                }
            }
            WorkerTimerCallbackStorage::WebIdl {
                callback,
                target_context,
                kind,
            } => {
                let callback = callback.prepare(scope);
                let context = v8::Local::new(scope, target_context);
                let Some(prepared) =
                    PreparedWebIdlCallbackExecution::worker(scope, callback, context)
                else {
                    return WorkerTimerCallbackOutcome::Returned;
                };
                let mut arguments = Vec::new();
                let receiver = match kind {
                    WorkerWebIdlCallbackKind::Timer => {
                        arguments.extend(
                            extra_args
                                .iter()
                                .map(|argument| v8::Local::new(scope, argument)),
                        );
                        scope.get_current_context().global(scope).into()
                    }
                    WorkerWebIdlCallbackKind::AnimationFrame { timestamp } => {
                        arguments.push(v8::Number::new(scope, *timestamp).into());
                        v8::undefined(scope).into()
                    }
                };
                let callback_name = match kind {
                    WorkerWebIdlCallbackKind::Timer => "worker timer callback",
                    WorkerWebIdlCallbackKind::AnimationFrame { .. } => {
                        "worker requestAnimationFrame callback"
                    }
                };
                match prepared.invoke_with_report(
                    scope,
                    "callback",
                    "worker callback threw",
                    callback_name,
                    receiver,
                    &arguments,
                ) {
                    WebIdlCallbackExecutionOutcome::Returned(_)
                    | WebIdlCallbackExecutionOutcome::Unavailable => {
                        WorkerTimerCallbackOutcome::Returned
                    }
                    WebIdlCallbackExecutionOutcome::Failed(report) => {
                        WorkerTimerCallbackOutcome::Threw(report)
                    }
                }
            }
        }
    }
}
