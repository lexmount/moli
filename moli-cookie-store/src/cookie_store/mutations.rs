use super::*;
use crate::Cookie;
use std::sync::Arc;

/// Canonical identity, shared by all write, expiry, eviction and access paths.
/// Source scheme, source port and host-only are metadata, not key components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CookieStoreKey {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub partition_key: Option<crate::CookiePartitionKey>,
}
impl CookieStoreKey {
    pub fn for_cookie(cookie: &Cookie<'_>) -> Self {
        Self {
            domain: String::from(&cookie.domain),
            path: String::from(&cookie.path),
            name: cookie.name().to_owned(),
            partition_key: cookie.partition_key().cloned(),
        }
    }
}
#[derive(Debug, Clone)]
pub enum CookieStoreMutation {
    Upsert { cookie: Cookie<'static> },
    Delete(CookieStoreKey),
    Clear,
}
/// Native nonblocking sink. It must not enter JS, run SQL, or call back into
/// the canonical store while its mutation is being accepted.
pub trait CookieStoreObserver: std::fmt::Debug + Send + Sync {
    fn changed(&self, mutation: CookieStoreMutation);
}
impl CookieStore {
    pub fn set_observer(&mut self, observer: Option<Arc<dyn CookieStoreObserver>>) {
        self.observer = observer;
    }
    pub(super) fn notify(&self, mutation: CookieStoreMutation) {
        if let Some(observer) = &self.observer {
            observer.changed(mutation);
        }
    }
}
impl Clone for CookieStore {
    fn clone(&self) -> Self {
        Self {
            cookies: self.cookies.clone(),
            next_creation_index: self.next_creation_index,
            next_access_index: self.next_access_index,
            limits: self.limits,
            #[cfg(feature = "public_suffix")]
            public_suffix_list: self.public_suffix_list.clone(),
            // A diagnostic/site-data snapshot must never become a second
            // active persistence writer for the same logical partition.
            observer: None,
        }
    }
}
