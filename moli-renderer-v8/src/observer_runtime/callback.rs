//! Exact Realm ownership for observer callback functions.
//!
//! An observer and its callback can originate in different Window realms. The
//! observer controller owns batching and delivery, while this residence keeps
//! both realm identities beside the Web IDL callback-function value. A
//! delivery is valid only while both identities are current.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use moli_webidl_callback::{PreparedWebIdlCallbackFunction, WebIdlCallbackFunction};

use crate::{
    native_bridge::{
        JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
        WindowExecutionContextOwner,
    },
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunction, WindowWebIdlCallbackFunction,
        WindowWebIdlCallbackFunctionOutcome, invoke_window_webidl_callback_function,
    },
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

    pub(super) fn prepare(&self, scope: &mut v8::PinScope<'_, '_>) -> PreparedObserverCallback {
        PreparedObserverCallback {
            callback: self.callback.prepare(scope),
            observer_identity: self.observer_identity,
        }
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

/// Stable, renderer-local identity for one observer callback residence.
///
/// ResizeObserver and PerformanceObserver keep their API-specific pending
/// queues and V8-traced callback/context values on their JS objects. This ID
/// binds those private values to the exact observer and callback Window
/// identities without making Rust a permanent GC root.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ObserverCallbackId(u32);

impl ObserverCallbackId {
    pub(crate) const fn as_u32(self) -> u32 {
        self.0
    }

    pub(crate) fn from_number(value: f64) -> Option<Self> {
        if !value.is_finite() || value.fract() != 0.0 || !(1.0..=(u32::MAX as f64)).contains(&value)
        {
            return None;
        }
        Some(Self(value as u32))
    }
}

pub(super) struct ObserverCallbackBinding {
    observer_identity: Option<WindowExecutionContextIdentity>,
    callback_identity: Option<WindowExecutionContextIdentity>,
    // PerformanceObserver's registered-observer set keeps an actively
    // observing wrapper alive. ResizeObserver does not use this root. Exact
    // Realm retirement or disconnect removes it before the V8 cycle can be
    // collected.
    active_performance_observer: Option<Rc<v8::Global<v8::Object>>>,
}

impl ObserverCallbackBinding {
    pub(super) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        observer: v8::Local<'_, v8::Object>,
        callback: &WebIdlCallbackFunction,
    ) -> Self {
        let observer_identity = observer.get_creation_context(scope).and_then(|context| {
            host.window_execution_context_identity_for_v8_context(scope, context)
        });
        let callback_context = callback.relevant_context(scope);
        let callback_identity =
            host.window_execution_context_identity_for_v8_context(scope, callback_context);
        Self {
            observer_identity,
            callback_identity,
            active_performance_observer: None,
        }
    }

    fn is_current(&self, host: &JsContextHost) -> bool {
        self.observer_identity
            .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
            && self
                .callback_identity
                .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
    }

    fn is_owned_by(&self, owner: WindowExecutionContextOwner) -> bool {
        self.observer_identity
            .is_some_and(|identity| identity.owner() == owner)
            || self
                .callback_identity
                .is_some_and(|identity| identity.owner() == owner)
    }

    fn belongs_to_context_token(&self, context_token: RuntimeObservableContextToken) -> bool {
        self.observer_identity
            .is_some_and(|identity| identity.realm_token() == context_token)
            || self
                .callback_identity
                .is_some_and(|identity| identity.realm_token() == context_token)
    }
}

#[derive(Default)]
struct ObserverCallbackRegistryState {
    next_id: u32,
    bindings: HashMap<ObserverCallbackId, ObserverCallbackBinding>,
}

/// Shared owner-local identity registry for observer APIs whose batching and
/// callback values live on their JavaScript objects.
///
/// The `Rc<RefCell<_>>` is deliberate: a V8 weak finalizer must be able to
/// release the small identity binding after the observer becomes unreachable,
/// without retaining a raw `JsContextHost` pointer. The registry never roots
/// the callback or observer, so callback↔observer cycles remain V8-collectable.
#[derive(Clone, Default)]
pub(super) struct ObserverCallbackRegistry {
    state: Rc<RefCell<ObserverCallbackRegistryState>>,
}

