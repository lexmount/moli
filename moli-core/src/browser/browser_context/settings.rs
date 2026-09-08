use std::collections::HashSet;
#[cfg(any(test, feature = "test-support"))]
use std::path::Path;

use moli_browser_profile::BrowserIdentityProfile;
#[cfg(any(test, feature = "test-support"))]
use moli_cookie_jar::{BrowserCookieStore, SharedBrowserCookieStore};
use moli_cookie_jar::{
    CookieSource, NetworkCookieRequestContext, StoredCookie, StoredCookiePartitionKey,
    StoredCookieQueryReport, StoredCookieSetReport,
};

use crate::{
    RendererBrowserContextRuntimeId, RendererOwnerLocalHostId,
    browser::{
        BrowserContextPageStorageHandles, ContextEmulationDefaults, ContextNetworkPolicy,
        DownloadAccessError, DownloadBody, DownloadObservation, DownloadPolicy,
        EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
        OriginStorageUsage, PermissionDefaults, SiteDataClearOptions, StoragePartitionKind,
        WebContentsHandle,
    },
    network::{ResourceRequestClient, new_shared_web_storage_store},
    page::PermissionOverrideRegistration,
    runtime::RendererSharedWorkerRuntimeDiagnostics,
};
#[cfg(any(test, feature = "test-support"))]
use crate::{
    browser::BrowserContextResourceStorageHandles,
    network::SharedWebStorageStore,
    storage::{SharedIndexedDbManager, SharedStorageBucketStore},
};
use moli_fetch::Request;
use url::Url;

use super::BrowserContext;

impl BrowserContext {
    pub fn download_policy(&self) -> Option<&DownloadPolicy> {
        self.download_policy.as_ref()
    }

    pub fn set_download_policy(&mut self, policy: Option<DownloadPolicy>) {
        self.download_policy = policy;
    }

    pub fn start_download_request(
        &mut self,
        policy: &DownloadPolicy,
        client: ResourceRequestClient,
        request: Request,
        suggested_filename: Option<String>,
    ) -> Result<Option<DownloadObservation>, String> {
        self.downloads
            .start_request(policy, client, request, suggested_filename)
    }

    pub fn start_download_response(
        &mut self,
        web_contents: WebContentsHandle,
        policy: &DownloadPolicy,
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    ) -> Result<Option<DownloadObservation>, String> {
        self.web_contents(web_contents)?;
        self.downloads.start_response(policy, url, headers, body)
    }

    pub fn cancel_download(&self, guid: &str) -> Option<Result<(), DownloadAccessError>> {
        self.downloads.cancel(guid)
    }

    pub fn read_download_artifact(
        &self,
        guid: &str,
    ) -> Option<Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, DownloadAccessError>> {
        self.downloads.read_artifact(guid)
    }

    pub fn set_permission_override(
        &mut self,
        defaults: &mut PermissionDefaults,
        registration: PermissionOverrideRegistration,
    ) {
        self.permission_overrides.set(defaults, registration);
    }

    pub fn clear_permission_overrides(&mut self) {
        self.permission_overrides.clear();
    }

    pub fn permission_override_count(&self) -> usize {
        self.permission_overrides.override_count()
    }

    pub fn permission_snapshot(
        &self,
        defaults: &PermissionDefaults,
    ) -> Vec<PermissionOverrideRegistration> {
        self.permission_overrides.snapshot(defaults)
    }

    pub fn storage_partition_kind(&self) -> StoragePartitionKind {
        self.storage_partition.kind
    }

