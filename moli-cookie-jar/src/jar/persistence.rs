use super::*;
use crate::{StoredCookiePartitionKey, model::stored_partition_key_from_core};
use cookie_store::{CookieStoreKey, CookieStoreMutation, CookieStoreObserver};
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
        let mut store = Self::default();
        let mut restored = 0;
        for cookie in cookies {
            if cookie.is_expired() {
                continue;
            }
            let (input, url) = canonical_cookie_input_from_stored_cookie(&cookie, None)
                .ok_or_else(|| "invalid persisted cookie identity".to_owned())?;
            let native = cookie_store::Cookie::try_from_canonical_input(input, &url)
                .map_err(|error| error.to_string())?
                .with_persistent_ordering(cookie.creation_index, cookie.last_access_index);
            if !store
                .full_core
                .restore_with_context(native, &insert_context(&url, CookieSource::Cdp))
                .is_accepted()
            {
                return Err("persisted cookie rejected by canonical policy".into());
            }
            restored += 1;
        }
        if store.full_core.iter_unexpired().count() != restored {
            return Err("persisted cookie identities or quotas are inconsistent".into());
        }
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str) -> StoredCookie {
        StoredCookie {
            name: name.into(),
            value: "value".into(),
            domain: "restore.test".into(),
            host_only: true,
            path: "/".into(),
            secure: true,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Unspecified,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Secure,
            source_port: 443,
            creation_index: 0,
            last_access_index: 0,
        }
    }

    #[test]
    fn restore_preserves_ordering_and_advances_subsequent_writes() {
        let mut first = cookie("first");
        first.creation_index = 17;
        first.last_access_index = 103;
        let mut second = cookie("second");
        second.creation_index = 3;
        second.last_access_index = 7;
        let mut store =
            BrowserCookieStore::from_persisted([first.clone(), second.clone()]).unwrap();
        assert_eq!(store.cookies(), vec![second, first]);
        assert!(
            store
                .upsert_with_request_url_report(cookie("new"), None, CookieSource::Cdp)
                .is_accepted()
        );
        let added = store
            .cookies()
            .into_iter()
            .find(|cookie| cookie.name == "new")
            .unwrap();
        assert_eq!(added.creation_index, 18);
        assert_eq!(added.last_access_index, 104);
    }

    #[test]
    fn restore_keeps_canonical_security_validation() {
        let mut invalid = cookie("__Secure-invalid");
        invalid.secure = false;
        assert!(BrowserCookieStore::from_persisted([invalid]).is_err());
        let mut invalid = cookie("invalid-samesite");
        invalid.same_site = StoredCookieSameSite::None;
        invalid.secure = false;
        assert!(BrowserCookieStore::from_persisted([invalid]).is_err());
        let mut invalid = cookie("invalid-domain");
        invalid.domain = "invalid host.test".into();
        assert!(BrowserCookieStore::from_persisted([invalid]).is_err());
        let mut invalid = cookie("oversized");
        invalid.value = "x".repeat(4097);
        assert!(BrowserCookieStore::from_persisted([invalid]).is_err());
    }

    #[test]
    fn restore_rejects_duplicate_identities_and_overquota_snapshots() {
        assert!(
            BrowserCookieStore::from_persisted([cookie("duplicate"), cookie("duplicate")]).is_err()
        );
        assert!(
            BrowserCookieStore::from_persisted((0..181).map(|i| cookie(&format!("cookie-{i}"))))
                .is_err()
        );
    }
}
