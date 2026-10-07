use std::{cell::RefCell, ops::Deref, rc::Weak};

use crate::native_bridge::{JsContextHost, JsContextHostBridgeRef, RuntimeObservableContextToken};

use super::RendererDeferredContextHostReleaseQueue;

/// One execution owner, created before fallible Window bootstrap and moved
/// with the Context through prebootstrap and publication. Retained JS objects
/// own their native backing separately; they never own this runtime record.
pub(crate) struct WindowRealmRuntime {
    state: Option<WindowRealmRuntimeState>,
    deferred_releases: RendererDeferredContextHostReleaseQueue,
    closed: bool,
}

pub(crate) struct WindowRealmRuntimeState {
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) runtime_observable_context_token: RuntimeObservableContextToken,
    pub(crate) bridge_ref: Option<JsContextHostBridgeRef>,
    host: Weak<RefCell<JsContextHost>>,
    isolated: bool,
}

impl std::fmt::Debug for WindowRealmRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowRealmRuntimeState")
            .field("realm_token", &self.runtime_observable_context_token)
            .field("isolated", &self.isolated)
            .finish_non_exhaustive()
    }
}

impl WindowRealmRuntime {
    pub(super) fn new(
        scope: &mut v8::PinScope<'_, '_, ()>,
        context: v8::Local<'_, v8::Context>,
        host: Weak<RefCell<JsContextHost>>,
        runtime_observable_context_token: RuntimeObservableContextToken,
        deferred_releases: RendererDeferredContextHostReleaseQueue,
        isolated: bool,
    ) -> Self {
        Self {
            state: Some(WindowRealmRuntimeState {
                context: v8::Global::new(scope, context),
                runtime_observable_context_token,
                bridge_ref: None,
                host,
                isolated,
            }),
            deferred_releases,
            closed: false,
        }
    }

    pub(super) fn install_bridge_ref(&mut self, bridge_ref: JsContextHostBridgeRef) {
        let previous = self.state.as_mut().unwrap().bridge_ref.replace(bridge_ref);
        assert!(
            previous.is_none(),
            "a Window realm has one native bridge owner"
        );
    }

    pub(crate) fn mark_closed(&mut self) {
        self.closed = true;
    }

    pub(super) fn release_bridge_ref(&mut self) {
        self.state.as_mut().unwrap().bridge_ref.take();
    }
}

impl Deref for WindowRealmRuntime {
    type Target = WindowRealmRuntimeState;

    fn deref(&self) -> &Self::Target {
        self.state
            .as_ref()
            .expect("a live runtime owns its Context")
    }
}

impl Drop for WindowRealmRuntime {
    fn drop(&mut self) {
        if !self.closed {
            // A cache replacement or bootstrap error can occur during a V8
            // callback with the host already borrowed. Drop only queues the
            // record; cleanup runs after the outer entered-isolate operation.
            let mut state = self.state.take().unwrap();
            // Before publication this bridge token is the only native owner
            // installed in the Context. Do not defer it past recovery of the
            // bootstrap's DocumentRuntime, which the host only borrows.
            // Published realms already own their host through the Context.
            state.bridge_ref.take();
            self.deferred_releases.defer_window_realm_close(state);
        }
    }
}

impl WindowRealmRuntimeState {
    pub(super) fn close_in_entered_isolate(self, isolate: &mut v8::OwnedIsolate) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let host_ptr = host.as_ptr();
        let child_handle = (!self.isolated)
            .then(|| {
                crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(
                    scope,
                )
            })
            .flatten();
        // An unpublished partial bootstrap has not acquired a Context-owned
        // native host. Its outer owner still tears down that native backing.
        if unsafe { &*host_ptr }.document_host_is_published()
            && crate::util::context_host_ptr_from_context_slot(context).is_some()
        {
            crate::util::retain_context_host_for_document_realm(
                context,
                host.clone(),
                unsafe { &*host_ptr }.deferred_context_host_release_queue(),
            );
            crate::native_bridge::clear_context_wrapper_cache_for_teardown(scope, false);
        }
        {
            let host = &mut *host.borrow_mut();
            host.retire_window_realm_resources(self.runtime_observable_context_token);
            if self.isolated {
                host.retire_isolated_window_execution_context(
                    self.runtime_observable_context_token,
                );
            } else {
                host.retire_window_execution_contexts_for_context_token(
                    self.runtime_observable_context_token,
                );
            }
        }
        if let Some(handle) = child_handle {
            let owns_current_proxy = unsafe { &*host_ptr }
                .existing_child_browsing_context_window_wrapper(scope, handle)
                .and_then(|proxy| proxy.get_creation_context(scope))
                .is_some_and(|current| current == context);
            if owns_current_proxy {
                context.detach_global();
                if !unsafe { &mut *host_ptr }
                    .preserve_child_window_proxy_between_realms(scope, handle)
                {
                    tracing::warn!(
                        child_handle = handle.index(),
                        "failed to park a child WindowProxy after aborted realm publication"
                    );
                }
            }
        }
    }
}
