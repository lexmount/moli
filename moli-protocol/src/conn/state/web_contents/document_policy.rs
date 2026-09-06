use moli_core::{page::PermissionOverrideRegistration, runtime::PreparedDocumentPagePolicy};
use moli_fetch::FetchConfig;
use url::Url;

use super::{WebContents, network_request_policy::merge_extra_header_layers};
use crate::conn::state::browser_context::{
    BrowserContextStoragePartitionHandles, ContextEmulationDefaults,
};

/// Source-free context inheritance, captured during Browser admission. It is
/// not a second stored policy and contains no inspection/session configuration.
pub(in crate::conn::state) struct InheritedDocumentPolicy {
    pub(in crate::conn::state) fetch_config: FetchConfig,
    pub(in crate::conn::state) extra_headers: Vec<(String, String)>,
    pub(in crate::conn::state) emulation: ContextEmulationDefaults,
    pub(in crate::conn::state) permissions: Vec<PermissionOverrideRegistration>,
    pub(in crate::conn::state) storage: BrowserContextStoragePartitionHandles,
}

impl WebContents {
    pub(super) fn configure_navigation_resources(
        &mut self,
        inherited: &InheritedDocumentPolicy,
    ) -> Result<
        (
            moli_browser_profile::BrowserIdentityProfile,
            Vec<(String, String)>,
            bool,
        ),
        String,
    > {
        let navigator_identity = self
            .browser_identity_override
            .clone()
            .unwrap_or_else(|| inherited.fetch_config.browser_identity().clone());
        let extra_http_headers = merge_extra_header_layers(&[
            &inherited.extra_headers,
            &self.network_request_policy.extra_headers,
        ]);
        let network_offline = self.network_offline
            || self
                .emulation_policy
                .network_conditions
                .or(inherited.emulation.network_conditions)
                .is_some_and(|conditions| !conditions.navigator_online());
        let engine = self
            .navigation_engine
            .as_mut()
            .ok_or("navigation WebContents engine unavailable")?;
        engine.set_browser_identity_override(navigator_identity.clone());
        engine.set_http_proxy_override(inherited.fetch_config.http_proxy().map(str::to_owned));
        engine
            .set_http_no_proxy_override(inherited.fetch_config.http_no_proxy().map(str::to_owned));
        engine.set_tls_verify_host(
            self.tls_verify_host_override
                .unwrap_or(inherited.fetch_config.tls_verify_host()),
        );
        engine.set_extra_http_headers(&extra_http_headers);
        engine.set_network_offline(network_offline);
        engine.set_blocked_url_patterns(&self.network_request_policy.blocked_url_patterns);
        engine.set_bypass_service_worker(self.network_request_policy.bypass_service_worker);
        engine.set_cache_disabled(self.network_request_policy.cache_disabled);
        engine
            .ensure_resource_runtime_ready_for_navigation_storage(
                inherited
                    .storage
                    .resource_storage_handles(self.session_storage.store().clone())
                    .into_navigation_storage(),
            )
            .map_err(|error| format!("failed to initialize resource runtime: {error}"))?;
        Ok((navigator_identity, extra_http_headers, network_offline))
    }

    pub(in crate::conn::state) fn capture_document_policy(
        &mut self,
        inherited: InheritedDocumentPolicy,
        final_url: &Url,
    ) -> Result<PreparedDocumentPagePolicy, String> {
        let (navigator_identity, extra_http_headers, network_offline) =
            self.configure_navigation_resources(&inherited)?;
        // Contexts share a renderer runtime, not a Page transport identity.
        // Never copy the resource runtime most recently registered by a peer.
        let browser_resource_runtime = self
            .navigation_engine
            .as_ref()
            .expect("configured engine")
            .resource_request_client()
            .ok_or("resource request client unavailable")?
            .browser_resource_runtime();
        let idle_override = self
            .main_frame
            .current_document
            .as_ref()
            .and_then(|document| {
                moli_site::same_site_urls(document.page.final_url(), final_url, true)
                    .then(|| document.page.idle_override())
                    .flatten()
            });
        Ok(PreparedDocumentPagePolicy {
            permission_overrides: inherited.permissions,
            extra_http_headers,
            locale_override: self.locale_override.clone().or(inherited.emulation.locale),
            timezone_override: self
                .timezone_override
                .clone()
                .or(inherited.emulation.timezone),
            script_execution_disabled: self.emulation_policy.script_execution_disabled,
            bypass_content_security_policy: self.bypass_content_security_policy,
            cpu_throttling_rate: self.emulation_policy.cpu_throttling_rate,
            emulated_media: (&self.emulation_policy.emulated_media).into(),
            idle_override,
            viewport_surface: self
                .emulation_policy
                .emulated_device_metrics
                .as_ref()
                .or(inherited.emulation.device_metrics.as_ref())
                .map(|metrics| metrics.viewport_surface().to_page_viewport_surface()),
            browser_resource_runtime,
            navigator_identity,
            network_offline,
            bypass_service_worker: self.network_request_policy.bypass_service_worker,
            cache_disabled: self.network_request_policy.cache_disabled,
            blocked_url_patterns: self.network_request_policy.blocked_url_patterns.clone(),
            fetch_subresource_interception_enabled: self.fetch_subresource_interception.0,
            fetch_subresource_interception_resource_type: self.fetch_subresource_interception.1,
        })
    }
}
