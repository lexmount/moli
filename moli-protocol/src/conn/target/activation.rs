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
    /// Completes the renderer-surface half of a foreground transition that was
    /// staged synchronously while creating a new target.
    pub(crate) async fn complete_staged_target_activation_async(
        &mut self,
        transition: &TargetActivationTransition,
    ) -> CompletedTargetActivation {
        let Some(previous_target_id) = transition.deactivated_target_id() else {
            return CompletedTargetActivation::new(Vec::new());
        };
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

        let mut protocol_events = Vec::new();
        if transition.changed_active_target() {
            // Chromium's PageHandler reports RenderWidgetHost visibility only
            // while that attachment has an active screencast. Hide the old
            // surface before exposing the selected one.
            protocol_events.extend(hidden_screencast_sessions.into_iter().map(|session_id| {
                BackgroundProtocolEvent::page_screencast_visibility_changed(
                    session_id.as_deref(),
                    false,
                )
            }));
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
