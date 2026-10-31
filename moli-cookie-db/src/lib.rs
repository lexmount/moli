//! SQLite persistence for the canonical browser cookie jar. Matching, quota
//! and eviction remain in moli-cookie-store; this crate only records deltas.
use moli_cookie_jar::{
    BrowserCookieObserver, BrowserCookieStore, BrowserCookieStoreHandle, CookiePriority,
    CookieStoreError, SharedBrowserCookieStore, StoredCookie, StoredCookieIdentity,
    StoredCookieMutation, StoredCookiePartitionKey, StoredCookieSameSite, StoredCookieSourceScheme,
};
use moli_sqlite::{
    OpenMode, RuntimeHandle,
    writeback::{WriteBatch, WritebackClient, WritebackError, WritebackOwner, WritebackPolicy},
};
use rusqlite::{Connection, params};
use std::{collections::BTreeMap, fmt, path::PathBuf, sync::Arc, time::Duration};
mod codec;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    partition: Vec<u8>,
    domain: String,
    path: String,
    name: String,
}
impl From<StoredCookieIdentity> for Key {
    fn from(key: StoredCookieIdentity) -> Self {
        Self {
            partition: codec::partition_bytes(key.partition_key.as_ref()),
            domain: key.domain,
            path: key.path,
            name: key.name,
        }
    }
}
impl From<&StoredCookie> for Key {
    fn from(cookie: &StoredCookie) -> Self {
        StoredCookieIdentity::from(cookie).into()
    }
}
enum Change {
    Put(StoredCookie),
    Delete,
}
#[derive(Default)]
struct Batch {
    clear: bool,
    changes: BTreeMap<Key, Change>,
}
impl Batch {
    fn change(change: StoredCookieMutation) -> Self {
        match change {
            StoredCookieMutation::Upsert { cookie } => Self {
                clear: false,
                changes: BTreeMap::from([(Key::from(&cookie), Change::Put(cookie))]),
            },
            StoredCookieMutation::Delete(key) => Self {
                clear: false,
                changes: BTreeMap::from([(key.into(), Change::Delete)]),
            },
            StoredCookieMutation::Clear => Self {
                clear: true,
                changes: BTreeMap::new(),
            },
        }
    }
    fn apply(&self, connection: &Connection) -> Result<(), WritebackError> {
        if self.clear {
            connection.execute("DELETE FROM cookies", [])?;
        }
        for (key, change) in &self.changes {
            match change {
                Change::Put(cookie) => codec::upsert(connection, key, cookie)?,
                Change::Delete => {
                    connection.execute("DELETE FROM cookies WHERE partition_key=?1 AND domain=?2 AND path=?3 AND name=?4",params![key.partition,key.domain,key.path,key.name])?;
                }
            }
        }
        Ok(())
    }
}
impl WriteBatch for Batch {
    fn is_empty(&self) -> bool {
        !self.clear && self.changes.is_empty()
    }
    fn merge(&mut self, newer: Self) {
        if newer.clear {
            *self = newer;
            return;
        }
        self.changes.extend(newer.changes);
    }

    fn write(&self, connection: &mut Connection) -> Result<(), WritebackError> {
        let result: Result<(), WritebackError> = (|| {
            let tx = connection.transaction()?;
            self.apply(&tx)?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = &result {
            tracing::warn!(%error, "Cookie persistence batch failed; the batch will not be retried");
        }
        result
    }
}
struct Writer(WritebackClient<Batch>);
impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CookieSQLiteWriter")
    }
}
impl BrowserCookieObserver for Writer {
    fn changed(&self, change: StoredCookieMutation) {
        if let Err(error) = self.0.record(Batch::change(change)) {
            tracing::warn!(error=%error,"Cookie persistence rejected a mutation");
        }
    }
}
pub struct CookieDatabase {
    owner: WritebackOwner<Batch>,
}
impl CookieDatabase {
    /// Open and restore immediately, for explicit imports and validation.
    pub fn open(
        path: PathBuf,
        mode: OpenMode,
        runtime: RuntimeHandle,
    ) -> Result<(SharedBrowserCookieStore, Self), WritebackError> {
        let (store, database) = Self::deferred(path, mode, runtime)?;
        drop(store.lock().map_err(WritebackError::new)?);
        Ok((store, database))
    }
    /// Attach the live jar without touching disk. Its first access restores
    /// persisted cookies and attaches exactly one persistence observer.
    pub fn deferred(
        path: PathBuf,
        mode: OpenMode,
        runtime: RuntimeHandle,
    ) -> Result<(SharedBrowserCookieStore, Self), WritebackError> {
        let owner = WritebackOwner::deferred(
            path,
            mode,
            runtime,
            WritebackPolicy {
                delay: Duration::from_secs(30),
                early_commit_operations: Some(512),
            },
            codec::initialize,
        )?;
        let client = owner.client();
        let store = BrowserCookieStoreHandle::deferred(move || {
            restore(client).map_err(CookieStoreError::new)
        });
        Ok((store, Self { owner }))
    }
    pub fn flush(&self) -> Result<(), WritebackError> {
        self.owner.client().flush()
    }
    pub fn close(&self) -> Result<(), WritebackError> {
        self.owner.close()
    }
}

fn restore(client: WritebackClient<Batch>) -> Result<BrowserCookieStore, WritebackError> {
    let rows = client.query(codec::read)?;
    let (obsolete, live): (Vec<_>, Vec<_>) = rows.into_iter().partition(|cookie| {
        cookie.is_expired()
            || matches!(
                cookie.partition_key,
                Some(StoredCookiePartitionKey::Opaque { .. })
            )
    });
    let mut jar = BrowserCookieStore::from_persisted(live).map_err(WritebackError::new)?;
    jar.set_persistence_observer(Arc::new(Writer(client.clone())));
    let mut cleanup = Batch::default();
    for cookie in obsolete {
        cleanup.changes.insert(Key::from(&cookie), Change::Delete);
    }
    client.record(cleanup)?;
    Ok(jar)
}

#[cfg(test)]
mod tests;
