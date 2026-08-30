#![cfg_attr(docsrs, feature(doc_cfg))]
//! # cookie_store
//! Provides an implementation for storing and retrieving [`Cookie`]s per the path and domain matching
//! rules specified in [RFC6265](https://datatracker.ietf.org/doc/html/rfc6265).
//!
//! ## Example
//! Please refer to the [reqwest_cookie_store](https://crates.io/crates/reqwest_cookie_store) for
//! an example of using this library along with [reqwest](https://crates.io/crates/reqwest).
//!
//! ## Feature flags
#![doc = document_features::document_features!()]

pub use ::cookie::{Cookie as RawCookie, ParseError as RawCookieParseError, SameSite};

mod cookie;
pub use crate::cookie::Error as CookieError;
pub use crate::cookie::{
    CanonicalCookieInput, Cookie, CookiePartitionKey, CookiePriority, CookieResult,
    CookieSourceScheme,
};
mod cookie_domain;
pub use crate::cookie_domain::CookieDomain;
mod cookie_expiration;
pub use crate::cookie_expiration::CookieExpiration;
mod cookie_path;
pub use crate::cookie_path::CookiePath;
mod cookie_store;
pub use crate::cookie_store::{
    BrowserSiteContext, CookieAccessQueryResult, CookieAccessResult, CookieAccessSemantics,
    CookieAccessSource, CookieDeleteFilter, CookieEffectiveSameSite, CookieExclusionReason,
    CookieInclusionStatus, CookieQueryResult, CookieScopeSemantics, CookieSetAccessResult,
    CookieSetRejectionReason, CookieSetResult, CookieSetWarningReason, CookieStore,
    CookieStoreLimits, CookieWarningReason, CookieWithAccessResult, ExcludedCookie,
    HttpRequestType, InsertContext, QueryContext, SameSiteContext, SameSiteContextDowngradeType,
    SameSiteContextHttpMethod, SameSiteContextMetadata, SameSiteContextRedirectType,
    SameSiteContextTrackMetadata, SameSiteRequestContext, StorageAccessStatus, StoreAction,
};
#[cfg(feature = "public_suffix")]
mod public_suffix;
#[cfg(feature = "public_suffix")]
pub use crate::public_suffix::CookiePublicSuffixList;
mod utils;

#[derive(Debug)]
pub struct IdnaErrors(idna::Errors);

impl std::fmt::Display for IdnaErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "IDNA errors: {:#?}", self.0)
    }
}

impl std::error::Error for IdnaErrors {}

impl From<idna::Errors> for IdnaErrors {
    fn from(e: idna::Errors) -> Self {
        IdnaErrors(e)
    }
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
