//! Owner-local registrations for callbacks whose values and pending records
//! live on JavaScript objects. Window and Worker share identity lookup,
//! activation, ordering and cleanup; only execution authorization differs.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use indexmap::IndexMap;
use moli_webidl_callback::{PreparedWebIdlCallbackFunction, WebIdlCallbackFunction};

use super::{CommonRuntimeBinding, RuntimeExecutionContext};
use crate::{
    native_bridge::{
        RuntimeObservableContextToken, WindowExecutionContextIdentity, WindowExecutionContextOwner,
    },
    webidl_callback_execution::PreparedWebIdlCallbackExecution,
};

/// IDs are local to one common runtime and never reused. Lookup also checks
/// the exact JS owner, so an ID from another runtime cannot alias a binding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CallbackRegistrationId(u32);

impl CallbackRegistrationId {
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum CallbackRegistrationKind {
    ResizeObserver,
    PerformanceObserver,
    ReportingObserver,
}

/// V8-traced callback and conversion-time context anchors, temporarily read
/// from the API object's private slots. The registry retains none of them.
pub(crate) struct CallbackResidence<'s> {
    id: CallbackRegistrationId,
    callback: v8::Local<'s, v8::Object>,
    relevant_global: v8::Local<'s, v8::Object>,
    incumbent_global: v8::Local<'s, v8::Object>,
}

impl<'s> CallbackResidence<'s> {
    pub(crate) fn from_parts(
        id: CallbackRegistrationId,
        callback: v8::Local<'s, v8::Object>,
        relevant_global: v8::Local<'s, v8::Object>,
        incumbent_global: v8::Local<'s, v8::Object>,
    ) -> Self {
        Self {
            id,
            callback,
            relevant_global,
            incumbent_global,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        CallbackRegistrationId,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
    ) {
        (
            self.id,
            self.callback,
            self.relevant_global,
            self.incumbent_global,
        )
    }
}

#[derive(Clone, Copy)]
enum RegistrationIdentity {
    Window {
        observer: Option<WindowExecutionContextIdentity>,
        callback: Option<WindowExecutionContextIdentity>,
    },
    Worker,
}

#[derive(Clone)]
struct CallbackBinding {
    kind: CallbackRegistrationKind,
    observer: Rc<v8::Weak<v8::Object>>,
    // This binding contains only weak owner references, never a V8 Context
    // root. It does not keep a callback-observer cycle alive.
    execution: Rc<CommonRuntimeBinding>,
    identity: RegistrationIdentity,
}

impl CallbackBinding {
    fn is_current(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        match self.identity {
            RegistrationIdentity::Window { observer, callback } => {
                let Some(host_ptr) = self.execution.window_host_ptr() else {
                    return false;
                };
                // The binding verifies the original host's backing lifetime.
                // No host borrow or registry borrow survives into author JS.
                let host = unsafe { &*host_ptr };
                observer.is_some_and(|identity| {
                    host.window_execution_context_identity_is_current(identity)
                }) && callback.is_some_and(|identity| {
                    host.window_execution_context_identity_is_current(identity)
                })
            }
            RegistrationIdentity::Worker => {
                let context = scope.get_current_context();
                context
                    .get_slot::<CommonRuntimeBinding>()
                    .is_some_and(|binding| Rc::ptr_eq(&binding, &self.execution))
                    && self.execution.worker_can_deliver_task()
                    && !scope.is_execution_terminating()
            }
        }
    }

    fn is_owned_by(&self, owner: WindowExecutionContextOwner) -> bool {
        self.window_identities()
            .into_iter()
            .flatten()
            .any(|identity| identity.owner() == owner)
    }

    fn belongs_to_context_token(&self, token: RuntimeObservableContextToken) -> bool {
        self.window_identities()
            .into_iter()
            .flatten()
            .any(|identity| identity.realm_token() == token)
    }

