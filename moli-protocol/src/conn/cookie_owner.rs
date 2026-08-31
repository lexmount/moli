use url::Url;

#[cfg(test)]
use moli_cookie_jar::{
    CookieSiteDataOperation, CookieSiteDataOperationPreviewReport, CookieSiteDataOperationReport,
    CookieStorageClearTarget, CookieStorageStateSnapshot, site_key_for_host,
};
use moli_cookie_jar::{CookieSource, StoredCookie, StoredCookieSetReport};
use moli_core::page::{DocumentCookieBackendConnectionState, DocumentCookieOwnerSnapshot};
#[cfg(test)]
use moli_core::page::{
    DocumentCookieCacheSnapshot, DocumentCookieGetFreshnessStatus, DocumentCookieSetReadinessStatus,
};

#[cfg(test)]
use super::cookie_manager_surface::{
    BrowserContextCookieWriteCapabilitySnapshot, BrowserContextDocumentCookieCapabilitySnapshot,
    BrowserContextDocumentCookieTelemetrySnapshot, BrowserContextFirstCookieRequest,
    BrowserContextStructuredCookieWriteSnapshot,
};
use super::{
    BrowserContext,
    cookie_manager_surface::{
        BrowserContextCookieBackendConnectionState, BrowserContextCookieManagerSurfaceSnapshot,
        BrowserContextDefaultCookieWriteUrlSource,
        BrowserContextStructuredCookieWriteBackendStatus,
    },
};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieGetFreshnessStatus {
    NoLivePage,
    PolicyBlocked,
    NeedsBackendReconnect,
    NoEntry,
    NeedsRevalidationAfterDocumentWrite,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieSetReadinessStatus {
    NoLivePage,
    PolicyBlocked,
    NeedsBackendReconnect,
    ReadyWillInvalidateCache,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextDocumentCookieCapabilitySurfaceSnapshot {
    pub(crate) manager_surface: BrowserContextCookieManagerSurfaceSnapshot,
    pub(crate) capability: Option<BrowserContextDocumentCookieCapabilitySnapshot>,
    pub(crate) write_capability: Option<BrowserContextCookieWriteCapabilitySnapshot>,
    pub(crate) backend_connection_state: BrowserContextCookieBackendConnectionState,
    pub(crate) first_cookie_request: Option<BrowserContextFirstCookieRequest>,
    pub(crate) telemetry: Option<BrowserContextDocumentCookieTelemetrySnapshot>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextDocumentCookieFreshnessSnapshot {
    pub(crate) cache: Option<DocumentCookieCacheSnapshot>,
    pub(crate) cookie_get_freshness_status: BrowserContextCookieGetFreshnessStatus,
    pub(crate) cookie_set_readiness_status: BrowserContextCookieSetReadinessStatus,
    pub(crate) cookie_get_would_need_backend_access: bool,
    pub(crate) cookie_get_would_need_backend_reconnect: bool,
    pub(crate) cookie_get_would_hit_cache: bool,
    pub(crate) cookie_get_would_revalidate_after_write: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextDocumentCookieFacadeSnapshot {
    pub(crate) has_loaded_page: bool,
    pub(crate) page_attachment_id: Option<u64>,
    pub(crate) cookie_store_generation: Option<u64>,
    pub(crate) structured_write: BrowserContextStructuredCookieWriteSnapshot,
    pub(crate) capability_surface: BrowserContextDocumentCookieCapabilitySurfaceSnapshot,
    pub(crate) freshness: BrowserContextDocumentCookieFreshnessSnapshot,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieBoundarySnapshot {
    pub(crate) facade: BrowserContextDocumentCookieFacadeSnapshot,
    pub(crate) storage_state: CookieStorageStateSnapshot,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieBoundaryOperationPreviewReport {
    pub(crate) current_boundary: BrowserContextCookieBoundarySnapshot,
    pub(crate) current_target_boundary: BrowserContextCookieBoundarySnapshot,
    pub(crate) preview: CookieSiteDataOperationPreviewReport,
    pub(crate) resulting_boundary: BrowserContextCookieBoundarySnapshot,
    pub(crate) resulting_target_boundary: BrowserContextCookieBoundarySnapshot,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieBoundaryOperationReport {
    pub(crate) replaced_boundary: BrowserContextCookieBoundarySnapshot,
    pub(crate) replaced_target_boundary: BrowserContextCookieBoundarySnapshot,
    pub(crate) report: CookieSiteDataOperationReport,
    pub(crate) resulting_boundary: BrowserContextCookieBoundarySnapshot,
    pub(crate) resulting_target_boundary: BrowserContextCookieBoundarySnapshot,
}

fn browser_context_structured_cookie_write_backend_status()
-> BrowserContextStructuredCookieWriteBackendStatus {
    BrowserContextStructuredCookieWriteBackendStatus::Available
}

fn browser_context_cookie_backend_connection_state(
    owner: &DocumentCookieOwnerSnapshot,
) -> BrowserContextCookieBackendConnectionState {
    match owner.backend_connection_state {
        DocumentCookieBackendConnectionState::Attached => {
            BrowserContextCookieBackendConnectionState::Attached
        }
        DocumentCookieBackendConnectionState::Disconnected => {
            BrowserContextCookieBackendConnectionState::Disconnected
        }
    }
}

#[cfg(test)]
fn browser_context_cookie_get_freshness_status(
    owner: &DocumentCookieOwnerSnapshot,
) -> BrowserContextCookieGetFreshnessStatus {
    match owner.cookie_get_freshness_status {
        DocumentCookieGetFreshnessStatus::PolicyBlocked => {
            BrowserContextCookieGetFreshnessStatus::PolicyBlocked
        }
        DocumentCookieGetFreshnessStatus::NeedsBackendReconnect => {
            BrowserContextCookieGetFreshnessStatus::NeedsBackendReconnect
        }
        DocumentCookieGetFreshnessStatus::NeedsRevalidationAfterDocumentWrite => {
            BrowserContextCookieGetFreshnessStatus::NeedsRevalidationAfterDocumentWrite
        }
        DocumentCookieGetFreshnessStatus::Reusable
        | DocumentCookieGetFreshnessStatus::NoEntry
        | DocumentCookieGetFreshnessStatus::UrlMismatch
        | DocumentCookieGetFreshnessStatus::StoreGenerationMismatch
        | DocumentCookieGetFreshnessStatus::FacadeGenerationMismatch => {
            // BrowserContext freshness is a manager-owned "what would the next
            // browser-context level get need?" projection. Live-page cache
            // residency and staleness details belong to the page/document
            // owner; from the BrowserContext view they all collapse to "a
            // fresh backend-backed get is still required".
            BrowserContextCookieGetFreshnessStatus::NoEntry
        }
    }
}

#[cfg(test)]
fn browser_context_cookie_get_would_need_backend_access(
    owner: &DocumentCookieOwnerSnapshot,
) -> bool {
    matches!(
        browser_context_cookie_get_freshness_status(owner),
        BrowserContextCookieGetFreshnessStatus::NoEntry
            | BrowserContextCookieGetFreshnessStatus::NeedsRevalidationAfterDocumentWrite
    )
}

#[cfg(test)]
fn browser_context_cookie_get_would_hit_cache(owner: &DocumentCookieOwnerSnapshot) -> bool {
    // The BrowserContext view intentionally does not promise reuse of the
    // live page's internal document-cookie cache.
    let _ = owner;
    false
}

#[cfg(test)]
fn browser_context_cookie_set_readiness_status(
    owner: &DocumentCookieOwnerSnapshot,
) -> BrowserContextCookieSetReadinessStatus {
    match owner.cookie_set_readiness_status {
        DocumentCookieSetReadinessStatus::PolicyBlocked => {
            BrowserContextCookieSetReadinessStatus::PolicyBlocked
        }
        DocumentCookieSetReadinessStatus::NeedsBackendReconnect => {
            BrowserContextCookieSetReadinessStatus::NeedsBackendReconnect
        }
        DocumentCookieSetReadinessStatus::ReadyWillInvalidateCache => {
            BrowserContextCookieSetReadinessStatus::ReadyWillInvalidateCache
        }
    }
}

fn browser_context_cookie_manager_surface_snapshot(
    browser_context: &BrowserContext,
    owner: Option<&DocumentCookieOwnerSnapshot>,
) -> BrowserContextCookieManagerSurfaceSnapshot {
    let snapshot = browser_context.raw_cookie_manager_surface_snapshot();
    let current_document_url = browser_context
        .loaded_page()
        .map(|page| page.final_url().clone());
    let navigation_initiator_url = browser_context
        .loaded_page()
        .and_then(|page| page.navigation_initiator_url().cloned());
    let requested_document_url = browser_context
        .loaded_page()
        .map(|page| page.requested_url().clone());
    let navigation_was_redirected = browser_context
        .loaded_page()
        .is_some_and(|page| page.navigation_redirected());
    let navigation_redirect_count = browser_context
        .loaded_page()
        .map(|page| page.navigation_redirect_count())
        .unwrap_or(0);
    let (default_cookie_write_url, default_cookie_write_url_source) =
        browser_context.default_cookie_write_url_with_source();
    let structured_write_backend_status = browser_context_structured_cookie_write_backend_status();
    let backend_connection_state = owner
        .map(browser_context_cookie_backend_connection_state)
        .unwrap_or(BrowserContextCookieBackendConnectionState::NoLivePage);
    snapshot.hydrated(
        owner,
        current_document_url,
        navigation_initiator_url,
        requested_document_url,
        navigation_was_redirected,
        navigation_redirect_count,
        default_cookie_write_url,
        default_cookie_write_url_source,
        backend_connection_state,
        structured_write_backend_status,
    )
}

#[cfg(test)]
fn browser_context_document_cookie_capability_surface_snapshot(
    browser_context: &BrowserContext,
    owner: Option<&DocumentCookieOwnerSnapshot>,
) -> BrowserContextDocumentCookieCapabilitySurfaceSnapshot {
    let manager_surface = browser_context_cookie_manager_surface_snapshot(browser_context, owner);
    let backend_connection_state = manager_surface.backend_connection_state;
    let capability = manager_surface.document_cookie_capability_snapshot();
    let write_capability = manager_surface.document_cookie_write_capability_snapshot();
    let first_cookie_request = manager_surface.first_cookie_request();
    let telemetry = manager_surface.document_cookie_telemetry_snapshot();

    BrowserContextDocumentCookieCapabilitySurfaceSnapshot {
        manager_surface,
        capability,
        write_capability,
        backend_connection_state,
        first_cookie_request,
        telemetry,
    }
}

#[cfg(test)]
fn browser_context_document_cookie_freshness_snapshot(
    owner: Option<&DocumentCookieOwnerSnapshot>,
) -> BrowserContextDocumentCookieFreshnessSnapshot {
    let cache = owner.map(|owner| owner.cache.clone());
    let cookie_get_freshness_status = owner
        .map(browser_context_cookie_get_freshness_status)
        .unwrap_or(BrowserContextCookieGetFreshnessStatus::NoLivePage);
    let cookie_set_readiness_status = owner
        .map(browser_context_cookie_set_readiness_status)
        .unwrap_or(BrowserContextCookieSetReadinessStatus::NoLivePage);
    // Keep these owner-level operational facts as total booleans. "No live
    // page" is already modeled by the lifecycle/readiness enums, so callers
    // should not need an extra nullability layer here.
    let cookie_get_would_need_backend_access =
        owner.is_some_and(browser_context_cookie_get_would_need_backend_access);
    let cookie_get_would_need_backend_reconnect =
        owner.is_some_and(|owner| owner.cookie_get_would_need_backend_reconnect);
    let cookie_get_would_hit_cache = owner.is_some_and(browser_context_cookie_get_would_hit_cache);
    let cookie_get_would_revalidate_after_write =
        owner.is_some_and(|owner| owner.cookie_get_would_revalidate_after_write);

    BrowserContextDocumentCookieFreshnessSnapshot {
        cache,
        cookie_get_freshness_status,
        cookie_set_readiness_status,
        cookie_get_would_need_backend_access,
        cookie_get_would_need_backend_reconnect,
        cookie_get_would_hit_cache,
        cookie_get_would_revalidate_after_write,
    }
}

#[cfg(test)]
fn browser_context_document_cookie_facade_snapshot(
    browser_context: &BrowserContext,
    owner: Option<&DocumentCookieOwnerSnapshot>,
) -> BrowserContextDocumentCookieFacadeSnapshot {
    let has_loaded_page = browser_context.has_loaded_page();
    let capability_surface =
        browser_context_document_cookie_capability_surface_snapshot(browser_context, owner);
    let structured_write = capability_surface.manager_surface.structured_write.clone();
    BrowserContextDocumentCookieFacadeSnapshot {
        has_loaded_page,
        page_attachment_id: browser_context
            .page_attachment_id()
            .map(super::TargetPageAttachmentId::get),
        cookie_store_generation: Some(browser_context.document_cookie_generation()),
        structured_write,
        capability_surface,
        freshness: browser_context_document_cookie_freshness_snapshot(owner),
    }
}

impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn cookie_manager_surface_snapshot(
        &self,
    ) -> BrowserContextCookieManagerSurfaceSnapshot {
        let owner = self.document_cookie_owner_snapshot();
        browser_context_cookie_manager_surface_snapshot(self, owner.as_ref())
    }

    pub(crate) fn cookie_manager_surface_snapshot_without_live_page(
        &self,
    ) -> BrowserContextCookieManagerSurfaceSnapshot {
        assert!(
            !self.has_loaded_page(),
            "live-page document-cookie owner snapshots must use the pending/async BrowserContext snapshot helpers"
        );
        browser_context_cookie_manager_surface_snapshot(self, None)
    }

    pub(crate) fn cookie_manager_surface_snapshot_with_owner(
        &self,
        owner: &DocumentCookieOwnerSnapshot,
    ) -> BrowserContextCookieManagerSurfaceSnapshot {
        browser_context_cookie_manager_surface_snapshot(self, Some(owner))
    }

    #[cfg(test)]
    pub(crate) async fn cookie_manager_surface_snapshot_async(
        &mut self,
    ) -> BrowserContextCookieManagerSurfaceSnapshot {
        let owner = self.document_cookie_owner_snapshot_async().await;
        browser_context_cookie_manager_surface_snapshot(self, owner.as_ref())
    }

    #[cfg(test)]
    pub(crate) async fn document_cookie_capability_surface_snapshot_async(
        &mut self,
    ) -> BrowserContextDocumentCookieCapabilitySurfaceSnapshot {
        let owner = self.document_cookie_owner_snapshot_async().await;
        browser_context_document_cookie_capability_surface_snapshot(self, owner.as_ref())
    }

    #[cfg(test)]
    pub(crate) async fn document_cookie_freshness_snapshot_async(
        &mut self,
    ) -> BrowserContextDocumentCookieFreshnessSnapshot {
        let owner = self.document_cookie_owner_snapshot_async().await;
        browser_context_document_cookie_freshness_snapshot(owner.as_ref())
    }

    pub(crate) fn execute_structured_cookie_write_with_manager_surface(
        &self,
        manager_surface: &BrowserContextCookieManagerSurfaceSnapshot,
        cookie: StoredCookie,
        request_url: Option<Url>,
    ) -> StoredCookieSetReport {
        if let Some(report) = manager_surface.normalized_cookie_facade_rejection(&cookie) {
            return report;
        }

        self.with_cookie_store_mut(|store| {
            store.upsert_with_request_url_report(cookie, request_url.as_ref(), CookieSource::Cdp)
        })
    }

    #[cfg(test)]
    pub(super) fn cookie_boundary_snapshot_from_storage_state(
        &self,
        storage_state: CookieStorageStateSnapshot,
    ) -> BrowserContextCookieBoundarySnapshot {
        BrowserContextCookieBoundarySnapshot {
            facade: self.document_cookie_facade_snapshot(),
            storage_state,
        }
    }

    #[cfg(test)]
    pub(super) fn storage_state_snapshot_for_clear_target(
        state: &CookieStorageStateSnapshot,
        target: &CookieStorageClearTarget,
    ) -> CookieStorageStateSnapshot {
        match target {
            CookieStorageClearTarget::WholeStore => state.clone(),
            CookieStorageClearTarget::RegistrableSites(sites) => {
                let wanted = sites
                    .iter()
                    .filter_map(|site| site_key_for_host(site))
                    .collect::<std::collections::BTreeSet<_>>();
                let live_site_data = state
                    .live_site_data
                    .iter()
                    .filter(|site| wanted.contains(&site.name))
                    .cloned()
                    .collect::<Vec<_>>();
                let persistent_site_data = state
                    .persistent_site_data
                    .iter()
                    .filter(|site| wanted.contains(&site.name))
                    .cloned()
                    .collect::<Vec<_>>();
                CookieStorageStateSnapshot {
                    store_generation: state.store_generation,
                    live_cookie_count: live_site_data.iter().map(|site| site.cookie_count).sum(),
                    live_site_data,
                    persistent_cookie_count: persistent_site_data
                        .iter()
                        .map(|site| site.cookie_count)
                        .sum(),
                    persistent_site_data,
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_facade_snapshot(
        &self,
    ) -> BrowserContextDocumentCookieFacadeSnapshot {
        let owner = self.document_cookie_owner_snapshot();
        browser_context_document_cookie_facade_snapshot(self, owner.as_ref())
    }

    #[cfg(test)]
    pub(crate) async fn document_cookie_facade_snapshot_async(
        &mut self,
    ) -> BrowserContextDocumentCookieFacadeSnapshot {
        let owner = self.document_cookie_owner_snapshot_async().await;
        browser_context_document_cookie_facade_snapshot(self, owner.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn cookie_boundary_snapshot(&self) -> BrowserContextCookieBoundarySnapshot {
        BrowserContextCookieBoundarySnapshot {
            facade: self.document_cookie_facade_snapshot(),
            storage_state: self.cookie_storage_state_snapshot(),
        }
    }

    #[cfg(test)]
    pub(crate) fn cookie_boundary_snapshot_for_sites(
        &self,
        sites: &[&str],
    ) -> BrowserContextCookieBoundarySnapshot {
        BrowserContextCookieBoundarySnapshot {
            facade: self.document_cookie_facade_snapshot(),
            // The facade still belongs to the whole browsing context even when
            // a higher-level site-data owner asks for a site-scoped storage
            // slice.
            storage_state: self.cookie_storage_state_snapshot_for_sites(sites),
        }
    }

    #[cfg(test)]
    fn cookie_boundary_snapshot_for_target(
        &self,
        target: &CookieStorageClearTarget,
    ) -> BrowserContextCookieBoundarySnapshot {
        match target {
            CookieStorageClearTarget::WholeStore => self.cookie_boundary_snapshot(),
            CookieStorageClearTarget::RegistrableSites(sites) => self
                .cookie_boundary_snapshot_for_sites(
                    &sites.iter().map(String::as_str).collect::<Vec<_>>(),
                ),
        }
    }

    #[cfg(test)]
    pub(crate) fn preview_cookie_boundary_operation(
        &self,
        operation: &CookieSiteDataOperation,
    ) -> Result<BrowserContextCookieBoundaryOperationPreviewReport, String> {
        let current_boundary = self.cookie_boundary_snapshot();
        let target = match operation {
            CookieSiteDataOperation::Clear { target, .. } => target,
        };
        let current_target_boundary = self.cookie_boundary_snapshot_for_target(target);
        let preview = self.preview_cookie_site_data_operation(operation)?;
        let resulting_target_boundary = self.cookie_boundary_snapshot_from_storage_state(
            Self::storage_state_snapshot_for_clear_target(preview.resulting_state(), target),
        );
        let resulting_boundary = BrowserContextCookieBoundarySnapshot {
            facade: current_boundary.facade.clone(),
            storage_state: preview.resulting_state().clone(),
        };
        Ok(BrowserContextCookieBoundaryOperationPreviewReport {
            current_boundary,
            current_target_boundary,
            preview,
            resulting_boundary,
            resulting_target_boundary,
        })
    }

    #[cfg(test)]
    pub(crate) fn apply_cookie_boundary_operation(
        &self,
        operation: &CookieSiteDataOperation,
    ) -> Result<BrowserContextCookieBoundaryOperationReport, String> {
        let replaced_boundary = self.cookie_boundary_snapshot();
        let target = match operation {
            CookieSiteDataOperation::Clear { target, .. } => target,
        };
        let replaced_target_boundary = self.cookie_boundary_snapshot_for_target(target);
        let report = self.apply_cookie_site_data_operation(operation)?;
        let resulting_boundary = self.cookie_boundary_snapshot();
        let resulting_target_boundary = self.cookie_boundary_snapshot_for_target(target);
        Ok(BrowserContextCookieBoundaryOperationReport {
            replaced_boundary,
            replaced_target_boundary,
            report,
            resulting_boundary,
            resulting_target_boundary,
        })
    }

    #[cfg(test)]
    pub(crate) async fn document_cookie_telemetry_snapshot_async(
        &mut self,
    ) -> Option<BrowserContextDocumentCookieTelemetrySnapshot> {
        self.document_cookie_capability_surface_snapshot_async()
            .await
            .telemetry
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_owner_snapshot(&self) -> Option<DocumentCookieOwnerSnapshot> {
        assert!(
            !self.has_loaded_page(),
            "live-page document-cookie owner snapshots must use the async BrowserContext snapshot helpers"
        );
        None
    }

    #[cfg(test)]
    pub(crate) async fn document_cookie_owner_snapshot_async(
        &mut self,
    ) -> Option<DocumentCookieOwnerSnapshot> {
        let page = self
            .page_targets
            .active_mut()?
            .runtime_slot
            .loaded_page_mut()?;
        page.document_cookie_owner_snapshot_async().await.ok()
    }

    pub(super) fn default_cookie_write_url_with_source(
        &self,
    ) -> (Option<Url>, BrowserContextDefaultCookieWriteUrlSource) {
        // Blink's cookie facade owns a default cookie URL for structured API
        // writes so callers do not have to redundantly thread the same
        // document URL into every `set()` call. Prefer the live page URL when
        // one exists; otherwise fall back to the BrowserContext's current URL.
        if let Some(url) = self
            .loaded_page()
            .map(|page| page.final_url().clone())
            .filter(|url| matches!(url.scheme(), "http" | "https"))
        {
            return (
                Some(url),
                BrowserContextDefaultCookieWriteUrlSource::LoadedPage,
            );
        }

        if let Some(url) = self
            .page_targets
            .active()
            .and_then(|host| Url::parse(host.target_url()).ok())
            .filter(|url| matches!(url.scheme(), "http" | "https"))
        {
            return (
                Some(url),
                BrowserContextDefaultCookieWriteUrlSource::BrowserContextUrl,
            );
        }

        (None, BrowserContextDefaultCookieWriteUrlSource::Unavailable)
    }
}
