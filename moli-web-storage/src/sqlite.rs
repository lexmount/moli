use super::{store::Edit, *};
use moli_sqlite::writeback::{
    WriteBatch, WritebackClient, WritebackError, WritebackOwner, WritebackPolicy,
};
use moli_sqlite::{OpenMode, RuntimeHandle};
use rusqlite::{Connection, OptionalExtension, params};
#[cfg(test)]
use std::path::Path;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub(super) const APPLICATION_ID: i32 = 0x4d4c4c53; // MLLS
const SCHEMA_VERSION: i32 = 1;
pub struct SqliteWebStorageOwner {
    owner: WritebackOwner<Batch>,
}
struct Area {
    state: Mutex<Option<MemoryWebStorageArea>>,
}
pub struct WebStorageAreaLease {
    _area: Arc<Area>,
}
#[derive(Default)]
struct Cache {
    areas: HashMap<String, Weak<Area>>,
    insertions: usize,
}
pub(super) struct SqliteBackend {
    pub(super) client: WritebackClient<Batch>,
    cache: Mutex<Cache>,
    pub(super) namespace: parking_lot::RwLock<()>,
}
const ORIGIN_KEYS_SQL: &str = "SELECT storage_key FROM areas
    WHERE storage_key=?1 OR (storage_key>=?2 AND storage_key<?3)";

