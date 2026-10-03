use crate::conn::{BackgroundProtocolEvent, BrowserContext, CdpConnection, CommandOwnerScope};

/// The stable Target identities on both sides of one foreground selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetActivationTransition {
    selected_target_id: String,
    previous_active_target_id: Option<String>,
}

impl TargetActivationTransition {
    pub(crate) fn new(
        selected_target_id: impl Into<String>,
        previous_active_target_id: Option<String>,
    ) -> Self {
        Self {
            selected_target_id: selected_target_id.into(),
            previous_active_target_id,
        }
    }

    fn selected_target_id(&self) -> &str {
        &self.selected_target_id
    }

    fn previous_active_target_id(&self) -> Option<&str> {
        self.previous_active_target_id.as_deref()
    }

    fn deactivated_target_id(&self) -> Option<&str> {
        self.previous_active_target_id()
            .filter(|target_id| *target_id != self.selected_target_id())
    }

    fn changed_active_target(&self) -> bool {
        self.previous_active_target_id() != Some(self.selected_target_id())
    }
}

/// Successful target selection and the Page events caused by its visibility change.
///
/// The events stay attached to the completion so every activation entry point
/// must preserve their position relative to its own response or owner action.
#[derive(Debug)]
pub(crate) struct CompletedTargetActivation {
    protocol_events: Vec<BackgroundProtocolEvent>,
}

impl CompletedTargetActivation {
    fn new(protocol_events: Vec<BackgroundProtocolEvent>) -> Self {
        Self { protocol_events }
    }

    pub(crate) fn into_protocol_events(self) -> Vec<BackgroundProtocolEvent> {
        self.protocol_events
    }
}

impl CdpConnection {
    fn target_document_is_visible(&self, target_id: &str) -> bool {
        self.browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
            .find_map(|context| context.document_activity_for_target(target_id))
            .is_some_and(|activity| activity.visible)
    }
    pub(crate) fn select_browser_focus_for_target(&mut self, target_id: &str) {
        let Some(browser_context_id) = self
            .target_session_route_for_target_id(target_id)
            .and_then(|route| route.browser_context_id().map(str::to_owned))
        else {
            return;
        };
        for context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            context.window_is_focused = context.id == browser_context_id;
        }
    }

    pub(crate) async fn apply_browser_document_activity_async(&mut self) -> anyhow::Result<()> {
        for context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            let activities = context
                .page_targets
                .iter()
                .filter(|target| !target.has_pending_javascript_dialog())
                .map(|target| {
                    (
                        target.target_id().to_owned(),
                        context
                            .document_activity_for_target(target.target_id())
                            .unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            for (target_id, activity) in activities {
                if let Some(page) = context
                    .page_target_mut(&target_id)
                    .and_then(|host| host.runtime_slot.loaded_page_mut())
                    && page.document_activity() != activity
                {
                    // An unchanged page is not a participant in this focus
                    // transition. Do not wait for its renderer to become idle.
                    page.set_document_activity_async(activity).await?;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn restore_browser_focus_after_removal_async(&mut self) {
        if !self
            .browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
            .any(|context| context.window_is_focused && context.active_target_id().is_some())
            && let Some(target_id) = self
                .browser_context
                .iter()
                .chain(self.inactive_browser_contexts.iter())
                .find_map(BrowserContext::active_target_id_owned)
        {
            self.select_browser_focus_for_target(&target_id);
        }
        if let Err(error) = self.apply_browser_document_activity_async().await {
            tracing::warn!(%error, "failed to update document activity after target removal");
        }
    }

    /// Completes the renderer-surface half of a foreground transition that was
    /// staged synchronously while creating a new target.
    pub(crate) async fn complete_staged_target_activation_async(
        &mut self,
        transition: &TargetActivationTransition,
    ) -> CompletedTargetActivation {
        if let Err(error) = self.apply_browser_document_activity_async().await {
            tracing::warn!(%error, "failed to update document activity after target creation");
        }
        let Some(previous_target_id) = transition.deactivated_target_id() else {
            return CompletedTargetActivation::new(Vec::new());
        };
        if self.target_document_is_visible(previous_target_id) {
            return CompletedTargetActivation::new(Vec::new());
        }
        let protocol_events = self
            .page_screencast_session_ids_for_target(previous_target_id)
            .into_iter()
            .map(|session_id| {
                BackgroundProtocolEvent::page_screencast_visibility_changed(
                    session_id.as_deref(),
                    false,
                )
            })
            .collect();
        if let Err(error) = self
            .apply_background_target_surface_overrides_async(previous_target_id)
            .await
        {
            tracing::warn!(
                target_id = previous_target_id,
                selected_target_id = transition.selected_target_id(),
                %error,
                "failed to update Page visibility after target activation"
            );
        }
        CompletedTargetActivation::new(protocol_events)
    }

    pub(crate) async fn select_page_target_for_connection_async(
        &mut self,
        target_id: &str,
    ) -> anyhow::Result<Option<CompletedTargetActivation>> {
        self.select_browser_focus_for_target(target_id);
        let was_visible = self.target_document_is_visible(target_id);
        let previous_active_target_id = self
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id_owned);
        let transition = TargetActivationTransition::new(target_id, previous_active_target_id);
        let hidden_screencast_sessions = transition
            .deactivated_target_id()
            .map(|active_target_id| self.page_screencast_session_ids_for_target(active_target_id))
            .unwrap_or_default();
        let Some(browser_context) = self.browser_context.as_mut() else {
            anyhow::bail!("BrowserContextNotLoaded");
        };
        let selected = browser_context.select_page_target_async(target_id).await?;
        if !selected {
            return Ok(None);
        }
        self.refresh_active_browser_context_loader_async().await;
        self.notify_target_host_activated(target_id);
        self.apply_browser_document_activity_async().await?;

        let mut protocol_events = Vec::new();
        if transition.changed_active_target() {
            // Chromium's PageHandler reports RenderWidgetHost visibility only
            // while that attachment has an active screencast. Hide the old
            // surface before exposing the selected one.
            if transition
                .deactivated_target_id()
                .is_some_and(|previous| !self.target_document_is_visible(previous))
            {
                protocol_events.extend(hidden_screencast_sessions.into_iter().map(|session_id| {
                    BackgroundProtocolEvent::page_screencast_visibility_changed(
                        session_id.as_deref(),
                        false,
                    )
                }));
            }
            if !was_visible {
                protocol_events.extend(
                    self.page_screencast_session_ids_for_target(target_id)
                        .into_iter()
                        .map(|session_id| {
                            BackgroundProtocolEvent::page_screencast_visibility_changed(
                                session_id.as_deref(),
                                true,
                            )
                        }),
                );
            }
        }
        Ok(Some(CompletedTargetActivation::new(protocol_events)))
    }

    pub(crate) fn page_screencast_session_ids_for_target(
        &mut self,
        target_id: &str,
    ) -> Vec<Option<String>> {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return Vec::new();
        };
        let owner = CommandOwnerScope::for_route(route);
        self.page_event_session_ids_for_owner(&owner)
            .into_iter()
            .filter(|session_id| {
                let event_owner = session_id
                    .as_deref()
                    .map(CommandOwnerScope::for_session)
                    .unwrap_or_else(|| owner.clone());
                self.target_page_session_state_for_owner(&event_owner)
                    .is_some_and(|state| state.page_screencast.is_active())
            })
            .collect()
    }
}
