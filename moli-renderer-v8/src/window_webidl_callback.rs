//! Renderer ownership for a Window-realm Web IDL callback function.
//!
//! `moli-webidl-callback` owns the callback object and the relevant and
//! incumbent contexts required by Web IDL. The renderer must additionally
//! bind an asynchronous callback to the exact Window execution context that
//! supplied it. Keeping that identity here prevents the renderer-neutral
//! crate from learning about Page, Document, popup, or child-frame lifetime.

use moli_webidl_callback::{
    PreparedWebIdlCallbackFunction, WebIdlCallbackFunction, invoke_webidl_callback_function,
};

use crate::{
    exception_reporting::{
        CallbackExceptionLogLevel, V8ExceptionReport, invoke_callback_with_report,
    },
    native_bridge::{
        JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
        WindowExecutionContextOwner,
    },
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
};

/// A callback-function residence traced through an owning V8 object.
///
/// Some Web APIs already keep their pending algorithm state on a JavaScript
/// object. Rooting the callback independently in Rust would keep
/// callback-to-owner cycles alive after that object becomes unreachable. This
/// carrier instead stores the callback and both Web IDL context anchors in
/// private V8 slots. It acquires a temporary Rust root only while one
/// invocation is prepared.
///
/// Page, Document, task, and Promise ownership deliberately remain outside
/// this type. The API owner must retain this carrier in its existing
/// residence and decide what a retired callback Realm means.
pub(crate) struct V8TracedWindowWebIdlCallbackFunction<'s> {
    callback: V8TracedWebIdlCallbackFunction<'s>,
}

impl<'s> V8TracedWindowWebIdlCallbackFunction<'s> {
    pub(crate) fn new(scope: &mut v8::PinScope<'s, '_>, callback: WebIdlCallbackFunction) -> Self {
        Self {
            callback: V8TracedWebIdlCallbackFunction::new(scope, callback),
        }
    }

    pub(crate) const fn from_object(carrier: v8::Local<'s, v8::Object>) -> Self {
        Self {
            callback: V8TracedWebIdlCallbackFunction::from_object(carrier),
        }
    }

    pub(crate) const fn into_object(self) -> v8::Local<'s, v8::Object> {
        self.callback.into_object()
    }

    pub(crate) fn prepare(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
    ) -> PreparedWindowWebIdlCallbackFunction {
        let callback = self.callback.prepare(scope);
        let relevant_context = callback.relevant_context(scope);
        let Some(relevant_identity) =
            host.window_execution_context_identity_for_v8_context(scope, relevant_context)
        else {
            return PreparedWindowWebIdlCallbackFunction::Retired;
        };
        PreparedWindowWebIdlCallbackFunction::Live {
            callback,
            relevant_identity,
        }
    }
}

pub(crate) enum WindowWebIdlCallbackFunction {
    Live {
        callback: WebIdlCallbackFunction,
        relevant_identity: WindowExecutionContextIdentity,
    },
    Retired,
}

impl WindowWebIdlCallbackFunction {
    pub(crate) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        callback: WebIdlCallbackFunction,
    ) -> Self {
        let relevant_context = callback.relevant_context(scope);
        let relevant_identity =
            host.window_execution_context_identity_for_v8_context(scope, relevant_context);
        match relevant_identity {
            Some(relevant_identity) => Self::Live {
                callback,
                relevant_identity,
            },
            None => Self::Retired,
        }
    }

    pub(crate) fn prepare(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> PreparedWindowWebIdlCallbackFunction {
        match self {
            Self::Live {
                callback,
                relevant_identity,
            } => PreparedWindowWebIdlCallbackFunction::Live {
                callback: callback.prepare(scope),
                relevant_identity: *relevant_identity,
            },
            Self::Retired => PreparedWindowWebIdlCallbackFunction::Retired,
        }
    }

    pub(crate) const fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        match self {
            Self::Live {
                relevant_identity, ..
            } => Some(*relevant_identity),
            Self::Retired => None,
        }
    }

    pub(crate) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        match self {
            Self::Live { callback, .. } => Some(callback.relevant_context(scope)),
            Self::Retired => None,
        }
    }

    pub(crate) fn is_owned_by(&self, owner: WindowExecutionContextOwner) -> bool {
        matches!(
            self,
            Self::Live {
                relevant_identity,
                ..
            } if relevant_identity.owner() == owner
        )
    }

    pub(crate) fn belongs_to_context_token(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> bool {
        matches!(
            self,
            Self::Live {
                relevant_identity,
                ..
            } if relevant_identity.realm_token() == context_token
        )
    }
}

