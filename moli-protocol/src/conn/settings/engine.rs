use super::*;
use crate::conn::{EmulatedGeolocationOverrideState, EmulatedNetworkConditions};

impl CdpConnection {
    pub(crate) fn apply_active_engine_fetch_overrides(&mut self) {
        let defaults = self.document_fetch_defaults();
        let browser_globals = self.browser_global_overrides.clone();
        if let Some(context) = self.browser_context.as_mut()
            && let Err(error) = context.configure_selected_navigation_policy(
                defaults,
                &browser_globals.extra_headers,
                browser_globals.network_conditions,
            )
        {
            tracing::warn!(%error, "Browser navigation policy configuration failed");
        }
    }

    pub async fn set_tls_verify_host_async(&mut self, enabled: bool) {
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.set_tls_verify_host_override(enabled);
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

    pub fn user_agent(&self) -> String {
        self.browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.reported_active_user_agent_override())
            .or_else(|| {
                self.global_browser_identity_override
                    .as_ref()
                    .map(|identity| identity.user_agent().to_owned())
            })
            .unwrap_or_else(|| self.base_browser_identity.user_agent().to_owned())
    }

    pub async fn set_user_agent_override_async(&mut self, user_agent: impl Into<String>) {
        let user_agent = user_agent.into();
        let browser_identity = moli_browser_profile::BrowserIdentityProfile::new(
            user_agent.clone(),
            self.base_browser_identity.accept_language(),
        );
        if let Some(browser_context) = self.browser_context.as_mut() {
            let target_id = browser_context
                .active_target_id_owned()
                .expect("active target");
            browser_context
                .set_base_browser_identity_override_for_target(&target_id, Some(browser_identity));
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
        self.browser_global_overrides.network_conditions = conditions;
    }

    pub(crate) fn set_global_geolocation_override(
        &mut self,
        override_state: Option<EmulatedGeolocationOverrideState>,
    ) {
        self.browser_global_overrides.geolocation = override_state;
    }

    #[cfg(test)]
    pub(crate) async fn set_http_proxy_override_async(&mut self, proxy: Option<String>) {
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.set_http_proxy_override_for_test(proxy);
        } else {
            self.base_http_proxy = proxy;
        }
        self.apply_active_engine_fetch_overrides();
        self.rebuild_resource_runtime_for_loaded_page_async().await;
    }

    pub fn http_proxy(&self) -> Option<String> {
        self.browser_context
            .as_ref()
            .and_then(|bc| bc.network_policy().http_proxy)
            .or_else(|| self.base_http_proxy.clone())
    }

    pub(crate) fn http_proxy_for_session_owner_owned(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.navigation_load_inputs_for_session_owner(session_id)
            .http_proxy_override
            .or_else(|| self.base_http_proxy.clone())
    }

    pub fn http_no_proxy(&self) -> Option<String> {
        self.browser_context
            .as_ref()
            .and_then(|bc| bc.network_policy().http_no_proxy)
            .or_else(|| self.base_http_no_proxy.clone())
    }

    pub(crate) fn fetch_config(&self) -> moli_fetch::FetchConfig {
        self.browser_context
            .as_ref()
            .and_then(|context| context.page_navigation_fetch_config(context.active_target_id()?))
            .unwrap_or_else(|| self.document_fetch_defaults())
    }

    pub(crate) fn base_browser_identity(&self) -> &moli_browser_profile::BrowserIdentityProfile {
        &self.base_browser_identity
    }
}
