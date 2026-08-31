use super::*;
use crate::conn::{EmulatedGeolocationOverrideState, EmulatedNetworkConditions};

impl CdpConnection {
    pub(crate) fn apply_active_engine_fetch_overrides(&mut self) {
        let browser_identity = self
            .browser_context
            .as_ref()
            .and_then(|bc| bc.effective_active_browser_identity_override_owned())
            .or_else(|| self.global_browser_identity_override.clone())
            .unwrap_or_else(|| self.base_browser_identity.clone());
        let http_proxy = self
            .browser_context
            .as_ref()
            .and_then(|bc| bc.effective_active_http_proxy_override_owned())
            .or_else(|| self.base_http_proxy.clone());
        let http_no_proxy = self
            .browser_context
            .as_ref()
            .and_then(|bc| bc.effective_active_http_no_proxy_override_owned())
            .or_else(|| self.base_http_no_proxy.clone());
        let tls_verify_host = self
            .browser_context
            .as_ref()
            .and_then(|bc| bc.effective_active_tls_verify_host_override())
            .unwrap_or(self.base_tls_verify_host);
        let bypass_service_worker = self
            .browser_context
            .as_ref()
            .and_then(|bc| bc.page_targets.active())
            .is_some_and(|host| host.network_policy.bypass_service_worker());
        let engine = self.active_navigation_engine_mut();
        engine.set_browser_identity_override(browser_identity);
        engine.set_http_proxy_override(http_proxy);
        engine.set_http_no_proxy_override(http_no_proxy);
        engine.set_tls_verify_host(tls_verify_host);
        engine.set_bypass_service_worker(bypass_service_worker);
    }

    pub async fn set_tls_verify_host_async(&mut self, enabled: bool) {
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.default_tls_verify_host_override = Some(enabled);
        } else {
            self.base_tls_verify_host = enabled;
        }
        self.apply_active_engine_fetch_overrides();
        self.rebuild_resource_runtime_for_loaded_page_async().await;
    }

    pub fn tls_verify_host(&self) -> bool {
        self.browser_context
            .as_ref()
            .and_then(|bc| bc.effective_active_tls_verify_host_override())
            .unwrap_or(self.base_tls_verify_host)
    }

    pub fn user_agent(&self) -> &str {
        self.browser_context
            .as_ref()
            .and_then(|bc| bc.effective_active_browser_identity_override())
            .or(self.global_browser_identity_override.as_ref())
            .unwrap_or(&self.base_browser_identity)
            .user_agent()
    }

    pub async fn set_user_agent_override_async(&mut self, user_agent: impl Into<String>) {
        let user_agent = user_agent.into();
        let browser_identity = moli_browser_profile::BrowserIdentityProfile::new(
            user_agent.clone(),
            self.base_browser_identity.accept_language(),
        );
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context
                .active_page_target_mut()
                .network_policy
                .set_browser_identity_override(browser_identity);
        } else {
            self.base_browser_identity = browser_identity;
        }
        self.apply_active_engine_fetch_overrides();
        self.rebuild_resource_runtime_for_loaded_page_async().await;
    }

    pub(crate) fn set_global_browser_identity_override_from_user_agent(
        &mut self,
        user_agent: Option<String>,
    ) {
        self.global_browser_identity_override = user_agent.as_ref().map(|user_agent| {
            moli_browser_profile::BrowserIdentityProfile::new(
                user_agent.clone(),
                self.base_browser_identity.accept_language(),
            )
        });
        self.apply_active_engine_fetch_overrides();
    }

    pub(crate) fn set_global_network_conditions(
        &mut self,
        conditions: Option<EmulatedNetworkConditions>,
    ) {
        self.global_network_conditions = conditions;
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.global_network_conditions = conditions;
        }
        for browser_context in &mut self.inactive_browser_contexts {
            browser_context.global_network_conditions = conditions;
        }
    }

    pub(crate) fn set_global_geolocation_override(
        &mut self,
        override_state: Option<EmulatedGeolocationOverrideState>,
    ) {
        self.global_geolocation_override = override_state.clone();
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.global_geolocation_override = override_state.clone();
        }
        for browser_context in &mut self.inactive_browser_contexts {
            browser_context.global_geolocation_override = override_state.clone();
        }
    }

    #[cfg(test)]
    pub(crate) async fn set_http_proxy_override_async(&mut self, proxy: Option<String>) {
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.default_http_proxy_override = proxy;
        } else {
            self.base_http_proxy = proxy;
        }
        self.apply_active_engine_fetch_overrides();
        self.rebuild_resource_runtime_for_loaded_page_async().await;
    }

    pub fn http_proxy(&self) -> Option<&str> {
        self.browser_context
            .as_ref()
            .and_then(|bc| {
                bc.page_targets
                    .active()
                    .and_then(|host| host.http_proxy_override.as_deref())
                    .or(bc.default_http_proxy_override.as_deref())
            })
            .or(self.base_http_proxy.as_deref())
    }

    pub(crate) fn http_proxy_for_session_owner_owned(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.navigation_load_inputs_for_session_owner(session_id)
            .http_proxy_override
            .or_else(|| self.base_http_proxy.clone())
    }

    pub fn http_no_proxy(&self) -> Option<&str> {
        self.browser_context
            .as_ref()
            .and_then(|bc| {
                bc.page_targets
                    .active()
                    .and_then(|host| host.http_no_proxy_override.as_deref())
                    .or(bc.default_http_no_proxy_override.as_deref())
            })
            .or(self.base_http_no_proxy.as_deref())
    }

    pub(crate) fn fetch_config(&self) -> &moli_fetch::FetchConfig {
        self.browser_context
            .as_ref()
            .and_then(|context| context.page_targets.active())
            .and_then(crate::conn::state::PageTargetHost::navigation_engine)
            .map(moli_core::runtime::NavigationEngine::fetch_config)
            .unwrap_or_else(|| self.standalone_navigation_engine.fetch_config())
    }

    pub(crate) fn base_browser_identity(&self) -> &moli_browser_profile::BrowserIdentityProfile {
        &self.base_browser_identity
    }
}
