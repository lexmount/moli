use std::collections::BTreeMap;

use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::style_engine::StyleViewport;

#[derive(Clone, Copy)]
pub(crate) struct RealtimeEnvironmentNotification {
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) window_resize: bool,
    pub(crate) visual_resize: bool,
}

struct RealtimeMediaQuery {
    target: WindowDocumentTaskTarget,
    media: String,
    matches: bool,
    listening: bool,
    queued: bool,
}

/// Native sampling state only. The existing realm-local MQL registry retains
/// wrappers and listeners; no V8 values, timers or second layout tree live here.
#[derive(Default)]
pub(super) struct RealtimeEnvironmentState {
    next_query: u64,
    queries: BTreeMap<u64, RealtimeMediaQuery>,
    notifications: Vec<RealtimeEnvironmentNotification>,
    // Last observed screen sizes outlive geometry-cache eviction. Rebuilding
    // the same viewport must not fabricate another resize notification.
    frame_viewports: Vec<(WindowDocumentTaskTarget, (u32, u32))>,
}

impl RealtimeEnvironmentState {
    pub(super) fn has_pending(&self) -> bool {
        !self.notifications.is_empty() || self.queries.values().any(|query| query.queued)
    }

    pub(super) fn rebind(&mut self, retired: WindowDocumentOwner, current: WindowDocumentOwner) {
        for target in self
            .queries
            .values_mut()
            .map(|query| &mut query.target)
            .chain(
                self.notifications
                    .iter_mut()
                    .map(|pending| &mut pending.target),
            )
            .chain(self.frame_viewports.iter_mut().map(|(target, _)| target))
        {
            if target.owner() == retired {
                *target = WindowDocumentTaskTarget::new(current, target.dispatch_scope());
            }
        }
    }

    pub(super) fn retain(&mut self, mut current: impl FnMut(WindowDocumentTaskTarget) -> bool) {
        self.queries.retain(|_, query| current(query.target));
        self.notifications.retain(|pending| current(pending.target));
        self.frame_viewports.retain(|(target, _)| current(*target));
    }
}

impl JsContextHost {
    pub(super) fn sample_realtime_frame_viewport(
        &self,
        target: WindowDocumentTaskTarget,
        size: (u32, u32),
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let changed = {
            let mut environment = state.environment.borrow_mut();
            if let Some((_, previous)) = environment
                .frame_viewports
                .iter_mut()
                .find(|(sampled, _)| *sampled == target)
            {
                let changed = *previous != size;
                *previous = size;
                changed
            } else {
                environment.frame_viewports.push((target, size));
                size != (0, 0)
            }
        };
        if changed {
            self.queue_realtime_environment_notification(target);
            self.sample_realtime_media_queries(Some(target));
        }
    }

