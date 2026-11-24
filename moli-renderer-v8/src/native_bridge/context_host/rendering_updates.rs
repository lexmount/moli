use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{OwnerDispatchScope, PageScriptHost, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::{
    document_runtime::{DomHandle, EventTargetHandle},
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PendingRenderingUpdatePayload {
    DocumentReveal,
    DocumentScrollEvents,
    AnimationStartScan(EventTargetHandle),
    SvgAnimationEvents(crate::document_runtime::DomHandle),
    /// Flush a top-level Document's autofocus candidates after DOMContentLoaded.
    /// The candidate is intentionally resolved at execution time.
    PostParseAutofocus,
    AnimationFrameCallbacks(super::WindowExecutionContextOwner),
    IntersectionObserverDelivery(super::WindowExecutionContextOwner),
    EnvironmentChange(PendingEnvironmentChange),
    NonLayoutEnvironmentChange(PendingEnvironmentChange),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PendingEnvironmentChange {
    pub(super) previous_window_size: (Option<f64>, Option<f64>),
    pub(super) previous_visual_viewport: Option<moli_page_types::MobileViewport>,
    pub(super) previous_activity: moli_page_types::DocumentActivity,
    pub(super) previous_orientation: moli_page_types::ScreenOrientation,
    dispatch_focus_events: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostParseAutofocusAdmission {
    /// The exact lifecycle owner was replaced before admission.
    StaleOwner,
    /// Autofocus is processed or no candidates remain.
    NotNeeded,
    /// One exact rendering-update entry owns the pending flush.
    Published,
    /// The Page rendering route retired before the entry could be stored.
    RouteClosed,
}

pub(super) type RenderingUpdateState = ExactWindowDocumentTaskLedger<
    RendererPageRenderingUpdateTaskId,
    RendererPageRenderingUpdateTaskKind,
    PendingRenderingUpdatePayload,
>;

pub(super) fn coalesce_animation_start_scan<I: Copy + Eq, P: PartialEq>(
    updates: &mut ExactWindowDocumentTaskLedger<I, RendererPageRenderingUpdateTaskKind, P>,
    target: WindowDocumentTaskTarget,
    event_target: EventTargetHandle,
    window_scan: P,
) -> bool {
    let kind = RendererPageRenderingUpdateTaskKind::AnimationStartScan;
    let scans_document = event_target == EventTargetHandle::Window;
    let Some(index) = updates.find_slot_index(target, kind, |pending| {
        scans_document || pending == &window_scan
    }) else {
        return false;
    };
    if scans_document {
        // Keep the earliest queued opportunity. Its document scan covers every
        // queued element scan owned by this exact document.
        *updates.payload_at_mut(index) = window_scan;
        let keep_id = updates.at(index).task_id();
        updates.retain(|pending| {
            pending.target() != target || pending.kind() != kind || pending.task_id() == keep_id
        });
    }
    true
}

impl PageScriptHost {
    /// Request an initial opportunity from a completed Document lifecycle, not
    /// from Window registration. Native rendering work never acquires a timer.
    pub(crate) fn queue_document_reveal_for_dispatch_scope(
        &mut self,
        dispatch_scope: OwnerDispatchScope,
    ) -> bool {
        // Standalone DOM hosts have no Page rendering source.
        if self.page_task_capabilities.get().is_none() {
            return false;
        }
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
        else {
            return false;
        };
        let Some(document) = self.current_rendering_update_document(target) else {
            return false;
        };
        if !self.document_needs_reveal(document) {
            return false;
        }
        if self.rendering_updates.contains_target(target) {
            return true;
        }
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::DocumentReveal,
            PendingRenderingUpdatePayload::DocumentReveal,
        )
    }

    pub(crate) fn queue_visible_document_reveals(&mut self) {
        for dispatch_scope in self.rendering_update_document_scopes() {
            self.queue_document_reveal_for_dispatch_scope(dispatch_scope);
        }
    }

    pub(crate) fn queue_document_intersection_delivery(
        &mut self,
        target: WindowDocumentTaskTarget,
        owner: super::WindowExecutionContextOwner,
    ) {
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::IntersectionObserverDelivery,
            PendingRenderingUpdatePayload::IntersectionObserverDelivery(owner),
        );
    }

    /// Publish post-parse autofocus as a rendering update for the exact main
    /// Document after DOMContentLoaded or a subsequent candidate insertion.
    ///
    /// The DOMContentLoaded completion caller admits after its listeners and
    /// microtasks. Re-checking the exact owner here prevents a listener's
    /// `document.open()` from retargeting old lifecycle work to the replacement
    /// Document.
    pub(crate) fn queue_main_document_post_parse_autofocus(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> PostParseAutofocusAdmission {
        if !self.main_document_task_owner_is_current(owner) {
            return PostParseAutofocusAdmission::StaleOwner;
        }
        let target = WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(owner),
            OwnerDispatchScope::Top,
        );
        self.queue_document_post_parse_autofocus(target, self.document_handle())
    }

    pub(super) fn queue_document_post_parse_autofocus(
        &mut self,
        target: WindowDocumentTaskTarget,
        document: crate::document_runtime::DomHandle,
    ) -> PostParseAutofocusAdmission {
        if !super::super::element::post_parse_autofocus_is_pending(self, document) {
            return PostParseAutofocusAdmission::NotNeeded;
        }
        if self
            .rendering_updates
            .find_slot_index(
                target,
                RendererPageRenderingUpdateTaskKind::DocumentReveal,
                |_| true,
            )
            .is_some()
        {
            return PostParseAutofocusAdmission::Published;
        }
        if self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::PostParseAutofocus,
            PendingRenderingUpdatePayload::PostParseAutofocus,
        ) {
            PostParseAutofocusAdmission::Published
        } else {
            PostParseAutofocusAdmission::RouteClosed
        }
    }

    /// Queue the Document's pending `scroll` and `scrollend` entries into the
    /// stable rendering source. Route closure retires the Host-local payload;
    /// it never falls back to a timer or hidden drain.
    pub(crate) fn queue_document_scroll_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
            PendingRenderingUpdatePayload::DocumentScrollEvents,
        )
    }

    /// Queue one lightweight CSS `animationstart` compatibility scan on the
    /// owning Document's rendering source. Repeated listener registration for
    /// the same event target coalesces until that scan is consumed.
    pub(crate) fn queue_animation_start_scan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        event_target: EventTargetHandle,
        event_type: &(impl crate::event_type::EventTypeKey + ?Sized),
    ) -> bool {
        if !matches!(
            event_type.as_str(),
            Some("animationstart" | "webkitAnimationStart")
        ) {
            return false;
        }
        let target = match event_target {
            EventTargetHandle::Window => self.current_window_document_task_target(scope),
            EventTargetHandle::Node(handle) => {
                self.window_document_task_target_for_node(scope, handle)
            }
            EventTargetHandle::ChildWindow(_) | EventTargetHandle::PopupWindow(_) => None,
        };
        let Some(target) = target else {
            return false;
        };
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::AnimationStartScan,
            PendingRenderingUpdatePayload::AnimationStartScan(event_target),
        )
    }

    /// Capture each current Document before changing the native environment.
    /// Repeated updates keep the first viewport/activity baseline; media lists
    /// already retain their own last reported match.
    pub(crate) fn queue_environment_change(&mut self) {
        if self.layout_policy().is_realtime() {
            // Screen commits sample and queue Realtime notifications. They
            // must not also enter the legacy immediate rendering-update FIFO.
            return;
        }
        self.queue_legacy_environment_change(false, None);
    }

    pub(crate) fn queue_non_layout_environment_change(&mut self) {
        self.queue_legacy_environment_change(self.layout_policy().is_realtime(), None);
    }

    pub(crate) fn queue_viewport_change(
        &mut self,
        viewport: Option<crate::protocol_types::ViewportSurface>,
    ) {
        if !self.layout_policy().is_realtime() {
            self.queue_environment_change();
        } else if self
            .viewport_surface()
            .unwrap_or_default()
            .screen_orientation
            != viewport.unwrap_or_default().screen_orientation
        {
            // Like Blink's MiscPlatformAPI orientation task, this is separate
            // from resize/media frame phases. Keep the existing owner route.
            self.queue_non_layout_environment_change();
        }
    }

    fn environment_window_size(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> (Option<f64>, Option<f64>) {
        if let OwnerDispatchScope::Child(handle) = dispatch_scope
            && let Some(viewport) = super::super::element::iframe_handle_viewport(self, handle)
        {
            return (viewport.width, viewport.height);
        }
        let surface = self.viewport_surface().unwrap_or_default();
        (
            Some(f64::from(surface.inner_width)),
            Some(f64::from(surface.inner_height)),
        )
    }

    pub(crate) fn queue_document_activity_change(
        &mut self,
        next: moli_page_types::DocumentActivity,
    ) {
        let focused_document = (self.document_activity().focused != next.focused)
            .then(|| self.focused_document_and_element().0);
        self.queue_legacy_environment_change(self.layout_policy().is_realtime(), focused_document);
    }

    fn queue_legacy_environment_change(
        &mut self,
        exclude_layout_events: bool,
        focused_document: Option<DomHandle>,
    ) {
        let scopes = std::iter::once(OwnerDispatchScope::Top).chain(
            self.child_browsing_context_handles_in_document_order()
                .into_iter()
                .map(OwnerDispatchScope::Child),
        );
        for dispatch_scope in scopes {
            if self
                .current_registered_window_execution_context_identity(dispatch_scope)
                .is_none()
            {
                continue;
            }
            let Some(target) =
                self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
            else {
                continue;
            };
            let dispatch_focus_events = focused_document.is_some_and(|document| {
                self.current_rendering_update_document(target) == Some(document)
            });
            if let Some(index) = self.rendering_updates.find_slot_index(
                target,
                RendererPageRenderingUpdateTaskKind::EnvironmentChange,
                |_| true,
            ) {
                // Only the selected Document owns system focus events. Update
                // that recipient if this activity change coalesces with an
                // environment update already queued for the same Window.
                if focused_document.is_some()
                    && let PendingRenderingUpdatePayload::EnvironmentChange(change)
                    | PendingRenderingUpdatePayload::NonLayoutEnvironmentChange(change) =
                        self.rendering_updates.payload_at_mut(index)
                {
                    change.dispatch_focus_events = dispatch_focus_events;
                }
                continue;
            }
            let change = PendingEnvironmentChange {
                previous_window_size: self.environment_window_size(dispatch_scope),
                previous_visual_viewport: matches!(dispatch_scope, OwnerDispatchScope::Top).then(
                    || {
                        self.viewport_surface()
                            .unwrap_or_default()
                            .visual_viewport_geometry()
                    },
                ),
                previous_activity: self.document_activity(),
                previous_orientation: self
                    .viewport_surface()
                    .unwrap_or_default()
                    .screen_orientation,
                dispatch_focus_events,
            };
            let payload = if exclude_layout_events {
                PendingRenderingUpdatePayload::NonLayoutEnvironmentChange(change)
            } else {
                PendingRenderingUpdatePayload::EnvironmentChange(change)
            };
            self.queue_rendering_update(
                target,
                RendererPageRenderingUpdateTaskKind::EnvironmentChange,
                payload,
            );
        }
    }

    pub(super) fn queue_rendering_update(
        &mut self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
        payload: PendingRenderingUpdatePayload,
    ) -> bool {
        if self.layout_policy().is_realtime()
            && matches!(
                kind,
                RendererPageRenderingUpdateTaskKind::DocumentScrollEvents
                    | RendererPageRenderingUpdateTaskKind::PostParseAutofocus
                    | RendererPageRenderingUpdateTaskKind::AnimationStartScan
            )
        {
            return self.queue_realtime_rendering_update(target, kind, payload);
        }
        if let PendingRenderingUpdatePayload::AnimationStartScan(event_target) = payload
            && coalesce_animation_start_scan(
                &mut self.rendering_updates,
                target,
                event_target,
                PendingRenderingUpdatePayload::AnimationStartScan(EventTargetHandle::Window),
            )
        {
            return true;
        }
        if self
            .rendering_updates
            .find_slot_index(target, kind, |pending| pending == &payload)
            .is_some()
        {
            return true;
        }

        let task_id = self
            .rendering_updates
            .allocate_task_id(RendererPageRenderingUpdateTaskId::from_raw);
        self.rendering_updates
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
        if self
            .page_rendering_update_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        let removed = self.rendering_updates.remove_exact(task_id, target, kind);
        debug_assert_eq!(
            removed.as_ref().map(|pending| pending.payload()),
            Some(&payload)
        );
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            "retired rendering update after stable route closure"
        );
        false
    }

    pub(crate) fn current_pending_rendering_update_task(
        &self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageRenderingUpdateTaskKind,
    )> {
        if matches!(task_id, RendererPageRenderingUpdateTaskId::Frame(_)) {
            let (id, target) = self.current_realtime_rendering_opportunity()?;
            return (id == task_id)
                .then_some((target, RendererPageRenderingUpdateTaskKind::FrameLifecycle));
        }
        let pending = self.rendering_updates.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    /// Consume and apply one update already authorized against its exact
    /// Window/Document owner. Removal happens before dispatch so reentrant work
    /// always receives a distinct subsequent rendering turn.
    pub(crate) fn apply_authorized_rendering_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        task_id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
    ) -> Option<(bool, Option<super::WindowExecutionContextOwner>)> {
        let payload = self
            .rendering_updates
            .remove_exact(task_id, target, kind)?
            .into_payload();
        let animation_owner = match payload {
            PendingRenderingUpdatePayload::AnimationFrameCallbacks(owner) => Some(owner),
            _ => None,
        };
        // Snapshot the eligible Documents before author code. Reveal all of
        // them in frame-tree order before flushing autofocus or running any
        // other rendering phase. Revalidate each captured owner after every
        // earlier Document's listeners.
        let documents = self.document_reveal_targets_for_rendering_update(target);
        let mut invoked = false;
        for &(document_target, _) in &documents {
            invoked |= self.reveal_document_for_rendering_update(scope, host_ptr, document_target);
        }
        for &(document_target, document) in &documents {
            if self.current_rendering_update_document(document_target) == Some(document)
                && self.top_level_document_for_document(document) == Some(document)
                && !super::super::document::document_is_hidden(self, document)
                && super::super::element::post_parse_autofocus_is_pending(self, document)
            {
                invoked |=
                    self.dispatch_authorized_post_parse_autofocus(scope, host_ptr, document_target);
            }
        }
        if self.current_rendering_update_document(target).is_none() {
            self.discard_rendering_update_payload(payload);
            return Some((invoked, animation_owner));
        }
        invoked |= match payload {
            PendingRenderingUpdatePayload::DocumentReveal
            | PendingRenderingUpdatePayload::PostParseAutofocus => false,
            PendingRenderingUpdatePayload::DocumentScrollEvents => {
                self.dispatch_authorized_document_scroll_events(scope, host_ptr, target)
            }
            PendingRenderingUpdatePayload::AnimationStartScan(event_target) => {
                self.dispatch_authorized_animation_start_scan(scope, host_ptr, target, event_target)
            }
            PendingRenderingUpdatePayload::SvgAnimationEvents(document) => {
                self.dispatch_authorized_svg_animation_events(scope, host_ptr, target, document)
            }
            PendingRenderingUpdatePayload::AnimationFrameCallbacks(owner) => {
                self.dispatch_authorized_animation_frame_callbacks(scope, host_ptr, target, owner)
            }
            PendingRenderingUpdatePayload::IntersectionObserverDelivery(owner) => {
                let Some(resolved) =
                    self.resolve_authorized_window_document_task_context(scope, target)
                else {
                    return Some((false, None));
                };
                let scope = &mut v8::ContextScope::new(scope, resolved.context);
                let previous = target.dispatch_scope().enter(scope);
                let watchdog = crate::v8_execution_watchdog::V8ExecutionWatchdog::arm(
                    crate::v8_execution_watchdog::V8ExecutionWatchdogKind::RenderingObservers,
                    scope.thread_safe_handle(),
                    crate::v8_execution_watchdog::SCRIPT_TURN_WATCHDOG_TIMEOUT,
                );
                crate::observer_runtime::deliver_document_intersections(
                    scope, host_ptr, owner, target,
                );
                watchdog.disarm();
                target.dispatch_scope().restore(scope, previous);
                true
            }
            PendingRenderingUpdatePayload::EnvironmentChange(change) => {
                self.dispatch_authorized_environment_change(scope, host_ptr, target, change, true)
            }
            PendingRenderingUpdatePayload::NonLayoutEnvironmentChange(change) => {
                self.dispatch_authorized_environment_change(scope, host_ptr, target, change, false)
            }
        };
        Some((invoked, animation_owner))
    }

    pub(crate) fn can_apply_unobserved_document_reveal(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        self.document_reveal_targets_for_rendering_update(target)
            .into_iter()
            .all(|(target, document)| {
                !self.document_reveal_has_observers(target, document)
                    && !super::super::element::post_parse_autofocus_is_pending(self, document)
            })
    }

    pub(crate) fn apply_unobserved_document_reveal(
        &mut self,
        task_id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> Option<bool> {
        let documents = self.document_reveal_targets_for_rendering_update(target);
        self.rendering_updates.remove_exact(
            task_id,
            target,
            RendererPageRenderingUpdateTaskKind::DocumentReveal,
        )?;
        for (target, document) in documents {
            if self.current_rendering_update_document(target) == Some(document)
                && self.document_needs_reveal(document)
            {
                self.dom_host_mut()
                    .node_mut(document)?
                    .data_mut()
                    .as_document_mut()?
                    .mark_revealed();
            }
        }
        Some(false)
    }

    pub(crate) fn discard_pending_rendering_update_task(
        &mut self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        if matches!(task_id, RendererPageRenderingUpdateTaskId::Frame(_)) {
            return self.discard_realtime_rendering_opportunity(task_id);
        }
        let Some(pending) = self.rendering_updates.remove(task_id) else {
            return false;
        };
        self.discard_rendering_update_payload(pending.into_payload());
        true
    }

    fn discard_rendering_update_payload(&mut self, payload: PendingRenderingUpdatePayload) {
        match payload {
            PendingRenderingUpdatePayload::AnimationFrameCallbacks(owner) => {
                // document.open() rotates the parser/lifecycle incarnation without
                // replacing the Window's animation callback map. Retire the stale
                // rendering entry and admit a new exact-Document entry explicitly.
                self.publish_animation_frame_rendering_update(owner);
            }
            PendingRenderingUpdatePayload::SvgAnimationEvents(document) => {
                self.discard_svg_document_events(document);
            }
            _ => {}
        }
    }

    /// Reveal at the beginning of every actual rendering opportunity. The
    /// intrinsic event's nullable transition is null without an inbound transfer.
    fn reveal_document_for_rendering_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(document_handle) = self.current_rendering_update_document(target) else {
            return false;
        };
        if !self.document_needs_reveal(document_handle) {
            return false;
        }
        let Some(document) = self
            .dom_host_mut()
            .node_mut(document_handle)
            .and_then(|node| node.data_mut().as_document_mut())
        else {
            return false;
        };
        document.mark_revealed();
        if !self.document_reveal_has_observers(target, document_handle) {
            // A WindowProxy facade alone cannot observe this event. Do not
            // materialize an otherwise unobserved initial child realm merely
            // to construct a disposable event wrapper.
            return false;
        }
        let Some((_, context)) = self.resolve_current_rendering_update_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = target.dispatch_scope();
        let previous = dispatch_scope.enter(scope);
        let watchdog = crate::v8_execution_watchdog::V8ExecutionWatchdog::arm(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::LifecycleEvent,
            scope.thread_safe_handle(),
            crate::v8_execution_watchdog::SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        let dispatched = if let Some(event) =
            crate::context_bootstrap::construct_original_page_reveal_event(scope)
        {
            let event_target = match dispatch_scope {
                OwnerDispatchScope::Top => Some(EventTargetHandle::Window),
                OwnerDispatchScope::Child(handle) => self
                    .current_child_window_event_target(handle)
                    .map(EventTargetHandle::ChildWindow),
                OwnerDispatchScope::LightweightPopup(id) => self
                    .current_popup_window_event_target(id)
                    .map(EventTargetHandle::PopupWindow),
            };
            event_target.is_some_and(|event_target| {
                self.dispatch_public_event_best_effort(
                    scope,
                    host_ptr,
                    event_target,
                    event,
                    "document reveal event",
                )
                .is_ok()
            })
        } else {
            false
        };
        if watchdog.disarm() == crate::v8_execution_watchdog::V8ExecutionWatchdogOutcome::TimedOut {
            tracing::warn!("document reveal event exceeded its execution deadline");
        }
        dispatch_scope.restore(scope, previous);
        dispatched
    }

    fn current_rendering_update_document(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> Option<crate::document_runtime::DomHandle> {
        if !self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) {
            return None;
        }
        let document = match target.dispatch_scope() {
            OwnerDispatchScope::Top => Some(self.document_handle()),
            OwnerDispatchScope::Child(handle) => {
                self.child_browsing_context_document_handle(handle)
            }
            OwnerDispatchScope::LightweightPopup(id) => self.lightweight_popup_document_handle(id),
        }?;
        self.document_is_fully_active(document).then_some(document)
    }

    fn document_needs_reveal(&self, document: crate::document_runtime::DomHandle) -> bool {
        !super::super::document::document_is_hidden(self, document)
            && self
                .dom_host()
                .node(document)
                .and_then(|node| node.as_document())
                .is_some_and(|document| !document.has_been_revealed())
    }

    fn document_has_page_reveal_content_attribute(
        &self,
        document: crate::document_runtime::DomHandle,
    ) -> bool {
        self.dom_host()
            .node(document)
            .and_then(|node| node.as_document())
            .and_then(|data| data.body_or_frameset_handle(self.dom_host().dom(), document))
            .and_then(|body| self.dom_host().node(body))
            .and_then(|node| node.as_element())
            .is_some_and(|body| body.attribute("onpagereveal").is_some())
    }

    fn document_reveal_has_observers(
        &self,
        target: WindowDocumentTaskTarget,
        document: crate::document_runtime::DomHandle,
    ) -> bool {
        if self
            .current_registered_window_execution_context_identity(target.dispatch_scope())
            .is_some()
        {
            // Existing realms may own retained/adopted wrappers in another world.
            return true;
        }
        match target.dispatch_scope() {
            OwnerDispatchScope::Top => {
                let event = self.prepare_native_event_dispatch(
                    EventTargetHandle::Window,
                    EventTargetHandle::Window,
                    "pagereveal",
                );
                self.native_event_has_observers(&event)
            }
            OwnerDispatchScope::Child(handle) => {
                self.current_child_window_event_target(handle)
                    .is_some_and(|target| {
                        self.child_window_event_has_observers(target, "pagereveal")
                    })
                    || self.document_has_page_reveal_content_attribute(document)
            }
            OwnerDispatchScope::LightweightPopup(_) => true,
        }
    }

    fn document_reveal_targets_for_rendering_update(
        &self,
        selected: WindowDocumentTaskTarget,
    ) -> Vec<(WindowDocumentTaskTarget, crate::document_runtime::DomHandle)> {
        let mut roots = std::collections::HashSet::new();
        for target in std::iter::once(selected).chain(self.rendering_updates.targets()) {
            if let Some(document) = self.current_rendering_update_document(target)
                && let Some(root) = self.top_level_document_for_document(document)
            {
                roots.insert(root);
            }
        }
        // A top-level rendering opportunity includes its active descendants,
        // including new iframes that have not published a payload of their own.
        let documents = self
            .rendering_update_document_scopes()
            .into_iter()
            .filter_map(|scope| {
                let target = self.current_window_document_task_target_for_dispatch_scope(scope)?;
                let document = self.current_rendering_update_document(target)?;
                let root = self.top_level_document_for_document(document)?;
                (roots.contains(&root)
                    && !super::super::document::document_is_hidden(self, document))
                .then_some((target, document))
            })
            .collect::<Vec<_>>();
        if documents.iter().any(|(_, document)| {
            self.document_needs_reveal(*document)
                || super::super::element::post_parse_autofocus_is_pending(self, *document)
        }) {
            documents
        } else {
            Vec::new()
        }
    }

    fn rendering_update_document_scopes(&self) -> Vec<OwnerDispatchScope> {
        let mut roots = vec![(OwnerDispatchScope::Top, self.document_handle())];
        let mut popups = self
            .lightweight_popup_browsing_contexts
            .keys()
            .copied()
            .collect::<Vec<_>>();
        popups.sort_unstable();
        roots.extend(popups.into_iter().filter_map(|id| {
            self.lightweight_popup_document_handle(id)
                .map(|document| (OwnerDispatchScope::LightweightPopup(id), document))
        }));
        let mut scopes = Vec::new();
        for (root, document) in roots {
            scopes.push(root);
            scopes.extend(
                self.child_browsing_context_handles_in_document_order_for_document(document)
                    .into_iter()
                    .map(OwnerDispatchScope::Child),
            );
        }
        scopes
    }

    pub(super) fn dispatch_authorized_animation_start_scan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
        event_target: EventTargetHandle,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let dispatched = super::super::element::dispatch_animation_start_scan(
            scope,
            host_ptr,
            resolved.document_handle,
            event_target,
        );
        dispatch_scope.restore(scope, previous_scope);
        dispatched
    }

    pub(super) fn dispatch_authorized_post_parse_autofocus(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let focused = super::super::element::process_post_parse_autofocus(
            scope,
            host_ptr,
            resolved.document_handle,
        );
        dispatch_scope.restore(scope, previous_scope);
        focused
    }

    fn dispatch_authorized_environment_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
        change: PendingEnvironmentChange,
        include_layout_events: bool,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let context_scope = &mut v8::ContextScope::new(scope, resolved.context);
        let previous_scope = target.dispatch_scope().enter(context_scope);
        let global = context_scope.get_current_context().global(context_scope);
        let current_window_size = self.environment_window_size(target.dispatch_scope());
        let current_activity = self.document_activity();
        // Resize precedes media reporting. Every listener can replace its
        // Document, so abandon the remaining events as soon as the owner retires.
        let dispatched = (|| {
            let mut dispatched = false;
            let window_target = match target.dispatch_scope() {
                OwnerDispatchScope::Child(handle) => {
                    let Some(child_target) = self.current_child_window_event_target(handle) else {
                        return false;
                    };
                    EventTargetHandle::ChildWindow(child_target)
                }
                _ => EventTargetHandle::Window,
            };

            let resized = change.previous_window_size != current_window_size;
            let visual_resized = change.previous_visual_viewport.map_or(resized, |previous| {
                previous
                    != self
                        .viewport_surface()
                        .unwrap_or_default()
                        .visual_viewport_geometry()
            });
            if include_layout_events
                && resized
                && let Ok(event) = crate::host::create_host_event(
                    context_scope,
                    "resize",
                    global.into(),
                    global.into(),
                    false,
                    false,
                )
            {
                dispatched |= self
                    .dispatch_public_event_best_effort(
                        context_scope,
                        host_ptr,
                        window_target,
                        event,
                        "window resize event",
                    )
                    .is_ok();
            }

            if include_layout_events && visual_resized {
                if !self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                ) {
                    return dispatched;
                }
                dispatched |= crate::context_bootstrap::dispatch_window_visual_viewport_resize(
                    context_scope,
                    global,
                );
            }

            if !self.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            ) {
                return dispatched;
            }
            if include_layout_events {
                dispatched |= crate::context_bootstrap::dispatch_media_query_list_change_events(
                    context_scope,
                );
            }
            if !self.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            ) {
                return dispatched;
            }

            if change.previous_orientation
                != self
                    .viewport_surface()
                    .unwrap_or_default()
                    .screen_orientation
            {
                dispatched |= crate::context_bootstrap::dispatch_screen_orientation_change(
                    context_scope,
                    global,
                );
                if !self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                ) {
                    return dispatched;
                }
            }
            // Losing focus precedes hiding; showing precedes gaining focus.
            // Install each intermediate state before its event so callbacks
            // observe that transition, including callbacks in child Documents.
            let previous = change.previous_activity;
            let transitions = [
                (previous.focused && !current_activity.focused).then_some((
                    "blur",
                    moli_page_types::DocumentActivity {
                        focused: false,
                        visible: previous.visible,
                    },
                )),
                (previous.visible != current_activity.visible).then_some((
                    "visibilitychange",
                    moli_page_types::DocumentActivity {
                        focused: previous.focused && current_activity.focused,
                        visible: current_activity.visible,
                    },
                )),
                (!previous.focused && current_activity.focused)
                    .then_some(("focus", current_activity)),
            ];
            for (event_type, activity) in transitions.into_iter().flatten() {
                if !self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                ) {
                    return dispatched;
                }
                self.set_document_activity(activity);
                if event_type != "visibilitychange" {
                    if change.dispatch_focus_events {
                        dispatched |= super::super::element::dispatch_system_focus_change(
                            context_scope,
                            host_ptr,
                            resolved.document_handle,
                            window_target,
                            activity.focused,
                        );
                    }
                    continue;
                }
                let event_target = EventTargetHandle::Node(resolved.document_handle);
                let Ok(document) =
                    crate::host::event_target_value(context_scope, host_ptr, event_target)
                else {
                    continue;
                };
                if let Ok(event) = crate::host::create_host_event(
                    context_scope,
                    event_type,
                    document,
                    document,
                    true,
                    false,
                ) {
                    dispatched |= self
                        .dispatch_public_event_best_effort(
                            context_scope,
                            host_ptr,
                            event_target,
                            event,
                            "document activity event",
                        )
                        .is_ok();
                }
            }

            dispatched
        })();
        // A listener can retire its Document midway through the transition.
        // The browser's requested final activity still belongs to the Page.
        self.set_document_activity(current_activity);
        target
            .dispatch_scope()
            .restore(context_scope, previous_scope);
        dispatched
    }

    /// Apply an already-authorized Document rendering update. Realm lookup is
    /// resolution only: exact current/stale arbitration happened before this
    /// method was called.
    fn dispatch_authorized_document_scroll_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        self.dispatch_authorized_document_scroll_events_in_current_context(
            scope,
            host_ptr,
            target,
            resolved.document_handle,
        )
    }

    fn dispatch_authorized_document_scroll_events_in_current_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
        document_handle: crate::document_runtime::DomHandle,
    ) -> bool {
        if !crate::window_host::dispatch_document_event_for_handle(
            scope,
            host_ptr,
            document_handle,
            "scroll",
        ) {
            return false;
        }

        // A scroll handler can replace its Document. Never retarget the
        // already-pending `scrollend` entry to that replacement.
        if self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) {
            let _ = crate::window_host::dispatch_document_event_for_handle(
                scope,
                host_ptr,
                document_handle,
                "scrollend",
            );
        }
        true
    }
}
