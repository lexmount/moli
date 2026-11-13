//! Renderer execution policy for Web IDL callback functions shared by Window
//! and Worker APIs. Callback values and Realm entry remain in the neutral
//! callback crate; API owners retain scheduling, registration, return-value
//! conversion, and the choice to propagate or report exceptions.

use moli_webidl_callback::{PreparedWebIdlCallbackFunction, invoke_webidl_callback_function};

use crate::{
    common_runtime::{CommonRuntimeBinding, RuntimeExecutionContext},
    context_bootstrap::dispatch_window_callback_exception,
    exception_reporting::{
        CallbackExceptionLogLevel, V8ExceptionReport, invoke_callback_with_report,
    },
    native_bridge::{
        PageScriptHost, WindowExecutionContextBinding, WindowExecutionContextIdentity,
    },
    v8_execution_watchdog::V8ExecutionWatchdog,
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    worker::dispatch_current_worker_callback_exception,
};

pub(crate) fn invoke_traced_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    carrier: v8::Local<'s, v8::Object>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
    invoke: moli_webapi::WebIdlCallbackInvocation<'s>,
) -> Option<moli_webapi::WebIdlCallbackResult<'s>> {
    let callback = PreparedWebIdlCallbackExecution::from_traced(scope, carrier)?;
    match callback.invoke_with(scope, receiver, arguments, invoke) {
        WebIdlCallbackExecutionOutcome::Returned(value) => Some(Ok(value)),
        WebIdlCallbackExecutionOutcome::Failed(error) => Some(Err(error)),
        WebIdlCallbackExecutionOutcome::Unavailable => None,
    }
}

/// A short-lived invocation snapshot. It must not be retained as an async
/// task binding: the API's registration or task owner supplies that lifetime.
pub(crate) struct PreparedWebIdlCallbackExecution {
    callback: PreparedWebIdlCallbackFunction,
    environment: CallbackExecutionEnvironment,
    binding: std::rc::Rc<CommonRuntimeBinding>,
    resource_initiator: Option<crate::resource_initiator::CallbackInitiator>,
}

pub(crate) enum WebIdlCallbackExecutionOutcome<R, E> {
    Returned(R),
    Failed(E),
    Unavailable,
}

