use super::devtools_session::DevToolsNetworkSessionState;
use super::emulation::EmulatedMediaOverrides;
use super::javascript_dialog::TargetJavaScriptDialogState;
use super::page_target_host::PageTargetHost;
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
    pub(crate) fn effective_user_agent_override(&self) -> Option<&str> {
        self.devtools_sessions
            .effective_user_agent_override()
            .or_else(|| {
                self.network_policy
                    .base_browser_identity
                    .profile()
                    .map(moli_browser_profile::BrowserIdentityProfile::user_agent)
            })
    }

    pub(crate) fn effective_policy(&self) -> EffectiveTargetPolicy {
        let devtools_network = self.devtools_sessions.effective_network_policy();
        let mut extra_headers = self.network_policy.base_extra_headers.clone();
        overlay_extra_headers(&mut extra_headers, &devtools_network.extra_headers);
        EffectiveTargetPolicy {
            cache_disabled: self.network_policy.base_cache_disabled
                || devtools_network.cache_disabled,
            bypass_service_worker: devtools_network.bypass_service_worker,
            blocked_url_patterns: devtools_network.blocked_url_patterns,
            extra_headers,
            browser_identity_override: self
                .devtools_sessions
                .effective_network_browser_identity_override()
                .or_else(|| self.network_policy.base_browser_identity.profile_owned()),
            renderer_browser_identity_override: self
                .devtools_sessions
                .effective_renderer_browser_identity_override()
                .or_else(|| self.network_policy.base_browser_identity.profile_owned()),
            locale_override: self
                .devtools_sessions
                .effective_locale_override()
                .map(str::to_owned)
                .or_else(|| self.base_locale_override.clone()),
            timezone_override: self
                .devtools_sessions
                .effective_timezone_override()
                .map(str::to_owned)
                .or_else(|| self.base_timezone_override.clone()),
        }
    }

    pub(crate) fn effective_renderer_browser_identity_override_owned(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.effective_policy()
            .renderer_browser_identity_override_owned()
    }

    pub(crate) fn mutate_devtools_network_session_state<T>(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        f: impl FnOnce(&mut DevToolsNetworkSessionState) -> T,
    ) -> T {
        let session = self.devtools_sessions.ensure_session(session_key);
        f(&mut session.network_session_state)
    }

    pub(crate) fn set_devtools_browser_identity_override(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        browser_identity_override: Option<super::DevToolsBrowserIdentityOverride>,
    ) {
        self.devtools_sessions
            .set_browser_identity_override(session_key, browser_identity_override);
    }

    pub(crate) fn set_devtools_locale_override(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.devtools_sessions
            .set_locale_override(session_key, locale_override)
    }

    pub(crate) fn set_devtools_timezone_override(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.devtools_sessions
            .set_timezone_override(session_key, timezone_override)
    }

    pub(crate) fn set_base_locale_override(&mut self, locale_override: Option<String>) {
        self.base_locale_override = locale_override;
    }

    pub(crate) fn set_base_timezone_override(&mut self, timezone_override: Option<String>) {
        self.base_timezone_override = timezone_override;
    }

    pub(crate) fn clear_devtools_network_state(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.devtools_sessions.clear_network_state(session_key);
    }

    pub(crate) fn clear_devtools_emulation_state(
        &mut self,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) {
        self.devtools_sessions.clear_emulation_state(session_key);
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
            || self.network_policy != TargetNetworkPolicyState::default()
            || self.http_proxy_override.is_some()
            || self.http_no_proxy_override.is_some()
            || self.tls_verify_host_override.is_some()
            || self.base_locale_override.is_some()
            || self.base_timezone_override.is_some()
            || self.network_conditions.is_some()
            || self.geolocation_override.is_some()
            || self.emulated_media != EmulatedMediaOverrides::default()
            || self.emulated_device_metrics.is_some()
            || self.cpu_throttling_rate != 1.0
            || self.input_intercept_drags_enabled
            || self.input_drag_intercepted
            || self.touch_emulation_enabled
            || self.emit_touch_events_for_mouse
            || self.focus_emulation_enabled
            || self.script_execution_disabled
            || self.css_enabled
            || self.fetch_owner.config_snapshot() != super::fetch::TargetFetchConfig::default()
    }

    /// Clears target-level state owned by the primary DevTools handlers.
    ///
    /// Per-session handler state, Fetch state, and Network observation state
    /// have their own disposal steps. Keeping them out of this helper makes
    /// the final session-registry removal a pure commit operation.
    pub(crate) fn reset_primary_session_target_state_fields(&mut self) {
        self.network_policy.clear_session_scoped_state();
        self.http_proxy_override = None;
        self.http_no_proxy_override = None;
        self.tls_verify_host_override = None;
        self.network_conditions = None;
        self.geolocation_override = None;
        self.emulated_device_metrics = None;
        self.input_intercept_drags_enabled = false;
        self.input_drag_intercepted = false;
        self.touch_emulation_enabled = false;
        self.emit_touch_events_for_mouse = false;
        self.focus_emulation_enabled = false;
        self.script_execution_disabled = false;
        self.css_enabled = false;
    }
}

