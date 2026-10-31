//! A shared jar can be attached to pages and network clients before it is used.
//! First access restores it once; failures remain errors, never an empty jar.
use super::BrowserCookieStore;
use parking_lot::{Mutex, MutexGuard};
use std::{
    fmt,
    sync::{Arc, OnceLock},
};

#[derive(Debug, Clone)]
pub struct CookieStoreError(Arc<str>);
impl CookieStoreError {
    pub fn new(error: impl fmt::Display) -> Self {
        Self(error.to_string().into())
    }
}
impl fmt::Display for CookieStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CookieStoreError {}

type Initializer = Box<dyn FnOnce() -> Result<BrowserCookieStore, CookieStoreError> + Send>;
pub struct BrowserCookieStoreHandle {
    store: OnceLock<Result<Mutex<BrowserCookieStore>, CookieStoreError>>,
    initialize: Mutex<Option<Initializer>>,
}
pub type SharedBrowserCookieStore = Arc<BrowserCookieStoreHandle>;
impl BrowserCookieStoreHandle {
    pub fn deferred(
        initialize: impl FnOnce() -> Result<BrowserCookieStore, CookieStoreError> + Send + 'static,
    ) -> SharedBrowserCookieStore {
        Arc::new(Self {
            store: OnceLock::new(),
            initialize: Mutex::new(Some(Box::new(initialize))),
        })
    }
    pub fn lock(&self) -> Result<MutexGuard<'_, BrowserCookieStore>, CookieStoreError> {
        self.store
            .get_or_init(|| {
                let initialize = self
                    .initialize
                    .lock()
                    .take()
                    .expect("cookie initializer runs once");
                initialize().map(Mutex::new)
            })
            .as_ref()
            .map(Mutex::lock)
            .map_err(Clone::clone)
    }
    /// Metadata-only inspection must not trigger a database load.
    pub fn lock_if_initialized(
        &self,
    ) -> Option<Result<MutexGuard<'_, BrowserCookieStore>, CookieStoreError>> {
        self.store
            .get()
            .map(|store| store.as_ref().map(Mutex::lock).map_err(Clone::clone))
    }
}
impl fmt::Debug for BrowserCookieStoreHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserCookieStoreHandle")
            .field("initialized", &self.store.get().is_some())
            .finish()
    }
}
impl BrowserCookieStore {
    pub fn into_shared(self) -> SharedBrowserCookieStore {
        Arc::new(BrowserCookieStoreHandle {
            store: OnceLock::from(Ok(Mutex::new(self))),
            initialize: Mutex::new(None),
        })
    }
}
pub fn new_shared_browser_cookie_store() -> SharedBrowserCookieStore {
    BrowserCookieStore::default().into_shared()
}
