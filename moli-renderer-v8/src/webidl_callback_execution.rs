//! Renderer execution policy for Web IDL callback functions shared by Window
//! and Worker APIs. Callback values and Realm entry remain in the neutral
//! callback crate; API owners retain scheduling, registration, return-value
//! conversion, and the choice to propagate or report exceptions.

use moli_webidl_callback::{PreparedWebIdlCallbackFunction, invoke_webidl_callback_function};

use crate::{
    common_runtime::{CommonRuntimeBinding, RuntimeExecutionContext},
    exception_reporting::{
        CallbackExceptionLogLevel, V8ExceptionReport, invoke_callback_with_report,
    },
    host::report_event_callback_exception,
    native_bridge::{JsContextHost, WindowExecutionContextIdentity},
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    worker::dispatch_current_worker_callback_exception,
};

/// A short-lived invocation snapshot. It must not be retained as an async
/// task binding: the API's registration or task owner supplies that lifetime.
pub(crate) struct PreparedWebIdlCallbackExecution {
    callback: PreparedWebIdlCallbackFunction,
    environment: CallbackExecutionEnvironment,
    binding: std::rc::Rc<CommonRuntimeBinding>,
}

pub(crate) enum WebIdlCallbackExecutionOutcome<R, E> {
    Returned(R),
    Failed(E),
    Unavailable,
}

enum CallbackExecutionEnvironment {
    Window {
        // Some API registrations, including observers, authorize both the
        // API object's owner and the callback's potentially different Realm.
        owner_identity: Option<WindowExecutionContextIdentity>,
        callback_identity: WindowExecutionContextIdentity,
    },
    Worker {
        context: v8::Global<v8::Context>,
    },
}

impl PreparedWebIdlCallbackExecution {
    pub(crate) fn window(
        scope: &mut v8::PinScope<'_, '_>,
        callback: PreparedWebIdlCallbackFunction,
        host_ptr: *mut JsContextHost,
        owner_identity: Option<WindowExecutionContextIdentity>,
        callback_identity: WindowExecutionContextIdentity,
    ) -> Option<Self> {
        let (binding, context) = CommonRuntimeBinding::from_scope(scope)?;
        if !matches!(context, RuntimeExecutionContext::Window(installed) if installed == host_ptr) {
            return None;
        }
        Some(Self {
            callback,
            environment: CallbackExecutionEnvironment::Window {
                owner_identity,
                callback_identity,
            },
            binding,
        })
    }