    fn window_identities(&self) -> [Option<WindowExecutionContextIdentity>; 2] {
        match self.identity {
            RegistrationIdentity::Window { observer, callback } => [observer, callback],
            RegistrationIdentity::Worker => [None, None],
        }
    }
}

#[derive(Default)]
struct CallbackRegistryState {
    next_id: u32,
    bindings: HashMap<CallbackRegistrationId, CallbackBinding>,
    // Activation order is distinct from construction order. Disconnect
    // removes the root; observing again appends it to the registered set.
    active: IndexMap<CallbackRegistrationId, Rc<v8::Global<v8::Object>>>,
    active_resize: HashMap<CallbackRegistrationId, Rc<v8::Global<v8::Object>>>,
    active_reporting: IndexMap<CallbackRegistrationId, Rc<v8::Global<v8::Object>>>,
}

/// A weak identity table plus the explicitly active observer roots.
/// Callback values remain V8-traced. Native finalizers use a weak reference to
/// this table, avoiding raw host pointers and independent callback GC roots.
#[derive(Default)]
pub(crate) struct CallbackRegistry {
    state: Rc<RefCell<CallbackRegistryState>>,
}

impl CallbackRegistry {
    pub(crate) fn register<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        observer: v8::Local<'s, v8::Object>,
        callback: WebIdlCallbackFunction,
        kind: CallbackRegistrationKind,
    ) -> Option<CallbackResidence<'s>> {
        let context = scope.get_current_context();
        let execution = context.get_slot::<CommonRuntimeBinding>()?;
        let (runtime, owner) = execution.resolve_owner(context)?;
        if !Rc::ptr_eq(&runtime.callbacks.state, &self.state) {
            return None;
        }
        let observer_context = observer.get_creation_context(scope)?;
        let relevant_context = callback.relevant_context(scope);
        let identity = match owner {
            RuntimeExecutionContext::Window(host_ptr) => {
                let host = unsafe { &*host_ptr };
                RegistrationIdentity::Window {
                    observer: host
                        .window_execution_context_identity_for_v8_context(scope, observer_context),
                    callback: host
                        .window_execution_context_identity_for_v8_context(scope, relevant_context),
                }
            }
            RuntimeExecutionContext::Worker => {
                if observer_context != context || relevant_context != context {
                    return None;
                }
                RegistrationIdentity::Worker
            }
        };
        let callback_value = v8::Local::<v8::Object>::try_from(callback.value(scope)).ok()?;
        let relevant_global = relevant_context.global(scope);
        let incumbent_global = callback.incumbent_context(scope).global(scope);
        // Create weak handles outside the registry borrow: V8 may run native
        // finalizers during allocations, including another binding's cleanup.
        let observer_weak = Rc::new(v8::Weak::new(scope, observer));
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_id = state
                .next_id
                .checked_add(1)
                .expect("callback registration id space exhausted");
            let id = CallbackRegistrationId(state.next_id);
            let replaced = state.bindings.insert(
                id,
                CallbackBinding {
                    kind,
                    observer: observer_weak,
                    execution,
                    identity,
                },
            );
            assert!(replaced.is_none(), "callback registration ID was reused");
            id
        };
        let state = Rc::downgrade(&self.state);
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, observer, move || {
            if let Some(state) = state.upgrade() {
                let retired = {
                    let mut state = state.borrow_mut();
                    (
                        state.bindings.remove(&id),
                        state.active.shift_remove(&id),
                        state.active_resize.remove(&id),
                        state.active_reporting.shift_remove(&id),
                    )
                };
                drop(retired);
            }
        });
        Some(CallbackResidence::from_parts(
            id,
            callback_value,
            relevant_global,
            incumbent_global,
        ))
    }

    fn lookup(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
        kind: CallbackRegistrationKind,
    ) -> Option<CallbackBinding> {
        let binding = self.state.borrow().bindings.get(&id)?.clone();
        if binding.kind != kind || binding.observer.to_local(scope)? != observer {
            return None;
        }
        Some(binding)
    }

    pub(crate) fn is_current(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
        kind: CallbackRegistrationKind,
    ) -> bool {
        self.lookup(scope, id, observer, kind)
            .is_some_and(|binding| binding.is_current(scope))
    }

    pub(crate) fn prepare<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        observer: v8::Local<'s, v8::Object>,
        kind: CallbackRegistrationKind,
        residence: CallbackResidence<'s>,
    ) -> Option<PreparedWebIdlCallbackExecution> {
        let binding = self.lookup(scope, residence.id, observer, kind)?;
        if !binding.is_current(scope) {
            return None;
        }
        let relevant = residence.relevant_global.get_creation_context(scope)?;
        let incumbent = residence.incumbent_global.get_creation_context(scope)?;
        let callback = PreparedWebIdlCallbackFunction::try_new(
            scope,
            residence.callback,
            relevant,
            incumbent,
        )?;
        match binding.identity {
            RegistrationIdentity::Window {
                observer,
                callback: callback_identity,
            } => PreparedWebIdlCallbackExecution::window(
                scope,
                callback,
                binding.execution.window_host_ptr()?,
                Some(observer?),
                callback_identity?,
            ),
            RegistrationIdentity::Worker => {
                let context = observer.get_creation_context(scope)?;
                PreparedWebIdlCallbackExecution::worker(scope, callback, context)
            }
        }
    }

    pub(crate) fn activate_resize_observer(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
    ) -> Option<WindowExecutionContextIdentity> {
        let binding = self.lookup(
            scope,
            id,
            observer,
            CallbackRegistrationKind::ResizeObserver,
        )?;
        if !binding.is_current(scope) {
            return None;
        }
        let identity = binding.window_identities()[0]?;
        let root = Rc::new(v8::Global::new(scope, observer));
        {
            let mut state = self.state.borrow_mut();
            if !state.bindings.contains_key(&id) {
                return None;
            }
            state
                .active_resize
                .entry(id)
                .or_insert_with(|| Rc::clone(&root));
        }
        Some(identity)
    }

    pub(crate) fn deactivate_resize_observer(&self, id: CallbackRegistrationId) {
        let retired = self.state.borrow_mut().active_resize.remove(&id);
        drop(retired);
    }

    pub(crate) fn active_resize_observers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: Option<WindowExecutionContextOwner>,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        let mut registrations: Vec<_> = {
            let state = self.state.borrow();
            state
                .active_resize
                .iter()
                .filter_map(|(id, root)| {
                    Some((*id, Rc::clone(root), state.bindings.get(id)?.clone()))
                })
                .collect()
        };
        registrations.sort_unstable_by_key(|(id, _, _)| id.as_u32());
        registrations
            .into_iter()
            .filter_map(|(_, root, binding)| {
                let identity = binding.window_identities()[0]?;
                if owner.is_some_and(|owner| owner != identity.owner())
                    || !binding.is_current(scope)
                {
                    return None;
                }
                Some(v8::Local::new(scope, root.as_ref()))
            })
            .collect()
    }

    pub(crate) fn resize_observer_identities(&self) -> Vec<WindowExecutionContextIdentity> {
        let bindings: Vec<_> = {
            let state = self.state.borrow();
            state
                .active_resize
                .keys()
                .filter_map(|id| state.bindings.get(id).cloned())
                .collect()
        };
        let mut identities = Vec::new();
        for binding in bindings {
            let Some(host_ptr) = binding.execution.window_host_ptr() else {
                continue;
            };
            let [Some(observer), Some(callback)] = binding.window_identities() else {
                continue;
            };
            let host = unsafe { &*host_ptr };
            if host.window_execution_context_identity_is_current(observer)
                && host.window_execution_context_identity_is_current(callback)
                && !identities.contains(&observer)
            {
                identities.push(observer);
            }
        }
        identities
    }

    pub(crate) fn activate_performance_observer(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
    ) -> bool {
        if !self.is_current(
            scope,
            id,
            observer,
            CallbackRegistrationKind::PerformanceObserver,
        ) {
            return false;
        }
        let root = Rc::new(v8::Global::new(scope, observer));
        {
            let mut state = self.state.borrow_mut();
            // A native cleanup during V8 allocation may have retired the
            // registration. Never leave an active root without its binding.
            if !state.bindings.contains_key(&id) {
                return false;
            }
            state.active.entry(id).or_insert_with(|| Rc::clone(&root));
        }
        true
    }

    pub(crate) fn deactivate_performance_observer(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
    ) {
        if self
            .lookup(
                scope,
                id,
                observer,
                CallbackRegistrationKind::PerformanceObserver,
            )
            .is_some()
        {
            let retired = self.state.borrow_mut().active.shift_remove(&id);
            drop(retired);
        }
    }

    pub(crate) fn active_performance_observers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        timeline_context: v8::Local<'s, v8::Context>,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        let roots: Vec<_> = self.state.borrow().active.values().map(Rc::clone).collect();
        roots
            .iter()
            .map(|root| v8::Local::new(scope, root.as_ref()))
            .filter(|observer| observer.get_creation_context(scope) == Some(timeline_context))
            .collect()
    }

    pub(crate) fn activate_reporting_observer(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
    ) -> bool {
        if !self.is_current(
            scope,
            id,
            observer,
            CallbackRegistrationKind::ReportingObserver,
        ) {
            return false;
        }
        // Allocating before borrowing permits V8 finalizers to retire other registrations.
        let root = Rc::new(v8::Global::new(scope, observer));
        let mut state = self.state.borrow_mut();
        if !state.bindings.contains_key(&id) {
            return false;
        }
        state
            .active_reporting
            .entry(id)
            .or_insert_with(|| Rc::clone(&root));
        true
    }

    pub(crate) fn deactivate_reporting_observer(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CallbackRegistrationId,
        observer: v8::Local<'_, v8::Object>,
    ) {
        if self
            .lookup(
                scope,
                id,
                observer,
                CallbackRegistrationKind::ReportingObserver,
            )
            .is_some()
        {
            let retired = self.state.borrow_mut().active_reporting.shift_remove(&id);
            drop(retired);
        }
    }

    pub(crate) fn active_reporting_observers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        let roots: Vec<_> = {
            let state = self.state.borrow();
            state
                .active_reporting
                .iter()
                .filter_map(|(id, root)| Some((Rc::clone(root), state.bindings.get(id)?.clone())))
                .collect()
        };
        roots
            .into_iter()
            .filter_map(|(root, binding)| {
                if !binding.is_current(scope) {
                    return None;
                }
                let observer = v8::Local::new(scope, root.as_ref());
                (observer.get_creation_context(scope) == Some(context)).then_some(observer)
            })
            .collect()
    }

    pub(crate) fn retire_execution_context_owner(
        &self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        self.retire_bindings(|binding| binding.is_owned_by(owner))
    }

    pub(crate) fn retire_context_token(&self, token: RuntimeObservableContextToken) -> usize {
        self.retire_bindings(|binding| binding.belongs_to_context_token(token))
    }

    fn retire_bindings(&self, mut retire: impl FnMut(&CallbackBinding) -> bool) -> usize {
        let retired = {
            let mut state = self.state.borrow_mut();
            let ids: Vec<_> = state
                .bindings
                .iter()
                .filter_map(|(id, binding)| retire(binding).then_some(*id))
                .collect();
            ids.into_iter()
                .map(|id| {
                    (
                        state.bindings.remove(&id),
                        state.active.shift_remove(&id),
                        state.active_resize.remove(&id),
                        state.active_reporting.shift_remove(&id),
                    )
                })
                .collect::<Vec<_>>()
        };
        retired.len()
    }

    pub(super) fn clear_for_context_teardown(&self) {
        let retired = {
            let mut state = self.state.borrow_mut();
            (
                std::mem::take(&mut state.bindings),
                std::mem::take(&mut state.active),
                std::mem::take(&mut state.active_resize),
                std::mem::take(&mut state.active_reporting),
            )
        };
        drop(retired);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.state.borrow().bindings.len()
    }
}
