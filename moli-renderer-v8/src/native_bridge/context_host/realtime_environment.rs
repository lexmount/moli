use std::{collections::BTreeMap, rc::Rc};

use super::{
    PageScriptHost, RuntimeObservableContextToken, WindowDocumentOwner, WindowDocumentTaskTarget,
    WindowExecutionContextIdentity, WindowExecutionContextOwner,
};

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
    queued: bool,
    execution_context: WindowExecutionContextIdentity,
    list: Rc<v8::Weak<v8::Object>>,
}

/// An owned candidate from one Document's frozen media notification batch.
/// Upgrade the weak list only after releasing the environment-state borrow.
pub(crate) struct RealtimeMediaQueryNotification {
    pub(crate) id: u64,
    pub(crate) media: String,
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) list: Rc<v8::Weak<v8::Object>>,
}

/// Native sampling of subscribed media query lists. The realm-local listener
/// registry roots active lists; this state holds weak handles to their wrappers.
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
    }

    pub(crate) fn allocate_realtime_media_query_id(&self) -> anyhow::Result<u64> {
        let state = self.realtime_layout_state.as_ref().expect("Realtime MQL");
        let mut environment = state.environment.borrow_mut();
        environment.next_query = environment
            .next_query
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("media query identity exhausted"))?;
        Ok(environment.next_query)
    }

    pub(crate) fn register_realtime_media_query<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: u64,
        target: WindowDocumentTaskTarget,
        media: String,
        matches: bool,
        list: v8::Local<'s, v8::Object>,
    ) -> anyhow::Result<()> {
        let context = list
            .get_creation_context(scope)
            .ok_or_else(|| anyhow::anyhow!("media query list has no creation context"))?;
        let execution_context = self
            .window_execution_context_identity_for_v8_context(scope, context)
            .ok_or_else(|| anyhow::anyhow!("media query list has no registered Realm"))?;
        anyhow::ensure!(
            self.window_execution_context_identity_is_current(execution_context)
                && execution_context.dispatch_scope() == target.dispatch_scope()
                && self.current_window_execution_context_owner(target.dispatch_scope())
                    == Some(execution_context.owner())
                && self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                ),
            "media query list belongs to a retired Window document"
        );
        // Creating/upgrading V8 weak handles must not hold a state borrow.
        let list = Rc::new(v8::Weak::new(scope, list));
        let state = self.realtime_layout_state.as_ref().expect("Realtime MQL");
        state.environment.borrow_mut().queries.insert(
            id,
            RealtimeMediaQuery {
                target,
                media,
                matches,
                queued: false,
                execution_context,
                list,
            },
        );
        Ok(())
    }

    pub(crate) fn realtime_media_query_is_current(
        &self,
        target: WindowDocumentTaskTarget,
        id: u64,
    ) -> bool {
        self.realtime_layout_state.as_ref().is_some_and(|state| {
            state
                .environment
                .borrow()
                .queries
                .get(&id)
                .is_some_and(|query| {
                    query.target == target
                        && self.window_document_owner_is_current_for_dispatch_scope(
                            target.owner(),
                            target.dispatch_scope(),
                        )
                        && self
                            .window_execution_context_identity_is_current(query.execution_context)
                })
        })
    }

    pub(crate) fn retire_realtime_media_query(&self, id: u64) {
        if let Some(state) = &self.realtime_layout_state {
            state.environment.borrow_mut().queries.remove(&id);
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(super) fn retire_realtime_media_query_owner(&self, owner: WindowExecutionContextOwner) {
        if let Some(state) = &self.realtime_layout_state {
            state
                .environment
                .borrow_mut()
                .queries
                .retain(|_, query| query.execution_context.owner() != owner);
        }
        self.refresh_realtime_rendering_opportunity();
    }

    pub(super) fn retire_realtime_media_query_realm(&self, token: RuntimeObservableContextToken) {
        if let Some(state) = &self.realtime_layout_state {
            state
                .environment
                .borrow_mut()
                .queries
                .retain(|_, query| query.execution_context.realm_token() != token);
        }
        self.refresh_realtime_rendering_opportunity();
    }

    #[cfg(test)]
    pub(crate) fn realtime_media_query_count_for_test(&self) -> usize {
        self.realtime_layout_state
            .as_ref()
            .map_or(0, |state| state.environment.borrow().queries.len())
    }

    pub(crate) fn realtime_media_query_matches(&self, id: u64) -> Option<bool> {
        let state = self.realtime_layout_state.as_ref()?;
        let environment = state.environment.borrow();
        let query = environment.queries.get(&id)?;
        Some(
            self.window_document_owner_is_current_for_dispatch_scope(
                query.target.owner(),
                query.target.dispatch_scope(),
            ) && self.window_execution_context_identity_is_current(query.execution_context)
                && query.matches,
        )
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
                ) && self.window_execution_context_identity_is_current(query.execution_context)
            })
            .map(|(id, query)| {
                let matches = self
                    .window_document_handle_for_dispatch_scope(query.target.dispatch_scope())
                    .is_some_and(|document| {
                        self.media_query_matches_for_document(document, &query.media)
                    });
                (*id, matches)
            })
            .collect::<Vec<_>>();
        {
            let mut environment = state.environment.borrow_mut();
            for (id, matches) in samples {
                let query = environment.queries.get_mut(&id).expect("sampled query");
                if matches != query.matches {
                    query.matches = matches;
                    query.queued = true;
                }
            }
        }
        self.refresh_realtime_rendering_opportunity();
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
                && self.window_execution_context_identity_is_current(query.execution_context)
                && self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
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
    ) -> Vec<RealtimeMediaQueryNotification> {
        let Some(state) = &self.realtime_layout_state else {
            return Vec::new();
        };
        state
            .environment
            .borrow_mut()
            .queries
            .iter_mut()
            .filter_map(|(id, query)| {
                if query.target == target && std::mem::take(&mut query.queued) {
                    Some(RealtimeMediaQueryNotification {
                        id: *id,
                        media: query.media.clone(),
                        execution_context: query.execution_context,
                        list: Rc::clone(&query.list),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub(super) fn prune_realtime_environment(&self) {
        if let Some(state) = &self.realtime_layout_state {
            let mut environment = state.environment.borrow_mut();
            environment.retain(|target| {
                self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
            });
            environment.queries.retain(|_, query| {
                self.window_execution_context_identity_is_current(query.execution_context)
            });
        }
    }
}
