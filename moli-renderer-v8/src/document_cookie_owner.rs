use url::Url;

use super::host::HostDocumentState;
use super::{
    DocumentCookieCacheSnapshot, DocumentCookieCacheStatus, DocumentCookieCapabilitySnapshot,
    DocumentCookieFacadeTelemetrySnapshot, DocumentCookieFirstOperation,
};
use moli_cookie_jar::{
    BrowserCookieStorageAccessStatus, SharedBrowserCookieStore,
    StoredCookieBrowserContextValueSource, StoredCookieExclusionReason,
    StoredCookieSetRejectionReason,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentCookieBackendConnectionState {
    Attached,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentCookieGetFreshnessStatus {
    PolicyBlocked,
    NeedsBackendReconnect,
    Reusable,
    NoEntry,
    UrlMismatch,
    StoreGenerationMismatch,
    FacadeGenerationMismatch,
    NeedsRevalidationAfterDocumentWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentCookieSetReadinessStatus {
    PolicyBlocked,
    NeedsBackendReconnect,
    ReadyWillInvalidateCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCookieWriteCapabilitySnapshot {
    pub write_enabled: bool,
    pub primary_rejection_reason: Option<StoredCookieSetRejectionReason>,
    pub blocked_reasons: Vec<StoredCookieSetRejectionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCookieBrowserContextSnapshot {
    pub site_for_cookies_url: Option<Url>,
    pub site_for_cookies_source: StoredCookieBrowserContextValueSource,
    pub top_frame_origin_url: Option<Url>,
    pub top_frame_origin_source: StoredCookieBrowserContextValueSource,
    pub storage_access_status: BrowserCookieStorageAccessStatus,
    pub storage_access_source: StoredCookieBrowserContextValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentCookieOwnerSnapshot {
    pub capability: DocumentCookieCapabilitySnapshot,
    pub cache: DocumentCookieCacheSnapshot,
    pub browser_context: DocumentCookieBrowserContextSnapshot,
    pub write_capability: DocumentCookieWriteCapabilitySnapshot,
    pub backend_connection_state: DocumentCookieBackendConnectionState,
    pub cookie_get_freshness_status: DocumentCookieGetFreshnessStatus,
    pub cookie_set_readiness_status: DocumentCookieSetReadinessStatus,
    pub cookie_get_would_need_backend_access: bool,
    pub cookie_get_would_need_backend_reconnect: bool,
    pub cookie_get_would_hit_cache: bool,
    pub cookie_get_would_revalidate_after_write: bool,
    pub first_cookie_request: Option<DocumentCookieFirstOperation>,
    pub telemetry: DocumentCookieFacadeTelemetrySnapshot,
}

fn document_cookie_get_would_need_backend_access(cache: &DocumentCookieCacheSnapshot) -> bool {
    match cache.status {
        DocumentCookieCacheStatus::Reusable
        | DocumentCookieCacheStatus::PolicyBlocked
        | DocumentCookieCacheStatus::StoreUnavailable => false,
        DocumentCookieCacheStatus::NoEntry
        | DocumentCookieCacheStatus::UrlMismatch
        | DocumentCookieCacheStatus::StoreGenerationMismatch
        | DocumentCookieCacheStatus::FacadeGenerationMismatch => true,
    }
}

fn document_cookie_get_would_hit_cache(cache: &DocumentCookieCacheSnapshot) -> bool {
    matches!(cache.status, DocumentCookieCacheStatus::Reusable)
}

fn document_cookie_backend_connection_state(
    cache: &DocumentCookieCacheSnapshot,
) -> DocumentCookieBackendConnectionState {
    if matches!(cache.status, DocumentCookieCacheStatus::StoreUnavailable) {
        DocumentCookieBackendConnectionState::Disconnected
    } else {
        DocumentCookieBackendConnectionState::Attached
    }
}

fn document_cookie_get_would_revalidate_after_write(
    cache: &DocumentCookieCacheSnapshot,
    telemetry: &DocumentCookieFacadeTelemetrySnapshot,
) -> bool {
    document_cookie_get_would_need_backend_access(cache)
        && telemetry.last_operation_was_set == Some(true)
}

fn document_cookie_policy_blocked(capability: &DocumentCookieCapabilitySnapshot) -> bool {
    capability
        .facade_status
        .blocked_reasons
        .iter()
        .any(|reason| {
            matches!(
                reason,
                StoredCookieExclusionReason::CookiesDisabled
                    | StoredCookieExclusionReason::StorageAccessBlocked
            )
        })
}

fn document_cookie_get_freshness_status(
    capability: &DocumentCookieCapabilitySnapshot,
    cache: &DocumentCookieCacheSnapshot,
    telemetry: &DocumentCookieFacadeTelemetrySnapshot,
) -> DocumentCookieGetFreshnessStatus {
    if document_cookie_policy_blocked(capability) {
        return DocumentCookieGetFreshnessStatus::PolicyBlocked;
    }

    match cache.status {
        DocumentCookieCacheStatus::PolicyBlocked => DocumentCookieGetFreshnessStatus::PolicyBlocked,
        DocumentCookieCacheStatus::StoreUnavailable => {
            DocumentCookieGetFreshnessStatus::NeedsBackendReconnect
        }
        _ => {
            if document_cookie_get_would_revalidate_after_write(cache, telemetry) {
                return DocumentCookieGetFreshnessStatus::NeedsRevalidationAfterDocumentWrite;
            }

            match cache.status {
                DocumentCookieCacheStatus::Reusable => DocumentCookieGetFreshnessStatus::Reusable,
                DocumentCookieCacheStatus::NoEntry => DocumentCookieGetFreshnessStatus::NoEntry,
                DocumentCookieCacheStatus::UrlMismatch => {
                    DocumentCookieGetFreshnessStatus::UrlMismatch
                }
                DocumentCookieCacheStatus::StoreGenerationMismatch => {
                    DocumentCookieGetFreshnessStatus::StoreGenerationMismatch
                }
                DocumentCookieCacheStatus::FacadeGenerationMismatch => {
                    DocumentCookieGetFreshnessStatus::FacadeGenerationMismatch
                }
                DocumentCookieCacheStatus::PolicyBlocked
                | DocumentCookieCacheStatus::StoreUnavailable => unreachable!(),
            }
        }
    }
}

fn document_cookie_set_readiness_status(
    capability: &DocumentCookieCapabilitySnapshot,
    backend_connection_state: DocumentCookieBackendConnectionState,
) -> DocumentCookieSetReadinessStatus {
    if matches!(
        backend_connection_state,
        DocumentCookieBackendConnectionState::Disconnected
    ) {
        return DocumentCookieSetReadinessStatus::NeedsBackendReconnect;
    }

    if document_cookie_policy_blocked(capability) {
        return DocumentCookieSetReadinessStatus::PolicyBlocked;
    }

    DocumentCookieSetReadinessStatus::ReadyWillInvalidateCache
}

fn document_cookie_write_capability_snapshot(
    capability: &DocumentCookieCapabilitySnapshot,
    backend_connection_state: DocumentCookieBackendConnectionState,
) -> DocumentCookieWriteCapabilitySnapshot {
    let primary_rejection_reason = if !capability.cookies_enabled_preference {
        Some(StoredCookieSetRejectionReason::CookiesDisabled)
    } else if capability
        .facade_status
        .blocked_reasons
        .contains(&StoredCookieExclusionReason::StorageAccessBlocked)
    {
        Some(StoredCookieSetRejectionReason::StorageAccessBlocked)
    } else if matches!(
        backend_connection_state,
        DocumentCookieBackendConnectionState::Disconnected
    ) {
        Some(StoredCookieSetRejectionReason::StoreUnavailable)
    } else {
        None
    };

    let blocked_reasons = match primary_rejection_reason {
        Some(reason) => vec![reason],
        None => Vec::new(),
    };

    DocumentCookieWriteCapabilitySnapshot {
        write_enabled: blocked_reasons.is_empty(),
        primary_rejection_reason,
        blocked_reasons,
    }
}

impl HostDocumentState {
    pub(super) fn set_cookie_store(&mut self, cookie_store: SharedBrowserCookieStore) {
        self.cookie_store = Some(cookie_store);
        self.invalidate_cookie_cache();
    }

    pub(super) fn clear_cookie_store(&mut self) {
        if self.cookie_store.take().is_some() {
            self.invalidate_cookie_cache();
        }
    }
    pub(super) fn document_cookie_owner_snapshot(&self) -> DocumentCookieOwnerSnapshot {
        self.document_cookie_owner_snapshot_for_url(self.url())
    }

    pub(super) fn document_cookie_owner_snapshot_for_url(
        &self,
        url: &Url,
    ) -> DocumentCookieOwnerSnapshot {
        let capability = self.document_cookie_capability_snapshot();
        let cache = self.document_cookie_cache_snapshot_for_url(url);
        let browser_context = self.document_cookie_browser_context_snapshot();
        let telemetry = self.document_cookie_telemetry_snapshot();
        let backend_connection_state = document_cookie_backend_connection_state(&cache);
        let write_capability =
            document_cookie_write_capability_snapshot(&capability, backend_connection_state);
        let cookie_get_freshness_status =
            document_cookie_get_freshness_status(&capability, &cache, &telemetry);
        let cookie_set_readiness_status =
            document_cookie_set_readiness_status(&capability, backend_connection_state);

        DocumentCookieOwnerSnapshot {
            capability,
            cache: cache.clone(),
            browser_context,
            write_capability,
            backend_connection_state,
            cookie_get_freshness_status,
            cookie_set_readiness_status,
            cookie_get_would_need_backend_access: document_cookie_get_would_need_backend_access(
                &cache,
            ),
            cookie_get_would_need_backend_reconnect: matches!(
                backend_connection_state,
                DocumentCookieBackendConnectionState::Disconnected
            ),
            cookie_get_would_hit_cache: document_cookie_get_would_hit_cache(&cache),
            cookie_get_would_revalidate_after_write:
                document_cookie_get_would_revalidate_after_write(&cache, &telemetry),
            first_cookie_request: telemetry.first_operation,
            telemetry,
        }
    }
}
