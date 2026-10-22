use std::collections::BTreeMap;

use super::*;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{
    JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget,
};

mod sampling;
pub(crate) use sampling::RealtimeResizeObserverSampling;

#[derive(Default)]
pub(super) struct RealtimeResizeObservers {
    pub(super) entries: BTreeMap<CallbackRegistrationId, RealtimeResizeObserver>,
    // Controller lifetime belongs to LocalWindow, not a V8 wrapper or Realm.
    // Empty controllers still participate in depth progression after all of
    // their weak observer wrappers have been collected.
    controllers: HashMap<WindowExecutionContextOwner, OwnerDispatchScope>,
    // Only one broadcast batch is active. The guard releases all source IDs
    // on completion or failure; this is not another observation registry.
    sampling: Rc<RefCell<Option<HashMap<DomHandle, bool>>>>,
}

/// Constructor residence survives disconnect while JS retains the observer.
/// Only active observations root the wrapper. Targets, active records and
/// last-reported sizes remain on that wrapper, shared with the existing API.
pub(super) struct RealtimeResizeObserver {
    target: WindowDocumentTaskTarget,
    observer: Option<Rc<v8::Global<v8::Object>>>,
    requested: bool,
}

impl CallbackRegistry {
    pub(crate) fn register_realtime_resize_observer(
        &self,
        host: &JsContextHost,
        id: CallbackRegistrationId,
    ) {
        let mut state = self.state.borrow_mut();
        let Some(binding) = state
            .bindings
            .get(&id)
            .filter(|binding| binding.is_current_window(host))
        else {
            return;
        };
        let Some(identity) = binding.window_observer_identity() else {
            return;
        };
        let Some(target) =
            host.current_window_document_task_target_for_dispatch_scope(identity.dispatch_scope())
        else {
            return;
        };
        let entries = state.realtime_resize.get_or_insert_with(Default::default);
        entries
            .controllers
            .insert(identity.owner(), identity.dispatch_scope());
        entries.entries.insert(
            id,
            RealtimeResizeObserver {
                target,
                observer: None,
                requested: false,
            },
        );
    }

    pub(crate) fn ensure_realtime_resize_observer_controller(
        &self,
        host: &JsContextHost,
        target: WindowDocumentTaskTarget,
    ) {
        if !host.realtime_rendering_target_is_runnable(target) {
            return;
        }
        let Some(owner) = host.current_window_execution_context_owner(target.dispatch_scope())
        else {
            return;
        };
        self.state
            .borrow_mut()
            .realtime_resize
            .get_or_insert_with(Default::default)
            .controllers
            .insert(owner, target.dispatch_scope());
    }

