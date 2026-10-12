use std::collections::BTreeMap;

use super::*;
use crate::native_bridge::{WindowDocumentOwner, WindowDocumentTaskTarget};

pub(super) type RealtimeResizeObservers = BTreeMap<ObserverCallbackId, RealtimeResizeObserver>;

/// Constructor residence survives disconnect while JS retains the observer.
/// Only active observations root the wrapper. Targets, active records and
/// last-reported sizes remain on that wrapper, shared with the existing API.
pub(super) struct RealtimeResizeObserver {
    target: WindowDocumentTaskTarget,
    observer: Option<Rc<v8::Global<v8::Object>>>,
    requested: bool,
}

impl ObserverCallbackRegistry {
    pub(in crate::observer_runtime) fn register_realtime_resize_observer(
        &self,
        host: &JsContextHost,
        id: ObserverCallbackId,
    ) {
        let mut state = self.state.borrow_mut();
        let Some(binding) = state
            .bindings
            .get(&id)
            .filter(|binding| binding.is_current(host))
        else {
            return;
        };
        let Some(target) = binding.observer_identity.and_then(|identity| {
            host.current_window_document_task_target_for_dispatch_scope(identity.dispatch_scope())
        }) else {
            return;
        };
        state
            .realtime_resize
            .get_or_insert_with(Default::default)
            .insert(
                id,
                RealtimeResizeObserver {
                    target,
                    observer: None,
                    requested: false,
                },
            );
    }

    pub(in crate::observer_runtime) fn activate_realtime_resize_observer<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        id: ObserverCallbackId,
        observer: v8::Local<'s, v8::Object>,
    ) -> bool {
        let needs_root = {
            let state = self.state.borrow();
            let Some(entry) = state
                .realtime_resize
                .as_ref()
                .and_then(|entries| entries.get(&id))
            else {
                return false;
            };
            if !state
                .bindings
                .get(&id)
                .is_some_and(|binding| binding.is_current(host))
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
            .and_then(|entries| entries.get_mut(&id))
        else {
            return false;
        };
        if let Some(root) = root {
            entry.observer = Some(root);
        }
        entry.requested = true;
        true
    }

    pub(in crate::observer_runtime) fn deactivate_realtime_resize_observer(
        &self,
        id: ObserverCallbackId,
    ) {
        let root = {
            let mut state = self.state.borrow_mut();
            let Some(entry) = state
                .realtime_resize
                .as_mut()
                .and_then(|entries| entries.get_mut(&id))
            else {
                return;
            };
            entry.requested = false;
            entry.observer.take()
        };
        drop(root);
    }

    pub(in crate::observer_runtime) fn request_realtime_resize_observer_checks(&self) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            for entry in entries.values_mut() {
                entry.requested |= entry.observer.is_some();
            }
        }
    }

    pub(in crate::observer_runtime) fn request_realtime_resize_observer_document(
        &self,
        target: WindowDocumentTaskTarget,
    ) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            for entry in entries.values_mut().filter(|entry| entry.target == target) {
                entry.requested |= entry.observer.is_some();
            }
        }
    }

    pub(in crate::observer_runtime) fn has_pending_realtime_resize_observers(
        &self,
        host: &JsContextHost,
        target: Option<WindowDocumentTaskTarget>,
    ) -> bool {
        let state = self.state.borrow();
        state.realtime_resize.as_ref().is_some_and(|entries| {
            entries.iter().any(|(id, entry)| {
                entry.requested
                    && entry.observer.is_some()
                    && target.is_none_or(|target| entry.target == target)
                    && host.realtime_rendering_target_is_runnable(entry.target)
                    && state
                        .bindings
                        .get(id)
                        .is_some_and(|binding| binding.is_current(host))
            })
        })
    }

    pub(in crate::observer_runtime) fn take_realtime_resize_observers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        target: WindowDocumentTaskTarget,
    ) -> Vec<(ObserverCallbackId, v8::Local<'s, v8::Object>)> {
        if !host.realtime_rendering_target_is_runnable(target) {
            return Vec::new();
        }
        let observers = {
            let mut state = self.state.borrow_mut();
            let ObserverCallbackRegistryState {
                bindings,
                realtime_resize,
                ..
            } = &mut *state;
            let Some(entries) = realtime_resize.as_mut() else {
                return Vec::new();
            };
            entries
                .iter_mut()
                .filter_map(|(id, entry)| {
                    if entry.target != target
                        || !bindings
                            .get(id)
                            .is_some_and(|binding| binding.is_current(host))
                    {
                        return None;
                    }
                    entry.requested = false;
                    entry
                        .observer
                        .as_ref()
                        .map(|observer| (*id, Rc::clone(observer)))
                })
                .collect::<Vec<_>>()
        };
        observers
            .into_iter()
            .map(|(id, observer)| (id, v8::Local::new(scope, observer.as_ref())))
            .collect()
    }

    pub(in crate::observer_runtime) fn rebind_realtime_resize_observer_document(
        &self,
        retired: WindowDocumentOwner,
        current: WindowDocumentOwner,
    ) {
        if let Some(entries) = self.state.borrow_mut().realtime_resize.as_mut() {
            for entry in entries
                .values_mut()
                .filter(|entry| entry.target.owner() == retired)
            {
                entry.target =
                    WindowDocumentTaskTarget::new(current, entry.target.dispatch_scope());
                entry.requested |= entry.observer.is_some();
            }
        }
    }

    pub(in crate::observer_runtime) fn retire_realtime_resize_observer_document(
        &self,
        retired: WindowDocumentOwner,
    ) {
        self.remove_realtime_resize_observers(|_, entry, _| entry.target.owner() == retired);
    }

    pub(in crate::observer_runtime) fn prune_realtime_resize_observers(
        &self,
        host: &JsContextHost,
    ) {
        self.remove_realtime_resize_observers(|id, entry, bindings| {
            !host.window_document_owner_is_current_for_dispatch_scope(
                entry.target.owner(),
                entry.target.dispatch_scope(),
            ) || !bindings
                .get(&id)
                .is_some_and(|binding| binding.is_current(host))
        });
    }

    fn remove_realtime_resize_observers(
        &self,
        mut remove: impl FnMut(
            ObserverCallbackId,
            &RealtimeResizeObserver,
            &HashMap<ObserverCallbackId, ObserverCallbackBinding>,
        ) -> bool,
    ) {
        let retired = {
            let mut state = self.state.borrow_mut();
            let ObserverCallbackRegistryState {
                bindings,
                realtime_resize,
                ..
            } = &mut *state;
            let Some(entries) = realtime_resize.as_mut() else {
                return;
            };
            let ids = entries
                .iter()
                .filter_map(|(id, entry)| remove(*id, entry, bindings).then_some(*id))
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| entries.remove(&id))
                .collect::<Vec<_>>()
        };
        drop(retired);
    }
}
