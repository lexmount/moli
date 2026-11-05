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
    // Native node ids are local to a Document, including in another Page.
    by_form: HashMap<
        (Option<crate::RendererDocumentLifecycleIdentity>, DomHandle),
        Vec<RendererPageFormNavigationTaskId>,
    >,
    related_by_form: HashMap<DomHandle, Vec<RelatedFormNavigation>>,
}

struct RelatedFormNavigation {
    target: crate::script_vm::WeakRendererPageScriptEnvironment,
    target_page_id: u64,
    task_id: RendererPageFormNavigationTaskId,
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
        if navigation.submitter.is_some() {
            self.cancel_related_form_navigations(navigation.form, None, true);
        }
        self.queue_captured_form_navigation_task(navigation, None)
            .is_some()
    }

    pub(in crate::native_bridge) fn queue_related_form_navigation_task(
        &mut self,
        target: crate::script_vm::RendererPageScriptEnvironment,
        mut navigation: PlannedFormNavigation,
    ) -> bool {
        let Some(source) = self.page_script_environment() else {
            return false;
        };
        let Some(target_host) = target.document_host() else {
            return false;
        };
        let source_origin = navigation
            .source_document
            .and_then(|document| self.owner_dispatch_scope_for_node(document))
            .and_then(|source| self.window_access_origin_for_dispatch_scope(source));
        let form = navigation.form;
        let replaces_all = navigation.submitter.is_some();
        if replaces_all {
            let previous = self
                .form_navigations
                .by_form
                .get(&(navigation.source_document_identity, form))
                .cloned()
                .unwrap_or_default();
            for task in previous {
                self.discard_pending_form_navigation_task(task);
            }
        }
        self.cancel_related_form_navigations(form, Some(target.page_id()), replaces_all);
        navigation.related_source = Some(source.downgrade());
        let target_runtime = unsafe { &mut *target_host.as_ptr() };
        navigation.prepare_for_related_target(target_runtime);
        let Some(task_id) =
            target_runtime.queue_captured_form_navigation_task(navigation, source_origin)
        else {
            return false;
        };
        self.form_navigations
            .related_by_form
            .entry(form)
            .or_default()
            .push(RelatedFormNavigation {
                target: target.downgrade(),
                target_page_id: target.page_id(),
                task_id,
            });
        true
    }

    fn cancel_related_form_navigations(
        &mut self,
        form: DomHandle,
        target_page_id: Option<u64>,
        all: bool,
    ) {
        let pending = self
            .form_navigations
            .related_by_form
            .remove(&form)
            .unwrap_or_default();
        let mut retained = Vec::new();
        for navigation in pending {
            if all || Some(navigation.target_page_id) == target_page_id {
                if let Some(host) = navigation
                    .target
                    .upgrade()
                    .and_then(|target| target.document_host())
                {
                    unsafe { &mut *host.as_ptr() }
                        .discard_pending_form_navigation_task(navigation.task_id);
                }
            } else {
                retained.push(navigation);
            }
        }
        if !retained.is_empty() {
            self.form_navigations.related_by_form.insert(form, retained);
        }
    }

    fn forget_related_form_navigation_source(
        &self,
        navigation: &PlannedFormNavigation,
        task_id: RendererPageFormNavigationTaskId,
    ) -> Option<std::rc::Rc<std::cell::RefCell<JsContextHost>>> {
        let host = navigation
            .related_source
            .as_ref()?
            .upgrade()?
            .document_host()?;
        let source = unsafe { &mut *host.as_ptr() };
        if source.root_document_lifecycle_identity() != navigation.source_document_identity {
            return None;
        }
        let page_id = self.page_script_environment()?.page_id();
        source.form_navigations.related_by_form.retain(|_, tasks| {
            tasks.retain(|task| task.target_page_id != page_id || task.task_id != task_id);
            !tasks.is_empty()
        });
        Some(host)
    }

    fn queue_captured_form_navigation_task(
        &mut self,
        navigation: PlannedFormNavigation,
        source_origin: Option<WindowAccessOrigin>,
    ) -> Option<RendererPageFormNavigationTaskId> {
        let target =
            self.current_window_document_task_target_for_dispatch_scope(navigation.destination)?;
        let form = navigation.form;
        let form_key = (navigation.source_document_identity, form);
        // Keep independent script submissions to different named navigables.
        // A submit-button default action replaces earlier work from its form.
        let previous = self
            .form_navigations
            .by_form
            .get(&form_key)
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
            .and_then(|source| self.window_access_origin_for_dispatch_scope(source))
            .or(source_origin);
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
            .entry(form_key)
            .or_default()
            .push(task_id);
        if self
            .page_form_navigation_sender()
            .send(target, task_id, kind, cancellation)
            .is_ok()
        {
            return Some(task_id);
        }
        self.discard_pending_form_navigation_task(task_id);
        None
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

    pub(super) fn cancel_planned_form_navigation_for_url(
        &mut self,
        destination: OwnerDispatchScope,
        url: Option<&url::Url>,
    ) {
        // A queued javascript URL does not supersede a pending submission.
        // The form task can navigate first and invalidate the script task.
        if url.is_none_or(|url| url.scheme() != "javascript") {
            self.cancel_planned_form_navigation_to(destination);
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

    pub(crate) fn has_planned_form_navigation_for(&self, form: DomHandle) -> bool {
        self.form_navigations
            .by_form
            .contains_key(&(self.root_document_lifecycle_identity(), form))
            || self.form_navigations.related_by_form.contains_key(&form)
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
        let form = navigation.form;
        let local_source = navigation.related_source.is_none();
        let related_source = self.forget_related_form_navigation_source(&navigation, task_id);
        // Clear the planned task before firing any event: listeners may submit again.
        self.form_navigations.forget(task_id);
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            let source_ptr = related_source
                .as_ref()
                .map_or(host_ptr, |source| source.as_ptr());
            if local_source || related_source.is_some() {
                crate::context_bootstrap::web_mcp::finish_form_navigation(
                    scope, source_ptr, form, false,
                );
            }
            return Some(false);
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch = target.dispatch_scope();
        // Keep the source origin if its old Document has retired while a
        // different target remains live.
        let source_origin = navigation
            .source_document
            .and_then(|document| self.owner_dispatch_scope_for_node(document))
            .and_then(|source| self.window_access_origin_for_dispatch_scope(source))
            .or(pending.source_origin);
        let source_can_access_target = source_origin
            .zip(self.window_access_origin_for_dispatch_scope(dispatch))
            .is_some_and(|(source, target)| source.can_access(&target));
        let previous = dispatch.enter(scope);
        // The selected target owns navigation work even if the source form
        // was adopted or its Document was replaced after submission.
        let applied =
            apply_planned_form_navigation(scope, host_ptr, navigation, source_can_access_target);
        let source_ptr = related_source
            .as_ref()
            .map_or(host_ptr, |source| source.as_ptr());
        if local_source || related_source.is_some() {
            crate::context_bootstrap::web_mcp::finish_form_navigation(
                scope, source_ptr, form, applied,
            );
        }
        dispatch.restore(scope, previous);
        Some(applied)
    }

    pub(crate) fn discard_pending_form_navigation_task(
        &mut self,
        task_id: RendererPageFormNavigationTaskId,
    ) -> bool {
        self.form_navigations.forget(task_id);
        let Some(pending) = self.form_navigations.tasks.remove(task_id) else {
            return false;
        };
        let pending = pending.into_payload();
        let related_source =
            self.forget_related_form_navigation_source(&pending.navigation, task_id);
        pending.cancellation.cancel();
        if pending.navigation.related_source.is_none() || related_source.is_some() {
            crate::context_bootstrap::web_mcp::note_form_navigation_cancelled(
                related_source
                    .as_ref()
                    .map_or(self, |source| unsafe { &mut *source.as_ptr() }),
                pending.navigation.form,
            );
        }
        true
    }
}
