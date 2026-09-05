use super::devtools_session::DevToolsNetworkSessionState;
use super::javascript_dialog::TargetJavaScriptDialogState;
use super::page_target_host::PageTargetHost;
use super::web_contents::NetworkRequestPolicy;
use crate::domains::audits_output_state::TargetAuditsSessionState;
use moli_core::page::V8InspectorSessionState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PerformanceTimeDomain {
    #[default]
    TimeTicks,
    ThreadTicks,
}

impl PerformanceTimeDomain {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TimeTicks => "timeTicks",
            Self::ThreadTicks => "threadTicks",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetPerformanceSessionState {
    enabled: bool,
    time_domain: PerformanceTimeDomain,
}

impl TargetPerformanceSessionState {
    pub(crate) fn enabled(self) -> bool {
        self.enabled
    }

    pub(crate) fn time_domain(self) -> PerformanceTimeDomain {
        self.time_domain
    }

    pub(crate) fn enable(&mut self, time_domain: PerformanceTimeDomain) -> bool {
        if self.enabled && self.time_domain != time_domain {
            return false;
        }
        self.time_domain = time_domain;
        self.enabled = true;
        true
    }

    pub(crate) fn disable(&mut self) {
        self.enabled = false;
    }

    pub(crate) fn set_time_domain(&mut self, time_domain: PerformanceTimeDomain) -> bool {
        if self.enabled {
            return false;
        }
        self.time_domain = time_domain;
        true
    }
}

impl PageTargetHost {
    // Browser.getVersion reports explicit frontend UA contributions, which
    // differ from a language-only runtime profile. This is projection state.
    pub(crate) fn reported_user_agent_override(&self) -> Option<&str> {
        self.devtools_sessions
            .reported_user_agent_override()
            .or_else(|| {
                self.base_browser_identity
                    .profile()
                    .map(moli_browser_profile::BrowserIdentityProfile::user_agent)
            })
    }

    pub(crate) fn browser_identity_override(
        &self,
    ) -> Option<&moli_browser_profile::BrowserIdentityProfile> {
        self.runtime_slot
            .page_slot()
            .contents
            .browser_identity_override
            .as_ref()
    }

    pub(crate) fn effective_policy(&self) -> EffectiveTargetPolicy {
        let contents = &self.runtime_slot.page_slot().contents;
        EffectiveTargetPolicy {
            network_request: contents.network_request_policy.clone(),
            browser_identity_override: contents.browser_identity_override.clone(),
            locale_override: contents.locale_override.clone(),
            timezone_override: contents.timezone_override.clone(),
        }
    }

    pub(crate) fn locale_override(&self) -> Option<&str> {
        self.runtime_slot
            .page_slot()
            .contents
            .locale_override
            .as_deref()
    }

    pub(crate) fn timezone_override(&self) -> Option<&str> {
        self.runtime_slot
            .page_slot()
            .contents
            .timezone_override
            .as_deref()
    }

    // Only contribution writes aggregate DevTools state. Replace this in-place
    // install with AgentHost -> BrowserHandle in Commits 14/22, before 24b.
    fn install_effective_network_request_policy(&mut self) {
        let mut policy = self.devtools_sessions.effective_network_policy();
        policy.cache_disabled |= self.base_network_request_policy.cache_disabled;
        let mut headers = self.base_network_request_policy.extra_headers.clone();
        overlay_extra_headers(&mut headers, &policy.extra_headers);
        policy.extra_headers = headers;
        self.runtime_slot
            .page_slot_mut()
            .contents
            .set_network_request_policy(policy);
    }

    pub(crate) fn set_base_cache_disabled(&mut self, disabled: bool) {
        self.base_network_request_policy.cache_disabled = disabled;
        self.install_effective_network_request_policy();
        let effective = self
            .runtime_slot
            .page_slot()
            .contents
            .network_request_policy
            .cache_disabled;
        if let Some(engine) = self.navigation_engine_mut() {
            engine.set_cache_disabled(effective);
        }
    }

    pub(crate) fn set_base_extra_headers(&mut self, headers: Vec<(String, String)>) {
        self.base_network_request_policy.extra_headers = headers;
        self.install_effective_network_request_policy();
    }

    pub(crate) fn network_offline(&self) -> bool {
        self.runtime_slot.page_slot().contents.network_offline
    }

    // Value-only bridge until AgentHost/BrowserHandle install (Commits 14/22).
    pub(crate) fn set_network_offline(&mut self, offline: bool) {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .set_network_offline(offline);
    }

    pub(crate) fn tls_verify_host_override(&self) -> Option<bool> {
        self.runtime_slot
            .page_slot()
            .contents
            .tls_verify_host_override
    }

    // Value-only bridge until AgentHost/BrowserHandle install (Commits 14/22).
    pub(crate) fn set_tls_verify_host_override(&mut self, enabled: Option<bool>) {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .set_tls_verify_host_override(enabled);
    }

