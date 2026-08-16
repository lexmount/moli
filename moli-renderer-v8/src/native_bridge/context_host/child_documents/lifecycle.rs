use super::super::JsContextHost;

#[derive(Clone, Copy)]
enum ChildDocumentInteractiveScriptDisposition {
    QueueOwnerWork,
    TakeReadyClassicOnCurrentStack,
}
use crate::{
    context_bootstrap::{
        construct_original_event, dispatch_beforeunload_for_runtime_owner,
        dispatch_pagehide_for_runtime_owner, dispatch_unload_for_runtime_owner,
    },
    detached_event_target::dispatch_detached_simple_event,
    document_runtime::DomHandle,
    dom::native::DocumentReadyState,
    frame_owner_model::{
        FrameDocumentCompleteLifecycleAction, FrameDocumentDescendantLoadCompletion,
        FrameDocumentDescendantLoadParent, FrameDocumentInteractiveLifecycleAction,
        FrameDocumentLifecycleAction, FrameDocumentLifecycleTaskEffect,
        FrameDocumentLoadDeliveryTask, FrameDocumentNavigationLoadBinding, FrameDocumentTaskOwner,
        FrameRealmId,
    },
    native_bridge::document::DETACHED_STATE_SLOT,
    util::{
        call_object_method, context_host_ptr_from_global_bridge, get_private_object, v8_string,
        v8str,
    },
};

#[derive(Clone, Copy, Debug, Default)]
struct ChildDocumentCompleteBlockers {
    lifecycle_delay: bool,
    incomplete_descendant: bool,
}

struct ChildDocumentInteractiveApplication {
    synchronous_work:
        Option<crate::document_script_scheduler::FrameDocumentClassicScriptSchedulerWork>,
    event_dispatched: bool,
}

impl ChildDocumentInteractiveApplication {
    fn task_effect(&self) -> FrameDocumentLifecycleTaskEffect {
        if self.event_dispatched {
            FrameDocumentLifecycleTaskEffect::EventDispatched
        } else {
            FrameDocumentLifecycleTaskEffect::ConsumedWithoutEvent
        }
    }
}

impl ChildDocumentCompleteBlockers {
    fn any(self) -> bool {
        self.lifecycle_delay || self.incomplete_descendant
    }
}

impl JsContextHost {
    fn route_child_document_lifecycle_action(
        &self,
        action: FrameDocumentLifecycleAction,
        realm_id: FrameRealmId,
    ) -> bool {
        self.page_child_frame_task_sender()
            .send_document_lifecycle(
                crate::page_task_queue::RendererPageChildDocumentLifecycleTarget::new(
                    action, realm_id,
                ),
            )
            .is_ok()
    }

    pub(crate) fn bind_child_modulepreload_terminal_event(
        &mut self,
        work: crate::frame_owner_model::FrameDocumentModulepreloadTerminalWork,
    ) -> Option<crate::frame_owner_model::FrameDocumentModulepreloadEventAction> {
        let Some(binding) = self
            .frame_owner_store
            .bind_current_child_modulepreload_event(work.owner(), work.client())
        else {
            tracing::debug!(
                owner = ?work.owner(),
                realm_id = ?work.realm_id(),
                link_handle = ?work.link_handle(),
                successful = work.successful(),
                "dropping stale child modulepreload terminal before event binding"
            );
            return None;
        };
        tracing::debug!(
            owner = ?work.owner(),
            realm_id = ?work.realm_id(),
            link_handle = ?work.link_handle(),
            load_delay_token = ?binding.load_delay_token(),
            successful = work.successful(),
            "bound child modulepreload terminal to lifecycle-owned event action"
        );
        Some(work.into_event_action(binding))
    }

