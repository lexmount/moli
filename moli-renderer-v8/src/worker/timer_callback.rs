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
        resource_initiator: Option<crate::script_continuation::CallbackResourceInitiator>,
    },
    WebIdl {
        callback: WebIdlCallbackFunction,
        target_context: v8::Global<v8::Context>,
        kind: WorkerWebIdlCallbackKind,
        resource_initiator: crate::script_continuation::CallbackResourceInitiator,
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
                resource_initiator: None,
            },
        }
    }

    pub(super) fn string_timer(
        scope: &mut v8::PinScope<'_, '_>,
        callback: v8::Local<'_, v8::Function>,
    ) -> Self {
        Self {
            storage: WorkerTimerCallbackStorage::BrowserFunction {
                callback: v8::Global::new(scope, callback),
                context: v8::Global::new(scope, scope.get_current_context()),
                resource_initiator: Some(
                    crate::script_continuation::CallbackResourceInitiator::capture(scope),
                ),
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
                resource_initiator: crate::script_continuation::CallbackResourceInitiator::capture(
                    scope,
                ),
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
                resource_initiator: crate::script_continuation::CallbackResourceInitiator::capture(
                    scope,
                ),
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
            WorkerTimerCallbackStorage::BrowserFunction {
                callback,
                resource_initiator,
                ..
            } => {
                let callback = v8::Local::new(scope, callback);
                let global = scope.get_current_context().global(scope);
                let arguments: Vec<_> = extra_args
                    .iter()
                    .map(|argument| v8::Local::new(scope, argument))
                    .collect();
                let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();
                let anchor = resource_initiator.as_ref().and_then(|registration| {
                    crate::script_continuation::callback_resource_initiator(
                        &mut scope,
                        Some(registration),
                    )
                });
                let result = crate::script_continuation::with_callback_resource_initiator(
                    &mut scope,
                    anchor,
                    |scope| callback.call(scope, global.into(), &arguments),
                );
                if result.is_some() {
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
                resource_initiator,
            } => {
                let callback = callback.prepare(scope);
                let context = v8::Local::new(scope, target_context);
                let Some(prepared) = PreparedWebIdlCallbackExecution::worker(
                    scope, callback, context,
                )
                .map(|prepared| prepared.with_resource_initiator(resource_initiator.clone())) else {
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
