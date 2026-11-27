use super::{PageScriptHost, WindowDocumentOwner, WindowDocumentTaskTarget};

#[derive(Clone, Copy)]
pub(crate) struct RealtimeEnvironmentNotification {
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) window_resize: bool,
    pub(crate) visual_resize: bool,
}

#[derive(Default)]
pub(super) struct RealtimeEnvironmentState {
    notifications: Vec<RealtimeEnvironmentNotification>,
    // Last observed screen sizes outlive geometry-cache eviction. Rebuilding
    // the same viewport must not fabricate another resize notification.
    frame_viewports: Vec<(WindowDocumentTaskTarget, (u32, u32))>,
}

impl RealtimeEnvironmentState {
    pub(super) fn has_pending(&self) -> bool {
        !self.notifications.is_empty()
    }

    pub(super) fn rebind(&mut self, retired: WindowDocumentOwner, current: WindowDocumentOwner) {
        for target in self
            .notifications
            .iter_mut()
            .map(|pending| &mut pending.target)
            .chain(self.frame_viewports.iter_mut().map(|(target, _)| target))
        {
            if target.owner() == retired {
                *target = WindowDocumentTaskTarget::new(current, target.dispatch_scope());
            }
        }
    }

    pub(super) fn retain(&mut self, mut current: impl FnMut(WindowDocumentTaskTarget) -> bool) {
        self.notifications.retain(|pending| current(pending.target));
        self.frame_viewports.retain(|(target, _)| current(*target));
    }
}

impl PageScriptHost {
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
            self.queue_realtime_environment_notification(target, true, true);
            self.sample_realtime_media_queries(Some(target));
        }
    }

    pub(crate) fn has_realtime_environment_notifications(&self) -> bool {
        self.realtime_layout_state
            .as_ref()
            .is_some_and(|state| state.environment.borrow().has_pending())
            || self.has_media_query_notifications()
    }

    /// Realtime commits sample immediately; other policies defer sampling to
    /// their existing environment task, coalescing changes until that turn.
    pub(super) fn sample_realtime_media_queries(&self, target: Option<WindowDocumentTaskTarget>) {
        if self.layout_policy().is_realtime() {
            self.sample_media_queries(target);
            self.refresh_realtime_rendering_opportunity();
        }
    }

    pub(super) fn queue_realtime_environment_notification(
        &self,
        target: WindowDocumentTaskTarget,
        window_resize: bool,
        visual_resize: bool,
    ) {
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
            window_resize,
            visual_resize,
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