    pub(crate) fn has_realtime_environment_notifications(&self) -> bool {
        self.realtime_layout_state
            .as_ref()
            .is_some_and(|state| state.environment.borrow().has_pending())
    }
    pub(crate) fn register_realtime_media_query(
        &self,
        target: WindowDocumentTaskTarget,
        media: String,
        matches: bool,
    ) -> anyhow::Result<u64> {
        let state = self.realtime_layout_state.as_ref().expect("Realtime MQL");
        let mut environment = state.environment.borrow_mut();
        let id = environment
            .next_query
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("media query identity exhausted"))?;
        environment.next_query = id;
        environment.queries.insert(
            id,
            RealtimeMediaQuery {
                target,
                media,
                matches,
                listening: false,
                queued: false,
            },
        );
        Ok(id)
    }

    pub(crate) fn set_realtime_media_query_listening(&self, id: u64, listening: bool) {
        if let Some(state) = &self.realtime_layout_state
            && let Some(query) = state.environment.borrow_mut().queries.get_mut(&id)
        {
            query.listening = listening;
        }
    }

    pub(crate) fn realtime_media_query_matches(&self, id: u64) -> bool {
        self.realtime_layout_state.as_ref().is_some_and(|state| {
            state
                .environment
                .borrow()
                .queries
                .get(&id)
                .is_some_and(|query| {
                    self.window_document_owner_is_current_for_dispatch_scope(
                        query.target.owner(),
                        query.target.dispatch_scope(),
                    ) && query.matches
                })
        })
    }

    /// Sampling happens when an actual screen input is committed, including a
    /// synchronous layout. A round trip can enqueue a change even if the final
    /// match equals the previous frame's match. Sampling never calls page JS.
    pub(super) fn sample_realtime_media_queries(&self, target: Option<WindowDocumentTaskTarget>) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let samples = state
            .environment
            .borrow()
            .queries
            .iter()
            .filter(|(_, query)| target.is_none_or(|target| target == query.target))
            .filter(|(_, query)| {
                self.window_document_owner_is_current_for_dispatch_scope(
                    query.target.owner(),
                    query.target.dispatch_scope(),
                )
            })
            .map(|(id, query)| {
                let viewport = self.realtime_environment_viewport(query.target);
                (
                    *id,
                    crate::context_bootstrap::evaluate_match_media_query_list_with_viewport(
                        &query.media,
                        Some(self.emulated_media()),
                        viewport,
                    ),
                )
            })
            .collect::<Vec<_>>();
        {
            let mut environment = state.environment.borrow_mut();
            for (id, matches) in samples {
                let query = environment.queries.get_mut(&id).expect("sampled query");
                if matches != query.matches {
                    query.matches = matches;
                    query.queued |= query.listening;
                }
            }
        }
        self.refresh_realtime_rendering_opportunity();
    }

    fn realtime_environment_viewport(&self, target: WindowDocumentTaskTarget) -> StyleViewport {
        match target.dispatch_scope() {
            OwnerDispatchScope::Child(frame) => {
                super::super::element::iframe_handle_viewport(self, frame)
                    .unwrap_or_else(|| self.style_viewport())
            }
            _ => self.style_viewport(),
        }
    }

    pub(super) fn queue_realtime_environment_notification(&self, target: WindowDocumentTaskTarget) {
        if self.realtime_layout_state.is_none() {
            return;
        }
        if self
            .current_registered_window_execution_context_identity(target.dispatch_scope())
            .is_none()
        {
            return;
        }
        self.restore_realtime_environment_notification(RealtimeEnvironmentNotification {
            target,
            window_resize: true,
            visual_resize: true,
        });
    }

    pub(crate) fn restore_realtime_environment_notification(
        &self,
        notification: RealtimeEnvironmentNotification,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        if !self.window_document_owner_is_current_for_dispatch_scope(
            notification.target.owner(),
            notification.target.dispatch_scope(),
        ) {
            return;
        }
        if notification.window_resize || notification.visual_resize {
            let mut environment = state.environment.borrow_mut();
            if let Some(pending) = environment
                .notifications
                .iter_mut()
                .find(|p| p.target == notification.target)
            {
                pending.window_resize |= notification.window_resize;
                pending.visual_resize |= notification.visual_resize;
            } else {
                environment.notifications.push(notification);
            }
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(crate) fn take_realtime_resize_notification(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> Option<RealtimeEnvironmentNotification> {
        let mut environment = self
            .realtime_layout_state
            .as_ref()?
            .environment
            .borrow_mut();
        let index = environment
            .notifications
            .iter()
            .position(|p| p.target == target)?;
        Some(environment.notifications.remove(index))
    }

    pub(crate) fn restore_realtime_media_query_notifications(
        &self,
        target: WindowDocumentTaskTarget,
        ids: &[u64],
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let mut environment = state.environment.borrow_mut();
        for id in ids {
            if let Some(query) = environment.queries.get_mut(id)
                && query.target == target
            {
                query.queued = true;
            }
        }
    }

    /// Snapshot a Document's event batch before dispatch. Mutations during a
    /// callback can queue the same list again for the next media phase.
    pub(crate) fn take_realtime_media_query_notifications(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> Vec<u64> {
        let Some(state) = &self.realtime_layout_state else {
            return Vec::new();
        };
        state
            .environment
            .borrow_mut()
            .queries
            .iter_mut()
            .filter_map(|(id, query)| {
                (query.target == target && std::mem::take(&mut query.queued)).then_some(*id)
            })
            .collect()
    }

    pub(super) fn prune_realtime_environment(&self) {
        if let Some(state) = &self.realtime_layout_state {
            state.environment.borrow_mut().retain(|target| {
                self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
            });
        }
    }
}
