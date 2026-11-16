use std::collections::{HashMap, HashSet};

use crate::{
    context_bootstrap::construct_original_event,
    document_runtime::DomHandle,
    frame_owner_model::DocumentLoadDelayTokenId,
    page_task_queue::{
        PageObjectResourceSelectionTargetEffect, RendererPageDomManipulationCancellation,
        RendererPageObjectResourceSelectionTaskId,
    },
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{OwnerDispatchScope, PageScriptHost, WindowDocumentOwner, WindowDocumentTaskTarget};

#[derive(Clone, Debug, Eq, PartialEq)]
struct InvalidObjectResource {
    target: WindowDocumentTaskTarget,
    data: String,
    type_hint: Option<String>,
}

#[derive(Debug)]
struct PendingSelection {
    element: DomHandle,
    request: InvalidObjectResource,
    cancellation: RendererPageDomManipulationCancellation,
}

#[derive(Clone, Copy, Debug)]
struct SelectionLoadDelay {
    target: WindowDocumentTaskTarget,
    token: Option<DocumentLoadDelayTokenId>,
}

#[derive(Default)]
pub(super) struct ObjectResourceSelectionState {
    tasks: ExactWindowDocumentTaskLedger<
        RendererPageObjectResourceSelectionTaskId,
        (),
        PendingSelection,
    >,
    attempts: HashMap<
        DomHandle,
        (
            RendererPageObjectResourceSelectionTaskId,
            InvalidObjectResource,
        ),
    >,
    // Keep the delay through event dispatch, after the payload is claimed, so
    // reentrant mutations cannot make the containing Document finish early.
    delays: HashMap<RendererPageObjectResourceSelectionTaskId, SelectionLoadDelay>,
}

impl Drop for ObjectResourceSelectionState {
    fn drop(&mut self) {
        self.tasks.retain(|pending| {
            pending.payload().cancellation.cancel();
            false
        });
    }
}

impl PageScriptHost {
    fn invalid_object_data(&self, element: DomHandle) -> Option<String> {
        if !self.child_browsing_context_host_is_object_element(element) {
            return None;
        }
        let data = self.dom_host().get_attribute(element, "data")?;
        (!data.trim().is_empty()
            && self
                .parse_child_frame_owner_attribute_url(element, data.trim())
                .is_none())
        .then_some(data)
    }

    fn invalid_object_resource(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
    ) -> Option<InvalidObjectResource> {
        let data = self.invalid_object_data(element)?;
        if !self.child_browsing_context_host_is_active(element)
            || self.embedded_content_is_suppressed(element)
            || !crate::native_bridge::element::element_is_being_rendered(self, element)
        {
            return None;
        }
        let type_hint = self.dom_host().get_attribute(element, "type");
        if let Some(mime) = type_hint
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| moli_web_mime::mime_essence(value.trim()))
            && !moli_web_mime::is_image_mime(&mime)
            && !moli_web_mime::is_supported_document_mime_type(&mime)
        {
            return None;
        }
        let target = self.window_document_task_target_for_node(scope, element)?;
        if self
            .document_resource_loader_for_dispatch_scope(target.dispatch_scope())
            .is_some_and(|loader| !loader.request_client().subframe_loading_enabled())
        {
            return None;
        }
        Some(InvalidObjectResource {
            target,
            data,
            type_hint,
        })
    }

    /// A failed URL parse selects fallback, never an about:blank navigable.
    /// The error belongs to the object selection's DOM-manipulation task.
    pub(super) fn refresh_invalid_object_resource_selection(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
    ) -> bool {
        if self.invalid_object_data(element).is_none() {
            self.cancel_object_resource_selection(element);
            return false;
        }
        if self.child_browsing_contexts.contains_key(&element) {
            Self::drop_child_browsing_context_subtree_with_window_realm(scope, self, element);
        }
        // Unload handlers can replace the resource while the old child retires.
        if self.invalid_object_data(element).is_none() {
            self.cancel_object_resource_selection(element);
            return false;
        }
        let Some(request) = self.invalid_object_resource(scope, element) else {
            self.cancel_object_resource_selection(element);
            return true;
        };
        if self
            .object_resource_selections
            .attempts
            .get(&element)
            .is_some_and(|(_, previous)| *previous == request)
        {
            return true;
        }
        self.cancel_object_resource_selection(element);
        let task_id = self
            .object_resource_selections
            .tasks
            .allocate_task_id(RendererPageObjectResourceSelectionTaskId::from_raw);
        let cancellation = RendererPageDomManipulationCancellation::new();
        let target = request.target;
        let token = target.owner().frame_document_owner().and_then(|owner| {
            self.frame_owner_store
                .acquire_object_resource_selection_load_delay(owner)
        });
        self.object_resource_selections
            .delays
            .insert(task_id, SelectionLoadDelay { target, token });
        self.object_resource_selections
            .attempts
            .insert(element, (task_id, request.clone()));
        self.object_resource_selections
            .tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                (),
                PendingSelection {
                    element,
                    request,
                    cancellation: cancellation.clone(),
                },
            ));
        if self
            .page_object_resource_selection_sender()
            .send(target, task_id, cancellation)
            .is_err()
        {
            self.cancel_object_resource_selection(element);
        }
        true
    }

    pub(super) fn cancel_object_resource_selection(&mut self, element: DomHandle) {
        let Some((task_id, _)) = self.object_resource_selections.attempts.remove(&element) else {
            return;
        };
        if let Some(pending) = self.object_resource_selections.tasks.remove(task_id) {
            pending.payload().cancellation.cancel();
            self.settle_object_resource_selection_delay(task_id, true);
        }
    }

    pub(super) fn retain_live_object_resource_selections(&mut self, live: &HashSet<DomHandle>) {
        let stale = self
            .object_resource_selections
            .attempts
            .keys()
            .copied()
            .filter(|element| !live.contains(element))
            .collect::<Vec<_>>();
        for element in stale {
            self.cancel_object_resource_selection(element);
        }
    }

    pub(super) fn clear_object_resource_selections(&mut self) {
        let tasks = self
            .object_resource_selections
            .delays
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for task_id in tasks {
            if let Some(pending) = self.object_resource_selections.tasks.remove(task_id) {
                pending.payload().cancellation.cancel();
            }
            self.settle_object_resource_selection_delay(task_id, false);
        }
        self.object_resource_selections.attempts.clear();
    }

    pub(crate) fn current_pending_object_resource_selection_target(
        &self,
        task_id: RendererPageObjectResourceSelectionTaskId,
    ) -> Option<WindowDocumentTaskTarget> {
        let pending = self.object_resource_selections.tasks.pending(task_id)?;
        self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )
    }

    pub(crate) fn discard_pending_object_resource_selection_task(
        &mut self,
        task_id: RendererPageObjectResourceSelectionTaskId,
    ) -> bool {
        let Some(pending) = self.object_resource_selections.tasks.remove(task_id) else {
            return false;
        };
        let pending = pending.into_payload();
        pending.cancellation.cancel();
        if self
            .object_resource_selections
            .attempts
            .get(&pending.element)
            .is_some_and(|(current, _)| *current == task_id)
        {
            self.object_resource_selections
                .attempts
                .remove(&pending.element);
        }
        self.settle_object_resource_selection_delay(task_id, true);
        true
    }

    pub(crate) fn apply_authorized_object_resource_selection(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        task_id: RendererPageObjectResourceSelectionTaskId,
        target: WindowDocumentTaskTarget,
    ) -> Option<PageObjectResourceSelectionTargetEffect> {
        let pending = self
            .object_resource_selections
            .tasks
            .remove_exact(task_id, target, ())?
            .into_payload();
        let effect = self.dispatch_invalid_object_resource(scope, host_ptr, task_id, &pending);
        self.settle_object_resource_selection_delay(task_id, true);
        Some(effect)
    }

    fn dispatch_invalid_object_resource(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        task_id: RendererPageObjectResourceSelectionTaskId,
        pending: &PendingSelection,
    ) -> PageObjectResourceSelectionTargetEffect {
        let absent = PageObjectResourceSelectionTargetEffect::CurrentOwnerHadNoEventTarget;
        if self
            .invalid_object_resource(scope, pending.element)
            .as_ref()
            != Some(&pending.request)
            || !self
                .object_resource_selections
                .attempts
                .get(&pending.element)
                .is_some_and(|(current, _)| *current == task_id)
        {
            return absent;
        }
        let target = pending.request.target;
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return absent;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let previous = target.dispatch_scope().enter(scope);
        let dispatched = construct_original_event(scope, "error").is_some_and(|event| {
            let _ = super::super::element::dispatch_public_event(
                scope,
                host_ptr,
                pending.element,
                event,
            );
            true
        });
        target.dispatch_scope().restore(scope, previous);
        if dispatched {
            PageObjectResourceSelectionTargetEffect::DispatchedToCurrentOwner
        } else {
            absent
        }
    }

    fn settle_object_resource_selection_delay(
        &mut self,
        task_id: RendererPageObjectResourceSelectionTaskId,
        wake: bool,
    ) {
        let Some(delay) = self.object_resource_selections.delays.remove(&task_id) else {
            return;
        };
        if let Some(owner) = delay.target.owner().frame_document_owner()
            && let Some(token) = delay.token
        {
            let settlement = self
                .frame_owner_store
                .settle_object_resource_selection_load_delay(owner, token);
            if wake
                && settlement.released()
                && let OwnerDispatchScope::Child(parent) = delay.target.dispatch_scope()
            {
                let _ =
                    self.queue_child_document_complete_lifecycle_if_ready_for_owner(parent, owner);
            }
        }
        if wake && let WindowDocumentOwner::LightweightPopup(owner) = delay.target.owner() {
            self.publish_lightweight_popup_load_event_if_ready(owner.popup_id());
        }
    }

    pub(super) fn object_resource_selection_delays_popup_load(&self, popup_id: u64) -> bool {
        self.object_resource_selections
            .delays
            .values()
            .any(|delay| {
                matches!(delay.target.owner(), WindowDocumentOwner::LightweightPopup(owner)
                if owner.popup_id() == popup_id)
            })
    }

    pub(super) fn has_pending_popup_object_resource_selections(&self) -> bool {
        self.object_resource_selections
            .delays
            .values()
            .any(|delay| {
                matches!(
                    delay.target.owner(),
                    WindowDocumentOwner::LightweightPopup(_)
                )
            })
    }
}
