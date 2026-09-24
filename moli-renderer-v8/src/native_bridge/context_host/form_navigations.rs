use std::collections::HashMap;

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::window_security_tokens::WindowAccessOrigin;
use super::{JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget};
use crate::document_runtime::DomHandle;
use crate::native_bridge::element::{PlannedFormNavigation, apply_planned_form_navigation};
use crate::page_task_queue::{
    RendererPageDomManipulationCancellation, RendererPageFormNavigationTaskId,
    RendererPageFormNavigationTaskKind,
};

#[derive(Debug)]
struct PendingFormNavigation {
    navigation: PlannedFormNavigation,
    cancellation: RendererPageDomManipulationCancellation,
    source_origin: Option<WindowAccessOrigin>,
}

#[derive(Default)]
pub(super) struct FormNavigationState {
    tasks: ExactWindowDocumentTaskLedger<
        RendererPageFormNavigationTaskId,
        RendererPageFormNavigationTaskKind,
        PendingFormNavigation,
    >,
    by_form: HashMap<DomHandle, Vec<RendererPageFormNavigationTaskId>>,
}

impl FormNavigationState {
    fn forget(&mut self, task_id: RendererPageFormNavigationTaskId) {
        self.by_form.retain(|_, tasks| {
            tasks.retain(|id| *id != task_id);
            !tasks.is_empty()
        });
    }
}

impl JsContextHost {
    pub(in crate::native_bridge) fn queue_form_navigation_task(
        &mut self,
        navigation: PlannedFormNavigation,
    ) -> bool {
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(navigation.destination)
        else {
            return false;
        };
        let form = navigation.form;
        // Keep independent script submissions to different named navigables.
        // A submit-button default action replaces earlier work from its form.
        let previous = self
            .form_navigations
            .by_form
            .get(&form)
            .cloned()
            .unwrap_or_default();
        for task_id in previous {
            let replaces = self
                .form_navigations
                .tasks
                .pending(task_id)
                .is_some_and(|pending| {
                    navigation.submitter.is_some()
                        || pending.payload().navigation.destination == navigation.destination
                });
            if replaces {
                self.discard_pending_form_navigation_task(task_id);
            }
        }
        self.cancel_planned_form_navigation_to(navigation.destination);
        if navigation.destination == OwnerDispatchScope::Top && !navigation.is_javascript_url() {
            self.clear_pending_location_navigation();
        }
        if let OwnerDispatchScope::Child(handle) = navigation.destination {
            if navigation.submitter.is_some() {
                self.cancel_pending_form_submission_child_navigations_for_form(form);
            } else {
                self.cancel_previous_pending_form_submission_child_navigation(form, handle);
            }
            // Admission replaces the target's pending navigation immediately;
            // the navigate event and request start still belong to the task.
            // In particular, javascript: string completion must not win over
            // a form submission made while evaluating that script.
            if !navigation.is_javascript_url() {
                self.cancel_pending_child_browsing_context_navigation(handle);
            }
        }
        let kind = RendererPageFormNavigationTaskKind::Navigate;
        let source_origin = navigation
            .source_document
            .and_then(|document| self.owner_dispatch_scope_for_node(document))
            .and_then(|source| self.window_access_origin_for_dispatch_scope(source));
        let task_id = self
            .form_navigations
            .tasks
            .allocate_task_id(RendererPageFormNavigationTaskId::from_raw);
        let cancellation = RendererPageDomManipulationCancellation::new();
        self.form_navigations
            .tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                kind,
                PendingFormNavigation {
                    navigation,
                    cancellation: cancellation.clone(),
                    source_origin,
                },
            ));
        self.form_navigations
            .by_form
            .entry(form)
            .or_default()
            .push(task_id);
        if self
            .page_form_navigation_sender()
            .send(target, task_id, kind, cancellation)
            .is_ok()
        {
            return true;
        }
        self.discard_pending_form_navigation_task(task_id);
        false
    }

    /// A subsequent navigation replaces a submission that has not begun yet.
    pub(crate) fn cancel_planned_form_navigation_to(&mut self, destination: OwnerDispatchScope) {
        let tasks: Vec<_> = self
            .form_navigations
            .by_form
            .values()
            .flatten()
            .copied()
            .filter(|task_id| {
                self.form_navigations
                    .tasks
                    .pending(*task_id)
                    .is_some_and(|pending| pending.payload().navigation.destination == destination)
            })
            .collect();
        for task_id in tasks {
            self.discard_pending_form_navigation_task(task_id);
        }
    }

    pub(crate) fn has_planned_form_navigation_to(&self, destination: OwnerDispatchScope) -> bool {
        self.form_navigations
            .by_form
            .values()
            .flatten()
            .any(|task_id| {
                self.form_navigations
                    .tasks
                    .pending(*task_id)
                    .is_some_and(|pending| pending.payload().navigation.destination == destination)
            })
    }

    pub(crate) fn current_pending_form_navigation_task(
        &self,
        task_id: RendererPageFormNavigationTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageFormNavigationTaskKind)> {
        let pending = self.form_navigations.tasks.pending(task_id)?;
        let current = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current, pending.kind()))
    }

    pub(crate) fn apply_authorized_form_navigation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task_id: RendererPageFormNavigationTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageFormNavigationTaskKind,
    ) -> Option<bool> {
        let pending = self
            .form_navigations
            .tasks
            .remove_exact(task_id, target, kind)?
            .into_payload();
        let navigation = pending.navigation;
        // Clear the planned task before firing any event: listeners may submit again.
        self.form_navigations.forget(task_id);
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return Some(false);
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch = target.dispatch_scope();
        // A cross-origin initiator must not expose its form or entry list to
        // the target's navigate listener. Keep the source origin if its old
        // Document has retired while a different target remains live.
        let source_origin = navigation
            .source_document
            .and_then(|document| self.owner_dispatch_scope_for_node(document))
            .and_then(|source| self.window_access_origin_for_dispatch_scope(source))
            .or(pending.source_origin);
        let fire_navigate_event = source_origin
            .zip(self.window_access_origin_for_dispatch_scope(dispatch))
            .is_some_and(|(source, target)| source.can_access(&target));
        let previous = dispatch.enter(scope);
        // The selected target owns navigation work even if the source form
        // was adopted or its Document was replaced after submission.
        let applied =
            apply_planned_form_navigation(scope, host_ptr, navigation, fire_navigate_event);
        dispatch.restore(scope, previous);
        Some(applied)
    }

    pub(crate) fn discard_pending_form_navigation_task(
        &mut self,
        task_id: RendererPageFormNavigationTaskId,
    ) -> bool {
        self.form_navigations.forget(task_id);
        self.form_navigations
            .tasks
            .remove(task_id)
            .is_some_and(|pending| {
                pending.into_payload().cancellation.cancel();
                true
            })
    }
}
