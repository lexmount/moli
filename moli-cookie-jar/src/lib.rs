//! Browser-facing cookie storage and policy types for Moli.
//!
//! This crate wraps the lower-level cookie store with the request, response,
//! storage, and site-context types used by `network`, `document.cookie`, and
//! CDP-facing code.

mod jar;
mod model;

pub use cookie_store::CookiePriority;
pub use cookie_store::cookie_creation_time;
pub use jar::{
    BrowserCookieObserver, BrowserCookieStore, BrowserCookieStoreHandle, CookieStoreError,
    SharedBrowserCookieStore, StoredCookieIdentity, StoredCookieMutation,
    new_shared_browser_cookie_store,
};
pub use model::{
    BrowserCookieFacadeContext, BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides,
    BrowserCookieStorageAccessStatus, CookieSiteDataClearReport, CookieSiteDataClearScope,
    CookieSiteDataScope, CookieSiteDataSummary, CookieSource, CookieStorageClearReport,
    CookieStorageClearTarget, CookieStorageStateDiff, CookieStorageStateSnapshot,
    NetworkCookieRequestContext, NetworkStorageAccessStatus, StoredCookie, StoredCookieDomainMatch,
    StoredCookieEffectiveSameSite, StoredCookiePartitionFilter, StoredCookiePartitionKey,
    StoredCookieSameSite, StoredCookieSetRejectionReason, StoredCookieSetReport,
    StoredCookieSetStatus, StoredCookieSetWarningReason, StoredCookieSourceScheme,
    advance_cookie_request_context, redirect_types_for_request, site_context_downgrade_type,
};
#[cfg(any(test, feature = "test-support"))]
pub use model::{
    CookieSiteDataClearPreviewReport, CookieSiteDataOperation,
    CookieSiteDataOperationPreviewReport, CookieSiteDataOperationReport,
    CookieStorageClearPreviewReport,
};
pub use model::{NetworkSiteContextMetadata, NetworkSiteContextTrackMetadata};
pub use model::{
    StoredCookieAccess, StoredCookieAccessSemantics, StoredCookieBrowserContextValueSource,
    StoredCookieExclusionReason, StoredCookieFacadeStatus, StoredCookieQueryReport,
    StoredCookieRequestSameSiteContext, StoredCookieSameSiteContextDowngradeType,
    StoredCookieSameSiteHttpMethod, StoredCookieSameSiteRedirectType, StoredCookieScopeSemantics,
    StoredCookieSiteContextBasis, StoredCookieStorageAccessStatus, StoredCookieWarningReason,
};
pub use moli_site::{host_is_public_suffix, same_site_urls, site_key_for_host};

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub use super::jar::BrowserCookieStore;
    pub use super::model::{
        CookieSiteDataChange, NetworkSameSiteContext, NetworkSameSiteContextDowngradeType,
        NetworkSameSiteRedirectType, NetworkSiteContext,
    };
}

#[cfg(test)]
mod tests;
