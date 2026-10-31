mod core_conversion;
mod network_context;
mod query_report;
mod set_report;
mod site_data;
mod stored_cookie;

pub use network_context::NetworkStorageAccessStatus;
pub use network_context::{
    BrowserCookieFacadeContext, BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides,
    BrowserCookieStorageAccessStatus, CookieSource, NetworkCookieRequestContext,
    NetworkCookieRequestType, NetworkSameSiteContext, NetworkSameSiteHttpMethod,
    NetworkSameSiteRedirectType, NetworkSiteContextMetadata, NetworkSiteContextTrackMetadata,
    advance_cookie_request_context, core_browser_site_context_from_facade,
    core_same_site_context_metadata_from_stored, redirect_types_for_request,
    site_context_downgrade_type,
};
pub use query_report::{
    StoredCookieAccess, StoredCookieAccessSemantics, StoredCookieBrowserContextValueSource,
    StoredCookieEffectiveSameSite, StoredCookieExclusionReason, StoredCookieFacadeStatus,
    StoredCookieQueryReport, StoredCookieRequestSameSiteContext,
    StoredCookieSameSiteContextDowngradeType, StoredCookieSameSiteHttpMethod,
    StoredCookieSameSiteRedirectType, StoredCookieScopeSemantics, StoredCookieSiteContextBasis,
    StoredCookieStorageAccessStatus, StoredCookieWarningReason,
};
pub use set_report::{
    StoredCookieSetRejectionReason, StoredCookieSetReport, StoredCookieSetStatus,
    StoredCookieSetWarningReason,
};
pub use site_data::{
    CookieSiteDataChange, CookieSiteDataClearReport, CookieSiteDataClearScope, CookieSiteDataScope,
    CookieSiteDataSummary, CookieStorageClearReport, CookieStorageClearTarget,
    CookieStorageStateDiff, CookieStorageStateSnapshot,
};
#[cfg(any(test, feature = "test-support"))]
pub use site_data::{
    CookieSiteDataClearPreviewReport, CookieSiteDataOperation,
    CookieSiteDataOperationPreviewReport, CookieSiteDataOperationReport,
    CookieStorageClearPreviewReport,
};
pub use stored_cookie::{
    StoredCookie, StoredCookieDomainMatch, StoredCookiePartitionFilter, StoredCookiePartitionKey,
    StoredCookieSameSite, StoredCookieSourceScheme,
};

#[cfg(any(test, feature = "test-support"))]
pub use network_context::{NetworkSameSiteContextDowngradeType, NetworkSiteContext};

pub(crate) use core_conversion::{
    core_partition_key_from_stored, stored_cookie_from_core, stored_partition_key_from_core,
    stored_query_report_from_core, stored_set_report_from_core,
};
pub(crate) use network_context::core_cookie_partition_key_for_url;
pub(super) use stored_cookie::{core_source_scheme_from_stored, has_invalid_cookie_octets};