enum CallbackExecutionEnvironment {
    Window {
        context: v8::Global<v8::Context>,
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
        host_ptr: *mut PageScriptHost,
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
                context: v8::Global::new(scope, scope.get_current_context()),
                owner_identity,
                callback_identity,
            },
            binding,
            resource_initiator: None,
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
            resource_initiator: None,
        })
    }

    /// Resolve execution policy through the Context's installed binding.
    /// APIs with additional registration-time identities use `window` or
    /// `worker` after their owner-specific authorization instead.
    pub(crate) fn from_traced<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        carrier: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        // Reject a missing dispatch environment before interpreting a carrier.
        // The callback below still resolves its own actual relevant-context host.
        CommonRuntimeBinding::from_scope(scope)?;
        let traced = V8TracedWebIdlCallbackFunction::from_object(carrier);
        let resource_initiator = traced.resource_initiator(scope);
        let callback = traced.prepare(scope);
        let context = callback.relevant_context(scope);
        let (binding, owner) = CommonRuntimeBinding::from_context(context)?;
        let environment = match owner {
            RuntimeExecutionContext::Window(host_ptr) => {
                let host = unsafe { &*host_ptr };
                let context = callback.relevant_context(scope);
                let callback_identity =
                    host.window_execution_context_identity_for_v8_context(scope, context)?;
                CallbackExecutionEnvironment::Window {
                    context: v8::Global::new(scope, context),
                    owner_identity: None,
                    callback_identity,
                }
            }
            RuntimeExecutionContext::Worker => CallbackExecutionEnvironment::Worker {
                context: v8::Global::new(scope, context),
            },
        };
        Some(Self {
            callback,
            environment,
            binding,
            resource_initiator: Some(resource_initiator),
        })
    }

    pub(crate) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        self.callback.relevant_context(scope)
    }

    pub(crate) fn with_resource_initiator(
        mut self,
        initiator: crate::resource_initiator::CallbackInitiator,
    ) -> Self {
        self.resource_initiator = Some(initiator);
        self
    }

    pub(crate) fn is_current(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        if V8ExecutionWatchdog::callback_dispatch_has_timed_out(scope) {
            return false;
        }
        // Validate the exact preparation Context independently of the caller's
        // ambient Realm. The neutral callback layer then enters the relevant
        // Realm, which may belong to another Window or Page owner.
        let relevant_context = self.callback.relevant_context(scope);
        match &self.environment {
            CallbackExecutionEnvironment::Window { context, .. } => {
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                self.environment
                    .can_invoke(scope, &self.binding, relevant_context)
            }
            CallbackExecutionEnvironment::Worker { .. } => {
                self.environment
                    .can_invoke(scope, &self.binding, relevant_context)
            }
        }
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
        let initiator =
            crate::resource_initiator::callback_anchor(scope, self.resource_initiator.as_ref());
        match invoke_webidl_callback_function(
            scope,
            &self.callback,
            receiver,
            arguments,
            |scope, callback, receiver, arguments| {
                crate::resource_initiator::with_callback_anchor(scope, initiator, |scope| {
                    invoke_function(scope, callback, receiver, arguments)
                })
            },
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
                self.report_exception(scope, event_type, report);
                true
            }
            WebIdlCallbackExecutionOutcome::Unavailable => false,
        }
    }

    /// Report an already-captured exception while its relevant Realm is
    /// entered. The caller chooses whether this shares the invocation budget.
    pub(crate) fn report_exception(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        event_type: &str,
        report: Box<V8ExceptionReport>,
    ) {
        if V8ExecutionWatchdog::callback_dispatch_has_timed_out(scope) {
            return;
        }
        match &self.environment {
            CallbackExecutionEnvironment::Window {
                callback_identity, ..
            } => {
                if event_type == "error" {
                    return;
                }
                let context = self.relevant_context(scope);
                let Some((_, RuntimeExecutionContext::Window(host_ptr))) =
                    CommonRuntimeBinding::from_context(context)
                else {
                    return;
                };
                let binding = WindowExecutionContextBinding::new(
                    callback_identity.owner(),
                    callback_identity.dispatch_scope(),
                    callback_identity.realm_token(),
                    v8::Global::new(scope, context),
                );
                binding.with_current_scope(scope, host_ptr, |scope, _| {
                    dispatch_window_callback_exception(scope, host_ptr, None, &report);
                });
            }
            CallbackExecutionEnvironment::Worker { context } => {
                if !matches!(
                    self.binding.execution_context(scope),
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

impl CallbackExecutionEnvironment {
    fn can_invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        binding: &CommonRuntimeBinding,
        relevant_context: v8::Local<'_, v8::Context>,
    ) -> bool {
        match (self, binding.callback_execution_context(scope)) {
            (
                Self::Window {
                    owner_identity,
                    callback_identity,
                    ..
                },
                Some(RuntimeExecutionContext::Window(host_ptr)),
            ) => {
                let host = unsafe { &*host_ptr };
                if !owner_identity.is_none_or(|identity| {
                    host.window_execution_context_identity_is_current(identity)
                }) {
                    return false;
                }
                let Some((_, RuntimeExecutionContext::Window(callback_host_ptr))) =
                    CommonRuntimeBinding::from_context(relevant_context)
                else {
                    return false;
                };
                let callback_host = unsafe { &*callback_host_ptr };
                let current =
                    callback_host.window_execution_context_identity_is_current(*callback_identity);
                if current {
                    host.debug_assert_not_in_structural_mutation("callback-function invocation");
                    callback_host
                        .debug_assert_not_in_structural_mutation("callback-function invocation");
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
}
