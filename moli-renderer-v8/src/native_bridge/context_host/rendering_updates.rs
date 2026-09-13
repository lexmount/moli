use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::{
    document_runtime::EventTargetHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind},
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PendingRenderingUpdatePayload {
    DocumentScrollEvents,
    AnimationStartScan(EventTargetHandle),
    /// Flush the main Document's autofocus candidates after DOMContentLoaded.
    /// The candidate is intentionally resolved at execution time.
    PostParseAutofocus,
    EnvironmentChange(PendingEnvironmentChange),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PendingEnvironmentChange {
    pub(super) previous_media: crate::protocol_types::EmulatedMediaOverrides,
    pub(super) previous_viewport: crate::style_engine::StyleViewport,
    pub(super) previous_activity: moli_page_types::DocumentActivity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostParseAutofocusAdmission {
    /// The exact lifecycle owner was replaced before admission.
    StaleOwner,
    /// Focus already exists or the Document has no eligible candidate.
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

impl JsContextHost {
    /// Publish post-parse autofocus as a rendering update for the exact main
    /// Document that just completed DOMContentLoaded.
    ///
    /// DOMContentLoaded listeners and their microtasks run before this
    /// admission. Re-checking the exact owner here prevents a listener's
    /// `document.open()` from retargeting old lifecycle work to the replacement
    /// Document.
    pub(crate) fn queue_main_document_post_parse_autofocus(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> PostParseAutofocusAdmission {
        if !self.main_document_task_owner_is_current(owner) {
            return PostParseAutofocusAdmission::StaleOwner;
        }
        if !super::super::element::post_parse_autofocus_is_pending(self) {
            return PostParseAutofocusAdmission::NotNeeded;
        }
        let target = WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(owner),
            OwnerDispatchScope::Top,
        );
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
    ) -> bool {
        let target = match event_target {
            EventTargetHandle::Window => self.current_window_document_task_target(scope),
            EventTargetHandle::Node(handle) => {
                self.window_document_task_target_for_node(scope, handle)
            }
            EventTargetHandle::ChildWindow(_) => None,
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

    /// Publish one coalesced environment-change task. Native values are
    /// updated synchronously; this payload retains the first snapshot so a
    /// burst of protocol commands produces one observable rendering turn.
    pub(crate) fn queue_environment_change(
        &mut self,
        previous_media: crate::protocol_types::EmulatedMediaOverrides,
        previous_viewport: crate::style_engine::StyleViewport,
        previous_activity: moli_page_types::DocumentActivity,
    ) -> bool {
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(OwnerDispatchScope::Top)
        else {
            return false;
        };
        if self
            .rendering_updates
            .find_slot_index(
                target,
                RendererPageRenderingUpdateTaskKind::EnvironmentChange,
                |_| true,
            )
            .is_some()
        {
            return true;
        }
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::EnvironmentChange,
            PendingRenderingUpdatePayload::EnvironmentChange(PendingEnvironmentChange {
                previous_media,
                previous_viewport,
                previous_activity,
            }),
        )
    }

    fn queue_rendering_update(
        &mut self,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
        payload: PendingRenderingUpdatePayload,
    ) -> bool {
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
        let expected_payload = payload.clone();
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
            Some(&expected_payload)
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
        host_ptr: *mut JsContextHost,
        task_id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageRenderingUpdateTaskKind,
    ) -> Option<bool> {
        let payload = self
            .rendering_updates
            .remove_exact(task_id, target, kind)?
            .into_payload();
        Some(match payload {
            PendingRenderingUpdatePayload::DocumentScrollEvents => {
                self.dispatch_authorized_document_scroll_events(scope, host_ptr, target)
            }
            PendingRenderingUpdatePayload::AnimationStartScan(event_target) => {
                self.dispatch_authorized_animation_start_scan(scope, host_ptr, target, event_target)
            }
            PendingRenderingUpdatePayload::PostParseAutofocus => {
                self.dispatch_authorized_post_parse_autofocus(scope, host_ptr, target)
            }
            PendingRenderingUpdatePayload::EnvironmentChange(change) => {
                self.dispatch_authorized_environment_change(scope, host_ptr, target, change)
            }
        })
    }

    pub(crate) fn discard_pending_rendering_update_task(
        &mut self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        self.rendering_updates.remove(task_id).is_some()
    }

    fn dispatch_authorized_animation_start_scan(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
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

    fn dispatch_authorized_post_parse_autofocus(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let focused = super::super::element::process_post_parse_autofocus(scope, host_ptr);
        dispatch_scope.restore(scope, previous_scope);
        focused
    }

    fn dispatch_authorized_environment_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        change: PendingEnvironmentChange,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        // Environment changes are currently published for the top-level
        // Window. Keeping the exact target in the normal ledger still gives
        // us replacement-safe ownership and leaves child fan-out explicit for
        // a later frame-tree extension.
        if target.dispatch_scope() != OwnerDispatchScope::Top {
            return false;
        }
        let context_scope = &mut v8::ContextScope::new(scope, resolved.context);
        let previous_scope = target.dispatch_scope().enter(context_scope);
        let global = context_scope.get_current_context().global(context_scope);
        let current_media = self.emulated_media().clone();
        let current_viewport = self.style_viewport();
        let current_activity = self.document_activity();
        let mut dispatched = false;

        if change.previous_media != current_media {
            crate::context_bootstrap::dispatch_media_query_list_change_events(
                context_scope,
                &change.previous_media,
                change.previous_viewport,
                &current_media,
                current_viewport,
            );
            dispatched = true;
        }

        if change.previous_viewport != current_viewport {
            crate::context_bootstrap::update_cached_window_visual_viewport_dimensions(
                context_scope,
                global,
                current_viewport
                    .width
                    .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
                current_viewport
                    .height
                    .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
            );
            if let Ok(event) = crate::host::create_host_event(
                context_scope,
                "resize",
                global.into(),
                global.into(),
                false,
                false,
            ) {
                dispatched |= self
                    .dispatch_public_event_best_effort(
                        context_scope,
                        host_ptr,
                        EventTargetHandle::Window,
                        event,
                        "window resize event",
                    )
                    .is_ok();
            }

            if let Some(visual_viewport) = global
                .get(
                    context_scope,
                    crate::util::v8str(context_scope, "visualViewport").into(),
                )
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                && let Ok(event) = crate::host::create_host_event(
                    context_scope,
                    "resize",
                    visual_viewport.into(),
                    visual_viewport.into(),
                    false,
                    false,
                )
            {
                dispatched |= crate::context_bootstrap::dispatch_simple_event_target_event(
                    context_scope,
                    visual_viewport,
                    "__moliVisualViewportListeners",
                    "resize",
                    event,
                );
            }
        }

        if change.previous_activity != current_activity {
            if change.previous_activity.visible != current_activity.visible
                && let Some(document) = global
                    .get(
                        context_scope,
                        crate::util::v8str(context_scope, "document").into(),
                    )
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                && let Ok(event) = crate::host::create_host_event(
                    context_scope,
                    "visibilitychange",
                    document.into(),
                    document.into(),
                    false,
                    false,
                )
            {
                dispatched |= self
                    .dispatch_public_event_best_effort(
                        context_scope,
                        host_ptr,
                        EventTargetHandle::Node(resolved.document_handle),
                        event,
                        "document visibilitychange event",
                    )
                    .is_ok();
            }
            if change.previous_activity.focused != current_activity.focused {
                let event_type = if current_activity.focused {
                    "focus"
                } else {
                    "blur"
                };
                if let Some(event) = super::super::element::construct_focus_event(
                    context_scope,
                    event_type,
                    None,
                    false,
                ) {
                    dispatched |= self
                        .dispatch_public_event_best_effort(
                            context_scope,
                            host_ptr,
                            EventTargetHandle::Window,
                            event,
                            "window focus state event",
                        )
                        .is_ok();
                }
            }
        }
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
        host_ptr: *mut JsContextHost,
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
        host_ptr: *mut JsContextHost,
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