    // Like request policy, this value-only bridge is replaced by the typed
    // AgentHost/BrowserHandle install in Commits 14/22, before 24b.
    fn install_effective_browser_identity(&mut self) {
        let identity = self
            .devtools_sessions
            .effective_browser_identity_override()
            .or_else(|| self.base_browser_identity.profile_owned());
        self.runtime_slot
            .page_slot_mut()
            .contents
            .set_browser_identity_override(identity);
    }

    pub(crate) fn set_base_browser_identity_override(
        &mut self,
        identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) {
        if let Some(identity) = identity {
            self.base_browser_identity.replace_profile(identity);
        } else {
            self.base_browser_identity = Default::default();
        }
        self.install_effective_browser_identity();
    }

    pub(crate) fn set_base_user_agent_override(
        &mut self,
        user_agent: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.base_browser_identity
            .set_user_agent(user_agent, fallback);
        self.install_effective_browser_identity();
    }

    pub(crate) fn set_base_accept_language_override(
        &mut self,
        language: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.base_browser_identity
            .set_accept_language(language, fallback);
        self.install_effective_browser_identity();
    }

    #[cfg(test)]
    pub(crate) fn set_user_agent_override_for_test(&mut self, user_agent: String) {
        self.set_base_browser_identity_override(Some(
            moli_browser_profile::BrowserIdentityProfile::new(
                user_agent,
                moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE,
            ),
        ));
    }

    pub(crate) fn mutate_devtools_network_session_state<T>(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        f: impl FnOnce(&mut DevToolsNetworkSessionState) -> T,
    ) -> T {
        let session = self.devtools_sessions.ensure_session(session_key);
        let result = f(&mut session.network_session_state);
        self.install_effective_network_request_policy();
        result
    }

    pub(crate) fn set_devtools_browser_identity_override(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        browser_identity_override: Option<super::DevToolsBrowserIdentityOverride>,
    ) {
        self.devtools_sessions
            .set_browser_identity_override(session_key, browser_identity_override);
        self.install_effective_browser_identity();
    }

    pub(crate) fn set_devtools_locale_override(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.devtools_sessions
            .set_locale_override(session_key, locale_override)?;
        self.install_effective_locale();
        Ok(())
    }

    pub(crate) fn set_devtools_timezone_override(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.devtools_sessions
            .set_timezone_override(session_key, timezone_override)?;
        self.install_effective_timezone();
        Ok(())
    }

    pub(crate) fn set_base_locale_override(&mut self, locale_override: Option<String>) {
        self.base_locale_override = locale_override;
        self.install_effective_locale();
    }

    pub(crate) fn set_base_timezone_override(&mut self, timezone_override: Option<String>) {
        self.base_timezone_override = timezone_override;
        self.install_effective_timezone();
    }

    // Claim arbitration stays in DevTools. Each bridge installs only its field;
    // Commits 14/22 replace these in-place writes with typed Browser commands.
    fn install_effective_locale(&mut self) {
        let locale = self
            .devtools_sessions
            .effective_locale_override()
            .map(str::to_owned)
            .or_else(|| self.base_locale_override.clone());
        self.runtime_slot
            .page_slot_mut()
            .contents
            .set_locale_override(locale);
    }

    fn install_effective_timezone(&mut self) {
        let timezone = self
            .devtools_sessions
            .effective_timezone_override()
            .map(str::to_owned)
            .or_else(|| self.base_timezone_override.clone());
        self.runtime_slot
            .page_slot_mut()
            .contents
            .set_timezone_override(timezone);
    }

    pub(crate) fn clear_devtools_network_state(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.devtools_sessions.clear_network_state(session_key);
        self.install_effective_network_request_policy();
    }

    pub(crate) fn clear_devtools_emulation_policy_state(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.devtools_sessions
            .clear_emulation_policy_state(session_key);
        self.install_effective_browser_identity();
        self.install_effective_locale();
        self.install_effective_timezone();
    }

    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.devtools_sessions.states().any(|session| {
            !session
                .page_session_state
                .javascript_dialog_state
                .is_empty()
        })
    }

    pub(crate) fn has_non_default_session_state(&self) -> bool {
        self.devtools_sessions.has_non_default_state()
            || self.runtime_slot.primary_network_events_enabled()
            || self.base_network_request_policy != BaseNetworkRequestPolicy::default()
            || self.network_offline()
            || self.base_browser_identity != super::BaseBrowserIdentityOverrideState::default()
            || self.browser_identity_override().is_some()
            || self.tls_verify_host_override().is_some()
            || self.base_locale_override.is_some()
            || self.base_timezone_override.is_some()
            || self.locale_override().is_some()
            || self.timezone_override().is_some()
            || *self.emulation_policy() != super::EmulationPolicy::default()
            || self
                .runtime_slot
                .page_slot()
                .contents
                .network_request_policy
                != NetworkRequestPolicy::default()
            || self.input_intercept_drags_enabled
            || self.input_drag_intercepted
            || self.css_enabled
            || self.fetch_owner.config_snapshot() != super::fetch::TargetFetchConfig::default()
    }

