use moli_core::runtime::NavigationEngine;

use crate::conn::{BackgroundProtocolEvent, BrowserContext, CdpConnection};

/// The stable Target identities on both sides of one foreground selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetActivationTransition {
    promoted_target_id: String,
    previous_active_target_id: Option<String>,
}

impl TargetActivationTransition {
    pub(crate) fn new(
        promoted_target_id: impl Into<String>,
        previous_active_target_id: Option<String>,
    ) -> Self {
        Self {
            promoted_target_id: promoted_target_id.into(),
            previous_active_target_id,
        }
    }

    fn promoted_target_id(&self) -> &str {
        &self.promoted_target_id
    }

    fn previous_active_target_id(&self) -> Option<&str> {
        self.previous_active_target_id.as_deref()
    }

    fn demoted_target_id(&self) -> Option<&str> {
        self.previous_active_target_id()
            .filter(|target_id| *target_id != self.promoted_target_id())
    }

    fn changed_active_target(&self) -> bool {
        self.previous_active_target_id() != Some(self.promoted_target_id())
    }
}

/// Successful target selection and the Page events caused by its surface move.
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
        let Some(demoted_target_id) = transition.demoted_target_id() else {
            return CompletedTargetActivation::new(Vec::new());
        };
        let protocol_events = self
            .page_screencast_session_ids_for_target(demoted_target_id)
            .into_iter()
            .map(|session_id| {
                BackgroundProtocolEvent::page_screencast_visibility_changed(
                    session_id.as_deref(),
                    false,
                )
            })
            .collect();
        if let Err(error) = self
            .apply_parked_target_surface_overrides_async(demoted_target_id)
            .await
        {
            tracing::warn!(
                target_id = demoted_target_id,
                promoted_target_id = transition.promoted_target_id(),
                %error,
                "failed to update Page visibility after target activation"
            );
        }
        CompletedTargetActivation::new(protocol_events)
    }

    pub(crate) fn handoff_navigation_engine_for_target_activation(&mut self, target_id: &str) {
        let Some(browser_context) = self.browser_context.as_ref() else {
            return;
        };
        let browser_context_id = browser_context.id.clone();
        let renderer_runtime = browser_context.renderer_runtime_owner_access();
        let navigation_runtime_config = self.engine.runtime_config();
        let promoted_key = (browser_context_id.clone(), target_id.to_owned());
        let promoted_engine = self
            .retained_background_navigation_engines
            .remove(&promoted_key);

        let active_target_id = browser_context.active_target_id().map(str::to_owned);
        let active_has_loaded_page = browser_context.has_loaded_page();
        if let Some(active_target_id) = active_target_id
            && active_has_loaded_page
        {
            let next_engine = promoted_engine.unwrap_or_else(|| {
                NavigationEngine::new_with_runtime_config_and_browser_context_access(
                    navigation_runtime_config,
                    renderer_runtime.clone(),
                )
                .expect("active BrowserContext owner must be live")
            });
            self.apply_scheduler_senders_to_navigation_engine(&next_engine);
            let active_engine = self
                .engine
                .replace(next_engine)
                .expect("active target must have a navigation engine");
            self.retain_background_navigation_engine(
                browser_context_id,
                active_target_id,
                active_engine,
            )
            .expect("demoted active engine must retain its exact BrowserContext owner");
        } else if let Some(promoted_engine) = promoted_engine {
            self.replace_navigation_engine(promoted_engine);
        }
    }

    pub(crate) fn handoff_navigation_engine_for_active_target_demotion(&mut self) {
        let Some(browser_context) = self.browser_context.as_ref() else {
            return;
        };
        let Some(active_target_id) = browser_context.active_target_id().map(str::to_owned) else {
            return;
        };
        if !browser_context.has_loaded_page() {
            return;
        }

        let browser_context_id = browser_context.id.clone();
        let renderer_runtime = browser_context.renderer_runtime_owner_access();
        let next_engine = NavigationEngine::new_with_runtime_config_and_browser_context_access(
            self.engine.runtime_config(),
            renderer_runtime,
        )
        .expect("active BrowserContext owner must be live");
        self.apply_scheduler_senders_to_navigation_engine(&next_engine);
        let active_engine = self
            .engine
            .replace(next_engine)
            .expect("active target must have a navigation engine");
        self.retain_background_navigation_engine(
            browser_context_id,
            active_target_id,
            active_engine,
        )
        .expect("demoted active engine must retain its exact BrowserContext owner");
    }

    pub(crate) async fn promote_background_target_to_active_for_connection_async(
        &mut self,
        target_id: &str,
    ) -> Result<Option<CompletedTargetActivation>, String> {
        let previous_active_target_id = self
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id_owned);
        let transition = TargetActivationTransition::new(target_id, previous_active_target_id);
        let hidden_screencast_sessions = transition
            .demoted_target_id()
            .map(|active_target_id| self.page_screencast_session_ids_for_target(active_target_id))
            .unwrap_or_default();
        self.handoff_navigation_engine_for_target_activation(target_id);
        let Some(browser_context) = self.browser_context.as_mut() else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let promoted = browser_context
            .promote_background_target_to_active_slot_async(target_id)
            .await?;
        if !promoted {
            return Ok(None);
        }
        self.refresh_active_browser_context_loader_async().await;
        self.notify_target_host_activated(target_id);

        let mut protocol_events = Vec::new();
        if transition.changed_active_target() {
            // Chromium's PageHandler reports RenderWidgetHost visibility only
            // while that attachment has an active screencast. Hide the old
            // surface before exposing the promoted one.
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
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        let conn = route_scope.conn_mut();
        conn.page_event_session_ids_for_session_owner(None)
            .into_iter()
            .filter(|session_id| {
                conn.target_page_session_state_for_session(session_id.as_deref())
                    .is_some_and(|state| state.page_screencast.is_active())
            })
            .collect()
    }
}
