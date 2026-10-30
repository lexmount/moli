//! SQLite persistence for the canonical browser cookie jar. Matching, quota
//! and eviction remain in moli-cookie-store; this crate only records deltas.
use moli_cookie_jar::{
    BrowserCookieObserver, BrowserCookieStore, CookiePriority, SharedBrowserCookieStore,
    StoredCookie, StoredCookieIdentity, StoredCookieMutation, StoredCookiePartitionKey,
    StoredCookieSameSite, StoredCookieSourceScheme,
};
use moli_sqlite::{
    OpenMode, RuntimeHandle,
    writeback::{
        FailureAdmission, WriteBatch, WritebackClient, WritebackError, WritebackOwner,
        WritebackPolicy,
    },
};
use parking_lot::Mutex;
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
    Put { cookie: StoredCookie, existed: bool },
    Delete,
    Touch(u64),
}
#[derive(Default)]
struct Batch {
    clear: bool,
    changes: BTreeMap<Key, Change>,
}
impl Batch {
    fn change(change: StoredCookieMutation) -> Self {
        match change {
            StoredCookieMutation::Upsert { cookie, existed } => Self {
                clear: false,
                changes: BTreeMap::from([(Key::from(&cookie), Change::Put { cookie, existed })]),
            },
            StoredCookieMutation::Delete(key) => Self {
                clear: false,
                changes: BTreeMap::from([(key.into(), Change::Delete)]),
            },
            StoredCookieMutation::Touch {
                key,
                last_access_index,
            } => Self {
                clear: false,
                changes: BTreeMap::from([(key.into(), Change::Touch(last_access_index))]),
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
                Change::Put { cookie, .. } => codec::upsert(connection, key, cookie)?,
                Change::Delete => {
                    connection.execute("DELETE FROM cookies WHERE partition_key=?1 AND domain=?2 AND path=?3 AND name=?4",params![key.partition,key.domain,key.path,key.name])?;
                }
                Change::Touch(index) => {
                    connection.execute("UPDATE cookies SET last_access_index=?5 WHERE partition_key=?1 AND domain=?2 AND path=?3 AND name=?4",params![key.partition,key.domain,key.path,key.name,index.to_be_bytes().as_slice()])?;
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
        for (key, change) in newer.changes {
            let previous = self.changes.remove(&key);
            match change {
                Change::Put { cookie, existed } => {
                    let existed = match previous {
                        Some(Change::Put { existed, .. }) => existed,
                        Some(Change::Delete | Change::Touch(_)) => true,
                        None => existed && !self.clear,
                    };
                    self.changes.insert(key, Change::Put { cookie, existed });
                }
                Change::Delete => {
                    if !matches!(previous, Some(Change::Put { existed: false, .. })) && !self.clear
                    {
                        self.changes.insert(key, Change::Delete);
                    }
                }
                Change::Touch(index) => match previous {
                    Some(Change::Put {
                        mut cookie,
                        existed,
                    }) => {
                        cookie.last_access_index = index;
                        self.changes.insert(key, Change::Put { cookie, existed });
                    }
                    Some(Change::Delete) => {
                        self.changes.insert(key, Change::Delete);
                    }
                    _ if !self.clear => {
                        self.changes.insert(key, Change::Touch(index));
                    }
                    _ => {}
                },
            }
        }
    }
    fn mark_commit_uncertain(&mut self) {
        for change in self.changes.values_mut() {
            if let Change::Put { existed, .. } = change {
                *existed = true;
            }
        }
    }
    fn write(&self, connection: &mut Connection) -> Result<(), WritebackError> {
        let tx = connection.transaction()?;
        self.apply(&tx)?;
        tx.commit()?;
        Ok(())
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
        if let Err(error) = self
            .0
            .record(Batch::change(change), FailureAdmission::Retain)
        {
            tracing::warn!(error=%error,"Cookie persistence rejected a mutation");
        }
    }
}
pub struct CookieDatabase {
    owner: WritebackOwner<Batch>,
}
impl CookieDatabase {
    /// Native partition initialization. The live jar has exactly one writer;
    /// cloning its data never clones this persistence attachment.
    pub fn open(
        path: PathBuf,
        mode: OpenMode,
        runtime: RuntimeHandle,
    ) -> Result<(SharedBrowserCookieStore, Self), WritebackError> {
        let owner = WritebackOwner::open(
            path,
            mode,
            runtime,
            WritebackPolicy {
                delay: Duration::from_secs(30),
                early_commit_operations: Some(512),
            },
            codec::initialize,
        )?;
        let rows = owner.client().query(codec::read)?;
        let (obsolete, live): (Vec<_>, Vec<_>) = rows.into_iter().partition(|cookie| {
            cookie.is_expired()
                || matches!(
                    cookie.partition_key,
                    Some(StoredCookiePartitionKey::Opaque { .. })
                )
        });
        let mut jar = BrowserCookieStore::from_persisted(live).map_err(WritebackError::new)?;
        let writer = Arc::new(Writer(owner.client()));
        jar.set_persistence_observer(writer);
        let mut cleanup = Batch::default();
        for cookie in obsolete {
            cleanup.changes.insert(Key::from(&cookie), Change::Delete);
        }
        owner.client().record(cleanup, FailureAdmission::Retain)?;
        Ok((Arc::new(Mutex::new(jar)), Self { owner }))
    }
    pub fn flush(&self) -> Result<(), WritebackError> {
        self.owner.client().flush()
    }
    pub fn close(&self) -> Result<(), WritebackError> {
        self.owner.close()
    }
    pub fn checkpoint(&self) -> Result<(), WritebackError> {
        self.flush()?;
        self.owner.client().query(|connection| {
            let busy: i32 =
                connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
            if busy != 0 {
                return Err(WritebackError::new("Cookie checkpoint is busy"));
            }
            Ok(())
        })
    }
    pub fn migration_fingerprint(&self) -> Result<Option<String>, WritebackError> {
        self.owner.client().query(codec::migration_fingerprint)
    }
}

/// Startup-only one-time JSON import. Use an owned staging path, then
/// checkpoint/close before publishing the file or switching the manifest.
pub fn create_from_legacy(
    path: PathBuf,
    cookies: Vec<StoredCookie>,
    fingerprint: String,
    migration_id: String,
    runtime: RuntimeHandle,
) -> Result<(), WritebackError> {
    let owner = WritebackOwner::<Batch>::open(
        path,
        OpenMode::Create,
        runtime,
        WritebackPolicy {
            delay: Duration::from_secs(30),
            early_commit_operations: Some(512),
        },
        codec::initialize,
    )?;
    owner.client().query(move |connection| {
        let mut jar = BrowserCookieStore::default();
        // Match the legacy importer: canonical rejection/eviction remain
        // authoritative; malformed JSON itself is rejected by its codec.
        for cookie in cookies {
            jar.upsert_with_request_url_report(cookie, None, moli_cookie_jar::CookieSource::Cdp);
        }
        let rows = jar.cookies();
        let batch = Batch {
            clear: true,
            changes: rows
                .into_iter()
                .map(|cookie| {
                    (
                        Key::from(&cookie),
                        Change::Put {
                            cookie,
                            existed: false,
                        },
                    )
                })
                .collect(),
        };
        let tx = connection.transaction()?;
        batch.apply(&tx)?;
        tx.execute("DELETE FROM migration", [])?;
        tx.execute(
            "INSERT INTO migration VALUES (?1,?2,1)",
            params![fingerprint, migration_id],
        )?;
        tx.commit()?;
        Ok(())
    })?;
    owner.client().query(|connection| {
        let busy: i32 =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(WritebackError::new("Cookie migration checkpoint is busy"));
        }
        Ok(())
    })?;
    owner.close()
}
impl CookieDatabase {
    pub fn inspect_migration(
        path: PathBuf,
        runtime: RuntimeHandle,
    ) -> Result<Option<(String, String)>, WritebackError> {
        WritebackOwner::<Batch>::inspect(path, runtime, |connection| {
            let application: i32 =
                connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
            let version: i32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if application != codec::APPLICATION_ID || version != 1 {
                return Err(WritebackError::new(
                    "unidentified Cookie migration database",
                ));
            }
            let count: i64 =
                connection.query_row("SELECT count(*) FROM migration", [], |r| r.get(0))?;
            if count != 1 {
                return Err(WritebackError::new("incomplete Cookie migration"));
            }
            Ok(Some(connection.query_row(
                "SELECT source_fingerprint,migration_id FROM migration WHERE complete=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?))
        })
    }
}

#[cfg(test)]
mod tests;
