use moli_cookie_jar::{
    BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides,
    BrowserCookieStorageAccessStatus, StoredCookie, StoredCookieBrowserContextValueSource,
    StoredCookieEffectiveSameSite, StoredCookieExclusionReason, StoredCookieSameSite,
    StoredCookieSetRejectionReason, StoredCookieSetReport, StoredCookieSetStatus, same_site_urls,
    site_key_for_host,
};
use url::Url;

#[cfg(test)]
use moli_core::page::Page;
use moli_core::page::{
    DocumentCookieBrowserContextSnapshot, DocumentCookieCacheLookupResult,
    DocumentCookieFacadeTelemetrySnapshot, DocumentCookieFirstOperation,
    DocumentCookieOwnerSnapshot,
};

use super::cookie_policy_surface::BrowserContextDocumentCookiePolicySurface;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieManagerPolicySnapshot {
    pub(crate) overrides: BrowserCookieFacadeOverrides,
    pub(crate) cookies_enabled_override: Option<bool>,
    pub(crate) browser_context_overrides: BrowserCookieFacadeContextOverrides,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BrowserContextCookieManagerCapabilitySnapshot {
    pub(crate) cookie_access_verdict: BrowserContextCookieManagerAccessVerdict,
    pub(crate) cookies_enabled_preference: Option<bool>,
    pub(crate) cookie_access_enabled: Option<bool>,
    pub(crate) store_available: Option<bool>,
    pub(crate) cookie_access_primary_block_reason: Option<StoredCookieExclusionReason>,
    pub(crate) cookie_access_blocked_reasons: Vec<StoredCookieExclusionReason>,
    pub(crate) cookie_write_verdict: BrowserContextCookieManagerWriteVerdict,
    pub(crate) cookie_write_enabled: Option<bool>,
    pub(crate) cookie_write_primary_rejection_reason: Option<StoredCookieSetRejectionReason>,
    pub(crate) cookie_write_blocked_reasons: Vec<StoredCookieSetRejectionReason>,
    pub(crate) view_generation: Option<u64>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextDocumentCookieCapabilitySnapshot {
    pub(crate) cookies_enabled_preference: bool,
    pub(crate) cookie_access_enabled: bool,
    pub(crate) store_available: bool,
    pub(crate) primary_block_reason: Option<StoredCookieExclusionReason>,
    pub(crate) blocked_reasons: Vec<StoredCookieExclusionReason>,
    pub(crate) view_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BrowserContextCookieManagerAccessVerdict {
    #[default]
    NoLivePage,
    Allowed,
    Blocked(StoredCookieExclusionReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BrowserContextCookieManagerWriteVerdict {
    #[default]
    NoLivePage,
    Allowed,
    Blocked(StoredCookieSetRejectionReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieManagerSiteRelationship {
    SameSite,
    CrossSite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieManagerDocumentFrameRelationship {
    TopLevelDocument,
    SameSiteSubframe,
    CrossSiteSubframe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieManagerNavigationTransitionKind {
    DirectNavigation,
    RedirectedNavigation,
    SameDocumentUrlUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieManagerEffectiveNavigationRelationshipSource {
    Initiator,
    RequestedDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieManagerNavigationContextSnapshot {
    // Manager-owned committed navigation summary. This keeps the initiator /
    // requested / current-document relationship contract together instead of
    // flattening more pairwise navigation fields into the broader browser
    // context snapshot.
    pub(crate) current_document_url: Url,
    pub(crate) current_document_site: Option<String>,
    pub(crate) navigation_initiator_url: Option<Url>,
    pub(crate) navigation_initiator_site: Option<String>,
    pub(crate) navigation_initiator_requested_relationship:
        Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) schemeful_navigation_initiator_requested_relationship:
        Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) navigation_initiator_relationship:
        Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) schemeful_navigation_initiator_relationship:
        Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) requested_document_url: Url,
    pub(crate) requested_document_site: Option<String>,
    // This only captures whether the live document URL diverged from the
    // original requested document URL. Same-document URL updates can make
    // these values differ even when the original navigation never redirected.
    pub(crate) requested_document_differs_from_current: bool,
    pub(crate) navigation_was_redirected: bool,
    pub(crate) navigation_redirect_count: usize,
    pub(crate) navigation_transition_kind: BrowserContextCookieManagerNavigationTransitionKind,
    pub(crate) effective_navigation_relationship_source:
        BrowserContextCookieManagerEffectiveNavigationRelationshipSource,
    pub(crate) effective_navigation_relationship: BrowserContextCookieManagerSiteRelationship,
    pub(crate) schemeful_effective_navigation_relationship:
        BrowserContextCookieManagerSiteRelationship,
    pub(crate) requested_document_relationship: BrowserContextCookieManagerSiteRelationship,
    pub(crate) schemeful_requested_document_relationship:
        BrowserContextCookieManagerSiteRelationship,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieManagerContextSnapshot {
    pub(crate) navigation: BrowserContextCookieManagerNavigationContextSnapshot,
    pub(crate) site_for_cookies_url: Option<Url>,
    pub(crate) site_for_cookies_site: Option<String>,
    pub(crate) site_for_cookies_relationship: Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) schemeful_site_for_cookies_relationship:
        Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) site_for_cookies_source: StoredCookieBrowserContextValueSource,
    pub(crate) top_frame_origin_url: Option<Url>,
    pub(crate) top_frame_origin_site: Option<String>,
    pub(crate) top_frame_origin_relationship: Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) schemeful_top_frame_origin_relationship:
        Option<BrowserContextCookieManagerSiteRelationship>,
    pub(crate) document_frame_relationship:
        Option<BrowserContextCookieManagerDocumentFrameRelationship>,
    pub(crate) schemeful_document_frame_relationship:
        Option<BrowserContextCookieManagerDocumentFrameRelationship>,
    pub(crate) top_frame_origin_source: StoredCookieBrowserContextValueSource,
    pub(crate) storage_access_status: BrowserCookieStorageAccessStatus,
    pub(crate) storage_access_source: StoredCookieBrowserContextValueSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BrowserContextCookieManagerGatingSnapshot {
    pub(crate) cookie_access_policy_verdict: BrowserContextCookieManagerAccessVerdict,
    pub(crate) cookie_access_primary_block_reason: Option<StoredCookieExclusionReason>,
    pub(crate) cookie_access_blocked_reasons: Vec<StoredCookieExclusionReason>,
    pub(crate) cookie_write_policy_verdict: BrowserContextCookieManagerWriteVerdict,
    pub(crate) cookie_write_primary_rejection_reason: Option<StoredCookieSetRejectionReason>,
    pub(crate) cookie_write_blocked_reasons: Vec<StoredCookieSetRejectionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserContextDefaultCookieWriteUrlSource {
    LoadedPage,
    BrowserContextUrl,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextCookieBackendConnectionState {
    NoLivePage,
    Attached,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextStructuredCookieWriteReadinessStatus {
    ReadyUsingLoadedPageUrl,
    ReadyUsingBrowserContextUrl,
    MissingScopedUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextStructuredCookieWriteBackendStatus {
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextStructuredCookieCommandVerdict {
    Ready,
    MissingScopedUrl,
    Blocked(StoredCookieSetRejectionReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieWriteCapabilitySnapshot {
    pub(crate) write_enabled: bool,
    pub(crate) primary_rejection_reason: Option<StoredCookieSetRejectionReason>,
    pub(crate) blocked_reasons: Vec<StoredCookieSetRejectionReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextStructuredCookieWriteSnapshot {
    pub(crate) default_cookie_write_url: Option<Url>,
    pub(crate) default_cookie_write_url_source: BrowserContextDefaultCookieWriteUrlSource,
    pub(crate) readiness_status: BrowserContextStructuredCookieWriteReadinessStatus,
    pub(crate) backend_status: BrowserContextStructuredCookieWriteBackendStatus,
    // Manager-owned command verdict for callers that need a single answer for
    // the default-scoped structured write path. This intentionally reflects
    // only the command-level contract that exists before per-cookie
    // normalization of explicit `url` / `domain` inputs.
    pub(crate) default_command_verdict: BrowserContextStructuredCookieCommandVerdict,
    pub(crate) normalized_write_capability: BrowserContextCookieWriteCapabilitySnapshot,
}

fn effective_same_site_from_stored_same_site(
    same_site: StoredCookieSameSite,
) -> StoredCookieEffectiveSameSite {
    match same_site {
        StoredCookieSameSite::Strict => StoredCookieEffectiveSameSite::Strict,
        StoredCookieSameSite::Lax => StoredCookieEffectiveSameSite::Lax,
        StoredCookieSameSite::None | StoredCookieSameSite::Unspecified => {
            StoredCookieEffectiveSameSite::NoRestriction
        }
    }
}

impl BrowserContextStructuredCookieWriteSnapshot {
    fn cookie_facade_rejection_report(
        &self,
        cookie: &StoredCookie,
        primary_rejection_reason: StoredCookieSetRejectionReason,
        rejection_reasons: Vec<StoredCookieSetRejectionReason>,
    ) -> StoredCookieSetReport {
        StoredCookieSetReport {
            status: StoredCookieSetStatus::Rejected(primary_rejection_reason),
            rejection_reasons,
            warning_reasons: Vec::new(),
            effective_same_site: Some(effective_same_site_from_stored_same_site(cookie.same_site)),
        }
    }

    pub(crate) fn normalized_cookie_facade_rejection(
        &self,
        cookie: &StoredCookie,
    ) -> Option<StoredCookieSetReport> {
        let primary_rejection_reason = self.normalized_write_capability.primary_rejection_reason?;
        let rejection_reasons = if self.normalized_write_capability.blocked_reasons.is_empty() {
            vec![primary_rejection_reason]
        } else {
            self.normalized_write_capability.blocked_reasons.clone()
        };
        Some(self.cookie_facade_rejection_report(
            cookie,
            primary_rejection_reason,
            rejection_reasons,
        ))
    }
}

fn browser_context_first_cookie_request(
    first_operation: DocumentCookieFirstOperation,
) -> Option<BrowserContextFirstCookieRequest> {
    Some(match first_operation {
        DocumentCookieFirstOperation::Set => BrowserContextFirstCookieRequest::Set,
        DocumentCookieFirstOperation::Get => BrowserContextFirstCookieRequest::Get,
        DocumentCookieFirstOperation::CookiesEnabled => {
            BrowserContextFirstCookieRequest::CookiesEnabled
        }
    })
}

fn browser_context_document_cookie_cache_lookup_result(
    value: DocumentCookieCacheLookupResult,
) -> BrowserContextDocumentCookieCacheLookupResult {
    match value {
        DocumentCookieCacheLookupResult::CacheMissFirstAccess => {
            BrowserContextDocumentCookieCacheLookupResult::CacheMissFirstAccess
        }
        DocumentCookieCacheLookupResult::CacheHitAfterGet => {
            BrowserContextDocumentCookieCacheLookupResult::CacheHitAfterGet
        }
        DocumentCookieCacheLookupResult::CacheHitAfterSet => {
            BrowserContextDocumentCookieCacheLookupResult::CacheHitAfterSet
        }
        DocumentCookieCacheLookupResult::CacheMissAfterGet => {
            BrowserContextDocumentCookieCacheLookupResult::CacheMissAfterGet
        }
        DocumentCookieCacheLookupResult::CacheMissAfterSet => {
            BrowserContextDocumentCookieCacheLookupResult::CacheMissAfterSet
        }
    }
}

fn browser_context_document_cookie_telemetry_snapshot(
    telemetry: &DocumentCookieFacadeTelemetrySnapshot,
) -> BrowserContextDocumentCookieTelemetrySnapshot {
    BrowserContextDocumentCookieTelemetrySnapshot {
        last_cache_lookup_result: telemetry
            .last_cache_lookup_result
            .map(browser_context_document_cookie_cache_lookup_result),
        last_operation_was_set: telemetry.last_operation_was_set,
        cache_hits: telemetry.cache_hits,
        store_reads: telemetry.store_reads,
        blocked_reads: telemetry.blocked_reads,
        unavailable_reads: telemetry.unavailable_reads,
        applied_writes: telemetry.applied_writes,
        rejected_writes: telemetry.rejected_writes,
        facade_blocked_writes: telemetry.facade_blocked_writes,
    }
}

fn registrable_site_from_url(url: Option<&Url>) -> Option<String> {
    site_key_for_host(url?.host_str()?)
}

fn same_site_relationship(
    current_document_url: &Url,
    other_url: Option<&Url>,
    schemeful: bool,
) -> Option<BrowserContextCookieManagerSiteRelationship> {
    let other_url = other_url?;
    Some(
        if same_site_urls(current_document_url, other_url, schemeful) {
            BrowserContextCookieManagerSiteRelationship::SameSite
        } else {
            BrowserContextCookieManagerSiteRelationship::CrossSite
        },
    )
}

fn document_frame_relationship(
    current_document_url: &Url,
    top_frame_origin_url: Option<&Url>,
    schemeful: bool,
) -> Option<BrowserContextCookieManagerDocumentFrameRelationship> {
    let top_frame_origin_url = top_frame_origin_url?;
    if current_document_url == top_frame_origin_url {
        return Some(BrowserContextCookieManagerDocumentFrameRelationship::TopLevelDocument);
    }

    Some(
        if same_site_urls(current_document_url, top_frame_origin_url, schemeful) {
            BrowserContextCookieManagerDocumentFrameRelationship::SameSiteSubframe
        } else {
            BrowserContextCookieManagerDocumentFrameRelationship::CrossSiteSubframe
        },
    )
}

fn browser_context_cookie_write_capability_snapshot(
    primary_rejection_reason: Option<StoredCookieSetRejectionReason>,
    blocked_reasons: &[StoredCookieSetRejectionReason],
) -> BrowserContextCookieWriteCapabilitySnapshot {
    BrowserContextCookieWriteCapabilitySnapshot {
        write_enabled: blocked_reasons.is_empty(),
        primary_rejection_reason,
        blocked_reasons: blocked_reasons.to_vec(),
    }
}

fn browser_context_structured_cookie_write_capability_snapshot(
    backend_status: BrowserContextStructuredCookieWriteBackendStatus,
) -> BrowserContextCookieWriteCapabilitySnapshot {
    let primary_rejection_reason = match backend_status {
        BrowserContextStructuredCookieWriteBackendStatus::Available => None,
    };
    let blocked_reasons = match primary_rejection_reason {
        Some(reason) => vec![reason],
        None => Vec::new(),
    };
    browser_context_cookie_write_capability_snapshot(primary_rejection_reason, &blocked_reasons)
}

fn browser_context_structured_cookie_command_verdict(
    readiness_status: BrowserContextStructuredCookieWriteReadinessStatus,
    capability: &BrowserContextCookieWriteCapabilitySnapshot,
) -> BrowserContextStructuredCookieCommandVerdict {
    match readiness_status {
        BrowserContextStructuredCookieWriteReadinessStatus::MissingScopedUrl => {
            BrowserContextStructuredCookieCommandVerdict::MissingScopedUrl
        }
        BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingLoadedPageUrl
        | BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingBrowserContextUrl => {
            match capability.primary_rejection_reason {
                Some(reason) => BrowserContextStructuredCookieCommandVerdict::Blocked(reason),
                None => BrowserContextStructuredCookieCommandVerdict::Ready,
            }
        }
    }
}

fn browser_context_structured_cookie_write_readiness_status(
    source: BrowserContextDefaultCookieWriteUrlSource,
) -> BrowserContextStructuredCookieWriteReadinessStatus {
    match source {
        BrowserContextDefaultCookieWriteUrlSource::LoadedPage => {
            BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingLoadedPageUrl
        }
        BrowserContextDefaultCookieWriteUrlSource::BrowserContextUrl => {
            BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingBrowserContextUrl
        }
        BrowserContextDefaultCookieWriteUrlSource::Unavailable => {
            BrowserContextStructuredCookieWriteReadinessStatus::MissingScopedUrl
        }
    }
}

impl BrowserContextCookieManagerCapabilitySnapshot {
    #[cfg(test)]
    fn cookie_access_primary_block_reason_from_verdict(
        &self,
    ) -> Option<StoredCookieExclusionReason> {
        match self.cookie_access_verdict {
            BrowserContextCookieManagerAccessVerdict::Blocked(reason) => Some(reason),
            BrowserContextCookieManagerAccessVerdict::Allowed
            | BrowserContextCookieManagerAccessVerdict::NoLivePage => None,
        }
    }

    #[cfg(test)]
    fn cookie_write_primary_rejection_reason_from_verdict(
        &self,
    ) -> Option<StoredCookieSetRejectionReason> {
        match self.cookie_write_verdict {
            BrowserContextCookieManagerWriteVerdict::Blocked(reason) => Some(reason),
            BrowserContextCookieManagerWriteVerdict::Allowed
            | BrowserContextCookieManagerWriteVerdict::NoLivePage => None,
        }
    }

    #[cfg(test)]
    fn document_cookie_capability_snapshot_with_gating(
        &self,
        gating: &BrowserContextCookieManagerGatingSnapshot,
    ) -> Option<BrowserContextDocumentCookieCapabilitySnapshot> {
        Some(BrowserContextDocumentCookieCapabilitySnapshot {
            cookies_enabled_preference: self.cookies_enabled_preference?,
            cookie_access_enabled: self.cookie_access_enabled?,
            store_available: self.store_available?,
            primary_block_reason: gating
                .cookie_access_primary_block_reason
                .or_else(|| self.cookie_access_primary_block_reason_from_verdict()),
            blocked_reasons: if gating.cookie_access_blocked_reasons.is_empty() {
                self.cookie_access_blocked_reasons.clone()
            } else {
                gating.cookie_access_blocked_reasons.clone()
            },
            view_generation: self.view_generation?,
        })
    }

    #[cfg(test)]
    fn document_cookie_write_capability_snapshot_with_gating(
        &self,
        gating: &BrowserContextCookieManagerGatingSnapshot,
    ) -> Option<BrowserContextCookieWriteCapabilitySnapshot> {
        Some(BrowserContextCookieWriteCapabilitySnapshot {
            write_enabled: self.cookie_write_enabled?,
            primary_rejection_reason: gating
                .cookie_write_primary_rejection_reason
                .or_else(|| self.cookie_write_primary_rejection_reason_from_verdict()),
            blocked_reasons: if gating.cookie_write_blocked_reasons.is_empty() {
                self.cookie_write_blocked_reasons.clone()
            } else {
                gating.cookie_write_blocked_reasons.clone()
            },
        })
    }
}

impl BrowserContextCookieManagerSurfaceSnapshot {
    pub(crate) fn hydrated(
        mut self,
        owner: Option<&DocumentCookieOwnerSnapshot>,
        current_document_url: Option<Url>,
        navigation_initiator_url: Option<Url>,
        requested_document_url: Option<Url>,
        navigation_was_redirected: bool,
        navigation_redirect_count: usize,
        default_cookie_write_url: Option<Url>,
        default_cookie_write_url_source: BrowserContextDefaultCookieWriteUrlSource,
        backend_connection_state: BrowserContextCookieBackendConnectionState,
        structured_write_backend_status: BrowserContextStructuredCookieWriteBackendStatus,
    ) -> Self {
        let readiness_status = browser_context_structured_cookie_write_readiness_status(
            default_cookie_write_url_source.clone(),
        );
        let normalized_write_capability =
            browser_context_structured_cookie_write_capability_snapshot(
                structured_write_backend_status,
            );
        let default_command_verdict = browser_context_structured_cookie_command_verdict(
            readiness_status,
            &normalized_write_capability,
        );
        self.structured_write = BrowserContextStructuredCookieWriteSnapshot {
            default_cookie_write_url,
            default_cookie_write_url_source: default_cookie_write_url_source.clone(),
            readiness_status,
            backend_status: structured_write_backend_status,
            default_command_verdict,
            normalized_write_capability,
        };
        self.backend_connection_state = backend_connection_state;
        self.policy_gating = Self::policy_gating_snapshot(owner);
        self.effective_gating = Self::effective_gating_snapshot(owner);
        self.effective_context = Self::context_snapshot(
            owner,
            current_document_url,
            navigation_initiator_url,
            requested_document_url,
            navigation_was_redirected,
            navigation_redirect_count,
        );
        self.capability = Self::capability_snapshot(owner);
        self.activity = Self::activity_snapshot(owner);
        self
    }

    fn capability_snapshot(
        owner: Option<&DocumentCookieOwnerSnapshot>,
    ) -> BrowserContextCookieManagerCapabilitySnapshot {
        let Some(owner) = owner else {
            return BrowserContextCookieManagerCapabilitySnapshot::default();
        };
        let cookie_access_verdict = match owner
            .capability
            .facade_status
            .blocked_reasons
            .first()
            .copied()
        {
            Some(reason) => BrowserContextCookieManagerAccessVerdict::Blocked(reason),
            None => BrowserContextCookieManagerAccessVerdict::Allowed,
        };
        let cookie_write_verdict = match owner.write_capability.primary_rejection_reason {
            Some(reason) => BrowserContextCookieManagerWriteVerdict::Blocked(reason),
            None => BrowserContextCookieManagerWriteVerdict::Allowed,
        };
        BrowserContextCookieManagerCapabilitySnapshot {
            cookie_access_verdict,
            cookies_enabled_preference: Some(owner.capability.cookies_enabled_preference),
            cookie_access_enabled: Some(owner.capability.facade_status.cookie_access_enabled),
            store_available: Some(owner.capability.facade_status.store_available),
            cookie_access_primary_block_reason: owner
                .capability
                .facade_status
                .blocked_reasons
                .first()
                .copied(),
            cookie_access_blocked_reasons: owner.capability.facade_status.blocked_reasons.clone(),
            cookie_write_verdict,
            cookie_write_enabled: Some(owner.write_capability.write_enabled),
            cookie_write_primary_rejection_reason: owner.write_capability.primary_rejection_reason,
            cookie_write_blocked_reasons: owner.write_capability.blocked_reasons.clone(),
            view_generation: Some(owner.capability.view_generation),
        }
    }

    fn policy_gating_snapshot(
        owner: Option<&DocumentCookieOwnerSnapshot>,
    ) -> BrowserContextCookieManagerGatingSnapshot {
        let Some(owner) = owner else {
            return BrowserContextCookieManagerGatingSnapshot::default();
        };
        let cookie_access_primary_block_reason = if !owner.capability.cookies_enabled_preference {
            Some(StoredCookieExclusionReason::CookiesDisabled)
        } else if owner
            .capability
            .facade_status
            .blocked_reasons
            .contains(&StoredCookieExclusionReason::StorageAccessBlocked)
        {
            Some(StoredCookieExclusionReason::StorageAccessBlocked)
        } else {
            None
        };
        let cookie_access_policy_verdict = match cookie_access_primary_block_reason {
            Some(reason) => BrowserContextCookieManagerAccessVerdict::Blocked(reason),
            None => BrowserContextCookieManagerAccessVerdict::Allowed,
        };
        let cookie_access_blocked_reasons = match cookie_access_primary_block_reason {
            Some(reason) => vec![reason],
            None => Vec::new(),
        };
        let cookie_write_primary_rejection_reason = if !owner.capability.cookies_enabled_preference
        {
            Some(StoredCookieSetRejectionReason::CookiesDisabled)
        } else if owner
            .write_capability
            .blocked_reasons
            .contains(&StoredCookieSetRejectionReason::StorageAccessBlocked)
        {
            Some(StoredCookieSetRejectionReason::StorageAccessBlocked)
        } else {
            None
        };
        let cookie_write_policy_verdict = match cookie_write_primary_rejection_reason {
            Some(reason) => BrowserContextCookieManagerWriteVerdict::Blocked(reason),
            None => BrowserContextCookieManagerWriteVerdict::Allowed,
        };
        let cookie_write_blocked_reasons = match cookie_write_primary_rejection_reason {
            Some(reason) => vec![reason],
            None => Vec::new(),
        };
        BrowserContextCookieManagerGatingSnapshot {
            cookie_access_policy_verdict,
            cookie_access_primary_block_reason,
            cookie_access_blocked_reasons,
            cookie_write_policy_verdict,
            cookie_write_primary_rejection_reason,
            cookie_write_blocked_reasons,
        }
    }

    fn effective_gating_snapshot(
        owner: Option<&DocumentCookieOwnerSnapshot>,
    ) -> BrowserContextCookieManagerGatingSnapshot {
        let Some(owner) = owner else {
            return BrowserContextCookieManagerGatingSnapshot::default();
        };
        let cookie_access_primary_block_reason = owner
            .capability
            .facade_status
            .blocked_reasons
            .first()
            .copied();
        let cookie_access_policy_verdict = match cookie_access_primary_block_reason {
            Some(reason) => BrowserContextCookieManagerAccessVerdict::Blocked(reason),
            None => BrowserContextCookieManagerAccessVerdict::Allowed,
        };
        let cookie_access_blocked_reasons = owner.capability.facade_status.blocked_reasons.clone();
        let cookie_write_primary_rejection_reason = owner.write_capability.primary_rejection_reason;
        let cookie_write_policy_verdict = match cookie_write_primary_rejection_reason {
            Some(reason) => BrowserContextCookieManagerWriteVerdict::Blocked(reason),
            None => BrowserContextCookieManagerWriteVerdict::Allowed,
        };
        let cookie_write_blocked_reasons = owner.write_capability.blocked_reasons.clone();
        BrowserContextCookieManagerGatingSnapshot {
            cookie_access_policy_verdict,
            cookie_access_primary_block_reason,
            cookie_access_blocked_reasons,
            cookie_write_policy_verdict,
            cookie_write_primary_rejection_reason,
            cookie_write_blocked_reasons,
        }
    }

    fn context_snapshot(
        owner: Option<&DocumentCookieOwnerSnapshot>,
        current_document_url: Option<Url>,
        navigation_initiator_url: Option<Url>,
        requested_document_url: Option<Url>,
        navigation_was_redirected: bool,
        navigation_redirect_count: usize,
    ) -> Option<BrowserContextCookieManagerContextSnapshot> {
        let DocumentCookieBrowserContextSnapshot {
            site_for_cookies_url,
            site_for_cookies_source,
            top_frame_origin_url,
            top_frame_origin_source,
            storage_access_status,
            storage_access_source,
        } = owner?.browser_context.clone();
        let current_document_url = current_document_url?;
        let requested_document_url = requested_document_url?;
        let requested_document_differs_from_current =
            requested_document_url != current_document_url;
        let navigation_transition_kind = if navigation_was_redirected {
            BrowserContextCookieManagerNavigationTransitionKind::RedirectedNavigation
        } else if requested_document_differs_from_current {
            BrowserContextCookieManagerNavigationTransitionKind::SameDocumentUrlUpdate
        } else {
            BrowserContextCookieManagerNavigationTransitionKind::DirectNavigation
        };
        let navigation_initiator_relationship = same_site_relationship(
            &current_document_url,
            navigation_initiator_url.as_ref(),
            false,
        );
        let schemeful_navigation_initiator_relationship = same_site_relationship(
            &current_document_url,
            navigation_initiator_url.as_ref(),
            true,
        );
        let requested_document_relationship =
            same_site_relationship(&current_document_url, Some(&requested_document_url), false)
                .unwrap_or(BrowserContextCookieManagerSiteRelationship::CrossSite);
        let schemeful_requested_document_relationship =
            same_site_relationship(&current_document_url, Some(&requested_document_url), true)
                .unwrap_or(BrowserContextCookieManagerSiteRelationship::CrossSite);
        let (
            effective_navigation_relationship_source,
            effective_navigation_relationship,
            schemeful_effective_navigation_relationship,
        ) = match (
            navigation_initiator_relationship.clone(),
            schemeful_navigation_initiator_relationship.clone(),
        ) {
            (Some(relationship), Some(schemeful_relationship)) => (
                BrowserContextCookieManagerEffectiveNavigationRelationshipSource::Initiator,
                relationship,
                schemeful_relationship,
            ),
            _ => (
                BrowserContextCookieManagerEffectiveNavigationRelationshipSource::RequestedDocument,
                requested_document_relationship.clone(),
                schemeful_requested_document_relationship.clone(),
            ),
        };
        Some(BrowserContextCookieManagerContextSnapshot {
            navigation: BrowserContextCookieManagerNavigationContextSnapshot {
                current_document_site: registrable_site_from_url(Some(&current_document_url)),
                current_document_url: current_document_url.clone(),
                navigation_initiator_site: registrable_site_from_url(
                    navigation_initiator_url.as_ref(),
                ),
                navigation_initiator_requested_relationship: same_site_relationship(
                    &requested_document_url,
                    navigation_initiator_url.as_ref(),
                    false,
                ),
                schemeful_navigation_initiator_requested_relationship: same_site_relationship(
                    &requested_document_url,
                    navigation_initiator_url.as_ref(),
                    true,
                ),
                navigation_initiator_relationship,
                schemeful_navigation_initiator_relationship,
                navigation_initiator_url,
                requested_document_site: registrable_site_from_url(Some(&requested_document_url)),
                requested_document_differs_from_current,
                navigation_was_redirected,
                navigation_redirect_count,
                navigation_transition_kind,
                effective_navigation_relationship_source,
                effective_navigation_relationship,
                schemeful_effective_navigation_relationship,
                requested_document_relationship,
                schemeful_requested_document_relationship,
                requested_document_url,
            },
            site_for_cookies_site: registrable_site_from_url(site_for_cookies_url.as_ref()),
            site_for_cookies_relationship: same_site_relationship(
                &current_document_url,
                site_for_cookies_url.as_ref(),
                false,
            ),
            schemeful_site_for_cookies_relationship: same_site_relationship(
                &current_document_url,
                site_for_cookies_url.as_ref(),
                true,
            ),
            site_for_cookies_url,
            site_for_cookies_source,
            top_frame_origin_site: registrable_site_from_url(top_frame_origin_url.as_ref()),
            top_frame_origin_relationship: same_site_relationship(
                &current_document_url,
                top_frame_origin_url.as_ref(),
                false,
            ),
            schemeful_top_frame_origin_relationship: same_site_relationship(
                &current_document_url,
                top_frame_origin_url.as_ref(),
                true,
            ),
            document_frame_relationship: document_frame_relationship(
                &current_document_url,
                top_frame_origin_url.as_ref(),
                false,
            ),
            schemeful_document_frame_relationship: document_frame_relationship(
                &current_document_url,
                top_frame_origin_url.as_ref(),
                true,
            ),
            top_frame_origin_url,
            top_frame_origin_source,
            storage_access_status,
            storage_access_source,
        })
    }

    fn activity_snapshot(
        owner: Option<&DocumentCookieOwnerSnapshot>,
    ) -> BrowserContextCookieManagerActivitySnapshot {
        let Some(owner) = owner else {
            return BrowserContextCookieManagerActivitySnapshot::default();
        };
        BrowserContextCookieManagerActivitySnapshot {
            first_cookie_request: owner
                .first_cookie_request
                .and_then(browser_context_first_cookie_request),
            telemetry: Some(browser_context_document_cookie_telemetry_snapshot(
                &owner.telemetry,
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_capability_snapshot(
        &self,
    ) -> Option<BrowserContextDocumentCookieCapabilitySnapshot> {
        self.capability
            .document_cookie_capability_snapshot_with_gating(&self.effective_gating)
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_write_capability_snapshot(
        &self,
    ) -> Option<BrowserContextCookieWriteCapabilitySnapshot> {
        self.capability
            .document_cookie_write_capability_snapshot_with_gating(&self.effective_gating)
    }

    #[cfg(test)]
    pub(crate) fn document_cookie_telemetry_snapshot(
        &self,
    ) -> Option<BrowserContextDocumentCookieTelemetrySnapshot> {
        self.activity.telemetry.clone()
    }

    #[cfg(test)]
    pub(crate) fn first_cookie_request(&self) -> Option<BrowserContextFirstCookieRequest> {
        self.activity.first_cookie_request
    }

    pub(crate) fn normalized_cookie_facade_rejection(
        &self,
        cookie: &StoredCookie,
    ) -> Option<StoredCookieSetReport> {
        self.structured_write
            .normalized_cookie_facade_rejection(cookie)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextFirstCookieRequest {
    Set,
    Get,
    CookiesEnabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserContextDocumentCookieCacheLookupResult {
    CacheMissFirstAccess,
    CacheHitAfterGet,
    CacheHitAfterSet,
    CacheMissAfterGet,
    CacheMissAfterSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextDocumentCookieTelemetrySnapshot {
    pub(crate) last_cache_lookup_result: Option<BrowserContextDocumentCookieCacheLookupResult>,
    pub(crate) last_operation_was_set: Option<bool>,
    pub(crate) cache_hits: u64,
    pub(crate) store_reads: u64,
    pub(crate) blocked_reads: u64,
    pub(crate) unavailable_reads: u64,
    pub(crate) applied_writes: u64,
    pub(crate) rejected_writes: u64,
    pub(crate) facade_blocked_writes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BrowserContextCookieManagerActivitySnapshot {
    pub(crate) first_cookie_request: Option<BrowserContextFirstCookieRequest>,
    pub(crate) telemetry: Option<BrowserContextDocumentCookieTelemetrySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserContextCookieManagerSurfaceSnapshot {
    // Manager-owned browser policy state for document-facing cookie access.
    pub(crate) policy: BrowserContextCookieManagerPolicySnapshot,
    // Manager-owned browser policy verdicts, intentionally excluding backend
    // availability so DevTools/browser policy ownership can evolve separately
    // from live store connection state.
    pub(crate) policy_gating: BrowserContextCookieManagerGatingSnapshot,
    // Manager-owned effective browser-boundary verdicts after combining the
    // current policy surface with live backend/store availability.
    pub(crate) effective_gating: BrowserContextCookieManagerGatingSnapshot,
    // Manager-owned structured write owner contract for DevTools/CDP writes.
    pub(crate) structured_write: BrowserContextStructuredCookieWriteSnapshot,
    // Manager-owned lifecycle view of whether the live document-cookie backend
    // is attached, disconnected, or absent because no live page exists.
    pub(crate) backend_connection_state: BrowserContextCookieBackendConnectionState,
    // Manager-owned effective browser context for the live document. This is
    // `None`-shaped when no live document exists yet.
    pub(crate) effective_context: Option<BrowserContextCookieManagerContextSnapshot>,
    // Manager-owned view of the live document-cookie browser-boundary
    // capability after applying the current policy surface to the active page.
    // This is `None`-shaped when no live document exists yet.
    pub(crate) capability: BrowserContextCookieManagerCapabilitySnapshot,
    // Manager-owned view of live document-cookie activity/telemetry. This
    // keeps browser-facing request probes and telemetry shape attached to the
    // same manager contract as policy and capability.
    pub(crate) activity: BrowserContextCookieManagerActivitySnapshot,
}

impl Default for BrowserContextCookieManagerSurfaceSnapshot {
    fn default() -> Self {
        BrowserContextCookieManagerSurface::default().snapshot()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BrowserContextCookieManagerSurface {
    policy_surface: BrowserContextDocumentCookiePolicySurface,
}

impl BrowserContextCookieManagerSurface {
    pub(crate) fn from_snapshot(snapshot: BrowserContextCookieManagerSurfaceSnapshot) -> Self {
        Self {
            policy_surface: BrowserContextDocumentCookiePolicySurface::from_snapshot(
                super::cookie_policy_surface::BrowserContextDocumentCookiePolicySurfaceSnapshot {
                    overrides: snapshot.policy.overrides,
                    cookies_enabled_override: snapshot.policy.cookies_enabled_override,
                    browser_context_overrides: snapshot.policy.browser_context_overrides,
                    generation: snapshot.policy.generation,
                },
            ),
        }
    }

    pub(crate) fn snapshot(&self) -> BrowserContextCookieManagerSurfaceSnapshot {
        let policy_surface = self.policy_surface.snapshot();
        BrowserContextCookieManagerSurfaceSnapshot {
            policy: BrowserContextCookieManagerPolicySnapshot {
                overrides: policy_surface.overrides,
                cookies_enabled_override: policy_surface.cookies_enabled_override,
                browser_context_overrides: policy_surface.browser_context_overrides,
                generation: policy_surface.generation,
            },
            policy_gating: BrowserContextCookieManagerGatingSnapshot::default(),
            effective_gating: BrowserContextCookieManagerGatingSnapshot::default(),
            structured_write: BrowserContextStructuredCookieWriteSnapshot {
                default_cookie_write_url: None,
                default_cookie_write_url_source:
                    BrowserContextDefaultCookieWriteUrlSource::Unavailable,
                readiness_status:
                    BrowserContextStructuredCookieWriteReadinessStatus::MissingScopedUrl,
                backend_status: BrowserContextStructuredCookieWriteBackendStatus::Available,
                default_command_verdict:
                    BrowserContextStructuredCookieCommandVerdict::MissingScopedUrl,
                normalized_write_capability: BrowserContextCookieWriteCapabilitySnapshot {
                    write_enabled: true,
                    primary_rejection_reason: None,
                    blocked_reasons: Vec::new(),
                },
            },
            backend_connection_state: BrowserContextCookieBackendConnectionState::NoLivePage,
            effective_context: None,
            capability: BrowserContextCookieManagerCapabilitySnapshot::default(),
            activity: BrowserContextCookieManagerActivitySnapshot::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_policy_overrides(
        &mut self,
        overrides: &BrowserCookieFacadeOverrides,
    ) -> bool {
        self.policy_surface.set_overrides(overrides)
    }

    #[cfg(test)]
    pub(crate) fn clear_policy_overrides(&mut self) -> bool {
        self.policy_surface.clear_overrides()
    }

    #[cfg(test)]
    pub(crate) fn set_policy_cookies_enabled_override(&mut self, enabled: bool) -> bool {
        self.policy_surface.set_cookies_enabled_override(enabled)
    }

    #[cfg(test)]
    pub(crate) fn clear_policy_cookies_enabled_override(&mut self) -> bool {
        self.policy_surface.clear_cookies_enabled_override()
    }

    #[cfg(test)]
    pub(crate) fn set_policy_browser_context_overrides(
        &mut self,
        overrides: &BrowserCookieFacadeContextOverrides,
    ) -> bool {
        self.policy_surface.set_browser_context_overrides(overrides)
    }

    #[cfg(test)]
    pub(crate) fn clear_policy_browser_context_overrides(&mut self) -> bool {
        self.policy_surface.clear_browser_context_overrides()
    }

    #[cfg(test)]
    pub(crate) async fn apply_to_page_async(&self, page: &mut Page) {
        self.policy_surface.apply_to_page_async(page).await;
    }
}
