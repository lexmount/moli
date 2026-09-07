use super::CdpConnection;
use moli_core::browser::{BrowserContextId, DownloadBehavior, DownloadPolicy};
use std::collections::HashMap;

pub(crate) fn parse_download_behavior(value: &str) -> Option<DownloadBehavior> {
    match value {
        "default" => Some(DownloadBehavior::Default),
        "deny" => Some(DownloadBehavior::Deny),
        "allow" => Some(DownloadBehavior::Allow),
        "allowAndName" => Some(DownloadBehavior::AllowAndName),
        _ => None,
    }
}

/// DevTools observation does not own Browser download policy.
#[derive(Debug, Default)]
pub(super) struct DownloadSubscriptions {
    automation_events_enabled: bool,
    pub(super) webdriver_bidi_events_enabled: bool,
    browser_event_subscription_generations: HashMap<Option<String>, u64>,
    next_browser_event_subscription_generation: u64,
}

impl DownloadSubscriptions {
    pub(super) fn enable_webdriver_bidi_events(&mut self) -> bool {
        let changed = !self.webdriver_bidi_events_enabled;
        self.webdriver_bidi_events_enabled = true;
        changed
    }

    pub(super) fn disable_webdriver_bidi_events(&mut self) -> bool {
        let changed = self.webdriver_bidi_events_enabled;
        self.webdriver_bidi_events_enabled = false;
        changed
    }

    fn set_browser_events_enabled_for_session(&mut self, session_id: Option<&str>, enabled: bool) {
        self.next_browser_event_subscription_generation = self
            .next_browser_event_subscription_generation
            .wrapping_add(1);
        let session_id = session_id.map(str::to_owned);
        if enabled {
            self.browser_event_subscription_generations
                .insert(session_id, self.next_browser_event_subscription_generation);
        } else {
            self.browser_event_subscription_generations
                .remove(&session_id);
        }
    }

    pub(super) fn browser_event_observers(&self) -> Vec<(Option<String>, u64)> {
        let mut observers = self
            .browser_event_subscription_generations
            .iter()
            .map(|(session_id, generation)| (session_id.clone(), *generation))
            .collect::<Vec<_>>();
        observers.sort_by(|left, right| left.0.cmp(&right.0));
        observers
    }

    pub(super) fn browser_event_subscription_is_current(
        &self,
        session_id: Option<&str>,
        generation: u64,
    ) -> bool {
        self.browser_event_subscription_generations
            .get(&session_id.map(str::to_owned))
            .is_some_and(|current| *current == generation)
    }
}

impl CdpConnection {
    /// Translate one frontend's configuration into a Browser value and its
    /// separate observation state. None preserves an existing automation flag.
    pub(crate) fn configure_download_policy(
        &mut self,
        context_id: Option<&str>,
        policy: DownloadPolicy,
        automation_events: Option<bool>,
    ) -> Result<(), String> {
        if let Some(id) = context_id {
            let context = self
                .browser_context_by_id_mut(id)
                .ok_or("UnknownBrowserContextId")?;
            context.set_download_policy(Some(policy));
            // A local CDP configuration shadows inherited automation enablement
            // with false until explicitly opted in. This is frontend state.
            let subscription = context
                .automation_download_events_enabled
                .get_or_insert(false);
            if let Some(enabled) = automation_events {
                *subscription = enabled;
            }
        } else {
            self.download_policy = policy;
            if let Some(enabled) = automation_events {
                self.download_subscriptions.automation_events_enabled = enabled;
            }
        }
        Ok(())
    }

    pub(crate) fn reset_download_policy(&mut self, context_id: Option<&str>) -> Result<(), String> {
        if let Some(id) = context_id {
            let context = self
                .browser_context_by_id_mut(id)
                .ok_or("UnknownBrowserContextId")?;
            context.set_download_policy(None);
            context.automation_download_events_enabled = None;
        } else {
            self.download_policy = DownloadPolicy::default();
            self.download_subscriptions.automation_events_enabled = false;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn download_policy_for_browser_context(
        &self,
        context_id: Option<&str>,
    ) -> DownloadPolicy {
        context_id
            .and_then(|id| self.browser_context_by_id(id))
            .and_then(|context| context.download_policy())
            .unwrap_or_else(|| self.download_policy.clone())
    }

    #[cfg(test)]
    pub(crate) fn automation_download_events_enabled_for_context(
        &self,
        context_id: Option<&str>,
    ) -> bool {
        context_id
            .and_then(|id| self.browser_context_by_id(id))
            .and_then(|context| context.automation_download_events_enabled)
            .unwrap_or(self.download_subscriptions.automation_events_enabled)
    }

    pub(crate) fn download_configuration_for_browser_context(
        &self,
        context: BrowserContextId,
    ) -> Option<(DownloadPolicy, bool)> {
        let context = self.browser_context_by_browser_id(context)?;
        Some((
            context
                .download_policy()
                .unwrap_or_else(|| self.download_policy.clone()),
            context
                .automation_download_events_enabled
                .unwrap_or(self.download_subscriptions.automation_events_enabled),
        ))
    }

    pub(crate) fn set_browser_download_events_enabled_for_session(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) {
        self.download_subscriptions
            .set_browser_events_enabled_for_session(session_id, enabled);
    }

    #[cfg(test)]
    pub(crate) fn browser_download_event_session_ids(&self) -> Vec<Option<String>> {
        self.download_subscriptions
            .browser_event_observers()
            .into_iter()
            .map(|(session, _)| session)
            .collect()
    }
}