    pub(crate) fn worker<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        callback: PreparedWebIdlCallbackFunction,
        context: v8::Local<'s, v8::Context>,
    ) -> Option<Self> {
        let (binding, owner) = CommonRuntimeBinding::from_scope(scope)?;
        if !matches!(owner, RuntimeExecutionContext::Worker) {
            return None;
        }
        Some(Self {
            callback,
            environment: CallbackExecutionEnvironment::Worker {
                context: v8::Global::new(scope, context),
            },
            binding,
        })
    }

    /// Resolve execution policy through the Context's installed binding.
    /// APIs with additional registration-time identities use `window` or
    /// `worker` after their owner-specific authorization instead.
    pub(crate) fn from_traced<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        carrier: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let (binding, owner) = CommonRuntimeBinding::from_scope(scope)?;
        let callback = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
        let environment = match owner {
            RuntimeExecutionContext::Window(host_ptr) => {
                let host = unsafe { &*host_ptr };
                let context = callback.relevant_context(scope);
                let callback_identity =
                    host.window_execution_context_identity_for_v8_context(scope, context)?;
                CallbackExecutionEnvironment::Window {
                    owner_identity: None,
                    callback_identity,
                }
            }
            RuntimeExecutionContext::Worker => CallbackExecutionEnvironment::Worker {
                context: v8::Global::new(scope, scope.get_current_context()),
            },
        };
        Some(Self {
            callback,
            environment,
            binding,
        })
    }

    pub(crate) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        self.callback.relevant_context(scope)
    }

    pub(crate) fn is_current(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        self.environment.can_invoke(scope, &self.binding)
    }

    /// Execute the caller's invocation and return conversion while the Web
    /// IDL relevant/incumbent contexts are entered. This entry point neither
    /// captures nor reports exceptions on behalf of the API algorithm.
    pub(crate) fn invoke_with<'s, 'a, R, E>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Value>,
        arguments: &'a [v8::Local<'s, v8::Value>],
        invoke_function: impl FnOnce(
            &mut v8::PinScope<'s, '_>,
            v8::Local<'s, v8::Function>,
            v8::Local<'s, v8::Value>,
            &'a [v8::Local<'s, v8::Value>],
        ) -> Result<R, E>,
    ) -> WebIdlCallbackExecutionOutcome<R, E> {
        if !self.is_current(scope) {
            return WebIdlCallbackExecutionOutcome::Unavailable;
        }
        match invoke_webidl_callback_function(
            scope,
            &self.callback,
            receiver,
            arguments,
            invoke_function,
        ) {
            Ok(result) => WebIdlCallbackExecutionOutcome::Returned(result),
            Err(error) => WebIdlCallbackExecutionOutcome::Failed(error),
        }
    }

    /// Capture a notification callback result without selecting its reporting
    /// policy. Timer owners, for example, must disarm their watchdog before
    /// routing a captured exception.
    pub(crate) fn invoke_with_report<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        callback_kind: &str,
        log_label: &str,
        callback_name: &str,
        receiver: v8::Local<'s, v8::Value>,
        arguments: &[v8::Local<'s, v8::Value>],
    ) -> WebIdlCallbackExecutionOutcome<v8::Global<v8::Value>, Box<V8ExceptionReport>> {
        self.invoke_with(
            scope,
            receiver,
            arguments,
            |scope, callback, receiver, arguments| {
                invoke_callback_with_report(
                    scope,
                    callback_kind,
                    log_label,
                    CallbackExceptionLogLevel::Debug,
                    callback_name,
                    callback,
                    receiver,
                    arguments,
                )
            },
        )
    }

    /// Return whether delivery was attempted. A reported exception still
    /// counts as delivery; an unavailable execution binding does not.
    pub(crate) fn invoke_and_report<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event_type: &str,
        callback_name: &str,
        receiver: v8::Local<'s, v8::Value>,
        arguments: &[v8::Local<'s, v8::Value>],
    ) -> bool {
        // No registry borrow survives preparation. Author code may reenter
        // the API or register another callback synchronously.
        match self.invoke_with_report(
            scope,
            "callback",
            "Web IDL callback threw",
            callback_name,
            receiver,
            arguments,
        ) {
            WebIdlCallbackExecutionOutcome::Returned(_) => true,
            WebIdlCallbackExecutionOutcome::Failed(report) => {
                self.environment
                    .report_exception(scope, &self.binding, event_type, report);
                true
            }
            WebIdlCallbackExecutionOutcome::Unavailable => false,
        }
    }
}

impl CallbackExecutionEnvironment {
    fn can_invoke(&self, scope: &mut v8::PinScope<'_, '_>, binding: &CommonRuntimeBinding) -> bool {
        match (self, binding.callback_execution_context(scope)) {
            (
                Self::Window {
                    owner_identity,
                    callback_identity,
                },
                Some(RuntimeExecutionContext::Window(host_ptr)),
            ) => {
                let host = unsafe { &*host_ptr };
                let current = owner_identity.is_none_or(|identity| {
                    host.window_execution_context_identity_is_current(identity)
                }) && host
                    .window_execution_context_identity_is_current(*callback_identity);
                if current {
                    host.debug_assert_not_in_structural_mutation("callback-function invocation");
                }
                current
            }
            (Self::Worker { context }, Some(RuntimeExecutionContext::Worker)) => {
                scope.get_current_context() == v8::Local::new(scope, context)
                    // `close()` stops future task delivery, but synchronous
                    // APIs and already queued V8 microtasks may still run.
                    // API/task owners enforce their own closing policy.
                    && !scope.is_execution_terminating()
            }
            _ => false,
        }
    }

    fn report_exception(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        binding: &CommonRuntimeBinding,
        event_type: &str,
        report: Box<V8ExceptionReport>,
    ) {
        match self {
            Self::Window {
                callback_identity, ..
            } => {
                if let Some(host_ptr) = binding.window_host_ptr() {
                    report_event_callback_exception(
                        scope,
                        host_ptr,
                        event_type,
                        Some(*callback_identity),
                        None,
                        &report,
                    );
                }
            }
            Self::Worker { context } => {
                if !matches!(
                    binding.execution_context(scope),
                    Some(RuntimeExecutionContext::Worker)
                ) {
                    return;
                }
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                dispatch_current_worker_callback_exception(scope, *report);
            }
        }
    }
}