impl ObserverCallbackRegistry {
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.state.borrow().bindings.len()
    }

    pub(super) fn register(&self, binding: ObserverCallbackBinding) -> ObserverCallbackId {
        let mut state = self.state.borrow_mut();
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("observer callback id space exhausted");
        let id = ObserverCallbackId(state.next_id);
        let replaced = state.bindings.insert(id, binding);
        assert!(
            replaced.is_none(),
            "observer callback binding identities must not be reused"
        );
        id
    }

    pub(super) fn prepare<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        id: ObserverCallbackId,
        callback: v8::Local<'s, v8::Object>,
        relevant_global: v8::Local<'s, v8::Object>,
        incumbent_global: v8::Local<'s, v8::Object>,
    ) -> Option<PreparedObserverCallback> {
        let (observer_identity, callback_identity) = {
            let state = self.state.borrow();
            let binding = state.bindings.get(&id)?;
            if !binding.is_current(host) {
                return None;
            }
            (binding.observer_identity, binding.callback_identity)
        };
        let relevant_context = relevant_global.get_creation_context(scope)?;
        let incumbent_context = incumbent_global.get_creation_context(scope)?;
        let callback = PreparedWebIdlCallbackFunction::try_new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
        )?;
        Some(PreparedObserverCallback {
            callback: PreparedWindowWebIdlCallbackFunction::Live {
                callback,
                relevant_identity: callback_identity?,
            },
            observer_identity,
        })
    }

    pub(super) fn is_current(&self, host: &JsContextHost, id: ObserverCallbackId) -> bool {
        self.state
            .borrow()
            .bindings
            .get(&id)
            .is_some_and(|binding| binding.is_current(host))
    }

    pub(super) fn activate_performance_observer<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        id: ObserverCallbackId,
        observer: v8::Local<'s, v8::Object>,
    ) -> bool {
        // Build the V8 root before borrowing the registry. A GC/finalizer must
        // never reenter the same RefCell while an activation mutably borrows it.
        let observer = Rc::new(v8::Global::new(scope, observer));
        let mut state = self.state.borrow_mut();
        let Some(binding) = state.bindings.get_mut(&id) else {
            return false;
        };
        if !binding.is_current(host) {
            return false;
        }
        binding.active_performance_observer = Some(observer);
        true
    }

    pub(super) fn deactivate_performance_observer(&self, id: ObserverCallbackId) -> bool {
        let observer = self
            .state
            .borrow_mut()
            .bindings
            .get_mut(&id)
            .and_then(|binding| binding.active_performance_observer.take());
        observer.is_some()
    }

    pub(super) fn active_performance_observers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        let observers: Vec<_> = self
            .state
            .borrow()
            .bindings
            .values()
            .filter_map(|binding| binding.active_performance_observer.as_ref().map(Rc::clone))
            .collect();
        observers
            .iter()
            .map(|observer| v8::Local::new(scope, observer.as_ref()))
            .collect()
    }

    pub(super) fn retire_execution_context_owner(
        &self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        self.retire_bindings(|binding| binding.is_owned_by(owner))
    }

    pub(super) fn retire_context_token(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        self.retire_bindings(|binding| binding.belongs_to_context_token(context_token))
    }

    fn retire_bindings(
        &self,
        mut should_retire: impl FnMut(&ObserverCallbackBinding) -> bool,
    ) -> usize {
        let retired = {
            let mut state = self.state.borrow_mut();
            let ids: Vec<_> = state
                .bindings
                .iter()
                .filter_map(|(id, binding)| should_retire(binding).then_some(*id))
                .collect();
            ids.into_iter()
                .filter_map(|id| state.bindings.remove(&id))
                .collect::<Vec<_>>()
        };
        retired.len()
    }

    pub(super) fn finalizer_cleanup(&self, id: ObserverCallbackId) -> impl FnOnce() + 'static {
        let state = Rc::downgrade(&self.state);
        move || {
            if let Some(state) = Weak::upgrade(&state) {
                let retired = state.borrow_mut().bindings.remove(&id);
                drop(retired);
            }
        }
    }
}

pub(crate) struct PreparedObserverCallback {
    callback: PreparedWindowWebIdlCallbackFunction,
    observer_identity: Option<WindowExecutionContextIdentity>,
}

impl PreparedObserverCallback {
    pub(crate) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        self.callback.relevant_context(scope)
    }

    pub(crate) fn is_current(&self, host: &JsContextHost) -> bool {
        self.observer_identity
            .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
            && self.callback.is_current(host)
    }

    pub(crate) const fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        self.callback.relevant_identity()
    }

    pub(crate) fn invoke<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        callback_name: &str,
        receiver: v8::Local<'s, v8::Value>,
        arguments: &[v8::Local<'s, v8::Value>],
    ) -> WindowWebIdlCallbackFunctionOutcome {
        if !self.is_current(unsafe { &*host_ptr }) {
            return WindowWebIdlCallbackFunctionOutcome::Retired;
        }
        invoke_window_webidl_callback_function(
            scope,
            host_ptr,
            "callback",
            "observer callback threw",
            callback_name,
            &self.callback,
            receiver,
            arguments,
        )
    }
}
