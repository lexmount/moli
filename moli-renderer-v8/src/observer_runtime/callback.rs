//! Exact Realm ownership for observer callback functions.
//!
//! An observer and its callback can originate in different Window realms. The
//! observer controller owns batching and delivery, while this residence keeps
//! both realm identities beside the Web IDL callback-function value. A
//! delivery is valid only while both identities are current.

use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    native_bridge::{
        JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
        WindowExecutionContextOwner,
    },
    webidl_callback_execution::PreparedWebIdlCallbackExecution,
    window_webidl_callback::WindowWebIdlCallbackFunction,
};

pub(super) struct ObserverCallback {
    callback: WindowWebIdlCallbackFunction,
    observer_identity: Option<WindowExecutionContextIdentity>,
}

impl ObserverCallback {
    pub(super) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        observer: v8::Local<'_, v8::Object>,
        callback: WebIdlCallbackFunction,
    ) -> Self {
        let observer_identity = observer.get_creation_context(scope).and_then(|context| {
            host.window_execution_context_identity_for_v8_context(scope, context)
        });
        Self {
            callback: WindowWebIdlCallbackFunction::new(scope, host, callback),
            observer_identity,
        }
    }

    pub(super) fn prepare(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> Option<PreparedWebIdlCallbackExecution> {
        self.callback
            .prepare_execution(scope, host_ptr, Some(self.observer_identity?))
    }

    pub(super) fn is_owned_by(&self, owner: WindowExecutionContextOwner) -> bool {
        self.observer_identity
            .is_some_and(|identity| identity.owner() == owner)
            || self.callback.is_owned_by(owner)
    }

    pub(super) fn belongs_to_context_token(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> bool {
        self.observer_identity
            .is_some_and(|identity| identity.realm_token() == context_token)
            || self.callback.belongs_to_context_token(context_token)
    }
}
