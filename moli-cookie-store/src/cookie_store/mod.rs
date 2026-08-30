mod context;
mod policy;
mod query;
mod query_policy;
mod results;
mod storage;
mod store;
mod write;
mod write_policy;

pub use self::context::*;
pub use self::results::*;
pub use self::store::*;
pub(crate) use self::store::{
    CookieKey, DomainMap, Map, MAX_COOKIE_ATTRIBUTE_VALUE_BYTES, MAX_COOKIE_NAME_VALUE_BYTES,
};

#[derive(Debug, Default, Clone)]
/// An implementation for storing and retrieving [`Cookie`]s per the path and domain matching
/// rules specified in [RFC6265](https://datatracker.ietf.org/doc/html/rfc6265).
pub struct CookieStore {
    /// Cookies stored by domain, path, then name
    cookies: DomainMap,
    next_creation_index: u64,
    next_access_index: u64,
    limits: CookieStoreLimits,
    #[cfg(feature = "public_suffix")]
    /// If set, enables [public suffix](https://datatracker.ietf.org/doc/html/rfc6265#section-5.3) rejection based on the provided rule lookup.
    public_suffix_list: Option<std::sync::Arc<dyn crate::CookiePublicSuffixList>>,
}

#[cfg(test)]
mod tests;
