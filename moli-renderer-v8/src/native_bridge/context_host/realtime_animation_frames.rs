use std::collections::{BTreeMap, HashMap};

use moli_webidl_callback::WebIdlCallbackFunction;

use super::{
    JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken, WindowDocumentOwner,
    WindowDocumentTaskTarget, WindowExecutionContextBinding, WindowExecutionContextOwner,
};
use crate::window_webidl_callback::WindowWebIdlCallbackFunction;

/// A Window's associated Document owns the callback map and counter.
/// document.open() preserves that Web Document but replaces Moli's task owner;
/// only its explicit transaction may transfer this map to the successor owner.
struct WindowAnimationFrames {
    target: WindowDocumentTaskTarget,
    next_handle: u32,
    callbacks: BTreeMap<u32, WindowWebIdlCallbackFunction>,
}

#[derive(Default)]
pub(super) struct RealtimeAnimationFrames {
    windows: HashMap<WindowExecutionContextOwner, WindowAnimationFrames>,
}

impl RealtimeAnimationFrames {
    pub(super) fn is_empty(&self) -> bool {
        self.windows
            .values()
            .all(|window| window.callbacks.is_empty())
    }
}

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn realtime_animation_frame_count_for_test(&self) -> usize {
        self.realtime_layout_state
            .as_ref()
            .map(|state| {
                state
                    .animation_frames
                    .borrow()
                    .windows
                    .values()
                    .map(|window| window.callbacks.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    pub(in crate::native_bridge::context_host) fn retire_realtime_animation_frame_document(
        &self,
        retired: WindowDocumentOwner,
    ) {
        if let Some(state) = &self.realtime_layout_state {
            state
                .animation_frames
                .borrow_mut()
                .windows
                .retain(|_, window| window.target.owner() != retired);
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(super) fn rebind_realtime_animation_frame_document(
        &self,
        retired: WindowDocumentOwner,
        current: WindowDocumentOwner,
    ) {
        if let Some(state) = &self.realtime_layout_state {
            for window in state.animation_frames.borrow_mut().windows.values_mut() {
                if window.target.owner() == retired {
                    window.target =
                        WindowDocumentTaskTarget::new(current, window.target.dispatch_scope());
                }
            }
        }
    }

    pub(crate) fn register_realtime_animation_frame(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        binding: WindowExecutionContextBinding,
        callback: WebIdlCallbackFunction,
    ) -> Result<u32, &'static str> {
        if !binding.is_current(self) {
            return Ok(0);
        }
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(binding.dispatch_scope())
        else {
            return Ok(0);
        };
        let callback = WindowWebIdlCallbackFunction::new(scope, self, callback);
        if callback.relevant_identity().is_none() {
            return Ok(0);
        }
        let state = self
            .realtime_layout_state
            .as_ref()
            .expect("Realtime rAF registry");
        let handle =
            {
                let mut registry = state.animation_frames.borrow_mut();
                let window = registry.windows.entry(binding.owner()).or_insert_with(|| {
                    WindowAnimationFrames {
                        target,
                        next_handle: 0,
                        callbacks: BTreeMap::new(),
                    }
                });
                if window.target != target {
                    window.callbacks.clear();
                    window.target = target;
                    window.next_handle = 0;
                }
                let handle = window
                    .next_handle
                    .checked_add(1)
                    .ok_or("requestAnimationFrame handle space exhausted")?;
                window.next_handle = handle;
                window.callbacks.insert(handle, callback);
                handle
            };
        self.refresh_realtime_rendering_opportunity();
        Ok(handle)
    }

    pub(crate) fn cancel_realtime_animation_frame(
        &self,
        binding: WindowExecutionContextBinding,
        handle: u32,
    ) {
        if !binding.is_current(self) {
            return;
        }
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(binding.dispatch_scope())
        else {
            return;
        };
        if let Some(state) = &self.realtime_layout_state
            && let Some(window) = state
                .animation_frames
                .borrow_mut()
                .windows
                .get_mut(&binding.owner())
            && window.target == target
        {
            window.callbacks.remove(&handle);
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(super) fn has_runnable_realtime_animation_frames(&self) -> bool {
        self.realtime_layout_state.as_ref().is_some_and(|state| {
            state
                .animation_frames
                .borrow()
                .windows
                .values()
                .any(|window| {
                    !window.callbacks.is_empty()
                        && self.realtime_animation_frame_target_is_runnable(window.target)
                })
        })
    }

    pub(crate) fn realtime_animation_frame_target_is_runnable(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        self.document_activity().visible
            && self.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            )
            && self.owner_scripting_enabled(target.dispatch_scope())
    }

    /// Freeze participating Documents, not their callback lists. A parent may
    /// register a callback on a child whose rAF stage has not started yet.
    pub(crate) fn realtime_animation_frame_documents(&self) -> Vec<WindowDocumentTaskTarget> {
        let mut scopes = vec![OwnerDispatchScope::Top];
        scopes.extend(
            self.child_browsing_context_handles_in_document_order()
                .into_iter()
                .map(OwnerDispatchScope::Child),
        );
        for popup in self.open_lightweight_popup_ids() {
            scopes.push(OwnerDispatchScope::LightweightPopup(popup));
            if let Some(document) = self.lightweight_popup_document_handle(popup) {
                let mut children = Vec::new();
                self.collect_child_browsing_context_handles_in_document_order_from_document(
                    document,
                    &mut children,
                );
                scopes.extend(children.into_iter().map(OwnerDispatchScope::Child));
            }
        }
        scopes
            .into_iter()
            .filter_map(|scope| self.current_window_document_task_target_for_dispatch_scope(scope))
            .collect()
    }

    pub(crate) fn realtime_animation_frame_handles(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> Vec<u32> {
        if !self.realtime_animation_frame_target_is_runnable(target) {
            return Vec::new();
        }
        let Some(state) = &self.realtime_layout_state else {
            return Vec::new();
        };
        let Some(owner) = self.current_window_execution_context_owner(target.dispatch_scope())
        else {
            return Vec::new();
        };
        state
            .animation_frames
            .borrow()
            .windows
            .get(&owner)
            .filter(|window| window.target == target)
            .map(|window| window.callbacks.keys().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn take_realtime_animation_frame(
        &self,
        target: WindowDocumentTaskTarget,
        handle: u32,
    ) -> Option<WindowWebIdlCallbackFunction> {
        if !self.realtime_animation_frame_target_is_runnable(target) {
            return None;
        }
        let state = self.realtime_layout_state.as_ref()?;
        let owner = self.current_window_execution_context_owner(target.dispatch_scope())?;
        let mut registry = state.animation_frames.borrow_mut();
        let window = registry.windows.get_mut(&owner)?;
        (window.target == target)
            .then(|| window.callbacks.remove(&handle))
            .flatten()
    }

    pub(super) fn retire_realtime_animation_frame_owner(&self, owner: WindowExecutionContextOwner) {
        if let Some(state) = &self.realtime_layout_state {
            let mut registry = state.animation_frames.borrow_mut();
            registry.windows.remove(&owner);
            for window in registry.windows.values_mut() {
                window
                    .callbacks
                    .retain(|_, callback| !callback.is_owned_by(owner));
            }
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(super) fn retire_realtime_animation_frame_realm(
        &self,
        token: RuntimeObservableContextToken,
    ) {
        if let Some(state) = &self.realtime_layout_state {
            for window in state.animation_frames.borrow_mut().windows.values_mut() {
                window
                    .callbacks
                    .retain(|_, callback| !callback.belongs_to_context_token(token));
            }
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(super) fn prune_realtime_animation_frames(&self) {
        if let Some(state) = &self.realtime_layout_state {
            state
                .animation_frames
                .borrow_mut()
                .windows
                .retain(|owner, window| {
                    if !self.window_execution_context_owner_is_current(
                        *owner,
                        window.target.dispatch_scope(),
                    ) {
                        return false;
                    }
                    if !self.window_document_owner_is_current_for_dispatch_scope(
                        window.target.owner(),
                        window.target.dispatch_scope(),
                    ) {
                        return false;
                    } else {
                        window.callbacks.retain(|_, callback| {
                            callback.relevant_identity().is_some_and(|identity| {
                                self.window_execution_context_identity_is_current(identity)
                            })
                        });
                    }
                    true
                });
        }
    }
}
