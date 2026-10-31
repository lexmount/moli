//! Response-side and structured-write cookie set diagnostics.

use cookie_store::StoreAction;

use super::query_report::StoredCookieEffectiveSameSite;

/// Non-fatal diagnostics emitted while processing a Set-Cookie write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSetWarningReason {
    DomainAttributeIgnored,
    PathAttributeIgnored,
    SecureAccessGrantedNonCryptographic,
}

/// Reasons a Set-Cookie or structured upsert was rejected by browser policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSetRejectionReason {
    InvalidOctets,
    InvalidUrl,
    MissingCookieUrl,
    EmptyNameAndValue,
    EmptyNameValueContainsEquals,
    NameContainsEquals,
    PathMustStartWithSlash,
    InvalidPartitionKey,
    CookiesDisabled,
    StorageAccessBlocked,
    StoreUnavailable,
    NonHttpScheme,
    SecureOnly,
    NonRelativeScheme,
    DomainMismatch,
    SameSiteNoneRequiresSecure,
    PrefixViolation,
    SecureOverlay,
    NameValueTooLarge,
    PartitionedMissingPartitionKey,
    PartitionedRequiresSecure,
    UnsupportedPartitioned,
    StorageFull,
    Expired,
    Parse,

    PublicSuffix,
    UnspecifiedDomain,
}

/// Final write outcome after canonical parsing, validation, and quota handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSetStatus {
    Accepted(StoreAction),
    Rejected(StoredCookieSetRejectionReason),
}

/// Response-side Set-Cookie processing report.
///
/// Unlike `StoredCookieQueryReport`, this describes a write attempt: whether it
/// was accepted, which rejection/warning reasons applied, and which SameSite
/// value the canonical cookie engine used after defaulting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCookieSetReport {
    /// Accepted store action or primary rejection reason.
    pub status: StoredCookieSetStatus,
    /// Complete list of rejection reasons reported by the canonical engine.
    pub rejection_reasons: Vec<StoredCookieSetRejectionReason>,
    /// Non-fatal write diagnostics, such as ignored attributes.
    pub warning_reasons: Vec<StoredCookieSetWarningReason>,
    /// Effective SameSite value when parsing reached SameSite evaluation.
    pub effective_same_site: Option<StoredCookieEffectiveSameSite>,
}

impl StoredCookieSetReport {
    /// A failure before parsing or policy evaluation reached canonical storage.
    pub fn store_unavailable() -> Self {
        Self {
            status: StoredCookieSetStatus::Rejected(
                StoredCookieSetRejectionReason::StoreUnavailable,
            ),
            rejection_reasons: vec![StoredCookieSetRejectionReason::StoreUnavailable],
            warning_reasons: Vec::new(),
            effective_same_site: None,
        }
    }

    /// Returns true when the write changed or confirmed canonical store state.
    pub fn is_accepted(&self) -> bool {
        matches!(self.status, StoredCookieSetStatus::Accepted(_))
    }
}