    pub(crate) fn settle_child_modulepreload_link_event(
        &mut self,
        owner: FrameDocumentTaskOwner,
        binding: crate::frame_owner_model::FrameDocumentModulepreloadEventBinding,
        queue_lifecycle_followup: bool,
    ) -> bool {
        if !self
            .frame_owner_store
            .settle_child_modulepreload_event(owner, binding)
        {
            return false;
        }
        if queue_lifecycle_followup && binding.load_delay_token().is_some() {
            let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                binding.child_handle(),
                owner,
            );
        }
        true
    }

    fn queue_current_child_document_image_load_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) {
        let Some(snapshot) = self
            .frame_owner_current_child_snapshot(child_handle)
            .filter(|snapshot| {
                snapshot.scheduler_lane_id == owner.scheduler_lane_id
                    && snapshot.local_window_id == owner.local_window_id
                    && snapshot.document_id == owner.document_id
            })
        else {
            return;
        };
        let image_handles = (0..self.dom_host().dom().nodes().len())
            .map(DomHandle::new)
            .filter(|handle| {
                self.dom_host().owner_document_handle(*handle) == Some(snapshot.document_handle)
                    && self
                        .dom_host()
                        .node(*handle)
                        .and_then(crate::dom::native::Node::as_element)
                        .and_then(
                            crate::native_bridge::element::ImageResourceElementKind::for_element,
                        )
                        .is_some()
            })
            .collect::<Vec<_>>();
        let host_ptr = self as *mut JsContextHost;
        for image_handle in image_handles {
            crate::native_bridge::element::queue_image_load_event_if_needed_with_initiator(
                scope,
                host_ptr,
                image_handle,
                crate::types::SubresourceRequestInitiatorType::Parser,
            );
        }
    }

    fn queue_current_child_document_media_loads(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) {
        let Some(snapshot) = self
            .frame_owner_current_child_snapshot(child_handle)
            .filter(|snapshot| {
                snapshot.scheduler_lane_id == owner.scheduler_lane_id
                    && snapshot.local_window_id == owner.local_window_id
                    && snapshot.document_id == owner.document_id
            })
        else {
            return;
        };
        let handles = (0..self.dom_host().dom().nodes().len())
            .map(DomHandle::new)
            .filter(|handle| {
                self.dom_host().is_connected(*handle)
                    && self.dom_host().owner_document_handle(*handle)
                        == Some(snapshot.document_handle)
            })
            .collect::<Vec<_>>();
        let host_ptr = self as *mut JsContextHost;
        for handle in handles {
            let Some(element) = self
                .dom_host()
                .node(handle)
                .and_then(crate::dom::native::Node::as_element)
            else {
                continue;
            };
            if element.is_html_element("audio") || element.is_html_element("video") {
                crate::native_bridge::element::queue_media_load_if_needed(scope, host_ptr, handle);
            } else if element.is_html_element("track") {
                crate::native_bridge::element::queue_default_text_track_mode_if_needed(
                    scope, host_ptr, handle,
                );
                crate::native_bridge::element::queue_text_track_load_if_needed(
                    scope, host_ptr, handle,
                );
            }
        }
    }

    pub(crate) fn queue_child_document_interactive_lifecycle_action(
        &mut self,
        action: FrameDocumentInteractiveLifecycleAction,
    ) -> bool {
        let Some(realm_request) = self.request_child_frame_realm_materialization_for_owner(
            action.child_handle(),
            action.owner(),
        ) else {
            return false;
        };
        self.route_child_document_lifecycle_action(action.into(), realm_request.realm_id())
    }

    pub(crate) fn queue_child_document_domcontentloaded_if_ready(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, owner)
        {
            return false;
        }
        let Some(realm_request) =
            self.request_child_frame_realm_materialization_for_owner(child_handle, owner)
        else {
            return false;
        };
        let Some(action) = self
            .frame_owner_store
            .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        else {
            return false;
        };
        self.route_child_document_lifecycle_action(action.into(), realm_request.realm_id())
    }

    pub(crate) fn queue_child_document_domcontentloaded_if_ready_for_document_realm(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        let Some(snapshot) = self
            .frame_owner_store
            .current_child_owner_snapshot_for_realm(realm_id)
        else {
            return false;
        };
        if FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        ) != owner
        {
            return false;
        }
        self.queue_child_document_domcontentloaded_if_ready(snapshot.owner_handle, owner)
    }

    pub(crate) fn queue_child_document_complete_lifecycle_if_ready(
        &mut self,
        child_handle: DomHandle,
    ) -> bool {
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
        else {
            return false;
        };
        self.queue_child_document_complete_lifecycle_if_ready_for_owner(child_handle, owner)
    }

    pub(crate) fn queue_child_document_complete_lifecycle_if_ready_for_document_realm(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        let Some(snapshot) = self
            .frame_owner_store
            .current_child_owner_snapshot_for_realm(realm_id)
        else {
            return false;
        };
        let task_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        if task_owner.document_owner() != owner {
            return false;
        }
        self.queue_child_document_complete_lifecycle_if_ready_for_owner(
            snapshot.owner_handle,
            task_owner,
        )
    }

    pub(crate) fn queue_child_document_complete_lifecycle_if_ready_for_owner(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, owner)
        {
            return false;
        }
        let blockers = self.child_document_complete_blockers(child_handle, owner);
        if blockers.any() {
            tracing::debug!(
                ?child_handle,
                ?owner,
                lifecycle_delay = blockers.lifecycle_delay,
                incomplete_descendant = blockers.incomplete_descendant,
                "child document complete transition remains blocked"
            );
            return false;
        }
        if self
            .frame_owner_store
            .current_child_document_load_delivery_is_ready(child_handle, owner)
        {
            return self.queue_ready_child_host_load_task(FrameDocumentLoadDeliveryTask {
                child_handle,
                owner,
            });
        }
        let Some(realm_request) =
            self.request_child_frame_realm_materialization_for_owner(child_handle, owner)
        else {
            return false;
        };
        let Some(action) = self
            .frame_owner_store
            .prepare_current_child_document_complete_transition(child_handle, owner)
        else {
            return false;
        };
        if !self.route_child_document_lifecycle_action(action.into(), realm_request.realm_id()) {
            let _ = self
                .frame_owner_store
                .cancel_current_child_document_complete_transition(action);
            return false;
        }
        tracing::debug!(
            ?child_handle,
            ?owner,
            "queued document-owned child complete transition"
        );
        true
    }

    pub(in crate::native_bridge::context_host) fn note_child_frame_load_started_for_parent(
        &mut self,
        child_handle: DomHandle,
    ) {
        self.note_lightweight_popup_child_frame_load_started(child_handle);
        let released_parent = self
            .frame_owner_store
            .begin_child_frame_parent_document_load(child_handle);
        if let Some(completion) = released_parent {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
        tracing::debug!(
            ?child_handle,
            "processed child frame load start at its parent document lifecycle boundary"
        );
    }

    pub(in crate::native_bridge::context_host) fn replace_child_navigation_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentNavigationLoadBinding> {
        let navigation = self
            .frame_owner_store
            .replace_current_child_navigation_load(child_handle)?;
        tracing::debug!(
            ?child_handle,
            owner = ?navigation.owner(),
            navigation_id = ?navigation.navigation_id(),
            document_load_delay_token = ?navigation.document_load_delay_token(),
            "accepted child navigation lifecycle binding"
        );
        Some(navigation)
    }

    pub(in crate::native_bridge::context_host) fn ensure_child_navigation_load(
        &mut self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentNavigationLoadBinding> {
        let navigation = self
            .frame_owner_store
            .ensure_current_child_navigation_load(child_handle)?;
        Some(navigation)
    }

    pub(crate) fn current_child_navigation_load(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameDocumentNavigationLoadBinding> {
        self.frame_owner_store
            .current_child_navigation_load(child_handle)
    }

    pub(in crate::native_bridge::context_host) fn rebind_active_child_frame_load_to_parent(
        &mut self,
        child_handle: DomHandle,
    ) {
        self.note_lightweight_popup_child_frame_load_started(child_handle);
        let released_parent = self
            .frame_owner_store
            .rebind_active_child_frame_parent_document_load(child_handle);
        if let Some(completion) = released_parent {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
    }

    pub(in crate::native_bridge::context_host) fn finish_child_frame_navigation_without_load_dispatch(
        &mut self,
        child_handle: DomHandle,
        expected: FrameDocumentNavigationLoadBinding,
    ) -> bool {
        let Some(owner) = self
            .frame_owner_store
            .settle_current_child_navigation_load(child_handle, expected)
        else {
            tracing::debug!(
                ?child_handle,
                ?expected,
                "ignored stale child navigation finish"
            );
            return false;
        };
        tracing::debug!(
            ?child_handle,
            ?owner,
            navigation_id = ?expected.navigation_id(),
            document_load_delay_token = ?expected.document_load_delay_token(),
            "settled child navigation without load dispatch"
        );
        self.note_lightweight_popup_child_frame_load_finished(child_handle);
        let released_parent = self
            .frame_owner_store
            .cancel_child_frame_parent_document_load(child_handle);
        if let Some(completion) = released_parent {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
        true
    }

    pub(in crate::native_bridge::context_host) fn finish_child_frame_without_current_document_load_dispatch(
        &mut self,
        child_handle: DomHandle,
    ) {
        debug_assert!(
            self.frame_owner_store
                .current_child_document_task_owner(child_handle)
                .is_none()
        );
        self.note_lightweight_popup_child_frame_load_finished(child_handle);
        let released_parent = self
            .frame_owner_store
            .cancel_child_frame_parent_document_load(child_handle);
        if let Some(completion) = released_parent {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
    }

    pub(in crate::native_bridge::context_host) fn settle_child_navigation_load(
        &mut self,
        child_handle: DomHandle,
        expected: FrameDocumentNavigationLoadBinding,
        queue_document_complete: bool,
    ) -> bool {
        let Some(owner) = self
            .frame_owner_store
            .settle_current_child_navigation_load(child_handle, expected)
        else {
            tracing::debug!(
                ?child_handle,
                ?expected,
                "ignored stale child navigation terminal"
            );
            return false;
        };
        if queue_document_complete {
            let _ = self
                .queue_child_document_complete_lifecycle_if_ready_for_owner(child_handle, owner);
        }
        true
    }

    pub(in crate::native_bridge::context_host) fn detach_child_frame_owner_and_wake_parent(
        &mut self,
        child_handle: DomHandle,
    ) {
        self.note_lightweight_popup_child_frame_load_finished(child_handle);
        let released_parent = self.frame_owner_store.detach_child_frame(child_handle);
        if let Some(completion) = released_parent {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
    }

    pub(in crate::native_bridge::context_host) fn apply_pending_parent_descendant_completions(
        &mut self,
    ) {
        for completion in self
            .frame_owner_store
            .take_pending_parent_document_descendant_completions()
        {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
    }

    pub(in crate::native_bridge::context_host) fn reconcile_parent_lifecycle_after_descendant_completion(
        &mut self,
        completion: FrameDocumentDescendantLoadCompletion,
    ) {
        match completion.parent {
            FrameDocumentDescendantLoadParent::MainDocument => {
                if self.enqueue_main_document_completion_recheck(completion.parent_owner) {
                    tracing::debug!(
                        parent_owner = ?completion.parent_owner,
                        child_frame_id = ?completion.child_frame_id,
                        "queued exact main document completion recheck after descendant load release"
                    );
                } else {
                    tracing::debug!(
                        parent_owner = ?completion.parent_owner,
                        child_frame_id = ?completion.child_frame_id,
                        "main document completion recheck route retired with its Page"
                    );
                }
            }
            FrameDocumentDescendantLoadParent::ChildDocument(parent_child_handle) => {
                tracing::debug!(
                    ?parent_child_handle,
                    parent_owner = ?completion.parent_owner,
                    child_frame_id = ?completion.child_frame_id,
                    "released exact child document descendant load"
                );
                let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                    parent_child_handle,
                    completion.parent_owner,
                );
            }
        }
    }

    fn child_document_complete_blockers(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> ChildDocumentCompleteBlockers {
        ChildDocumentCompleteBlockers {
            lifecycle_delay: self
                .frame_owner_store
                .current_child_document_has_load_delay_tokens(child_handle, owner)
                .unwrap_or(true),
            incomplete_descendant: self
                .frame_owner_store
                .current_child_document_has_incomplete_descendants(child_handle, owner)
                .unwrap_or(true),
        }
    }

    pub(crate) fn child_document_lifecycle_action_is_current(
        &self,
        action: FrameDocumentLifecycleAction,
    ) -> bool {
        self.frame_owner_store
            .current_child_document_lifecycle_action_is_pending(action)
    }

    pub(crate) fn current_child_document_lifecycle_target(
        &self,
        expected: crate::page_task_queue::RendererPageChildDocumentLifecycleTarget,
    ) -> Option<crate::page_task_queue::RendererPageChildDocumentLifecycleTarget> {
        let action = expected.action();
        if !self.child_document_lifecycle_action_is_current(action)
            || self
                .frame_owner_store
                .current_materialized_realm_id_for_document_task_owner(action.owner())
                != Some(expected.realm_id())
        {
            return None;
        }
        Some(expected)
    }

    pub(crate) fn run_child_document_lifecycle_action(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentLifecycleAction,
    ) -> FrameDocumentLifecycleTaskEffect {
        if !self.child_document_lifecycle_action_is_current(action) {
            return FrameDocumentLifecycleTaskEffect::NotApplied;
        }
        match action {
            FrameDocumentLifecycleAction::Interactive(action) => {
                self.run_child_document_interactive_lifecycle_action(scope, action)
            }
            FrameDocumentLifecycleAction::DomContentLoaded(action) => {
                self.run_child_document_domcontentloaded_lifecycle_action(scope, action)
            }
            FrameDocumentLifecycleAction::Complete(action) => {
                self.run_child_document_complete_lifecycle_action(scope, action)
            }
        }
    }

    fn run_child_document_interactive_lifecycle_action(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentInteractiveLifecycleAction,
    ) -> FrameDocumentLifecycleTaskEffect {
        let Some(application) = self.apply_child_document_interactive_lifecycle_action(
            scope,
            action,
            ChildDocumentInteractiveScriptDisposition::QueueOwnerWork,
        ) else {
            return FrameDocumentLifecycleTaskEffect::NotApplied;
        };
        debug_assert!(
            application.synchronous_work.is_none(),
            "normal child lifecycle turns must queue parser-deferred script work"
        );
        application.task_effect()
    }

    pub(in crate::native_bridge::context_host) fn apply_child_document_interactive_for_script_created_parser_close(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentInteractiveLifecycleAction,
    ) -> Option<crate::document_script_scheduler::FrameDocumentClassicScriptSchedulerWork> {
        self.apply_child_document_interactive_lifecycle_action(
            scope,
            action,
            ChildDocumentInteractiveScriptDisposition::TakeReadyClassicOnCurrentStack,
        )
        .and_then(|application| application.synchronous_work)
    }

    fn apply_child_document_interactive_lifecycle_action(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentInteractiveLifecycleAction,
        script_disposition: ChildDocumentInteractiveScriptDisposition,
    ) -> Option<ChildDocumentInteractiveApplication> {
        let child_handle = action.child_handle();
        let document_handle = self.child_browsing_context_document_handle(child_handle);
        if !self
            .frame_owner_store
            .apply_current_child_document_interactive_transition(action)
        {
            return None;
        }

        let event_dispatched = if let Some(document) =
            self.child_browsing_context_document_wrapper(scope, child_handle)
        {
            dispatch_child_document_readiness_event(
                scope,
                document,
                DocumentReadyState::Interactive,
            );
            true
        } else if let Some(document_handle) = document_handle {
            let _ = self.set_dom_document_ready_state_for_handle(
                document_handle,
                DocumentReadyState::Interactive,
            );
            false
        } else {
            false
        };

        if self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, action.owner())
        {
            self.queue_current_child_document_image_load_events(
                scope,
                child_handle,
                action.owner(),
            );
            self.queue_current_child_document_media_loads(scope, child_handle, action.owner());
            let synchronous_work = match script_disposition {
                ChildDocumentInteractiveScriptDisposition::QueueOwnerWork => None,
                ChildDocumentInteractiveScriptDisposition::TakeReadyClassicOnCurrentStack => self
                    .frame_parser_deferred_script_order
                    .pending_head(action.owner().document_owner())
                    .and_then(|head| {
                        self.take_child_parser_deferred_classic_work_if_ready(
                            child_handle,
                            action.owner(),
                            head,
                        )
                    }),
            };
            let queued_document_script_ready = synchronous_work.is_some()
                || self
                    .queue_next_child_parser_deferred_script_if_ready(child_handle, action.owner());
            if !queued_document_script_ready {
                self.queue_child_document_domcontentloaded_if_ready(child_handle, action.owner());
            }
            return Some(ChildDocumentInteractiveApplication {
                synchronous_work,
                event_dispatched,
            });
        }
        Some(ChildDocumentInteractiveApplication {
            synchronous_work: None,
            event_dispatched,
        })
    }

    fn run_child_document_domcontentloaded_lifecycle_action(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: crate::frame_owner_model::FrameDocumentDomContentLoadedLifecycleAction,
    ) -> FrameDocumentLifecycleTaskEffect {
        let child_handle = action.child_handle();
        if !self
            .frame_owner_store
            .apply_current_child_document_domcontentloaded_transition(action)
        {
            return FrameDocumentLifecycleTaskEffect::NotApplied;
        }
        let event_dispatched = self
            .child_browsing_context_document_wrapper(scope, child_handle)
            .is_some_and(|document| {
                self.dispatch_child_document_event_for_owner(
                    scope,
                    child_handle,
                    action.owner(),
                    document,
                    "DOMContentLoaded",
                    true,
                    false,
                )
            });
        if self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, action.owner())
        {
            let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                child_handle,
                action.owner(),
            );
        }
        if event_dispatched {
            FrameDocumentLifecycleTaskEffect::EventDispatched
        } else {
            FrameDocumentLifecycleTaskEffect::ConsumedWithoutEvent
        }
    }

    fn run_child_document_complete_lifecycle_action(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentCompleteLifecycleAction,
    ) -> FrameDocumentLifecycleTaskEffect {
        let child_handle = action.child_handle();
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, action.owner())
        {
            return FrameDocumentLifecycleTaskEffect::NotApplied;
        }
        let blockers = self.child_document_complete_blockers(child_handle, action.owner());
        if blockers.any() {
            let canceled = self
                .frame_owner_store
                .cancel_current_child_document_complete_transition(action);
            tracing::debug!(
                ?child_handle,
                owner = ?action.owner(),
                ?blockers,
                canceled,
                "canceled child complete transition after readiness changed before its lifecycle turn"
            );
            return FrameDocumentLifecycleTaskEffect::ConsumedWithoutEvent;
        }
        if !self
            .frame_owner_store
            .apply_current_child_document_complete_transition(action)
        {
            return FrameDocumentLifecycleTaskEffect::NotApplied;
        }
        tracing::debug!(
            ?child_handle,
            owner = ?action.owner(),
            "applied document-owned child complete transition"
        );
        let event_dispatched = if let Some(document) =
            self.child_browsing_context_document_wrapper(scope, child_handle)
        {
            dispatch_child_document_readiness_event(scope, document, DocumentReadyState::Complete);
            true
        } else if let Some(document_handle) =
            self.child_browsing_context_document_handle(child_handle)
        {
            let _ = self.set_dom_document_ready_state_for_handle(
                document_handle,
                DocumentReadyState::Complete,
            );
            false
        } else {
            false
        };
        if self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, action.owner())
        {
            let _ = self.queue_child_document_complete_lifecycle_if_ready_for_owner(
                child_handle,
                action.owner(),
            );
        }
        if event_dispatched {
            FrameDocumentLifecycleTaskEffect::EventDispatched
        } else {
            FrameDocumentLifecycleTaskEffect::ConsumedWithoutEvent
        }
    }

    pub(crate) fn mark_current_child_document_unload_dispatched_after_navigation_traversal(
        &mut self,
        handle: DomHandle,
    ) -> bool {
        let Some(action) = self
            .frame_owner_store
            .begin_current_child_document_unload(handle)
        else {
            return false;
        };
        self.frame_owner_store
            .finish_current_child_document_unload(action)
    }

    pub(in crate::native_bridge::context_host) fn dispatch_child_browsing_context_unload_lifecycle_if_needed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) -> bool {
        let Some(window) = self.existing_child_browsing_context_window_wrapper(scope, handle)
        else {
            return false;
        };
        let Some(_document) = self.child_browsing_context_document_wrapper(scope, handle) else {
            return false;
        };
        let Some(action) = self
            .frame_owner_store
            .begin_current_child_document_unload(handle)
        else {
            return false;
        };
        tracing::debug!(
            ?handle,
            owner = ?action.owner(),
            "dispatching document-owned child unload lifecycle"
        );
        let execution_context_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
            action.owner().local_window_id,
        );
        dispatch_beforeunload_for_runtime_owner(scope, window);
        dispatch_pagehide_for_runtime_owner(scope, window);
        dispatch_unload_for_runtime_owner(scope, window);
        let finished = self
            .frame_owner_store
            .finish_current_child_document_unload(action);
        tracing::debug!(
            ?handle,
            owner = ?action.owner(),
            finished,
            "finished document-owned child unload lifecycle"
        );
        unsafe { &mut *self.runtime }
            .cancel_window_execution_context_timers(execution_context_owner);
        true
    }

    /// Dispatches the unload sequence used when `Document::open()` removes a
    /// descendant frame.
    ///
    /// This is intentionally distinct from navigation teardown: Chromium's
    /// document-open steps do not prompt the child with `beforeunload`, and
    /// dispatch pagehide/visibilitychange before unload while the parent
    /// document's listeners are still installed.
    fn dispatch_child_browsing_context_document_open_unload_lifecycle_if_needed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(window) = self.existing_child_browsing_context_window_wrapper(scope, handle)
        else {
            return;
        };
        let Some(document) = self.child_browsing_context_document_wrapper(scope, handle) else {
            return;
        };
        let Some(action) = self
            .frame_owner_store
            .begin_current_child_document_unload(handle)
        else {
            return;
        };
        let execution_context_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
            action.owner().local_window_id,
        );

        dispatch_pagehide_for_runtime_owner(scope, window);
        if let Some(event) = construct_original_event(scope, "visibilitychange") {
            let _ = call_object_method(scope, document, "dispatchEvent", &[event.into()]);
        }
        dispatch_unload_for_runtime_owner(scope, window);
        let _ = self
            .frame_owner_store
            .finish_current_child_document_unload(action);
        unsafe { &mut *self.runtime }
            .cancel_window_execution_context_timers(execution_context_owner);
    }

    pub(crate) fn dispatch_document_open_descendant_frame_unload_lifecycle(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        document_handle: DomHandle,
    ) {
        let handles = self.child_browsing_context_handles_in_document_order();
        for handle in handles {
            if self.dom_host().owner_document_handle(handle) != Some(document_handle) {
                continue;
            }
            self.dispatch_child_browsing_context_document_open_unload_lifecycle_if_needed(
                scope, handle,
            );
        }
    }
}

pub(in crate::native_bridge::context_host) fn dispatch_child_document_readiness_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    ready_state: DocumentReadyState,
) {
    set_child_document_ready_state(scope, document, ready_state);
    let _ =
        dispatch_detached_simple_event(scope, document, "readystatechange", false, false, false);
}

fn set_child_document_ready_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    ready_state: DocumentReadyState,
) {
    if let Some(state) = get_private_object(scope, document, DETACHED_STATE_SLOT) {
        if let Some(value) = v8_string(scope, ready_state.as_str()) {
            let _ = state.set(scope, v8str(scope, "readyState").into(), value.into());
        }
        return;
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Ok((object_host_ptr, document_handle)) =
            crate::native_bridge::node::node_runtime_and_handle_from_object(scope, document)
        && object_host_ptr == host_ptr
    {
        let _ = unsafe { &mut *host_ptr }
            .set_dom_document_ready_state_for_handle(document_handle, ready_state);
    }
}