    pub fn storage_partition_kind_label(&self) -> &'static str {
        self.storage_partition.kind_label()
    }

    pub fn page_storage_handles(
        &self,
        web_contents: Option<WebContentsHandle>,
    ) -> Result<BrowserContextPageStorageHandles, String> {
        let session_storage_store = match web_contents {
            Some(handle) => self.web_contents(handle)?.session_storage.store().clone(),
            None => self
                .selected_web_contents_id()
                .and_then(|id| self.web_contents.get(&id))
                .map(|contents| contents.session_storage.store().clone())
                .unwrap_or_else(new_shared_web_storage_store),
        };
        Ok(self
            .storage_partition
            .handles
            .page_storage_handles(session_storage_store))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn resource_storage_handles_for_test(&self) -> BrowserContextResourceStorageHandles {
        let session_storage_store = self
            .selected_web_contents_id()
            .and_then(|id| self.web_contents.get(&id))
            .map(|contents| contents.session_storage.store().clone())
            .unwrap_or_else(new_shared_web_storage_store);
        self.storage_partition
            .handles
            .resource_storage_handles(session_storage_store)
    }

    pub fn observe_request_cookie_access_report(
        &self,
        request_url: &Url,
        request_context: NetworkCookieRequestContext,
    ) -> Option<StoredCookieQueryReport> {
        let mut cookie_store = self.storage_partition.cookie_store().lock();
        let report =
            cookie_store.observe_cookie_access_report_for_request(request_url, request_context);
        (!report.included_cookies.is_empty() || !report.excluded_cookies.is_empty())
            .then_some(report)
    }

    pub fn storage_quota_for_origin(&self, origin: &str) -> (f64, bool) {
        self.storage_partition.storage_quota_for_origin(origin)
    }

    pub fn set_storage_quota_override(&mut self, origin: String, quota: f64) {
        self.storage_partition
            .set_storage_quota_override(origin, quota);
    }

    pub fn clear_storage_quota_override(&mut self, origin: &str) {
        self.storage_partition.clear_storage_quota_override(origin);
    }

    pub fn storage_usage_for_origin(
        &self,
        serialized_origin: &str,
    ) -> Result<OriginStorageUsage, String> {
        self.storage_partition.usage_for_origin(serialized_origin)
    }

    pub fn clear_site_data_for_origin(
        &mut self,
        origin: &Url,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        self.storage_partition
            .clear_site_data_for_origin(origin, options)
    }

    pub fn clear_site_data_for_storage_key(
        &mut self,
        storage_key: &moli_storage_key::MoliStorageKey,
        options: SiteDataClearOptions,
    ) -> Result<(), String> {
        self.storage_partition
            .clear_site_data_for_storage_key(storage_key, options)
    }

    pub fn clear_http_cache(&self) -> Result<(), String> {
        self.storage_partition.clear_http_cache()
    }

    pub fn snapshot_cookies(&self) -> Vec<StoredCookie> {
        self.storage_partition.cookie_store().lock().cookies()
    }

    pub fn store_cookie(
        &self,
        cookie: StoredCookie,
        request_url: Option<&Url>,
        source: CookieSource,
    ) -> StoredCookieSetReport {
        self.storage_partition
            .cookie_store()
            .lock()
            .upsert_with_request_url_report(cookie, request_url, source)
    }

    pub fn delete_cookies_with_partition_key(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
        partition_key: Option<&StoredCookiePartitionKey>,
    ) {
        self.storage_partition
            .cookie_store()
            .lock()
            .delete_cookies_with_partition_key(name, domain, path, url_host, partition_key);
    }

    pub fn browser_identity_override(&self) -> Option<&BrowserIdentityProfile> {
        self.browser_identity_override.as_ref()
    }

    pub fn set_browser_identity_override(&mut self, identity: Option<BrowserIdentityProfile>) {
        self.browser_identity_override = identity;
    }

    pub fn emulation_defaults(&self) -> &ContextEmulationDefaults {
        &self.emulation_defaults
    }

    pub fn set_default_locale_override(&mut self, locale: Option<String>) {
        self.emulation_defaults.locale = locale;
    }

    pub fn set_default_timezone_override(&mut self, timezone: Option<String>) {
        self.emulation_defaults.timezone = timezone;
    }

    pub fn set_default_network_conditions(
        &mut self,
        conditions: Option<EmulatedNetworkConditions>,
    ) {
        self.emulation_defaults.network_conditions = conditions;
    }

    pub fn set_default_geolocation_override(
        &mut self,
        geolocation: Option<EmulatedGeolocationOverrideState>,
    ) {
        self.emulation_defaults.geolocation = geolocation;
    }

    pub fn set_default_device_metrics(&mut self, metrics: EmulatedDeviceMetrics) -> bool {
        self.emulation_defaults
            .device_metrics
            .replace(metrics)
            .is_some()
    }

    pub fn network_policy(&self) -> &ContextNetworkPolicy {
        &self.network_policy
    }

    pub fn set_default_extra_headers(&mut self, headers: Vec<(String, String)>) {
        self.network_policy.extra_headers = headers;
    }

    pub fn set_network_policy(&mut self, policy: ContextNetworkPolicy) {
        self.network_policy = policy;
    }

    pub fn set_context_tls_verify_host_override(&mut self, enabled: bool) {
        self.network_policy.tls_verify_host = Some(enabled);
    }

    pub fn set_javascript_dialog_handler_enabled(&self, enabled: bool) {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
            .set_javascript_dialog_handler_enabled(enabled);
    }

    pub fn routes_renderer_browser_context_runtime(
        &self,
        runtime_id: RendererBrowserContextRuntimeId,
    ) -> bool {
        self.renderer_runtime_owner
            .as_ref()
            .is_some_and(|owner| owner.handle().id() == runtime_id)
    }

    pub fn renderer_memory_diagnostics(&self) -> serde_json::Value {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
            .moli_memory_diagnostics()
    }

    pub fn shared_worker_runtime_diagnostics(&self) -> RendererSharedWorkerRuntimeDiagnostics {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
            .shared_worker_runtime_diagnostics_for_diagnostics()
    }

    pub fn web_contents_for_renderer_owner(
        &self,
        owner_local_host_id: RendererOwnerLocalHostId,
    ) -> Option<WebContentsHandle> {
        let id = self.web_contents.iter().find_map(|(id, contents)| {
            contents
                .main_frame
                .current_document
                .as_ref()
                .is_some_and(|document| {
                    document.page.renderer_owner_local_host_id() == owner_local_host_id
                })
                .then_some(*id)
        })?;
        Some(WebContentsHandle::new(self.id, id))
    }

    pub fn loaded_document_renderer_owner_ids(&self) -> HashSet<u64> {
        self.web_contents
            .values()
            .filter_map(|contents| contents.main_frame.current_document.as_ref())
            .map(|document| document.page.renderer_owner_local_host_id().as_u64())
            .collect()
    }

    pub fn dedicated_worker_running_isolate_count(&self) -> usize {
        self.web_contents
            .values()
            .filter_map(|contents| contents.main_frame.current_document.as_ref())
            .map(|document| {
                document
                    .page
                    .dedicated_worker_running_worker_isolate_count_for_diagnostics()
            })
            .sum()
    }

    pub fn javascript_dialog_handler_enabled(&self) -> bool {
        self.renderer_runtime_owner
            .as_ref()
            .expect("BrowserContext renderer owner was already taken for teardown")
            .handle()
            .javascript_dialog_handler_enabled()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn http_cache_configuration_for_test(&self) -> (Option<&Path>, Option<u64>) {
        self.storage_partition.http_cache_configuration()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn cookie_store_for_test(&self) -> &SharedBrowserCookieStore {
        self.storage_partition.cookie_store()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_storage_store_for_test(&self) -> &SharedWebStorageStore {
        self.storage_partition.web_storage_store()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn selected_session_storage_store_for_test(&self) -> Option<&SharedWebStorageStore> {
        Some(
            self.web_contents
                .get(&self.selected_web_contents_id()?)?
                .session_storage
                .store(),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn indexed_db_manager_for_test(&self) -> &SharedIndexedDbManager {
        self.storage_partition.indexed_db_manager()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn storage_bucket_store_for_test(&self) -> &SharedStorageBucketStore {
        self.storage_partition.storage_bucket_store()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn replace_storage_bucket_store_for_test(
        &mut self,
        storage_bucket_store: SharedStorageBucketStore,
    ) {
        self.storage_partition
            .replace_storage_bucket_store(storage_bucket_store);
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn with_cookie_store_for_test<R>(&self, f: impl FnOnce(&BrowserCookieStore) -> R) -> R {
        let cookie_store = self.storage_partition.cookie_store().lock();
        f(&cookie_store)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn with_cookie_store_mut_for_test<R>(
        &self,
        f: impl FnOnce(&mut BrowserCookieStore) -> R,
    ) -> R {
        let mut cookie_store = self.storage_partition.cookie_store().lock();
        f(&mut cookie_store)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_http_proxy_override_for_test(&mut self, proxy: Option<String>) {
        self.network_policy.http_proxy = proxy;
    }
}