fn overlay_extra_headers(effective: &mut Vec<(String, String)>, layer: &[(String, String)]) {
    for (name, value) in layer {
        effective.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        effective.push((name.clone(), value.clone()));
    }
}

/// Immutable policy derived from target-owned base state and every attached
/// DevTools session. It is deliberately not stored on `PageTargetHost`: the
/// session registry remains the only source of truth.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EffectiveTargetPolicy {
    cache_disabled: bool,
    bypass_service_worker: bool,
    blocked_url_patterns: Vec<String>,
    extra_headers: Vec<(String, String)>,
    browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    renderer_browser_identity_override: Option<moli_browser_profile::BrowserIdentityProfile>,
    locale_override: Option<String>,
    timezone_override: Option<String>,
}

impl EffectiveTargetPolicy {
    pub(crate) fn delta(&self, next: &Self) -> EffectiveTargetPolicyDelta {
        EffectiveTargetPolicyDelta {
            network_request: self.cache_disabled != next.cache_disabled
                || self.bypass_service_worker != next.bypass_service_worker
                || self.blocked_url_patterns != next.blocked_url_patterns
                || self.extra_headers != next.extra_headers,
            browser_identity: self.browser_identity_override != next.browser_identity_override,
            renderer_browser_identity: self.renderer_browser_identity_override
                != next.renderer_browser_identity_override,
            locale: self.locale_override != next.locale_override,
            timezone: self.timezone_override != next.timezone_override,
        }
    }

    pub(crate) fn cache_disabled(&self) -> bool {
        self.cache_disabled
    }

    pub(crate) fn bypass_service_worker(&self) -> bool {
        self.bypass_service_worker
    }

    pub(crate) fn blocked_url_patterns(&self) -> &[String] {
        &self.blocked_url_patterns
    }

    pub(crate) fn extra_headers(&self) -> &[(String, String)] {
        &self.extra_headers
    }

    pub(crate) fn browser_identity_override(
        &self,
    ) -> Option<&moli_browser_profile::BrowserIdentityProfile> {
        self.browser_identity_override.as_ref()
    }

    pub(crate) fn renderer_browser_identity_override_owned(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.renderer_browser_identity_override.clone()
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
    pub(crate) renderer_browser_identity: bool,
    pub(crate) locale: bool,
    pub(crate) timezone: bool,
}

impl EffectiveTargetPolicyDelta {
    pub(crate) fn browser_identity_changed(self) -> bool {
        self.browser_identity || self.renderer_browser_identity
    }

