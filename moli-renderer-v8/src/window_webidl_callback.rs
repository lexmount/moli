//! Long-lived residence and exact Window identity for Web IDL callbacks.
//!
//! `moli-webidl-callback` owns the callback object and the relevant and
//! incumbent contexts required by Web IDL. The renderer must additionally
//! bind an asynchronous callback to the exact Window execution context that
//! supplied it. Keeping that identity here prevents the renderer-neutral
//! crate from learning about Page, Document, popup, or child-frame lifetime.
//! Single-invocation snapshots, Realm entry, and execution policy belong to
//! `webidl_callback_execution`; this module does not invoke JavaScript.

use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    native_bridge::{
        JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
        WindowExecutionContextOwner,
    },
    webidl_callback_execution::PreparedWebIdlCallbackExecution,
};

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

    /// Prepare a single invocation while keeping the captured callback Realm
    /// and any API-specific owner identity authoritative.
    pub(crate) fn prepare_execution(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        owner_identity: Option<WindowExecutionContextIdentity>,
    ) -> Option<PreparedWebIdlCallbackExecution> {
        let Self::Live {
            callback,
            relevant_identity,
        } = self
        else {
            return None;
        };
        let callback = callback.prepare(scope);
        PreparedWebIdlCallbackExecution::window(
            scope,
            callback,
            host_ptr,
            owner_identity,
            *relevant_identity,
        )
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