    pub(crate) fn retire_realtime_resize_observer_controller(
        &self,
        owner: WindowExecutionContextOwner,
    ) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            entries.controllers.remove(&owner);
        }
    }

    pub(crate) fn activate_realtime_resize_observer<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        id: CallbackRegistrationId,
        observer: v8::Local<'s, v8::Object>,
    ) -> bool {
        let needs_root = {
            let state = self.state.borrow();
            let Some(entry) = state
                .realtime_resize
                .as_ref()
                .and_then(|entries| entries.entries.get(&id))
            else {
                return false;
            };
            if !state
                .bindings
                .get(&id)
                .is_some_and(|binding| binding.is_current_window(host))
                || !host.window_document_owner_is_current_for_dispatch_scope(
                    entry.target.owner(),
                    entry.target.dispatch_scope(),
                )
            {
                return false;
            }
            entry.observer.is_none()
        };
        // V8 can run weak finalizers while creating a handle. Never hold the
        // registry borrow across root construction or destruction.
        let root = needs_root.then(|| Rc::new(v8::Global::new(scope, observer)));
        let mut state = self.state.borrow_mut();
        let Some(entry) = state
            .realtime_resize
            .as_mut()
            .and_then(|entries| entries.entries.get_mut(&id))
        else {
            return false;
        };
        if let Some(root) = root {
            entry.observer = Some(root);
        }
        entry.requested = true;
        true
    }

    pub(crate) fn deactivate_realtime_resize_observer(&self, id: CallbackRegistrationId) {
        let root = {
            let mut state = self.state.borrow_mut();
            let Some(entry) = state
                .realtime_resize
                .as_mut()
                .and_then(|entries| entries.entries.get_mut(&id))
            else {
                return;
            };
            entry.requested = false;
            entry.observer.take()
        };
        drop(root);
    }

    pub(crate) fn request_realtime_resize_observer_checks(&self) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            for entry in entries.entries.values_mut() {
                entry.requested |= entry.observer.is_some();
            }
        }
    }

    pub(crate) fn request_realtime_resize_observer_document(
        &self,
        target: WindowDocumentTaskTarget,
    ) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            for entry in entries
                .entries
                .values_mut()
                .filter(|entry| entry.target == target)
            {
                entry.requested |= entry.observer.is_some();
            }
        }
    }

    pub(crate) fn has_pending_realtime_resize_observers(
        &self,
        host: &JsContextHost,
        target: Option<WindowDocumentTaskTarget>,
    ) -> bool {
        let state = self.state.borrow();
        state.realtime_resize.as_ref().is_some_and(|entries| {
            entries.entries.iter().any(|(id, entry)| {
                entry.requested
                    && entry.observer.is_some()
                    && target.is_none_or(|target| entry.target == target)
                    && host.realtime_rendering_target_is_runnable(entry.target)
                    && state
                        .bindings
                        .get(id)
                        .is_some_and(|binding| binding.is_current_window(host))
            })
        })
    }

    pub(crate) fn take_realtime_resize_observers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        target: WindowDocumentTaskTarget,
    ) -> (
        bool,
        Vec<(CallbackRegistrationId, v8::Local<'s, v8::Object>)>,
    ) {
        if !host.realtime_rendering_target_is_runnable(target) {
            return (false, Vec::new());
        }
        let (has_controller, observers) = {
            let mut state = self.state.borrow_mut();
            let CallbackRegistryState {
                bindings,
                realtime_resize,
                ..
            } = &mut *state;
            let Some(entries) = realtime_resize.as_mut() else {
                return (false, Vec::new());
            };
            let has_controller = host
                .current_window_execution_context_owner(target.dispatch_scope())
                .is_some_and(|owner| {
                    entries.controllers.get(&owner) == Some(&target.dispatch_scope())
                });
            let observers = entries
                .entries
                .iter_mut()
                .filter_map(|(id, entry)| {
                    if entry.target != target
                        || !bindings
                            .get(id)
                            .is_some_and(|binding| binding.is_current_window(host))
                    {
                        return None;
                    }
                    entry.requested = false;
                    entry
                        .observer
                        .as_ref()
                        .map(|observer| (*id, Rc::clone(observer)))
                })
                .collect::<Vec<_>>();
            (has_controller, observers)
        };
        (
            has_controller,
            observers
                .into_iter()
                .map(|(id, observer)| (id, v8::Local::new(scope, observer.as_ref())))
                .collect(),
        )
    }

    pub(crate) fn rebind_realtime_resize_observer_document(
        &self,
        retired: WindowDocumentOwner,
        current: WindowDocumentOwner,
    ) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            for entry in entries
                .entries
                .values_mut()
                .filter(|entry| entry.target.owner() == retired)
            {
                entry.target =
                    WindowDocumentTaskTarget::new(current, entry.target.dispatch_scope());
                entry.requested |= entry.observer.is_some();
            }
        }
    }

    pub(crate) fn retire_realtime_resize_observer_document(&self, retired: WindowDocumentOwner) {
        self.remove_realtime_resize_observers(|_, entry, _| entry.target.owner() == retired);
    }

    pub(crate) fn prune_realtime_resize_observers(&self, host: &JsContextHost) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            entries.controllers.retain(|owner, dispatch_scope| {
                host.window_execution_context_owner_is_current(*owner, *dispatch_scope)
            });
        }
        self.remove_realtime_resize_observers(|id, entry, bindings| {
            !host.window_document_owner_is_current_for_dispatch_scope(
                entry.target.owner(),
                entry.target.dispatch_scope(),
            ) || !bindings
                .get(&id)
                .is_some_and(|binding| binding.is_current_window(host))
        });
    }

    fn remove_realtime_resize_observers(
        &self,
        mut remove: impl FnMut(
            CallbackRegistrationId,
            &RealtimeResizeObserver,
            &HashMap<CallbackRegistrationId, CallbackBinding>,
        ) -> bool,
    ) {
        let retired = {
            let mut state = self.state.borrow_mut();
            let CallbackRegistryState {
                bindings,
                realtime_resize,
                ..
            } = &mut *state;
            let Some(entries) = realtime_resize.as_mut() else {
                return;
            };
            let ids = entries
                .entries
                .iter()
                .filter_map(|(id, entry)| remove(*id, entry, bindings).then_some(*id))
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| entries.entries.remove(&id))
                .collect::<Vec<_>>()
        };
        drop(retired);
    }
}