    pub(crate) fn is_empty(self) -> bool {
        !self.network_request
            && !self.browser_identity
            && !self.renderer_browser_identity
            && !self.locale
            && !self.timezone
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TargetNetworkPolicyState {
    // Target-wide policy contributed by WebDriver BiDi or connection defaults.
    base_cache_disabled: bool,
    network_offline: bool,
    emulated_network_latency: f64,
    emulated_download_throughput: f64,
    emulated_upload_throughput: f64,
    emulated_connection_type: Option<String>,
    base_browser_identity: super::BaseBrowserIdentityOverrideState,
    // Target-scoped headers contributed by WebDriver BiDi.
    base_extra_headers: Vec<(String, String)>,
}

impl Default for TargetNetworkPolicyState {
    fn default() -> Self {
        Self {
            base_cache_disabled: false,
            network_offline: false,
            emulated_network_latency: 0.0,
            emulated_download_throughput: -1.0,
            emulated_upload_throughput: -1.0,
            emulated_connection_type: None,
            base_browser_identity: super::BaseBrowserIdentityOverrideState::default(),
            base_extra_headers: Vec::new(),
        }
    }
}

impl TargetNetworkPolicyState {
    pub(crate) fn set_base_cache_disabled(&mut self, cache_disabled: bool) {
        self.base_cache_disabled = cache_disabled;
    }

    pub(crate) fn clear_session_scoped_state(&mut self) {
        let base_cache_disabled = self.base_cache_disabled;
        let base_extra_headers = std::mem::take(&mut self.base_extra_headers);
        let base_browser_identity = std::mem::take(&mut self.base_browser_identity);
        *self = Self::default();
        self.base_cache_disabled = base_cache_disabled;
        self.base_extra_headers = base_extra_headers;
        self.base_browser_identity = base_browser_identity;
    }

    pub(crate) fn network_offline(&self) -> bool {
        self.network_offline
    }

    #[cfg(test)]
    pub(crate) fn set_network_offline(&mut self, network_offline: bool) {
        self.network_offline = network_offline;
    }

    pub(crate) fn replace_base_extra_headers(&mut self, extra_headers: Vec<(String, String)>) {
        self.base_extra_headers = extra_headers;
    }

    #[cfg(test)]
    pub(crate) fn push_extra_header(&mut self, header: (String, String)) {
        let mut headers = self.base_extra_headers.clone();
        headers.push(header);
        self.replace_base_extra_headers(headers);
    }

    #[cfg(test)]
    pub(crate) fn set_user_agent_override(&mut self, user_agent: String) {
        self.set_browser_identity_override(moli_browser_profile::BrowserIdentityProfile::new(
            user_agent,
            moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE,
        ));
    }

    pub(crate) fn set_browser_identity_override(
        &mut self,
        browser_identity: moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.base_browser_identity.replace_profile(browser_identity);
    }

    pub(crate) fn clear_browser_identity_override(&mut self) {
        self.base_browser_identity = super::BaseBrowserIdentityOverrideState::default();
    }

    pub(crate) fn set_base_user_agent_override(
        &mut self,
        user_agent: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.base_browser_identity
            .set_user_agent(user_agent, fallback);
    }

    pub(crate) fn set_base_accept_language_override(
        &mut self,
        accept_language: Option<String>,
        fallback: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        self.base_browser_identity
            .set_accept_language(accept_language, fallback);
    }

    #[cfg(test)]
    pub(crate) fn emulated_network_latency(&self) -> f64 {
        self.emulated_network_latency
    }

    #[cfg(test)]
    pub(crate) fn emulated_download_throughput(&self) -> f64 {
        self.emulated_download_throughput
    }

    #[cfg(test)]
    pub(crate) fn emulated_upload_throughput(&self) -> f64 {
        self.emulated_upload_throughput
    }

    #[cfg(test)]
    pub(crate) fn emulated_connection_type(&self) -> Option<&str> {
        self.emulated_connection_type.as_deref()
    }

    pub(crate) fn set_emulated_network_conditions(
        &mut self,
        offline: bool,
        latency: f64,
        download_throughput: f64,
        upload_throughput: f64,
        connection_type: Option<String>,
    ) -> bool {
        self.network_offline = offline;
        self.emulated_network_latency = latency;
        self.emulated_download_throughput = download_throughput;
        self.emulated_upload_throughput = upload_throughput;
        self.emulated_connection_type = connection_type;
        self.network_offline
    }
}

#[cfg(test)]
mod tests {
    use super::{PageScreencastConfig, PageScreencastFormat, PageScreencastSessionState};
    use crate::conn::PageTargetHost;
    use moli_page_types::DevToolsSessionKey;

    #[test]
    fn devtools_emulation_overrides_reveal_target_base_state_when_cleared() {
        let mut state = PageTargetHost::empty("TID-policy-test".to_owned());
        state.set_base_locale_override(Some("en-GB".to_owned()));
        state.set_base_timezone_override(Some("Europe/London".to_owned()));
        state.network_policy.set_browser_identity_override(
            moli_browser_profile::BrowserIdentityProfile::new("Moli/Base", "en-GB"),
        );

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
        state.clear_devtools_emulation_state(&DevToolsSessionKey::Primary);
        let effective = state.effective_policy();
        assert_eq!(effective.locale_override(), Some("en-GB"));
        assert_eq!(effective.timezone_override(), Some("Europe/London"));
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
