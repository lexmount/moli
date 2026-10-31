use super::*;
use crate::{StoredCookiePartitionKey, model::stored_partition_key_from_core};
use cookie_store::{CookieStore, CookieStoreKey, CookieStoreMutation, CookieStoreObserver};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoredCookieIdentity {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub partition_key: Option<StoredCookiePartitionKey>,
}
impl From<&StoredCookie> for StoredCookieIdentity {
    fn from(cookie: &StoredCookie) -> Self {
        Self {
            domain: cookie.domain.clone(),
            path: cookie.path.clone(),
            name: cookie.name.clone(),
            partition_key: cookie.partition_key.clone(),
        }
    }
}
impl From<CookieStoreKey> for StoredCookieIdentity {
    fn from(key: CookieStoreKey) -> Self {
        Self {
            domain: key.domain,
            path: key.path,
            name: key.name,
            partition_key: key
                .partition_key
                .as_ref()
                .map(stored_partition_key_from_core),
        }
    }
}
#[derive(Debug, Clone)]
pub enum StoredCookieMutation {
    Upsert { cookie: StoredCookie },
    Delete(StoredCookieIdentity),
    Clear,
}
pub trait BrowserCookieObserver: std::fmt::Debug + Send + Sync {
    fn changed(&self, change: StoredCookieMutation);
}
#[derive(Debug)]
struct Observer(Arc<dyn BrowserCookieObserver>);
impl CookieStoreObserver for Observer {
    fn changed(&self, change: CookieStoreMutation) {
        self.0.changed(match change {
            CookieStoreMutation::Upsert { cookie } => StoredCookieMutation::Upsert {
                cookie: crate::model::stored_cookie_from_core(&cookie),
            },
            CookieStoreMutation::Delete(key) => StoredCookieMutation::Delete(key.into()),
            CookieStoreMutation::Clear => StoredCookieMutation::Clear,
        });
    }
}
impl BrowserCookieStore {
    pub fn set_persistence_observer(&mut self, observer: Arc<dyn BrowserCookieObserver>) {
        self.full_core
            .set_observer(Some(Arc::new(Observer(observer))));
    }
    /// Restore a validated SQLite snapshot without treating reads as new
    /// writes or allocating fresh creation/access indices. JSON imports use
    /// the ordinary canonical insertion path before producing this snapshot.
    pub fn from_persisted(cookies: impl IntoIterator<Item = StoredCookie>) -> Result<Self, String> {
        let mut validation = Self::default();
        let mut restored = Vec::new();
        for cookie in cookies {
            if cookie.is_expired() {
                continue;
            }
            let (input, url) = canonical_cookie_input_from_stored_cookie(&cookie, None)
                .ok_or_else(|| "invalid persisted cookie identity".to_owned())?;
            if !validation
                .upsert_with_request_url_report(cookie.clone(), Some(&url), CookieSource::Cdp)
                .is_accepted()
            {
                return Err("persisted cookie rejected by canonical policy".into());
            }
            let native = cookie_store::Cookie::try_from_canonical_input(input, &url)
                .map_err(|error| error.to_string())?
                .with_persistent_ordering(cookie.creation_index, cookie.last_access_index);
            restored.push(native);
        }
        if validation.full_core.iter_unexpired().count() != restored.len() {
            return Err("persisted cookie identities or quotas are inconsistent".into());
        }
        let limits =
            CookieStoreLimits::new(DEFAULT_PER_DOMAIN_MAX_COOKIES, DEFAULT_GLOBAL_MAX_COOKIES);
        let full_core =
            CookieStore::from_cookies(restored.into_iter().map(Ok::<_, String>), false)?
                .with_shared_suffix_list(public_suffix_list())
                .with_limits(limits);
        Ok(Self {
            full_core,
            document_cookie_generation: 0,
        })
    }
}