    /// Clears target-level state owned by the primary DevTools handlers.
    ///
    /// Per-session handler state, Fetch state, and Network observation state
    /// have their own disposal steps. Keeping them out of this helper makes
    /// the final session-registry removal a pure commit operation.
    pub(crate) fn reset_primary_session_target_state_fields(&mut self) {
        self.set_network_offline(false);
        self.set_tls_verify_host_override(None);
        self.input_intercept_drags_enabled = false;
        self.input_drag_intercepted = false;
        self.css_enabled = false;
    }
}

fn overlay_extra_headers(effective: &mut Vec<(String, String)>, layer: &[(String, String)]) {
    for (name, value) in layer {
        effective.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        effective.push((name.clone(), value.clone()));
    }
}

/// Read-only migration snapshot of installed Browser values. Session raw input
/// is resolved on writes, never when reading policy or preparing navigation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EffectiveTargetPolicy {
    network_request: NetworkRequestPolicy,
    browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    locale_override: Option<String>,
    timezone_override: Option<String>,
}

impl EffectiveTargetPolicy {
    pub(crate) fn delta(&self, next: &Self) -> EffectiveTargetPolicyDelta {
        EffectiveTargetPolicyDelta {
            network_request: self.network_request != next.network_request,
            browser_identity: self.browser_identity_override != next.browser_identity_override,
            locale: self.locale_override != next.locale_override,
            timezone: self.timezone_override != next.timezone_override,
        }
    }

    pub(crate) fn cache_disabled(&self) -> bool {
        self.network_request.cache_disabled
    }

    pub(crate) fn bypass_service_worker(&self) -> bool {
        self.network_request.bypass_service_worker
    }

    pub(crate) fn blocked_url_patterns(&self) -> &[String] {
        &self.network_request.blocked_url_patterns
    }

    pub(crate) fn extra_headers(&self) -> &[(String, String)] {
        &self.network_request.extra_headers
    }

    pub(crate) fn browser_identity_override(
        &self,
    ) -> Option<&moli_browser_profile::BrowserIdentityProfile> {
        self.browser_identity_override.as_ref()
    }

    pub(crate) fn locale_override(&self) -> Option<&str> {
        self.locale_override.as_deref()
    }

    pub(crate) fn timezone_override(&self) -> Option<&str> {
        self.timezone_override.as_deref()
    }
}

/// Renderer surfaces that must be replayed after an effective policy change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EffectiveTargetPolicyDelta {
    pub(crate) network_request: bool,
    pub(crate) browser_identity: bool,
    pub(crate) locale: bool,
    pub(crate) timezone: bool,
}

