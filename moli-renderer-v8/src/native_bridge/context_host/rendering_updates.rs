use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::{
    document_runtime::EventTargetHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    page_task_queue::{RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PendingRenderingUpdatePayload {
    DocumentScrollEvents,
    AnimationStartScan(EventTargetHandle),
    /// Flush the main Document's autofocus candidates after DOMContentLoaded.
    /// The candidate is intentionally resolved at execution time.
    PostParseAutofocus,
    EnvironmentChange(PendingEnvironmentChange),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PendingEnvironmentChange {
    pub(super) previous_viewport: crate::style_engine::StyleViewport,
    pub(super) previous_activity: moli_page_types::DocumentActivity,
    pub(super) previous_orientation: moli_page_types::ScreenOrientation,
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
            if self
                .rendering_updates
                .find_slot_index(
                    target,
                    RendererPageRenderingUpdateTaskKind::EnvironmentChange,
                    |_| true,
                )
                .is_some()
            {
                continue;
            }
            let previous_viewport = match dispatch_scope {
                OwnerDispatchScope::Child(handle) => {
                    super::super::element::iframe_handle_viewport(self, handle)
                        .unwrap_or_else(|| self.style_viewport())
                }
                _ => self.style_viewport(),
            };
            self.queue_rendering_update(
                target,
                RendererPageRenderingUpdateTaskKind::EnvironmentChange,
                PendingRenderingUpdatePayload::EnvironmentChange(PendingEnvironmentChange {
                    previous_viewport,
                    previous_activity: self.document_activity(),
                    previous_orientation: self
                        .viewport_surface()
                        .unwrap_or_default()
                        .screen_orientation,
                }),
            );
        }
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
        let context_scope = &mut v8::ContextScope::new(scope, resolved.context);
        let previous_scope = target.dispatch_scope().enter(context_scope);
        let global = context_scope.get_current_context().global(context_scope);
        let current_viewport =
            crate::context_bootstrap::current_window_style_viewport(context_scope, self);
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

            let resized = change.previous_viewport.width != current_viewport.width
                || change.previous_viewport.height != current_viewport.height;
            if resized {
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
                            window_target,
                            event,
                            "window resize event",
                        )
                        .is_ok();
                }

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
            dispatched |=
                crate::context_bootstrap::dispatch_media_query_list_change_events(context_scope);
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
            if change.previous_activity != current_activity {
                if change.previous_activity.visible != current_activity.visible
                    && let Ok(document) = crate::host::event_target_value(
                        context_scope,
                        host_ptr,
                        EventTargetHandle::Node(resolved.document_handle),
                    )
                    && let Ok(event) = crate::host::create_host_event(
                        context_scope,
                        "visibilitychange",
                        document,
                        document,
                        true,
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
                if !self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                ) {
                    return dispatched;
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
                                window_target,
                                event,
                                "window focus state event",
                            )
                            .is_ok();
                    }
                }
            }
            dispatched
        })();
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