pub(crate) enum PreparedWindowWebIdlCallbackFunction {
    Live {
        callback: PreparedWebIdlCallbackFunction,
        relevant_identity: WindowExecutionContextIdentity,
    },
    Retired,
}

/// Result of invoking one prepared callback after exact Window currentness is
/// checked.
///
/// The error type remains owned by the API-specific invocation policy. Most
/// callback families use `V8ExceptionReport`; Promise-returning callbacks such
/// as ViewTransition additionally need to distinguish JavaScript throws from
/// host-side return-value normalization failure.
pub(crate) enum PreparedWindowWebIdlCallbackFunctionOutcome<R, E> {
    Returned(R),
    Failed(E),
    Retired,
}

impl PreparedWindowWebIdlCallbackFunction {
    pub(crate) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        match self {
            Self::Live { callback, .. } => Some(callback.relevant_context(scope)),
            Self::Retired => None,
        }
    }

    pub(crate) const fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        match self {
            Self::Live {
                relevant_identity, ..
            } => Some(*relevant_identity),
            Self::Retired => None,
        }
    }

    pub(crate) fn is_current(&self, host: &JsContextHost) -> bool {
        matches!(
            self,
            Self::Live {
                relevant_identity,
                ..
            } if host.window_execution_context_identity_is_current(*relevant_identity)
        )
    }

    pub(crate) fn invoke<'s, 'a, R, E>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        receiver: v8::Local<'s, v8::Value>,
        arguments: &'a [v8::Local<'s, v8::Value>],
        invoke_function: impl FnOnce(
            &mut v8::PinScope<'s, '_>,
            v8::Local<'s, v8::Function>,
            v8::Local<'s, v8::Value>,
            &'a [v8::Local<'s, v8::Value>],
        ) -> Result<R, E>,
    ) -> PreparedWindowWebIdlCallbackFunctionOutcome<R, E> {
        host.debug_assert_not_in_structural_mutation("callback-function invocation");
        if !self.is_current(host) {
            return PreparedWindowWebIdlCallbackFunctionOutcome::Retired;
        }
        let Self::Live { callback, .. } = self else {
            return PreparedWindowWebIdlCallbackFunctionOutcome::Retired;
        };
        match invoke_webidl_callback_function(scope, callback, receiver, arguments, invoke_function)
        {
            Ok(returned) => PreparedWindowWebIdlCallbackFunctionOutcome::Returned(returned),
            Err(error) => PreparedWindowWebIdlCallbackFunctionOutcome::Failed(error),
        }
    }
}

pub(crate) enum WindowWebIdlCallbackFunctionOutcome {
    Returned,
    Threw(Box<V8ExceptionReport>),
    Retired,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_window_webidl_callback_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    callback_kind: &str,
    log_label: &str,
    callback_name: &str,
    callback: &PreparedWindowWebIdlCallbackFunction,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> WindowWebIdlCallbackFunctionOutcome {
    let host = unsafe { &*host_ptr };
    match callback.invoke(
        scope,
        host,
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
    ) {
        PreparedWindowWebIdlCallbackFunctionOutcome::Returned(_) => {
            WindowWebIdlCallbackFunctionOutcome::Returned
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Failed(report) => {
            WindowWebIdlCallbackFunctionOutcome::Threw(report)
        }
        PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {
            WindowWebIdlCallbackFunctionOutcome::Retired
        }
    }
}