impl EffectiveTargetPolicyDelta {
    pub(crate) fn is_empty(self) -> bool {
        !self.network_request && !self.browser_identity && !self.locale && !self.timezone
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageScreencastFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageScreencastConfig {
    format: PageScreencastFormat,
    quality: u8,
    max_width: Option<u32>,
    max_height: Option<u32>,
    every_nth_frame: u32,
}

impl PageScreencastConfig {
    pub(crate) fn new(
        format: PageScreencastFormat,
        quality: u8,
        max_width: Option<u32>,
        max_height: Option<u32>,
        every_nth_frame: u32,
    ) -> Self {
        debug_assert!(every_nth_frame > 0);
        Self {
            format,
            quality,
            max_width,
            max_height,
            every_nth_frame,
        }
    }

    pub(crate) fn format(&self) -> PageScreencastFormat {
        self.format
    }

    pub(crate) fn quality(&self) -> u8 {
        self.quality
    }

    pub(crate) fn max_width(&self) -> Option<u32> {
        self.max_width
    }

    pub(crate) fn max_height(&self) -> Option<u32> {
        self.max_height
    }

    pub(crate) fn every_nth_frame(&self) -> u32 {
        self.every_nth_frame
    }
}

impl Default for PageScreencastConfig {
    fn default() -> Self {
        Self::new(PageScreencastFormat::Png, 80, None, None, 1)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PageScreencastSessionState {
    generation: i32,
    config: Option<PageScreencastConfig>,
    capture_in_progress: bool,
    awaiting_ack: bool,
}

impl PageScreencastSessionState {
    pub(crate) fn is_active(&self) -> bool {
        self.config.is_some()
    }

    pub(crate) fn generation(&self) -> i32 {
        self.generation
    }

    pub(crate) fn config(&self) -> Option<&PageScreencastConfig> {
        self.config.as_ref()
    }

    pub(crate) fn capture_in_progress(&self) -> bool {
        self.capture_in_progress
    }

    pub(crate) fn awaiting_ack(&self) -> bool {
        self.awaiting_ack
    }

    pub(crate) fn start(&mut self, config: PageScreencastConfig) -> i32 {
        self.generation = self
            .generation
            .checked_add(1)
            .filter(|generation| *generation > 0)
            .unwrap_or(1);
        self.config = Some(config);
        self.capture_in_progress = false;
        self.awaiting_ack = false;
        self.generation
    }

    pub(crate) fn stop(&mut self) {
        self.config = None;
        self.capture_in_progress = false;
        self.awaiting_ack = false;
    }

    pub(crate) fn capture_eligible(&self, generation: i32) -> bool {
        self.is_active()
            && self.generation == generation
            && !self.capture_in_progress
            && !self.awaiting_ack
    }

    pub(crate) fn begin_capture(&mut self, generation: i32) -> bool {
        if !self.capture_eligible(generation) {
            return false;
        }
        self.capture_in_progress = true;
        true
    }

    pub(crate) fn complete_capture(&mut self, generation: i32, frame_emitted: bool) -> bool {
        if !self.is_active() || self.generation != generation || !self.capture_in_progress {
            return false;
        }
        self.capture_in_progress = false;
        self.awaiting_ack = frame_emitted;
        true
    }

    pub(crate) fn acknowledge_frame(&mut self, generation: i32) -> bool {
        if !self.is_active() || self.generation != generation || !self.awaiting_ack {
            return false;
        }
        self.awaiting_ack = false;
        true
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TargetPageSessionState {
    pub(crate) input_events_ignored: bool,
    pub(crate) page_domain_enabled: bool,
    pub(crate) page_domain_subscription_generation: u64,
    pub(crate) page_lifecycle_events: bool,
    pub(crate) audits: TargetAuditsSessionState,
    pub(crate) log_enabled: bool,
    pub(crate) performance: TargetPerformanceSessionState,
    pub(crate) page_bypass_csp_enabled: bool,
    pub(crate) page_font_families: serde_json::Map<String, serde_json::Value>,
    pub(crate) page_file_chooser_opened_event_enabled: bool,
    pub(crate) page_intercept_file_chooser_dialog_enabled: bool,
    pub(crate) page_screencast: PageScreencastSessionState,
    pub(crate) javascript_dialog_state: TargetJavaScriptDialogState,
}

impl Default for TargetPageSessionState {
    fn default() -> Self {
        Self {
            input_events_ignored: false,
            page_domain_enabled: false,
            page_domain_subscription_generation: 0,
            page_lifecycle_events: false,
            audits: TargetAuditsSessionState::default(),
            log_enabled: false,
            performance: TargetPerformanceSessionState::default(),
            page_bypass_csp_enabled: false,
            page_font_families: serde_json::Map::new(),
            page_file_chooser_opened_event_enabled: false,
            page_intercept_file_chooser_dialog_enabled: false,
            page_screencast: PageScreencastSessionState::default(),
            javascript_dialog_state: TargetJavaScriptDialogState::default(),
        }
    }
}

impl TargetPageSessionState {
    pub(crate) fn enable_page_domain(&mut self, subscription_generation: u64) {
        if !self.page_domain_enabled {
            self.page_domain_enabled = true;
            self.page_domain_subscription_generation = subscription_generation;
        }
    }

    pub(crate) fn disable_page_domain(&mut self) {
        self.page_domain_enabled = false;
    }

    pub(crate) fn page_domain_subscription_generation(&self) -> Option<u64> {
        self.page_domain_enabled
            .then_some(self.page_domain_subscription_generation)
    }

    pub(crate) fn page_domain_subscription_is_current(&self, generation: u64) -> bool {
        self.page_domain_enabled && self.page_domain_subscription_generation == generation
    }

    pub(crate) fn clear_loaded_document_context_state(&mut self) {
        self.javascript_dialog_state.clear();
    }
}

impl PartialEq for TargetPageSessionState {
    fn eq(&self, other: &Self) -> bool {
        self.input_events_ignored == other.input_events_ignored
            && self.page_domain_enabled == other.page_domain_enabled
            && self.page_lifecycle_events == other.page_lifecycle_events
            && self.audits == other.audits
            && self.log_enabled == other.log_enabled
            && self.performance == other.performance
            && self.page_bypass_csp_enabled == other.page_bypass_csp_enabled
            && self.page_font_families == other.page_font_families
            && self.page_file_chooser_opened_event_enabled
                == other.page_file_chooser_opened_event_enabled
            && self.page_intercept_file_chooser_dialog_enabled
                == other.page_intercept_file_chooser_dialog_enabled
            && self.page_screencast == other.page_screencast
            && self.javascript_dialog_state == other.javascript_dialog_state
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetRuntimeSessionState {
    /// Protocol-side projection of whether this frontend session subscribed to
    /// Runtime domain events. Renderer V8 RuntimeAgent owns the real agent
    /// enabled state on loaded page / available worker paths.
    pub(crate) runtime_frontend_enabled: bool,
    /// Whether this frontend session has observed at least one live Runtime
    /// execution context for the current document/worker lifetime.
    pub(crate) runtime_contexts_reported_to_frontend: bool,
    pub(crate) inspector_enabled: bool,
    /// Mirrors Chromium's per-InspectorHandler crash delivery bit. It records
    /// whether this frontend session has ever received Inspector.targetCrashed,
    /// so a later renderer recovery can emit targetReloadedAfterCrash only to
    /// sessions that observed a crash.
    pub(crate) inspector_target_crashed_delivered: bool,
}

impl TargetRuntimeSessionState {
    pub(crate) fn record_inspector_target_crashed(&mut self) {
        self.inspector_target_crashed_delivered = true;
    }

    pub(crate) fn inspector_target_crashed_delivered(self) -> bool {
        self.inspector_target_crashed_delivered
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct InspectorSessionState {
    pub(crate) v8_state: Option<V8InspectorSessionState>,
}

/// Frontend base contributions; installed runtime policy belongs to WebContents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BaseNetworkRequestPolicy {
    cache_disabled: bool,
    extra_headers: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::{PageScreencastConfig, PageScreencastFormat, PageScreencastSessionState};
    use crate::conn::PageTargetHost;
    use moli_page_types::DevToolsSessionKey;

    #[test]
    fn tls_policy_survives_projection_drop_and_updates_without_session_state() {
        let mut target = PageTargetHost::empty("TID-owned-tls".into());
        target.set_tls_verify_host_override(Some(false));
        target.set_network_offline(true);
        let id = target.web_contents_id();
        let mut contents = std::mem::take(&mut target.runtime_slot.page_slot_mut().contents);
        drop(target);
        assert_eq!(contents.id(), id);
        assert_eq!(contents.tls_verify_host_override, Some(false));
        for enabled in [Some(true), None] {
            contents.set_tls_verify_host_override(enabled);
            assert_eq!(contents.tls_verify_host_override, enabled);
            assert!(contents.network_offline);
        }
    }

    #[test]
    fn network_offline_survives_projection_drop_without_overwriting_request_policy() {
        let mut target = PageTargetHost::empty("TID-owned-offline".into());
        target.set_network_offline(true);
        target.set_base_cache_disabled(true);
        target.clear_devtools_network_state(&DevToolsSessionKey::Primary);
        assert!(target.network_offline());
        let id = target.web_contents_id();
        let mut contents = std::mem::take(&mut target.runtime_slot.page_slot_mut().contents);
        drop(target);
        assert_eq!(contents.id(), id);
        assert!(contents.network_offline);
        contents.set_network_offline(false);
        assert!(!contents.network_offline);
        assert!(contents.network_request_policy.cache_disabled);
    }

    #[test]
    fn browser_identity_survives_projection_drop_and_updates_without_sessions() {
        let mut target = PageTargetHost::empty("TID-owned-identity".into());
        let base = moli_browser_profile::BrowserIdentityProfile::new("Moli/Base", "en-US");
        target.set_devtools_browser_identity_override(
            &DevToolsSessionKey::Primary,
            crate::conn::DevToolsBrowserIdentityOverride::from_command(
                &base,
                "Moli/Installed".into(),
                Some("fr-FR".into()),
                Some("TestPlatform".into()),
                None,
            ),
        );
        let installed = target.browser_identity_override().unwrap().clone();
        let snapshot = target.effective_policy();
        target
            .runtime_slot
            .page_slot_mut()
            .contents
            .set_browser_identity_override(Some(base.clone()));
        assert_eq!(target.browser_identity_override(), Some(&base));
        assert_eq!(
            target.effective_policy().browser_identity_override(),
            Some(&base)
        );
        assert_eq!(
            target.reported_user_agent_override(),
            Some("Moli/Installed")
        );
        assert_eq!(snapshot.browser_identity_override(), Some(&installed));
        let id = target.web_contents_id();
        let mut contents = std::mem::take(&mut target.runtime_slot.page_slot_mut().contents);
        drop(target);
        assert_eq!(contents.id(), id);
        assert_eq!(contents.browser_identity_override, Some(base));
        contents.set_browser_identity_override(Some(installed.clone()));
        assert_eq!(contents.browser_identity_override, Some(installed));
        contents.set_browser_identity_override(None);
        assert!(contents.browser_identity_override.is_none());
    }

    #[test]
    fn reported_user_agent_keeps_its_fallback_for_language_only_runtime_overrides() {
        let mut target = PageTargetHost::empty("TID-language-only".into());
        let base = moli_browser_profile::BrowserIdentityProfile::new("Moli/Base", "de-DE");
        target.set_base_browser_identity_override(Some(base.clone()));
        let handler_base =
            moli_browser_profile::BrowserIdentityProfile::new("Moli/Handler", "en-US");
        target.set_devtools_browser_identity_override(
            &DevToolsSessionKey::Primary,
            crate::conn::DevToolsBrowserIdentityOverride::from_command(
                &handler_base,
                String::new(),
                Some("fr-FR".into()),
                Some("TestPlatform".into()),
                None,
            ),
        );
        let snapshot = target.effective_policy();
        let profile = snapshot.browser_identity_override().unwrap();
        assert_eq!(profile.user_agent(), "Moli/Handler");
        assert_eq!(profile.accept_language(), "fr-FR");
        assert_eq!(profile.navigator_platform(), "TestPlatform");
        assert_eq!(target.browser_identity_override(), Some(profile));
        assert_eq!(target.reported_user_agent_override(), Some("Moli/Base"));
        target.set_base_browser_identity_override(None);
        assert_eq!(target.effective_policy(), snapshot);
        assert_eq!(target.reported_user_agent_override(), None);
        target.clear_devtools_emulation_policy_state(&DevToolsSessionKey::Primary);
        assert!(
            target
                .effective_policy()
                .browser_identity_override()
                .is_none()
        );
        target.set_base_browser_identity_override(Some(base.clone()));
        assert_eq!(
            target.effective_policy().browser_identity_override(),
            Some(&base)
        );
    }

    #[test]
    fn network_request_policy_survives_projection_drop_and_updates_without_sessions() {
        let mut target = PageTargetHost::empty("TID-independent-policy".into());
        target.mutate_devtools_network_session_state(&DevToolsSessionKey::Primary, |raw| {
            raw.network_enabled = true;
            raw.cache_disabled = true;
            raw.bypass_service_worker = true;
            raw.blocked_url_patterns = vec!["blocked/*".into()];
            raw.extra_headers = vec![("X-Owner".into(), "browser".into())];
        });
        let installed = target.effective_policy().network_request;
        // A Browser-side value update must be observable without rebuilding it
        // from the unchanged frontend contributions on every read.
        let independent = super::NetworkRequestPolicy {
            cache_disabled: false,
            ..installed.clone()
        };
        target
            .runtime_slot
            .page_slot_mut()
            .contents
            .set_network_request_policy(independent.clone());
        assert_eq!(target.effective_policy().network_request, independent);
        assert!(
            target
                .devtools_sessions
                .primary()
                .network_session_state
                .cache_disabled
        );
        let id = target.web_contents_id();
        let mut contents = std::mem::take(&mut target.runtime_slot.page_slot_mut().contents);
        drop(target);
        assert_eq!(contents.id(), id);
        assert_eq!(contents.network_request_policy, independent);
        contents.set_network_request_policy(installed.clone());
        assert_eq!(contents.network_request_policy, installed);
    }

    #[test]
    fn network_policy_preserves_enable_disable_and_base_precedence() {
        let mut target = PageTargetHost::empty("TID-network-policy".into());
        target.set_base_cache_disabled(true);
        let base_headers = vec![("X-Shared".into(), "base".into())];
        target.set_base_extra_headers(base_headers.clone());
        target.mutate_devtools_network_session_state(&DevToolsSessionKey::Primary, |raw| {
            raw.bypass_service_worker = true;
            raw.blocked_url_patterns = vec!["primary/*".into()];
            raw.extra_headers = vec![("x-shared".into(), "primary".into())];
        });
        let disabled = target.effective_policy();
        assert!(disabled.cache_disabled());
        assert!(!disabled.bypass_service_worker());
        assert!(disabled.blocked_url_patterns().is_empty());
        assert_eq!(disabled.extra_headers(), base_headers);

        target.mutate_devtools_network_session_state(&DevToolsSessionKey::Primary, |raw| {
            raw.network_enabled = true;
        });
        // Deliberately reverse lexical order: header precedence is attachment order.
        for session in ["SID-z", "SID-a"] {
            target.mutate_devtools_network_session_state(
                &DevToolsSessionKey::Attached(session.into()),
                |raw| {
                    raw.network_enabled = true;
                    raw.blocked_url_patterns = vec!["primary/*".into(), format!("{session}/*")];
                    raw.extra_headers = vec![("X-SHARED".into(), session.into())];
                },
            );
        }
        let combined = target.effective_policy();
        assert!(combined.cache_disabled());
        assert!(combined.bypass_service_worker());
        assert_eq!(
            combined.blocked_url_patterns(),
            ["primary/*", "SID-z/*", "SID-a/*"]
        );
        assert_eq!(
            combined.extra_headers(),
            [("X-SHARED".into(), "SID-a".into())]
        );
        target.clear_devtools_network_state(&DevToolsSessionKey::Attached("SID-a".into()));
        assert_eq!(
            target.effective_policy().extra_headers(),
            [("X-SHARED".into(), "SID-z".into())]
        );
        target.clear_devtools_network_state(&DevToolsSessionKey::Primary);
        assert!(!target.effective_policy().bypass_service_worker());
        target.clear_devtools_network_state(&DevToolsSessionKey::Attached("SID-z".into()));
        assert_eq!(target.effective_policy().extra_headers(), base_headers);
        assert!(target.effective_policy().blocked_url_patterns().is_empty());
        assert!(target.effective_policy().cache_disabled());
        target.set_base_cache_disabled(false);
        assert!(!target.effective_policy().cache_disabled());
        assert_eq!(
            combined.extra_headers(),
            [("X-SHARED".into(), "SID-a".into())]
        );
    }

    #[test]
    fn locale_and_timezone_survive_projection_drop_and_independent_field_updates() {
        let mut target = PageTargetHost::empty("TID-owned-locale".into());
        let session = DevToolsSessionKey::Primary;
        target
            .set_devtools_locale_override(&session, Some("fr-FR".into()))
            .unwrap();
        target
            .set_devtools_timezone_override(&session, Some("Europe/Paris".into()))
            .unwrap();
        let snapshot = target.effective_policy();
        target
            .runtime_slot
            .page_slot_mut()
            .contents
            .set_locale_override(Some("ja-JP".into()));
        target
            .set_devtools_timezone_override(&session, Some("UTC".into()))
            .unwrap();
        assert_eq!(target.locale_override(), Some("ja-JP"));
        assert_eq!(target.effective_policy().locale_override(), Some("ja-JP"));
        assert_eq!(
            target
                .devtools_sessions
                .primary()
                .emulation_session_state
                .locale_override
                .as_deref(),
            Some("fr-FR")
        );
        target
            .runtime_slot
            .page_slot_mut()
            .contents
            .set_timezone_override(Some("Asia/Tokyo".into()));
        target
            .set_devtools_locale_override(&session, Some("it-IT".into()))
            .unwrap();
        assert_eq!(target.timezone_override(), Some("Asia/Tokyo"));
        assert_eq!(
            target.effective_policy().timezone_override(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            target
                .devtools_sessions
                .primary()
                .emulation_session_state
                .timezone_override
                .as_deref(),
            Some("UTC")
        );
        assert_eq!(snapshot.locale_override(), Some("fr-FR"));
        assert_eq!(snapshot.timezone_override(), Some("Europe/Paris"));

        let id = target.web_contents_id();
        let mut contents = std::mem::take(&mut target.runtime_slot.page_slot_mut().contents);
        drop(target);
        assert_eq!(contents.id(), id);
        assert_eq!(contents.locale_override.as_deref(), Some("it-IT"));
        assert_eq!(contents.timezone_override.as_deref(), Some("Asia/Tokyo"));
        contents.set_locale_override(None);
        assert!(contents.locale_override.is_none());
        assert_eq!(contents.timezone_override.as_deref(), Some("Asia/Tokyo"));
        contents.set_timezone_override(None);
        assert!(contents.timezone_override.is_none());
    }

    #[test]
    fn locale_and_timezone_claims_only_change_their_effective_field() {
        let mut target = PageTargetHost::empty("TID-locale-policy".into());
        let locale_owner = DevToolsSessionKey::Primary;
        let timezone_owner = DevToolsSessionKey::Attached("SID-timezone".into());
        target.set_base_locale_override(Some("en-GB".into()));
        target.set_base_timezone_override(Some("Europe/London".into()));
        target.set_user_agent_override_for_test("Moli/Unchanged".into());
        target
            .set_devtools_locale_override(&locale_owner, Some("fr-FR".into()))
            .unwrap();
        target
            .set_devtools_timezone_override(&timezone_owner, Some("Europe/Paris".into()))
            .unwrap();
        let installed = target.effective_policy();
        for locale in [Some("de-DE".into()), None] {
            assert_eq!(
                target.set_devtools_locale_override(&timezone_owner, locale),
                Err("Another locale override is already in effect")
            );
            assert_eq!(target.effective_policy(), installed);
        }
        assert_eq!(
            target.set_devtools_timezone_override(&locale_owner, Some("Asia/Tokyo".into())),
            Err("Timezone override is already in effect")
        );
        assert_eq!(target.effective_policy(), installed);
        target
            .set_devtools_timezone_override(&locale_owner, None)
            .unwrap();
        assert_eq!(target.effective_policy(), installed);

        target.set_base_locale_override(Some("es-ES".into()));
        target.set_base_timezone_override(Some("Europe/Madrid".into()));
        assert_eq!(target.effective_policy(), installed);
        target
            .set_devtools_locale_override(&locale_owner, None)
            .unwrap();
        let locale_cleared = target.effective_policy();
        assert_eq!(locale_cleared.locale_override(), Some("es-ES"));
        assert_eq!(locale_cleared.timezone_override(), Some("Europe/Paris"));
        assert_eq!(
            installed.delta(&locale_cleared),
            super::EffectiveTargetPolicyDelta {
                locale: true,
                ..Default::default()
            }
        );
        target.clear_devtools_emulation_policy_state(&timezone_owner);
        let cleared = target.effective_policy();
        assert_eq!(cleared.timezone_override(), Some("Europe/Madrid"));
        assert_eq!(
            locale_cleared.delta(&cleared),
            super::EffectiveTargetPolicyDelta {
                timezone: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn devtools_emulation_overrides_reveal_target_base_state_when_cleared() {
        let mut state = PageTargetHost::empty("TID-policy-test".to_owned());
        state.set_base_locale_override(Some("en-GB".to_owned()));
        state.set_base_timezone_override(Some("Europe/London".to_owned()));
        state.set_base_browser_identity_override(Some(
            moli_browser_profile::BrowserIdentityProfile::new("Moli/Base", "en-GB"),
        ));

        state
            .set_devtools_locale_override(&DevToolsSessionKey::Primary, Some("fr-FR".to_owned()))
            .unwrap();
        state
            .set_devtools_timezone_override(
                &DevToolsSessionKey::Primary,
                Some("Europe/Paris".to_owned()),
            )
            .unwrap();
        state.set_devtools_browser_identity_override(
            &DevToolsSessionKey::Primary,
            crate::conn::DevToolsBrowserIdentityOverride::from_command(
                &moli_browser_profile::BrowserIdentityProfile::default(),
                "Moli/CDP".to_owned(),
                Some("fr-FR".to_owned()),
                None,
                None,
            ),
        );
        state
            .devtools_sessions
            .primary_mut()
            .emulation_session_state
            .overrides
            .cpu_throttling_rate = 4.0;
        let effective = state.effective_policy();
        assert_eq!(effective.locale_override(), Some("fr-FR"));
        assert_eq!(effective.timezone_override(), Some("Europe/Paris"));
        assert_eq!(
            effective
                .browser_identity_override()
                .map(moli_browser_profile::BrowserIdentityProfile::user_agent),
            Some("Moli/CDP")
        );

        state.clear_devtools_network_state(&DevToolsSessionKey::Primary);
        state.clear_devtools_emulation_policy_state(&DevToolsSessionKey::Primary);
        let effective = state.effective_policy();
        assert_eq!(effective.locale_override(), Some("en-GB"));
        assert_eq!(effective.timezone_override(), Some("Europe/London"));
        assert_eq!(
            state
                .devtools_sessions
                .primary()
                .emulation_session_state
                .overrides
                .cpu_throttling_rate,
            4.0,
            "clearing policy contributions must leave the handler's renderer state intact"
        );
        assert_eq!(
            effective
                .browser_identity_override()
                .map(moli_browser_profile::BrowserIdentityProfile::user_agent),
            Some("Moli/Base")
        );
    }

    fn jpeg_config() -> PageScreencastConfig {
        PageScreencastConfig::new(PageScreencastFormat::Jpeg, 80, Some(1200), Some(900), 1)
    }

    #[test]
    fn screencast_state_enforces_generation_and_single_outstanding_frame() {
        let mut state = PageScreencastSessionState::default();
        let first_generation = state.start(jpeg_config());
        assert_eq!(first_generation, 1);
        assert!(state.begin_capture(first_generation));
        assert!(!state.begin_capture(first_generation));
        assert!(state.complete_capture(first_generation, true));
        assert!(state.awaiting_ack());
        assert!(!state.begin_capture(first_generation));
        assert!(!state.acknowledge_frame(first_generation + 1));
        assert!(state.awaiting_ack());
        assert!(state.acknowledge_frame(first_generation));
        assert!(!state.awaiting_ack());
        assert!(state.begin_capture(first_generation));
    }

    #[test]
    fn repeated_start_and_stop_invalidate_old_capture_state() {
        let mut state = PageScreencastSessionState::default();
        let first_generation = state.start(jpeg_config());
        assert!(state.begin_capture(first_generation));

        let second_generation = state.start(PageScreencastConfig::default());
        assert_eq!(second_generation, first_generation + 1);
        assert!(!state.capture_in_progress());
        assert!(!state.awaiting_ack());
        assert!(!state.complete_capture(first_generation, true));
        assert!(state.begin_capture(second_generation));

        state.stop();
        assert!(!state.is_active());
        assert!(!state.capture_in_progress());
        assert!(!state.complete_capture(second_generation, true));
    }
}
