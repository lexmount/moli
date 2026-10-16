use super::{FrameDocumentLoadDeliveryTask, JsContextHost};
use crate::{
    context_bootstrap::{
        construct_original_event, construct_original_page_transition_event,
        navigate_child_meta_refresh, record_performance_load_event_end_for_window,
        record_performance_load_event_start_for_window,
    },
    document_runtime::{DocumentRuntime, DomHandle, EventTargetHandle, MetaRefreshNavigation},
    frame_owner_model::{
        FrameDocumentLoadDeliveryAction, FrameDocumentLoadDeliveryPhase,
        FrameDocumentLoadDeliveryProgress,
    },
    host::HostTimerOwner,
    meta_refresh_scheduler::MetaRefreshScheduler,
    util::v8str,
};
use serde::Serialize;
use std::convert::TryFrom;

pub(in crate::native_bridge::context_host) struct ChildMetaRefreshState {
    timer_id: Option<u32>,
    owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    scheduler: MetaRefreshScheduler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChildFrameAttachmentSnapshot {
    pub(crate) frame_id: String,
    pub(crate) parent_frame_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChildFrameNavigationSnapshot {
    pub(crate) frame_id: String,
    pub(crate) parent_frame_id: Option<String>,
    pub(crate) loader_id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) url: String,
    pub(crate) document_open_replacement: bool,
    #[serde(default)]
    pub(crate) security_origin_inherited: bool,
    #[serde(default)]
    pub(crate) security_origin_opaque: bool,
    #[serde(default)]
    pub(crate) document_network: Option<crate::protocol_types::ChildFrameDocumentNetworkSnapshot>,
}

impl ChildFrameNavigationSnapshot {
    fn into_protocol_snapshot(self) -> crate::protocol_types::ChildFrameNavigationSnapshot {
        crate::protocol_types::ChildFrameNavigationSnapshot {
            frame_id: self.frame_id,
            parent_frame_id: self.parent_frame_id,
            loader_id: self.loader_id,
            name: self.name,
            url: self.url,
            document_open_replacement: self.document_open_replacement,
            security_origin_inherited: self.security_origin_inherited,
            security_origin_opaque: self.security_origin_opaque,
            document_network: self.document_network,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildFrameHostLoadTaskOutcome {
    ConsumedWithoutCallback,
    CallbackDispatched,
}

impl ChildFrameHostLoadTaskOutcome {
    pub(crate) fn made_progress(&self) -> bool {
        matches!(
            self,
            Self::ConsumedWithoutCallback | Self::CallbackDispatched
        )
    }

    pub(crate) fn callback_was_dispatched(&self) -> bool {
        matches!(self, Self::CallbackDispatched)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ChildFrameLoadDeliveryResult {
    callback_dispatched: bool,
}

struct ChildFrameLoadDeliveryPhaseResult {
    progress: Option<FrameDocumentLoadDeliveryProgress>,
    callback_dispatched: bool,
}

impl ChildFrameLoadDeliveryPhaseResult {
    fn new(progress: Option<FrameDocumentLoadDeliveryProgress>, callback_dispatched: bool) -> Self {
        Self {
            progress,
            callback_dispatched,
        }
    }

    fn without_callback(progress: Option<FrameDocumentLoadDeliveryProgress>) -> Self {
        Self::new(progress, false)
    }

    fn after_callback(progress: Option<FrameDocumentLoadDeliveryProgress>) -> Self {
        Self::new(progress, true)
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn dispatch_ready_child_initial_empty_load_synchronously(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: FrameDocumentLoadDeliveryTask,
    ) -> bool {
        if !self.child_host_load_task_owner_is_current(task)
            || !self
                .frame_owner_store
                .current_child_document_load_delivery_is_ready(task.child_handle, task.owner)
        {
            return false;
        }
        let host_ptr = self as *mut JsContextHost;
        self.run_child_browsing_context_load_delivery(scope, host_ptr, task)
            .callback_dispatched
    }

    /// Queues an already-ready lifecycle delivery action.
    ///
    /// Callers must first claim the document-owned complete transition. This
    /// function deliberately does not inspect script, parser, stylesheet,
    /// navigation or descendant readiness.
    pub(in crate::native_bridge::context_host) fn queue_ready_child_host_load_task(
        &mut self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> bool {
        if !self.child_host_load_task_owner_is_current(task)
            || !self
                .frame_owner_store
                .current_child_document_load_delivery_is_ready(task.child_handle, task.owner)
        {
            return false;
        }
        let Some(admission) = self
            .frame_owner_store
            .reserve_current_child_document_load_delivery_task(task)
        else {
            return false;
        };
        let routed = self
            .page_task_capabilities
            .get()
            .expect("child load delivery must retain its Page task capabilities")
            .dom_manipulation()
            .send_child_host_load(
                crate::page_task_queue::RendererPageChildHostLoadTarget::new(admission),
            )
            .is_ok();
        if !routed {
            let _ = self
                .frame_owner_store
                .release_current_child_document_load_delivery_task_reservation(admission);
            return false;
        }
        tracing::debug!(
            child_handle = ?task.child_handle,
            owner = ?task.owner,
            "queued exact child load delivery on the DOM manipulation source"
        );
        true
    }

    #[cfg(test)]
    pub(crate) fn take_completed_child_frame_navigation_loads(
        &mut self,
    ) -> Vec<ChildFrameNavigationSnapshot> {
        std::mem::take(&mut self.completed_child_browsing_context_loads)
    }

    #[cfg(test)]
    pub(crate) fn take_completed_child_document_networks(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameDocumentNetworkActivitySnapshot> {
        std::mem::take(&mut self.completed_child_document_networks)
    }

    #[cfg(test)]
    pub(crate) fn take_pending_child_frame_tree_events(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameTreeEventSnapshot> {
        std::mem::take(&mut self.pending_child_frame_tree_events)
    }

    pub(crate) fn completed_child_frame_navigation_load_count(&self) -> usize {
        #[cfg(test)]
        {
            self.completed_child_browsing_context_loads.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }

    pub(in crate::native_bridge::context_host) fn queue_child_frame_attachment_event(
        &mut self,
        event: ChildFrameAttachmentSnapshot,
    ) {
        let event = crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(
            crate::protocol_types::ChildFrameAttachmentSnapshot {
                frame_id: event.frame_id,
                parent_frame_id: self.protocol_child_frame_parent_id(event.parent_frame_id),
            },
        );
        self.queue_child_frame_tree_event(event);
    }

    pub(in crate::native_bridge::context_host) fn queue_child_frame_detachment_event(
        &mut self,
        frame_id: String,
    ) {
        self.queue_child_frame_tree_event(
            crate::protocol_types::ChildFrameTreeEventSnapshot::Detached(
                crate::protocol_types::ChildFrameDetachmentSnapshot { frame_id },
            ),
        );
    }

    fn queue_child_frame_tree_event(
        &mut self,
        event: crate::protocol_types::ChildFrameTreeEventSnapshot,
    ) {
        if let Some(recorder) = self.command_turn_output.clone() {
            let source_document = self
                .root_document_lifecycle_identity()
                .expect("a command-bound child-frame tree event must retain its source Document");
            recorder.push_child_frame_tree_event(source_document, event);
            return;
        }
        if let Some(source_document) = self.root_document_lifecycle_identity()
            && self.append_live_turn_owner_action(
                crate::runtime::RendererOwnerAction::ChildFrameTree {
                    source_document,
                    event: event.clone(),
                },
            )
        {
            return;
        }
        #[cfg(test)]
        self.pending_child_frame_tree_events.push(event);
        #[cfg(not(test))]
        panic!("a production child-frame tree event must have a concrete renderer output sink");
    }

    fn protocol_child_frame_parent_id(&self, parent_frame_id: Option<String>) -> Option<String> {
        let main_frame_id = self
            .frame_owner_store
            .current_main_owner_snapshot()
            .map(|snapshot| snapshot.frame_id.0);
        parent_frame_id.filter(|parent_frame_id| Some(parent_frame_id) != main_frame_id.as_ref())
    }

    pub(crate) fn queue_child_frame_document_opened_event(&mut self, handle: DomHandle) {
        let parent_frame_id = self.child_browsing_context_parent_frame_id(handle);
        let Some(url) = self.child_browsing_context_current_url(handle) else {
            return;
        };
        let security_origin_opaque = self.child_browsing_context_has_opaque_origin(handle);
        let Some(entry) = self.child_browsing_contexts.get(&handle) else {
            return;
        };
        let identity = entry.frame_identity_snapshot();
        let protocol_event = crate::protocol_types::ChildFrameDocumentOpenedSnapshot {
            frame_id: identity.frame_id,
            parent_frame_id: self.protocol_child_frame_parent_id(parent_frame_id),
            loader_id: entry.current_document_loader_id().map(ToOwned::to_owned),
            name: identity.name,
            url: url.to_string(),
            security_origin_inherited: identity.security_origin_inherited,
            security_origin_opaque,
        };
        if let Some(recorder) = self.command_turn_output.clone() {
            let source_document = self.root_document_lifecycle_identity().expect(
                "a command-bound child Document-opened event must retain its source Document",
            );
            recorder.push_child_frame_document_opened(source_document, protocol_event);
            return;
        }
        if let Some(source_document) = self.root_document_lifecycle_identity()
            && self.append_live_turn_owner_action(
                crate::runtime::RendererOwnerAction::ChildFrameDocumentOpened {
                    source_document,
                    event: protocol_event.clone(),
                },
            )
        {
            return;
        }
        let _ = protocol_event;
        #[cfg(not(test))]
        panic!(
            "a production child Document-opened event must have a concrete renderer output sink"
        );
    }

    /// Delivers child Document loads exclusively through FrameOwnerStore's
    /// owner and phase admissions, including synchronous initial about:blank.
    /// DOM insertion only connects the frame; it must not synthesize load on
    /// native nodes or their paired foreign reflectors.
    fn run_child_browsing_context_load_delivery(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task: FrameDocumentLoadDeliveryTask,
    ) -> ChildFrameLoadDeliveryResult {
        let mut task = task;
        let mut result = ChildFrameLoadDeliveryResult::default();
        if moli_trace::window_message_trace_enabled() {
            let url = self
                .child_browsing_context_current_url(task.child_handle)
                .map(|url| url.to_string());
            tracing::info!(
                target: "moli_window_message_trace",
                child_handle = ?task.child_handle,
                url = ?url,
                stage = "child_host_load_dispatch_start",
            );
        }
        loop {
            let Some(action) = self
                .frame_owner_store
                .begin_current_child_document_load_delivery(task)
            else {
                return result;
            };
            let phase_result = match action.phase() {
                FrameDocumentLoadDeliveryPhase::WindowLoad => {
                    self.dispatch_child_window_load_phase(scope, action)
                }
                FrameDocumentLoadDeliveryPhase::OwnerElementLoad => {
                    self.dispatch_child_owner_element_load_phase(scope, host_ptr, action)
                }
                FrameDocumentLoadDeliveryPhase::PageShow => {
                    self.dispatch_child_pageshow_phase(scope, action)
                }
                FrameDocumentLoadDeliveryPhase::FrameFinish => {
                    ChildFrameLoadDeliveryPhaseResult::without_callback(
                        self.frame_owner_store
                            .finish_current_child_document_load_delivery(action),
                    )
                }
            };
            result.callback_dispatched |= phase_result.callback_dispatched;
            let progress = phase_result.progress;
            match progress {
                Some(FrameDocumentLoadDeliveryProgress::Continue(next_task)) => {
                    task = next_task;
                }
                Some(FrameDocumentLoadDeliveryProgress::AwaitingDescendantCompletion(
                    waiting_task,
                )) => {
                    tracing::debug!(
                        child_handle = ?waiting_task.child_handle,
                        owner = ?waiting_task.owner,
                        "paused child load delivery until exact descendant completion"
                    );
                    return result;
                }
                Some(FrameDocumentLoadDeliveryProgress::Finished(finish)) => {
                    let finished_child_handle = finish.child_handle;
                    let runtime = unsafe { &mut *self.runtime };
                    self.arm_child_meta_refresh_navigation(scope, runtime, finished_child_handle);
                    self.publish_child_frame_load_finish(finish);
                    self.note_lightweight_popup_child_frame_load_finished(finished_child_handle);
                    if moli_trace::window_message_trace_enabled() {
                        tracing::info!(
                            target: "moli_window_message_trace",
                            child_handle = ?task.child_handle,
                            stage = "child_host_load_dispatch_complete",
                        );
                    }
                    crate::window_host::signal_pending_window_message_reconsideration(self);
                    return result;
                }
                None => return result,
            }
        }
    }

    fn dispatch_child_window_load_phase(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentLoadDeliveryAction,
    ) -> ChildFrameLoadDeliveryPhaseResult {
        let handle = action.child_handle();
        if self
            .child_browsing_context_document_handle(handle)
            .is_some_and(|document| self.media_delays_document_load_event(document))
        {
            // The last media load flag queues the current Document's load
            // continuation. Requeuing here would spin while the network waits.
            self.frame_owner_store
                .abort_child_document_load_delivery(action);
            return ChildFrameLoadDeliveryPhaseResult::without_callback(None);
        }
        let performance_window = self.existing_child_browsing_context_window_wrapper(scope, handle);
        let Some(event) = construct_original_event(scope, "load") else {
            self.abort_and_requeue_child_load_delivery(action);
            return ChildFrameLoadDeliveryPhaseResult::without_callback(None);
        };
        if let Some(window) = performance_window {
            record_performance_load_event_start_for_window(scope, window);
        }
        self.dispatch_child_window_event_with_target_override(scope, handle, "load", event, true);
        if let Some(window) = performance_window {
            record_performance_load_event_end_for_window(scope, window);
        }
        let progress = self
            .frame_owner_store
            .finish_current_child_document_load_delivery(action);
        if !self.child_browsing_context_host_load_tail_is_current(action.task()) {
            self.trace_stale_child_load_delivery(action, "window_load");
            return ChildFrameLoadDeliveryPhaseResult::after_callback(None);
        }
        ChildFrameLoadDeliveryPhaseResult::after_callback(progress)
    }

    fn dispatch_child_owner_element_load_phase(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        action: FrameDocumentLoadDeliveryAction,
    ) -> ChildFrameLoadDeliveryPhaseResult {
        let handle = action.child_handle();
        self.record_frame_owner_resource_timing_before_load(scope, action);
        let Some(event) = construct_original_event(scope, "load") else {
            self.abort_and_requeue_child_load_delivery(action);
            return ChildFrameLoadDeliveryPhaseResult::without_callback(None);
        };
        let runtime = unsafe { &mut *self.runtime };
        let _ = runtime.dispatch_public_event_best_effort(
            scope,
            host_ptr,
            EventTargetHandle::Node(handle),
            event,
            "child browsing context host load event",
        );
        let progress = self
            .frame_owner_store
            .finish_current_child_document_load_delivery(action);
        if !self.child_browsing_context_host_load_tail_is_current(action.task()) {
            self.trace_stale_child_load_delivery(action, "owner_element_load");
            return ChildFrameLoadDeliveryPhaseResult::after_callback(None);
        }
        let runtime = unsafe { &mut *self.runtime };
        self.arm_child_meta_refresh_navigation(scope, runtime, handle);
        ChildFrameLoadDeliveryPhaseResult::after_callback(progress)
    }

    fn dispatch_child_pageshow_phase(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentLoadDeliveryAction,
    ) -> ChildFrameLoadDeliveryPhaseResult {
        let callback_dispatched = if let Some(event) =
            construct_original_page_transition_event(scope, "pageshow", false)
        {
            self.dispatch_child_window_event_with_target_override(
                scope,
                action.child_handle(),
                "pageshow",
                event,
                true,
            );
            true
        } else {
            false
        };
        let progress = self
            .frame_owner_store
            .finish_current_child_document_load_delivery(action);
        if !self.child_browsing_context_host_load_tail_is_current(action.task()) {
            self.trace_stale_child_load_delivery(action, "pageshow");
            return ChildFrameLoadDeliveryPhaseResult::new(None, callback_dispatched);
        }
        ChildFrameLoadDeliveryPhaseResult::new(progress, callback_dispatched)
    }

    fn abort_and_requeue_child_load_delivery(&mut self, action: FrameDocumentLoadDeliveryAction) {
        if self
            .frame_owner_store
            .abort_child_document_load_delivery(action)
        {
            let _ = self.queue_ready_child_host_load_task(action.task());
        }
    }

    fn trace_stale_child_load_delivery(
        &self,
        action: FrameDocumentLoadDeliveryAction,
        completed_phase: &'static str,
    ) {
        if moli_trace::window_message_trace_enabled() {
            tracing::info!(
                target: "moli_window_message_trace",
                child_handle = ?action.child_handle(),
                owner = ?action.owner(),
                completed_phase,
                stage = "child_host_load_tail_stale",
            );
        }
    }

    fn publish_child_frame_load_finish(
        &mut self,
        finish: crate::frame_owner_model::FrameDocumentLoadDispatchFinish,
    ) {
        let navigation_snapshot = self.take_child_frame_client_load_finish_snapshot(&finish);
        tracing::debug!(
            child_handle = ?finish.child_handle,
            owner = ?finish.owner,
            frame_id = %finish.frame_id.0,
            document_url = %finish.document_url,
            projected_frame_client_output = navigation_snapshot.is_some(),
            "published typed child frame load finish output"
        );
        if let Some(navigation_snapshot) = navigation_snapshot {
            if let Some(source_document) = self.root_document_lifecycle_identity()
                && self.append_live_turn_owner_action(
                    crate::runtime::RendererOwnerAction::ChildFrameLoad {
                        source_document,
                        event: navigation_snapshot.clone().into_protocol_snapshot(),
                    },
                )
            {
                // The concrete record owns both browser-state application and
                // protocol projection.
            } else {
                #[cfg(test)]
                self.completed_child_browsing_context_loads
                    .push(navigation_snapshot);
                #[cfg(not(test))]
                {
                    let _ = navigation_snapshot;
                    panic!(
                        "a production child-frame load must have a concrete renderer output sink"
                    );
                }
            }
        }
        // Releasing the descendant blocker can make the parent Document's
        // exact Load milestone observable immediately. Persist the child
        // projection first so a protocol observer of that milestone can
        // capture the corresponding frame/navigation/runtime facts without
        // racing a later child-output publication.
        if let Some(completion) = finish.parent_descendant_completion {
            self.reconcile_parent_lifecycle_after_descendant_completion(completion);
        }
    }

    fn take_child_frame_client_load_finish_snapshot(
        &mut self,
        finish: &crate::frame_owner_model::FrameDocumentLoadDispatchFinish,
    ) -> Option<ChildFrameNavigationSnapshot> {
        let security_origin_opaque =
            self.child_browsing_context_has_opaque_origin(finish.child_handle);
        let document_open_replacement = matches!(
            self.frame_owner_store
                .current_child_document_creation_kind(finish.child_handle),
            Some(crate::frame_owner_model::DocumentCreationKind::DocumentOpen { .. })
        );
        let parent_frame_id = self.protocol_child_frame_parent_id(
            finish
                .parent_frame_id
                .as_ref()
                .map(|frame_id| frame_id.0.clone()),
        );
        let entry = self.child_browsing_contexts.get_mut(&finish.child_handle)?;
        let identity = entry.frame_identity_snapshot();
        if identity.frame_id != finish.frame_id.0 {
            tracing::warn!(
                child_handle = ?finish.child_handle,
                owner = ?finish.owner,
                expected_frame_id = %finish.frame_id.0,
                live_frame_id = %identity.frame_id,
                "dropped stale child FrameClient load-finish projection"
            );
            return None;
        }
        Some(ChildFrameNavigationSnapshot {
            frame_id: finish.frame_id.0.clone(),
            // The renderer's reserved main-frame identity is local to the
            // FrameOwner store. Protocol projection replaces `None` with the
            // attached target's real root frame id; leaking the reserved
            // identity here would publish a child with parentId "main".
            parent_frame_id,
            loader_id: entry.current_document_loader_id().map(ToOwned::to_owned),
            name: identity.name,
            url: finish.document_url.to_string(),
            document_open_replacement,
            security_origin_inherited: identity.security_origin_inherited,
            security_origin_opaque,
            document_network: entry.take_completed_document_network_for_owner(finish.owner),
        })
    }

    fn child_browsing_context_host_load_tail_is_current(
        &self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> bool {
        let handle = task.child_handle;
        self.child_host_load_task_owner_is_current(task)
            && self.current_child_navigation_load(handle).is_none()
    }

    pub(crate) fn note_child_meta_refresh_after_mutation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime: &mut DocumentRuntime,
        document: DomHandle,
        navigation: MetaRefreshNavigation,
    ) {
        let Some(handle) = self.child_browsing_context_host_for_document_handle(document) else {
            return;
        };
        // Mutation application already owns the runtime borrow; use it directly
        // rather than borrowing it again through the host's runtime pointer.
        if self
            .child_browsing_contexts
            .get(&handle)
            .is_none_or(|entry| !entry.document_sandbox_allows_scripts())
        {
            return;
        }
        let Some(owner) = self.current_child_document_task_owner(handle) else {
            return;
        };
        if self
            .child_meta_refresh_navigations
            .get(&handle)
            .is_some_and(|task| task.owner != owner)
            && let Some(previous) = self.child_meta_refresh_navigations.remove(&handle)
            && let Some(timer_id) = previous.timer_id
        {
            runtime.cancel_timer(timer_id);
        }
        let task = self
            .child_meta_refresh_navigations
            .entry(handle)
            .or_insert_with(|| ChildMetaRefreshState {
                timer_id: None,
                owner,
                scheduler: MetaRefreshScheduler::default(),
            });
        if task.scheduler.note_candidates([navigation])
            && let Some(timer_id) = task.timer_id.take()
        {
            runtime.cancel_timer(timer_id);
        }
        self.arm_child_meta_refresh_navigation(scope, runtime, handle);
    }

    fn arm_child_meta_refresh_navigation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime: &mut DocumentRuntime,
        handle: DomHandle,
    ) {
        let Some(task) = self.child_meta_refresh_navigations.get(&handle) else {
            return;
        };
        // readyState can become complete before load callbacks are delivered.
        // Keep the captured candidate until the owner element's load finishes.
        if task.timer_id.is_some()
            || !self
                .frame_owner_store
                .current_child_document_owner_load_is_complete(handle, task.owner)
        {
            return;
        }
        let owner = task.owner;
        let Some(window) = self.child_window_proxy_records.live_window(scope, handle) else {
            return;
        };
        let Some(context) = window.get_creation_context(scope) else {
            return;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let data = v8::BigInt::new_from_u64(scope, handle.index() as u64);
        let Some(callback) = v8::Function::builder(child_meta_refresh_callback)
            .data(data.into())
            .build(scope)
        else {
            return;
        };
        let Some(task) = self.child_meta_refresh_navigations.get_mut(&handle) else {
            return;
        };
        // Retrying a failed timer post must keep the scheduler's original deadline.
        let Some(scheduled) = task
            .scheduler
            .finish_load(owner)
            .or_else(|| task.scheduler.armed_navigation())
        else {
            return;
        };
        let delay_ms = u64::try_from(
            scheduled
                .ready_at
                .saturating_duration_since(std::time::Instant::now())
                .as_nanos()
                .div_ceil(1_000_000),
        )
        .unwrap_or(u64::MAX);
        let document_id: v8::Local<v8::Value> =
            v8::BigInt::new_from_u64(scope, scheduled.owner.document_id.0).into();
        let revision: v8::Local<v8::Value> =
            v8::BigInt::new_from_u64(scope, scheduled.revision).into();
        let extra_args = vec![
            v8::Global::new(scope, document_id),
            v8::Global::new(scope, revision),
        ];
        let timer_id = runtime.queue_timeout(
            scope,
            callback,
            delay_ms,
            HostTimerOwner::ChildWindow(handle),
            extra_args,
        );
        if timer_id == 0 {
            return;
        }
        if let Some(task) = self.child_meta_refresh_navigations.get_mut(&handle)
            && task.owner == owner
            && task.timer_id.is_none()
        {
            task.timer_id = Some(timer_id);
        } else {
            runtime.cancel_timer(timer_id);
        }
    }

    pub(in crate::native_bridge::context_host) fn cancel_child_meta_refresh_navigation(
        &mut self,
        handle: DomHandle,
    ) {
        let Some(task) = self.child_meta_refresh_navigations.remove(&handle) else {
            return;
        };
        if let Some(timer_id) = task.timer_id {
            unsafe { &mut *self.runtime }.cancel_timer(timer_id);
        }
    }

    pub(crate) fn run_child_browsing_context_host_load_task_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task: FrameDocumentLoadDeliveryTask,
    ) -> ChildFrameHostLoadTaskOutcome {
        let mut task_outcome = ChildFrameHostLoadTaskOutcome::ConsumedWithoutCallback;
        let handle = task.child_handle;
        if !self.child_browsing_context_is_live(handle) {
            return task_outcome;
        }
        if let Some(window) = self.existing_child_browsing_context_window_wrapper(scope, handle)
            && let Some(document) = self.child_browsing_context_document_wrapper(scope, handle)
        {
            self.install_default_world_state_for_child_window(scope, handle, window, document);
        }
        let delivery = self.run_child_browsing_context_load_delivery(scope, host_ptr, task);
        if delivery.callback_dispatched {
            task_outcome = ChildFrameHostLoadTaskOutcome::CallbackDispatched;
        }
        task_outcome
    }

    pub(crate) fn child_host_load_task_owner_is_current(
        &self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> bool {
        self.frame_owner_store
            .child_document_task_owner_is_current(task.child_handle, task.owner)
    }

    pub(crate) fn child_host_load_task_is_current(
        &self,
        admission: crate::frame_owner_model::FrameDocumentLoadDeliveryAdmission,
    ) -> bool {
        let task = admission.task();
        self.child_browsing_context_host_load_tail_is_current(task)
            && self
                .frame_owner_store
                .current_child_document_load_delivery_is_ready(task.child_handle, task.owner)
            && self
                .frame_owner_store
                .current_child_document_load_delivery_task_is_reserved(admission)
    }

    pub(crate) fn current_child_host_load_target(
        &self,
        expected: crate::page_task_queue::RendererPageChildHostLoadTarget,
    ) -> Option<crate::page_task_queue::RendererPageChildHostLoadTarget> {
        self.child_host_load_task_is_current(expected.admission())
            .then_some(expected)
    }

    pub(crate) fn claim_current_child_host_load_task(
        &mut self,
        admission: crate::frame_owner_model::FrameDocumentLoadDeliveryAdmission,
    ) -> bool {
        self.child_host_load_task_is_current(admission)
            && self
                .frame_owner_store
                .release_current_child_document_load_delivery_task_reservation(admission)
    }

    pub(crate) fn discard_stale_child_host_load_task(
        &mut self,
        admission: crate::frame_owner_model::FrameDocumentLoadDeliveryAdmission,
    ) -> bool {
        self.frame_owner_store
            .release_current_child_document_load_delivery_task_reservation(admission)
    }
}

fn child_meta_refresh_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(handle) = child_meta_refresh_handle_from_data(scope, args.data()) else {
        return;
    };
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    if host
        .child_meta_refresh_navigations
        .get(&handle)
        .is_none_or(|task| task.timer_id.is_none())
    {
        return;
    }
    let Ok(document_id) = v8::Local::<v8::BigInt>::try_from(args.get(0)) else {
        return;
    };
    let (document_id, lossless) = document_id.u64_value();
    if !lossless {
        return;
    }
    let Ok(revision) = v8::Local::<v8::BigInt>::try_from(args.get(1)) else {
        return;
    };
    let (revision, lossless) = revision.u64_value();
    if !lossless {
        return;
    }
    let Some(owner) = host.current_child_document_task_owner(handle) else {
        return;
    };
    let Some(task) = host.child_meta_refresh_navigations.get_mut(&handle) else {
        return;
    };
    if task.owner != owner || owner.document_id.0 != document_id {
        return;
    }
    let Some(navigation) = task.scheduler.take_if_current(owner, revision) else {
        return;
    };
    // Keep the Document's revision counter so a later identical declaration
    // cannot authorize a stale callback from an earlier schedule.
    task.timer_id = None;
    let Some(initiator_url) = host.child_browsing_context_current_url(handle) else {
        return;
    };
    let Some(window) = host.child_browsing_context_window_wrapper(scope, handle) else {
        return;
    };
    let Some(location) = window
        .get(scope, v8str(scope, "location").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    navigate_child_meta_refresh(
        scope,
        location,
        &navigation.url,
        navigation.delay_ms,
        initiator_url,
    );
}

fn child_meta_refresh_handle_from_data(
    _scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (index, lossless) = big.u64_value();
    lossless.then(|| DomHandle::new(index as usize))
}