fn origin_prefix_range(origin: &str) -> (String, String) {
    let prefix = web_storage_area_key_prefix_for_origin(origin);
    let mut upper = prefix.clone();
    // The serialization prefix ends in '='; its successor bounds all suffixes.
    upper.pop();
    upper.push('>');
    (prefix, upper)
}
#[derive(Default)]
pub(super) struct Batch(BTreeMap<String, AreaDelta>);
struct AreaDelta {
    clear: bool,
    changes: BTreeMap<WebStorageString, KeyChange>,
    _pin: Arc<Area>,
}
enum KeyChange {
    Put(WebStorageString),
    Delete,
}
impl WriteBatch for Batch {
    const RETAIN_FAILED_BATCH: bool = true;
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn merge(&mut self, newer: Self) {
        for (key, mut next) in newer.0 {
            if let Some(mut previous) = self.0.remove(&key)
                && !next.clear
            {
                previous.changes.extend(next.changes);
                next.changes = previous.changes;
                next.clear = previous.clear;
            }
            self.0.insert(key, next);
        }
    }
    fn write(&self, connection: &mut Connection) -> std::result::Result<(), WritebackError> {
        let tx = connection.transaction()?;
        for (key, delta) in &self.0 {
            if delta.clear {
                tx.execute("DELETE FROM areas WHERE storage_key=?1", [key])?;
            }
            if delta.changes.is_empty() {
                continue;
            }
            let area = write_area(&tx, key)?;
            for (key, value) in &delta.changes {
                match value {
                    KeyChange::Put(value) => {
                        tx.execute("INSERT INTO entries(area_id,key,value) VALUES (?1,?2,?3) ON CONFLICT(area_id,key) DO UPDATE SET value=excluded.value", params![area, encode(key.as_units()), encode(value.as_units())])?;
                    }
                    KeyChange::Delete => {
                        tx.execute(
                            "DELETE FROM entries WHERE area_id=?1 AND key=?2",
                            params![area, encode(key.as_units())],
                        )?;
                    }
                }
            }
            tx.execute(
                "DELETE FROM areas WHERE area_id=?1 AND NOT EXISTS (SELECT 1 FROM entries WHERE area_id=?1)",
                [area],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
impl From<WritebackError> for WebStorageMutationError {
    fn from(error: WritebackError) -> Self {
        Self::Persistence(error.to_string())
    }
}
pub fn new_sqlite_web_storage_store(
    path: PathBuf,
    mode: OpenMode,
    runtime: RuntimeHandle,
) -> Result<(SharedWebStorageStore, SqliteWebStorageOwner)> {
    let (store, owner) = new_lazy_sqlite_web_storage_store(path, mode, runtime)?;
    owner.owner.client().query(|_| Ok(()))?;
    Ok((store, owner))
}

/// First data access opens the shared connection; acquiring a store or area
/// lease alone performs no filesystem work.
pub fn new_lazy_sqlite_web_storage_store(
    path: PathBuf,
    mode: OpenMode,
    runtime: RuntimeHandle,
) -> Result<(SharedWebStorageStore, SqliteWebStorageOwner)> {
    let owner = WritebackOwner::deferred(
        path,
        mode,
        runtime,
        WritebackPolicy {
            delay: std::time::Duration::from_secs(5),
            early_commit_operations: None,
        },
        initialize,
    )?;
    let backend = SqliteBackend {
        client: owner.client(),
        cache: Mutex::new(Cache::default()),
        namespace: parking_lot::RwLock::new(()),
    };
    let store = Arc::new(WebStorageStore {
        backend: WebStorageBackend::Sqlite(backend),
        mutation_subscribers: Mutex::new(Vec::new()),
    });
    Ok((store, SqliteWebStorageOwner { owner }))
}

/// Offline, explicit browser-data import under the destination profile lock.
/// Merge affected areas in one transaction; no live Storage view is exposed.
pub fn import_sqlite_web_storage(
    path: PathBuf,
    runtime: RuntimeHandle,
    areas: BTreeMap<String, Vec<(Vec<u16>, Vec<u16>)>>,
) -> Result<()> {
    let (_store, owner) = new_sqlite_web_storage_store(path, OpenMode::Create, runtime)?;
    let result = owner.owner.client().query(move |connection| {
        let tx = connection.transaction()?;
        for (key, entries) in areas {
            if entries.is_empty() {
                continue;
            }
            let mut area = read_area(&tx, &key)?;
            let mut changes = Vec::new();
            for (key, value) in entries {
                let key = WebStorageString::from_utf16_units(key);
                let value = WebStorageString::from_utf16_units(value);
                if area.values.get(&key) != Some(&value) {
                    area.values.insert(key.clone(), value.clone());
                    changes.push((key, value));
                }
            }
            if changes.is_empty() {
                continue;
            }
            let id = write_area(&tx, &key)?;
            let mut statement = tx.prepare_cached(
                "INSERT INTO entries(area_id,key,value) VALUES (?1,?2,?3) ON CONFLICT(area_id,key) DO UPDATE SET value=excluded.value",
            )?;
            for (key, value) in changes {
                statement.execute(params![id, encode(key.as_units()), encode(value.as_units())])?;
            }
        }
        tx.commit()?;
        Ok(())
    });
    let close = owner.close();
    result?;
    close
}

fn write_area(connection: &Connection, key: &str) -> std::result::Result<i64, WritebackError> {
    Ok(connection.query_row(
        "INSERT INTO areas(storage_key) VALUES (?1) ON CONFLICT(storage_key) DO UPDATE SET storage_key=excluded.storage_key RETURNING area_id",
        [key], |row| row.get(0))?)
}

fn read_area(
    connection: &Connection,
    key: &str,
) -> std::result::Result<MemoryWebStorageArea, WritebackError> {
    let id: Option<i64> = connection
        .query_row(
            "SELECT area_id FROM areas WHERE storage_key=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(MemoryWebStorageArea::default());
    };
    let mut statement = connection.prepare("SELECT key,value FROM entries WHERE area_id=?1")?;
    let rows = statement.query_map([id], |r| {
        Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut values = HashMap::new();
    for row in rows {
        let (key, value) = row?;
        values.insert(decode(key)?, decode(value)?);
    }
    let size = storage_area_size(&values);
    Ok(MemoryWebStorageArea { values, size })
}

pub(super) fn initialize(connection: &mut Connection) -> std::result::Result<(), WritebackError> {
    connection.pragma_update(None, "foreign_keys", true)?;
    let application: i32 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
    let version: i32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if application == 0 && version == 0 {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if tables != 0 {
            return Err(WritebackError::new("unidentified localStorage database"));
        }
        let tx = connection.transaction()?;
        tx.execute_batch("CREATE TABLE areas(area_id INTEGER PRIMARY KEY,storage_key TEXT NOT NULL UNIQUE);
            CREATE TABLE entries(area_id INTEGER NOT NULL REFERENCES areas(area_id) ON DELETE CASCADE,key BLOB NOT NULL CHECK(length(key)%2=0),value BLOB NOT NULL CHECK(length(value)%2=0),PRIMARY KEY(area_id,key)) WITHOUT ROWID;")?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
    } else if application != APPLICATION_ID || version != SCHEMA_VERSION {
        return Err(WritebackError::new(format!(
            "unsupported localStorage database identity/version {application}/{version}"
        )));
    }
    // Verify the actual schema even when a database claims the current version.
    connection.prepare("SELECT area_id,storage_key FROM areas LIMIT 0")?;
    connection.prepare("SELECT area_id,key,value FROM entries LIMIT 0")?;
    Ok(())
}
pub(super) fn encode(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}
fn decode(bytes: Vec<u8>) -> std::result::Result<WebStorageString, WritebackError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(WritebackError::new("invalid UTF-16LE localStorage row"));
    }
    Ok(WebStorageString::from_utf16_units(
        bytes
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect::<Vec<_>>(),
    ))
}
impl SqliteBackend {
    fn live_areas_for_origin(&self, origin: &str) -> BTreeMap<String, Arc<Area>> {
        let prefix = web_storage_area_key_prefix_for_origin(origin);
        self.cache
            .lock()
            .areas
            .iter()
            .filter(|(key, _)| key.as_str() == origin || key.starts_with(&prefix))
            .filter_map(|(key, area)| area.upgrade().map(|area| (key.clone(), area)))
            .collect()
    }
    fn area_keys_for_origin(
        &self,
        origin: &str,
    ) -> std::result::Result<BTreeSet<String>, WebStorageMutationError> {
        let (prefix, upper) = origin_prefix_range(origin);
        let origin = origin.to_owned();
        Ok(self.client.query(move |connection| {
            let mut statement = connection.prepare_cached(ORIGIN_KEYS_SQL)?;
            Ok(statement
                .query_map(params![origin, prefix, upper], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<std::result::Result<BTreeSet<_>, _>>()?)
        })?)
    }
    fn area(&self, origin: &str) -> Arc<Area> {
        let mut cache = self.cache.lock();
        if let Some(area) = cache.areas.get(origin).and_then(Weak::upgrade) {
            return area;
        }
        cache.insertions += 1;
        if cache.insertions >= 64 {
            cache.areas.retain(|_, area| area.strong_count() != 0);
            cache.insertions = 0;
        }
        let area = Arc::new(Area {
            state: Mutex::new(None),
        });
        cache.areas.insert(origin.to_owned(), Arc::downgrade(&area));
        area
    }
    fn load(
        &self,
        origin: &str,
        state: &mut Option<MemoryWebStorageArea>,
    ) -> std::result::Result<(), WebStorageMutationError> {
        self.client.ensure_open()?;
        if state.is_some() {
            return Ok(());
        }
        let origin = origin.to_owned();
        *state = Some(
            self.client
                .query(move |connection| read_area(connection, &origin))?,
        );
        Ok(())
    }
    pub(super) fn read<R>(
        &self,
        origin: &str,
        read: impl FnOnce(Option<&MemoryWebStorageArea>) -> R,
    ) -> std::result::Result<R, WebStorageMutationError> {
        let _namespace = self.namespace.read();
        let area = self.area(origin);
        let mut state = area.state.lock();
        self.load(origin, &mut state)?;
        Ok(read(state.as_ref()))
    }
    pub(super) fn edit<R>(
        &self,
        origin: &str,
        edit: impl FnOnce(
            &mut MemoryWebStorageArea,
        ) -> std::result::Result<Edit<R>, WebStorageMutationError>,
        publish: impl FnOnce(Edit<R>) -> R,
    ) -> std::result::Result<R, WebStorageMutationError> {
        let _namespace = self.namespace.read();
        self.edit_unfenced(origin, edit, publish)
    }
    pub(super) fn edit_unfenced<R>(
        &self,
        origin: &str,
        edit: impl FnOnce(
            &mut MemoryWebStorageArea,
        ) -> std::result::Result<Edit<R>, WebStorageMutationError>,
        publish: impl FnOnce(Edit<R>) -> R,
    ) -> std::result::Result<R, WebStorageMutationError> {
        let area = self.area(origin);
        let mut state = area.state.lock();
        self.load(origin, &mut state)?;
        let data = state.as_mut().expect("loaded area");
        let result = self.client.accept(|| {
            let result = edit(data)?;
            let mut batch = Batch::default();
            if let Some(mutation) = &result.mutation {
                let mut delta = AreaDelta {
                    clear: false,
                    changes: BTreeMap::new(),
                    _pin: area.clone(),
                };
                match mutation {
                    WebStorageMutation::ItemAdded { key, value, .. } => {
                        delta
                            .changes
                            .insert(key.clone(), KeyChange::Put(value.clone()));
                    }
                    WebStorageMutation::ItemUpdated { key, new_value, .. } => {
                        delta
                            .changes
                            .insert(key.clone(), KeyChange::Put(new_value.clone()));
                    }
                    WebStorageMutation::ItemRemoved { key, .. } => {
                        delta.changes.insert(key.clone(), KeyChange::Delete);
                    }
                    WebStorageMutation::ItemsCleared { .. } => delta.clear = true,
                }
                batch.0.insert(origin.to_owned(), delta);
            }
            Ok::<_, WebStorageMutationError>((result, batch))
        })?;
        Ok(publish(result))
    }
    pub(super) fn clear_origin_areas(
        &self,
        origin: &str,
        publish: impl Fn(WebStorageMutation),
    ) -> std::result::Result<bool, WebStorageMutationError> {
        let _namespace = self.namespace.write();
        let mut areas = self.live_areas_for_origin(origin);
        let stored_keys = self.area_keys_for_origin(origin)?;
        for key in &stored_keys {
            areas.entry(key.clone()).or_insert_with(|| self.area(key));
        }
        let mutations = self.client.accept(|| {
            let mut batch = Batch::default();
            let mut mutations = Vec::new();
            for (key, area) in &areas {
                let mut state = area.state.lock();
                let nonempty = state
                    .as_ref()
                    .map(|data| !data.values.is_empty())
                    .unwrap_or_else(|| stored_keys.contains(key));
                if !nonempty {
                    continue;
                }
                // An empty logical map fences reads until this deletion is
                // durable. Cold entries never need to be materialized, and a
                // later put composes after clear in the same writeback FIFO.
                *state = Some(MemoryWebStorageArea::default());
                batch.0.insert(
                    key.clone(),
                    AreaDelta {
                        clear: true,
                        changes: BTreeMap::new(),
                        _pin: area.clone(),
                    },
                );
                mutations.push(WebStorageMutation::ItemsCleared {
                    area_key: key.clone(),
                });
            }
            Ok::<_, WebStorageMutationError>((mutations, batch))
        })?;
        let changed = !mutations.is_empty();
        for mutation in mutations {
            publish(mutation);
        }
        Ok(changed)
    }
}
impl WebStorageStore {
    pub fn lease_area(
        &self,
        origin: &str,
    ) -> std::result::Result<Option<WebStorageAreaLease>, WebStorageMutationError> {
        let WebStorageBackend::Sqlite(sqlite) = &self.backend else {
            return Ok(None);
        };
        sqlite.client.ensure_open()?;
        let area = sqlite.area(origin);
        Ok(Some(WebStorageAreaLease { _area: area }))
    }
}
impl SqliteWebStorageOwner {
    pub fn flush(&self) -> Result<()> {
        self.owner.client().flush()?;
        Ok(())
    }
    pub fn close(&self) -> Result<()> {
        self.owner.close()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
