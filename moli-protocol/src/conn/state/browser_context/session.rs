use super::BrowserContext;
use crate::conn::state::devtools_session::DevToolsNetworkSessionState;
use crate::conn::state::javascript_dialog::TargetJavaScriptDialogState;
use crate::conn::state::page_agent_host::PageAgentHost;
use crate::conn::state::web_contents::NetworkRequestPolicy;
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

impl PageAgentHost {
    pub(crate) fn reported_user_agent_override(&self) -> Option<&str> {
        self.devtools_sessions
            .reported_user_agent_override()
            .or_else(|| {
                self.base_browser_identity
                    .profile()
                    .map(moli_browser_profile::BrowserIdentityProfile::user_agent)
            })
    }
}

impl BrowserContext {
    pub(crate) fn bypass_content_security_policy_for_target(&self, target_id: &str) -> bool {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .bypass_content_security_policy
    }

    // Value-only bridge until AgentHost/BrowserHandle install (Commits 14/22).
    fn install_effective_content_security_policy_for_target(&mut self, target_id: &str) {
        let bypass = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .page_bypass_csp_enabled();
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_bypass_content_security_policy(bypass);
    }

    pub(crate) fn set_devtools_bypass_csp_enabled_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
        enabled: bool,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session_key)
            .page_session_state
            .page_bypass_csp_enabled = enabled;
        self.install_effective_content_security_policy_for_target(target_id);
    }

    pub(crate) fn disable_devtools_page_domain_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.dismiss_devtools_javascript_dialogs_for_target(target_id, session_key);
        let state = &mut self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session_key)
            .page_session_state;
        state.disable_page_domain();
        state.page_lifecycle_events = false;
        state.page_bypass_csp_enabled = false;
        state.page_font_families.clear();
        state.page_file_chooser_opened_event_enabled = false;
        state.page_intercept_file_chooser_dialog_enabled = false;
        state.page_screencast.stop();
        self.install_effective_content_security_policy_for_target(target_id);
    }

    pub(crate) fn dispose_devtools_session_for_target(
        &mut self,
        target_id: &str,
        session_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) -> bool {
        let removed = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .dispose(session_id, session_key);
        let Some(mut removed) = removed else {
            return false;
        };
        self.dismiss_javascript_dialog_projections_for_target(
            target_id,
            removed
                .page_session_state
                .javascript_dialog_state
                .take_pending(),
        );
        self.install_effective_content_security_policy_for_target(target_id);
        true
    }

    // Browser.getVersion reports explicit frontend UA contributions, which
    // differ from a language-only runtime profile. This is projection state.
    pub(crate) fn browser_identity_override_for_target(
        &self,
        target_id: &str,
    ) -> Option<&moli_browser_profile::BrowserIdentityProfile> {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .browser_identity_override
            .as_ref()
    }

    pub(crate) fn effective_policy_for_target(&self, target_id: &str) -> EffectiveTargetPolicy {
        let contents = &self
            .web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live");
        EffectiveTargetPolicy {
            network_request: contents.network_request_policy.clone(),
            browser_identity_override: contents.browser_identity_override.clone(),
            locale_override: contents.locale_override.clone(),
            timezone_override: contents.timezone_override.clone(),
        }
    }

    pub(crate) fn locale_override_for_target(&self, target_id: &str) -> Option<&str> {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .locale_override
            .as_deref()
    }

    pub(crate) fn timezone_override_for_target(&self, target_id: &str) -> Option<&str> {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .timezone_override
            .as_deref()
    }

    // Only contribution writes aggregate DevTools state. Replace this in-place
    // install with AgentHost -> BrowserHandle in Commits 14/22, before 24b.
    fn install_effective_network_request_policy_for_target(&mut self, target_id: &str) {
        let projection = self
            .page_targets
            .get(target_id)
            .expect("resolved target projection must remain live");
        let mut policy = projection.devtools_sessions.effective_network_policy();
        policy.cache_disabled |= projection.base_network_request_policy.cache_disabled;
        let mut headers = projection.base_network_request_policy.extra_headers.clone();
        overlay_extra_headers(&mut headers, &policy.extra_headers);
        policy.extra_headers = headers;
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_network_request_policy(policy);
    }

    pub(crate) fn set_base_cache_disabled_for_target(&mut self, target_id: &str, disabled: bool) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .base_network_request_policy
            .cache_disabled = disabled;
        self.install_effective_network_request_policy_for_target(target_id);
    }

    pub(crate) fn set_base_extra_headers_for_target(
        &mut self,
        target_id: &str,
        headers: Vec<(String, String)>,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .base_network_request_policy
            .extra_headers = headers;
        self.install_effective_network_request_policy_for_target(target_id);
    }

    pub(crate) fn network_offline_for_target(&self, target_id: &str) -> bool {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .network_offline
    }

    // Value-only bridge until AgentHost/BrowserHandle install (Commits 14/22).
    pub(crate) fn set_network_offline_for_target(&mut self, target_id: &str, offline: bool) {
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_network_offline(offline);
    }

    pub(crate) fn tls_verify_host_override_for_target(&self, target_id: &str) -> Option<bool> {
        self.web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .tls_verify_host_override
    }

    // Value-only bridge until AgentHost/BrowserHandle install (Commits 14/22).
    pub(crate) fn set_tls_verify_host_override_for_target(
        &mut self,
        target_id: &str,
        enabled: Option<bool>,
    ) {
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_tls_verify_host_override(enabled);
    }

    // Like request policy, this value-only bridge is replaced by the typed
    // AgentHost/BrowserHandle install in Commits 14/22, before 24b.
    fn install_effective_browser_identity_for_target(&mut self, target_id: &str) {
        let identity = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .effective_browser_identity_override()
            .or_else(|| {
                self.page_targets
                    .get_mut(target_id)
                    .expect("resolved target projection must remain live")
                    .base_browser_identity
                    .profile_owned()
            });
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_browser_identity_override(identity);
    }

    pub(crate) fn set_base_browser_identity_override_for_target(
        &mut self,
        target_id: &str,
        identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) {
        if let Some(identity) = identity {
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection must remain live")
                .base_browser_identity
                .replace_profile(identity);
        } else {
            self.page_targets
                .get_mut(target_id)
                .expect("resolved target projection must remain live")
                .base_browser_identity = Default::default();
        }
        self.install_effective_browser_identity_for_target(target_id);
    }

    pub(crate) fn set_base_user_agent_override_for_target(
        &mut self,
        target_id: &str,
        user_agent: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .base_browser_identity
            .set_user_agent(user_agent, fallback);
        self.install_effective_browser_identity_for_target(target_id);
    }

    pub(crate) fn set_base_accept_language_override_for_target(
        &mut self,
        target_id: &str,
        language: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .base_browser_identity
            .set_accept_language(language, fallback);
        self.install_effective_browser_identity_for_target(target_id);
    }

    #[cfg(test)]
    pub(crate) fn set_user_agent_override_for_test_for_target(
        &mut self,
        target_id: &str,
        user_agent: String,
    ) {
        self.set_base_browser_identity_override_for_target(
            target_id,
            Some(moli_browser_profile::BrowserIdentityProfile::new(
                user_agent,
                moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE,
            )),
        );
    }

    pub(crate) fn mutate_devtools_network_session_state_for_target<T>(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
        f: impl FnOnce(&mut DevToolsNetworkSessionState) -> T,
    ) -> T {
        let session = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .ensure_session(session_key);
        let result = f(&mut session.network_session_state);
        self.install_effective_network_request_policy_for_target(target_id);
        result
    }

    pub(crate) fn set_devtools_browser_identity_override_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
        browser_identity_override: Option<crate::conn::state::DevToolsBrowserIdentityOverride>,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .set_browser_identity_override(session_key, browser_identity_override);
        self.install_effective_browser_identity_for_target(target_id);
    }

    pub(crate) fn set_devtools_locale_override_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .set_locale_override(session_key, locale_override)?;
        self.install_effective_locale_for_target(target_id);
        Ok(())
    }

    pub(crate) fn set_devtools_timezone_override_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .set_timezone_override(session_key, timezone_override)?;
        self.install_effective_timezone_for_target(target_id);
        Ok(())
    }

    pub(crate) fn set_base_locale_override_for_target(
        &mut self,
        target_id: &str,
        locale_override: Option<String>,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .base_locale_override = locale_override;
        self.install_effective_locale_for_target(target_id);
    }

    pub(crate) fn set_base_timezone_override_for_target(
        &mut self,
        target_id: &str,
        timezone_override: Option<String>,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .base_timezone_override = timezone_override;
        self.install_effective_timezone_for_target(target_id);
    }

    // Claim arbitration stays in DevTools. Each bridge installs only its field;
    // Commits 14/22 replace these in-place writes with typed Browser commands.
    fn install_effective_locale_for_target(&mut self, target_id: &str) {
        let projection = self
            .page_targets
            .get(target_id)
            .expect("resolved target projection must remain live");
        let locale = projection
            .devtools_sessions
            .effective_locale_override()
            .map(str::to_owned)
            .or_else(|| projection.base_locale_override.clone());
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_locale_override(locale);
    }

    fn install_effective_timezone_for_target(&mut self, target_id: &str) {
        let projection = self
            .page_targets
            .get(target_id)
            .expect("resolved target projection must remain live");
        let timezone = projection
            .devtools_sessions
            .effective_timezone_override()
            .map(str::to_owned)
            .or_else(|| projection.base_timezone_override.clone());
        self.web_contents_for_target_mut(target_id)
            .expect("resolved WebContents must remain live")
            .set_timezone_override(timezone);
    }

    pub(crate) fn clear_devtools_network_state_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .clear_network_state(session_key);
        self.install_effective_network_request_policy_for_target(target_id);
    }

    pub(crate) fn clear_devtools_emulation_policy_state_for_target(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live")
            .devtools_sessions
            .clear_emulation_policy_state(session_key);
        self.install_effective_browser_identity_for_target(target_id);
        self.install_effective_locale_for_target(target_id);
        self.install_effective_timezone_for_target(target_id);
    }

    pub(crate) fn has_pending_javascript_dialog_for_target(&self, target_id: &str) -> bool {
        !self
            .web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live")
            .javascript_dialogs
            .is_empty()
    }

    pub(crate) fn has_non_default_session_state_for_target(&self, target_id: &str) -> bool {
        let projection = self
            .page_targets
            .get(target_id)
            .expect("resolved target projection must remain live");
        let contents = self
            .web_contents_for_target(target_id)
            .expect("resolved WebContents must remain live");
        projection.devtools_sessions.has_non_default_state()
            || projection.runtime_slot.primary_network_events_enabled()
            || projection.base_network_request_policy != BaseNetworkRequestPolicy::default()
            || contents.network_offline
            || projection.base_browser_identity
                != crate::conn::state::BaseBrowserIdentityOverrideState::default()
            || contents.browser_identity_override.is_some()
            || contents.tls_verify_host_override.is_some()
            || contents.bypass_content_security_policy
            || projection.base_locale_override.is_some()
            || projection.base_timezone_override.is_some()
            || contents.locale_override.is_some()
            || contents.timezone_override.is_some()
            || contents.emulation_policy != crate::conn::state::EmulationPolicy::default()
            || contents.network_request_policy != NetworkRequestPolicy::default()
            || projection.input_intercept_drags_enabled
            || projection.input_drag_intercepted
            || projection.css_enabled
            || projection.fetch_owner.config_snapshot()
                != crate::conn::state::fetch::TargetFetchConfig::default()
    }

    /// Clears target-level state owned by the primary DevTools handlers.
    ///
    /// Per-session handler state, Fetch state, and Network observation state
    /// have their own disposal steps. Keeping them out of this helper makes
    /// the final session-registry removal a pure commit operation.
    pub(crate) fn reset_primary_session_target_state_fields_for_target(&mut self, target_id: &str) {
        self.set_network_offline_for_target(target_id, false);
        self.set_tls_verify_host_override_for_target(target_id, None);
        let projection = self
            .page_targets
            .get_mut(target_id)
            .expect("resolved target projection must remain live");
        projection.input_intercept_drags_enabled = false;
        projection.input_drag_intercepted = false;
        projection.css_enabled = false;
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
    pub(in crate::conn::state) page_bypass_csp_enabled: bool,
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
    #[cfg(test)]
    pub(crate) fn page_bypass_csp_enabled(&self) -> bool {
        self.page_bypass_csp_enabled
    }

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
pub(in crate::conn::state) struct BaseNetworkRequestPolicy {
    cache_disabled: bool,
    extra_headers: Vec<(String, String)>,
}

impl BaseNetworkRequestPolicy {
    pub(in crate::conn::state) fn with_cache_disabled(cache_disabled: bool) -> Self {
        Self {
            cache_disabled,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PageScreencastConfig, PageScreencastFormat, PageScreencastSessionState};
    use crate::conn::BrowserContext;
    use moli_page_types::DevToolsSessionKey;

    fn policy_context(target_id: &str) -> BrowserContext {
        let mut context = BrowserContext::new("BID-policy".into());
        context.set_active_target_id(target_id);
        context
    }

    #[test]
    fn csp_policy_survives_projection_drop_and_updates_without_session_state() {
        let target_id = "TID-owned-csp";
        let mut context = policy_context(target_id);
        context.set_devtools_bypass_csp_enabled_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            true,
        );
        context.set_network_offline_for_target(target_id, true);
        let contents_id = context.selected_web_contents_id().unwrap();
        drop(context.page_targets.remove(target_id).unwrap());
        let contents = context.physical.web_contents.get_mut(&contents_id).unwrap();
        assert!(contents.bypass_content_security_policy);
        contents.set_bypass_content_security_policy(false);
        assert!(!contents.bypass_content_security_policy);
        assert!(contents.network_offline);
    }

    #[test]
    fn tls_policy_survives_projection_drop_and_updates_without_session_state() {
        let target_id = "TID-owned-tls";
        let mut context = policy_context(target_id);
        context.set_tls_verify_host_override_for_target(target_id, Some(false));
        context.set_network_offline_for_target(target_id, true);
        let id = context.selected_web_contents_id().unwrap();
        let contents_id = context.selected_web_contents_id().unwrap();
        drop(context.page_targets.remove(target_id).unwrap());
        let contents = context.physical.web_contents.get_mut(&contents_id).unwrap();
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
        let target_id = "TID-owned-offline";
        let mut context = policy_context(target_id);
        context.set_network_offline_for_target(target_id, true);
        context.set_base_cache_disabled_for_target(target_id, true);
        context.clear_devtools_network_state_for_target(target_id, &DevToolsSessionKey::Primary);
        assert!(context.network_offline_for_target(target_id));
        let id = context.selected_web_contents_id().unwrap();
        let contents_id = context.selected_web_contents_id().unwrap();
        drop(context.page_targets.remove(target_id).unwrap());
        let contents = context.physical.web_contents.get_mut(&contents_id).unwrap();
        assert_eq!(contents.id(), id);
        assert!(contents.network_offline);
        contents.set_network_offline(false);
        assert!(!contents.network_offline);
        assert!(contents.network_request_policy.cache_disabled);
    }

    #[test]
    fn browser_identity_survives_projection_drop_and_updates_without_sessions() {
        let target_id = "TID-owned-identity";
        let mut context = policy_context(target_id);
        let base = moli_browser_profile::BrowserIdentityProfile::new("Moli/Base", "en-US");
        context.set_devtools_browser_identity_override_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            crate::conn::DevToolsBrowserIdentityOverride::from_command(
                &base,
                "Moli/Installed".into(),
                Some("fr-FR".into()),
                Some("TestPlatform".into()),
                None,
            ),
        );
        let installed = context
            .browser_identity_override_for_target(target_id)
            .unwrap()
            .clone();
        let snapshot = context.effective_policy_for_target(target_id);
        context
            .web_contents_for_target_mut(target_id)
            .unwrap()
            .set_browser_identity_override(Some(base.clone()));
        assert_eq!(
            context.browser_identity_override_for_target(target_id),
            Some(&base)
        );
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .browser_identity_override(),
            Some(&base)
        );
        assert_eq!(
            context.active_page_target().reported_user_agent_override(),
            Some("Moli/Installed")
        );
        assert_eq!(snapshot.browser_identity_override(), Some(&installed));
        let id = context.selected_web_contents_id().unwrap();
        let contents_id = context.selected_web_contents_id().unwrap();
        drop(context.page_targets.remove(target_id).unwrap());
        let contents = context.physical.web_contents.get_mut(&contents_id).unwrap();
        assert_eq!(contents.id(), id);
        assert_eq!(contents.browser_identity_override, Some(base));
        contents.set_browser_identity_override(Some(installed.clone()));
        assert_eq!(contents.browser_identity_override, Some(installed));
        contents.set_browser_identity_override(None);
        assert!(contents.browser_identity_override.is_none());
    }

    #[test]
    fn reported_user_agent_keeps_its_fallback_for_language_only_runtime_overrides() {
        let target_id = "TID-language-only";
        let mut context = policy_context(target_id);
        let base = moli_browser_profile::BrowserIdentityProfile::new("Moli/Base", "de-DE");
        context.set_base_browser_identity_override_for_target(target_id, Some(base.clone()));
        let handler_base =
            moli_browser_profile::BrowserIdentityProfile::new("Moli/Handler", "en-US");
        context.set_devtools_browser_identity_override_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            crate::conn::DevToolsBrowserIdentityOverride::from_command(
                &handler_base,
                String::new(),
                Some("fr-FR".into()),
                Some("TestPlatform".into()),
                None,
            ),
        );
        let snapshot = context.effective_policy_for_target(target_id);
        let profile = snapshot.browser_identity_override().unwrap();
        assert_eq!(profile.user_agent(), "Moli/Handler");
        assert_eq!(profile.accept_language(), "fr-FR");
        assert_eq!(profile.navigator_platform(), "TestPlatform");
        assert_eq!(
            context.browser_identity_override_for_target(target_id),
            Some(profile)
        );
        assert_eq!(
            context.active_page_target().reported_user_agent_override(),
            Some("Moli/Base")
        );
        context.set_base_browser_identity_override_for_target(target_id, None);
        assert_eq!(context.effective_policy_for_target(target_id), snapshot);
        assert_eq!(
            context.active_page_target().reported_user_agent_override(),
            None
        );
        context.clear_devtools_emulation_policy_state_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
        );
        assert!(
            context
                .effective_policy_for_target(target_id)
                .browser_identity_override()
                .is_none()
        );
        context.set_base_browser_identity_override_for_target(target_id, Some(base.clone()));
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .browser_identity_override(),
            Some(&base)
        );
    }

    #[test]
    fn network_request_policy_survives_projection_drop_and_updates_without_sessions() {
        let target_id = "TID-independent-policy";
        let mut context = policy_context(target_id);
        context.mutate_devtools_network_session_state_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            |raw| {
                raw.network_enabled = true;
                raw.cache_disabled = true;
                raw.bypass_service_worker = true;
                raw.blocked_url_patterns = vec!["blocked/*".into()];
                raw.extra_headers = vec![("X-Owner".into(), "browser".into())];
            },
        );
        let installed = context
            .effective_policy_for_target(target_id)
            .network_request;
        // A Browser-side value update must be observable without rebuilding it
        // from the unchanged frontend contributions on every read.
        let independent = super::NetworkRequestPolicy {
            cache_disabled: false,
            ..installed.clone()
        };
        context
            .web_contents_for_target_mut(target_id)
            .unwrap()
            .set_network_request_policy(independent.clone());
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .network_request,
            independent
        );
        assert!(
            context
                .active_page_target()
                .devtools_sessions
                .primary()
                .network_session_state
                .cache_disabled
        );
        let id = context.selected_web_contents_id().unwrap();
        let contents_id = context.selected_web_contents_id().unwrap();
        drop(context.page_targets.remove(target_id).unwrap());
        let contents = context.physical.web_contents.get_mut(&contents_id).unwrap();
        assert_eq!(contents.id(), id);
        assert_eq!(contents.network_request_policy, independent);
        contents.set_network_request_policy(installed.clone());
        assert_eq!(contents.network_request_policy, installed);
    }

    #[test]
    fn network_policy_preserves_enable_disable_and_base_precedence() {
        let target_id = "TID-network-policy";
        let mut context = policy_context(target_id);
        context.set_base_cache_disabled_for_target(target_id, true);
        let base_headers = vec![("X-Shared".into(), "base".into())];
        context.set_base_extra_headers_for_target(target_id, base_headers.clone());
        context.mutate_devtools_network_session_state_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            |raw| {
                raw.bypass_service_worker = true;
                raw.blocked_url_patterns = vec!["primary/*".into()];
                raw.extra_headers = vec![("x-shared".into(), "primary".into())];
            },
        );
        let disabled = context.effective_policy_for_target(target_id);
        assert!(disabled.cache_disabled());
        assert!(!disabled.bypass_service_worker());
        assert!(disabled.blocked_url_patterns().is_empty());
        assert_eq!(disabled.extra_headers(), base_headers);

        context.mutate_devtools_network_session_state_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            |raw| {
                raw.network_enabled = true;
            },
        );
        // Deliberately reverse lexical order: header precedence is attachment order.
        for session in ["SID-z", "SID-a"] {
            context.mutate_devtools_network_session_state_for_target(
                target_id,
                &DevToolsSessionKey::Attached(session.into()),
                |raw| {
                    raw.network_enabled = true;
                    raw.blocked_url_patterns = vec!["primary/*".into(), format!("{session}/*")];
                    raw.extra_headers = vec![("X-SHARED".into(), session.into())];
                },
            );
        }
        let combined = context.effective_policy_for_target(target_id);
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
        context.clear_devtools_network_state_for_target(
            target_id,
            &DevToolsSessionKey::Attached("SID-a".into()),
        );
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .extra_headers(),
            [("X-SHARED".into(), "SID-z".into())]
        );
        context.clear_devtools_network_state_for_target(target_id, &DevToolsSessionKey::Primary);
        assert!(
            !context
                .effective_policy_for_target(target_id)
                .bypass_service_worker()
        );
        context.clear_devtools_network_state_for_target(
            target_id,
            &DevToolsSessionKey::Attached("SID-z".into()),
        );
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .extra_headers(),
            base_headers
        );
        assert!(
            context
                .effective_policy_for_target(target_id)
                .blocked_url_patterns()
                .is_empty()
        );
        assert!(
            context
                .effective_policy_for_target(target_id)
                .cache_disabled()
        );
        context.set_base_cache_disabled_for_target(target_id, false);
        assert!(
            !context
                .effective_policy_for_target(target_id)
                .cache_disabled()
        );
        assert_eq!(
            combined.extra_headers(),
            [("X-SHARED".into(), "SID-a".into())]
        );
    }

    #[test]
    fn locale_and_timezone_survive_projection_drop_and_independent_field_updates() {
        let target_id = "TID-owned-locale";
        let mut context = policy_context(target_id);
        let session = DevToolsSessionKey::Primary;
        context
            .set_devtools_locale_override_for_target(target_id, &session, Some("fr-FR".into()))
            .unwrap();
        context
            .set_devtools_timezone_override_for_target(
                target_id,
                &session,
                Some("Europe/Paris".into()),
            )
            .unwrap();
        let snapshot = context.effective_policy_for_target(target_id);
        context
            .web_contents_for_target_mut(target_id)
            .unwrap()
            .set_locale_override(Some("ja-JP".into()));
        context
            .set_devtools_timezone_override_for_target(target_id, &session, Some("UTC".into()))
            .unwrap();
        assert_eq!(context.locale_override_for_target(target_id), Some("ja-JP"));
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .locale_override(),
            Some("ja-JP")
        );
        assert_eq!(
            context
                .active_page_target()
                .devtools_sessions
                .primary()
                .emulation_session_state
                .locale_override
                .as_deref(),
            Some("fr-FR")
        );
        context
            .web_contents_for_target_mut(target_id)
            .unwrap()
            .set_timezone_override(Some("Asia/Tokyo".into()));
        context
            .set_devtools_locale_override_for_target(target_id, &session, Some("it-IT".into()))
            .unwrap();
        assert_eq!(
            context.timezone_override_for_target(target_id),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            context
                .effective_policy_for_target(target_id)
                .timezone_override(),
            Some("Asia/Tokyo")
        );
        assert_eq!(
            context
                .active_page_target()
                .devtools_sessions
                .primary()
                .emulation_session_state
                .timezone_override
                .as_deref(),
            Some("UTC")
        );
        assert_eq!(snapshot.locale_override(), Some("fr-FR"));
        assert_eq!(snapshot.timezone_override(), Some("Europe/Paris"));

        let id = context.selected_web_contents_id().unwrap();
        let contents_id = context.selected_web_contents_id().unwrap();
        drop(context.page_targets.remove(target_id).unwrap());
        let contents = context.physical.web_contents.get_mut(&contents_id).unwrap();
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
        let target_id = "TID-locale-policy";
        let mut context = policy_context(target_id);
        let locale_owner = DevToolsSessionKey::Primary;
        let timezone_owner = DevToolsSessionKey::Attached("SID-timezone".into());
        context.set_base_locale_override_for_target(target_id, Some("en-GB".into()));
        context.set_base_timezone_override_for_target(target_id, Some("Europe/London".into()));
        context.set_user_agent_override_for_test_for_target(target_id, "Moli/Unchanged".into());
        context
            .set_devtools_locale_override_for_target(target_id, &locale_owner, Some("fr-FR".into()))
            .unwrap();
        context
            .set_devtools_timezone_override_for_target(
                target_id,
                &timezone_owner,
                Some("Europe/Paris".into()),
            )
            .unwrap();
        let installed = context.effective_policy_for_target(target_id);
        for locale in [Some("de-DE".into()), None] {
            assert_eq!(
                context.set_devtools_locale_override_for_target(target_id, &timezone_owner, locale),
                Err("Another locale override is already in effect")
            );
            assert_eq!(context.effective_policy_for_target(target_id), installed);
        }
        assert_eq!(
            context.set_devtools_timezone_override_for_target(
                target_id,
                &locale_owner,
                Some("Asia/Tokyo".into())
            ),
            Err("Timezone override is already in effect")
        );
        assert_eq!(context.effective_policy_for_target(target_id), installed);
        context
            .set_devtools_timezone_override_for_target(target_id, &locale_owner, None)
            .unwrap();
        assert_eq!(context.effective_policy_for_target(target_id), installed);

        context.set_base_locale_override_for_target(target_id, Some("es-ES".into()));
        context.set_base_timezone_override_for_target(target_id, Some("Europe/Madrid".into()));
        assert_eq!(context.effective_policy_for_target(target_id), installed);
        context
            .set_devtools_locale_override_for_target(target_id, &locale_owner, None)
            .unwrap();
        let locale_cleared = context.effective_policy_for_target(target_id);
        assert_eq!(locale_cleared.locale_override(), Some("es-ES"));
        assert_eq!(locale_cleared.timezone_override(), Some("Europe/Paris"));
        assert_eq!(
            locale_cleared,
            super::EffectiveTargetPolicy {
                locale_override: Some("es-ES".into()),
                ..installed
            }
        );
        context.clear_devtools_emulation_policy_state_for_target(target_id, &timezone_owner);
        let cleared = context.effective_policy_for_target(target_id);
        assert_eq!(cleared.timezone_override(), Some("Europe/Madrid"));
        assert_eq!(
            cleared,
            super::EffectiveTargetPolicy {
                timezone_override: Some("Europe/Madrid".into()),
                ..locale_cleared
            }
        );
    }

    #[test]
    fn devtools_emulation_overrides_reveal_target_base_state_when_cleared() {
        let target_id = "TID-policy-test";
        let mut context = policy_context(target_id);
        context.set_base_locale_override_for_target(target_id, Some("en-GB".to_owned()));
        context.set_base_timezone_override_for_target(target_id, Some("Europe/London".to_owned()));
        context.set_base_browser_identity_override_for_target(
            target_id,
            Some(moli_browser_profile::BrowserIdentityProfile::new(
                "Moli/Base",
                "en-GB",
            )),
        );

        context
            .set_devtools_locale_override_for_target(
                target_id,
                &DevToolsSessionKey::Primary,
                Some("fr-FR".to_owned()),
            )
            .unwrap();
        context
            .set_devtools_timezone_override_for_target(
                target_id,
                &DevToolsSessionKey::Primary,
                Some("Europe/Paris".to_owned()),
            )
            .unwrap();
        context.set_devtools_browser_identity_override_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
            crate::conn::DevToolsBrowserIdentityOverride::from_command(
                &moli_browser_profile::BrowserIdentityProfile::default(),
                "Moli/CDP".to_owned(),
                Some("fr-FR".to_owned()),
                None,
                None,
            ),
        );
        context
            .active_page_target_mut()
            .devtools_sessions
            .primary_mut()
            .emulation_session_state
            .overrides
            .as_mut()
            .unwrap()
            .cpu_throttling_rate = 4.0;
        let effective = context.effective_policy_for_target(target_id);
        assert_eq!(effective.locale_override(), Some("fr-FR"));
        assert_eq!(effective.timezone_override(), Some("Europe/Paris"));
        assert_eq!(
            effective
                .browser_identity_override()
                .map(moli_browser_profile::BrowserIdentityProfile::user_agent),
            Some("Moli/CDP")
        );

        context.clear_devtools_network_state_for_target(target_id, &DevToolsSessionKey::Primary);
        context.clear_devtools_emulation_policy_state_for_target(
            target_id,
            &DevToolsSessionKey::Primary,
        );
        let effective = context.effective_policy_for_target(target_id);
        assert_eq!(effective.locale_override(), Some("en-GB"));
        assert_eq!(effective.timezone_override(), Some("Europe/London"));
        assert_eq!(
            context
                .active_page_target()
                .devtools_sessions
                .primary()
                .emulation_session_state
                .overrides
                .as_ref()
                .unwrap()
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
