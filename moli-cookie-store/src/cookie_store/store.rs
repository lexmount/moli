use crate::CookieError;

#[cfg(feature = "preserve_order")]
use indexmap::IndexMap;
#[cfg(not(feature = "preserve_order"))]
use std::collections::HashMap;

#[cfg(feature = "preserve_order")]
pub(crate) type Map<K, V> = IndexMap<K, V>;
#[cfg(not(feature = "preserve_order"))]
pub(crate) type Map<K, V> = HashMap<K, V>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CookieKey {
    pub(crate) name: String,
    pub(crate) partition_key: Option<crate::CookiePartitionKey>,
}

impl CookieKey {
    pub(crate) fn new(
        name: impl Into<String>,
        partition_key: Option<crate::CookiePartitionKey>,
    ) -> Self {
        Self {
            name: name.into(),
            partition_key,
        }
    }

    pub(crate) fn for_cookie(cookie: &crate::cookie::Cookie<'_>) -> Self {
        Self::new(cookie.name(), cookie.partition_key().cloned())
    }
}

pub(crate) type NameMap = Map<CookieKey, crate::cookie::Cookie<'static>>;
pub(crate) type PathMap = Map<String, NameMap>;
pub(crate) type DomainMap = Map<String, PathMap>;
pub(crate) const MAX_COOKIE_NAME_VALUE_BYTES: usize = 4096;
pub(crate) const MAX_COOKIE_ATTRIBUTE_VALUE_BYTES: usize = 1024;

#[derive(PartialEq, Clone, Copy, Debug, Eq)]
pub enum StoreAction {
    /// The `Cookie` was successfully added to the store
    Inserted,
    /// The `Cookie` successfully expired a `Cookie` already in the store
    ExpiredExisting,
    /// The `Cookie` was added to the store, replacing an existing entry
    UpdatedExisting,
}

pub type StoreResult<T> = Result<T, crate::Error>;
pub type InsertResult = Result<StoreAction, CookieError>;

/// Partition scope for cookie deletion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CookiePartitionMatch<'a> {
    /// Match cookies in every partition, including unpartitioned cookies.
    #[default]
    Any,
    /// Match only cookies without a partition key.
    Unpartitioned,
    /// Match cookies in one exact partition, including opaque partitions.
    Exact(&'a crate::CookiePartitionKey),
    /// Match only serializable partitions, comparing each supplied attribute.
    Serialized {
        top_level_site: Option<&'a str>,
        has_cross_site_ancestor: Option<bool>,
    },
}

/// Filter used for deleting matching cookies from the store.
///
/// This is intentionally close to the browser-facing filters used by CDP and
/// Chromium manager APIs: exact name/domain/path constraints plus an optional
/// request host used to distinguish host-only cookies from domain cookies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CookieDeleteFilter<'a> {
    /// Match only cookies whose name equals this value.
    pub name: Option<&'a str>,
    /// Match only cookies whose canonical domain equals this value.
    pub domain: Option<&'a str>,
    /// Match the domain serialization verbatim, including the leading dot of
    /// domain cookies. This constraint is independent of canonical matching.
    pub serialized_domain: Option<&'a str>,
    /// Match only cookies whose canonical path equals this value.
    pub path: Option<&'a str>,
    /// Further constrain matches using browser host-only semantics for this
    /// request host.
    pub url_host: Option<&'a str>,
    /// Match all partitions by default, or explicitly constrain their scope.
    pub partition: CookiePartitionMatch<'a>,
}

/// Browser-oriented cookie store limits.
///
/// The default is effectively unlimited so existing generic `cookie_store`
/// callers preserve historical behavior unless they opt into browser-style
/// quota enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookieStoreLimits {
    /// Maximum number of unexpired cookies allowed for one canonical cookie
    /// domain bucket, independently for unpartitioned cookies and each
    /// partition key.
    pub per_domain_cookies: usize,
    /// Maximum number of unexpired unpartitioned cookies across the store.
    /// Partitioned cookies are limited per domain and partition instead.
    pub total_cookies: usize,
}

impl CookieStoreLimits {
    /// Construct a new limit configuration.
    pub const fn new(per_domain_cookies: usize, total_cookies: usize) -> Self {
        Self {
            per_domain_cookies,
            total_cookies,
        }
    }

    pub(super) const fn unlimited() -> Self {
        Self {
            per_domain_cookies: usize::MAX,
            total_cookies: usize::MAX,
        }
    }
}

impl Default for CookieStoreLimits {
    fn default() -> Self {
        Self::unlimited()
    }
}
