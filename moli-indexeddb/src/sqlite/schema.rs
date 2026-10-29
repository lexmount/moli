use super::*;
use crate::{usage::key_path_usage_bytes, validate_index_options, validate_object_store_options};

pub(super) fn read_metadata(
    connection: &Connection,
) -> Result<Arc<DatabaseMetadata>, IndexedDbError> {
    let mut statement = connection.prepare_cached(
        "SELECT name, version, state, usage FROM database_metadata WHERE singleton=1 AND key_format=1 AND key_path_format=1 AND value_format=1",
    )?;
    let mut rows = statement.query([])?;
    let row = rows
        .next()?
        .ok_or_else(|| corruption("missing IndexedDB database metadata"))?;
    let version: i64 = row.get(1)?;
    let usage: i64 = row.get(3)?;
    let state = text(row, 2)?;
    if version < 0 || usage < 0 || !matches!((state, version), ("creating", 0) | ("live", 1..)) {
        return Err(corruption("invalid IndexedDB database metadata"));
    }
    let name = owned_text(text(row, 0)?)?;
    drop(rows);
    drop(statement);
    let mut stores = BTreeMap::new();
    let mut statement = connection.prepare_cached(
        "SELECT store_id, name, key_path, auto_increment FROM object_stores ORDER BY name",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if id <= 0 {
            return Err(corruption("invalid object store ID"));
        }
        let key_path = match row.get_ref(2)? {
            rusqlite::types::ValueRef::Null => None,
            rusqlite::types::ValueRef::Blob(bytes) => Some(crate::KeyPath::decode_value(bytes)?),
            _ => return Err(corruption("invalid object store key path column")),
        };
        let options = ObjectStoreOptions {
            key_path,
            auto_increment: row.get(3)?,
        };
        validate_object_store_options(options.key_path.as_ref(), options.auto_increment)
            .map_err(|_| corruption("invalid object store options"))?;
        stores.insert(
            owned_text(text(row, 1)?)?,
            StoreMetadata {
                id,
                options,
                indexes: BTreeMap::new(),
            },
        );
    }
    // Join the store name instead of building a second map containing cloned
    // names. LEFT JOIN retains corrupt dangling indexes for explicit rejection.
    let mut statement = connection.prepare_cached(
        "SELECT i.index_id, s.name, i.name, i.key_path, i.is_unique, i.multi_entry FROM indexes i LEFT JOIN object_stores s ON s.store_id=i.store_id ORDER BY i.store_id, i.name",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let store = stores
            .get_mut(text(row, 1)?)
            .ok_or_else(|| corruption("index refers to an unknown object store"))?;
        if id <= 0 {
            return Err(corruption("invalid index ID"));
        }
        let bytes = row
            .get_ref(3)?
            .as_blob()
            .map_err(|_| corruption("invalid index key path column"))?;
        let options = IndexOptions {
            key_path: crate::KeyPath::decode_value(bytes)?,
            unique: row.get(4)?,
            multi_entry: row.get(5)?,
        };
        validate_index_options(&options.key_path, options.multi_entry)
            .map_err(|_| corruption("invalid index options"))?;
        store
            .indexes
            .insert(owned_text(text(row, 2)?)?, IndexMetadata { id, options });
    }
    if version == 0 && !stores.is_empty() {
        return Err(corruption("unpublished IndexedDB contains object stores"));
    }
    Ok(Arc::new(DatabaseMetadata {
        name,
        version: version as u64,
        usage: if version == 0 { 0 } else { usage as u64 },
        stores: Arc::new(stores),
    }))
}

impl DatabaseMetadata {
    pub(super) fn with_usage(&self, usage: u64) -> Result<Arc<Self>, IndexedDbError> {
        let name = owned_text(&self.name)?;
        Ok(Arc::new(Self {
            name,
            usage,
            version: self.version,
            stores: self.stores.clone(),
        }))
    }
}

fn text<'a>(row: &'a rusqlite::Row<'_>, column: usize) -> Result<&'a str, IndexedDbError> {
    row.get_ref(column)?
        .as_str()
        .map_err(|_| corruption("invalid schema text column"))
}

fn owned_text(value: &str) -> Result<String, IndexedDbError> {
    let mut output = String::new();
    output
        .try_reserve_exact(value.len())
        .map_err(|_| IndexedDbError::Resource("Unable to allocate IndexedDB schema text".into()))?;
    output.push_str(value);
    Ok(output)
}

pub(super) fn store_usage(name: &str, options: &ObjectStoreOptions) -> u64 {
    name.len() as u64 + 9 + options.key_path.as_ref().map_or(0, key_path_usage_bytes)
}

pub(super) fn index_usage(name: &str, options: &IndexOptions) -> u64 {
    name.len() as u64 + 2 + key_path_usage_bytes(&options.key_path)
}

impl Database {
    pub(super) fn versionchange(
        &self,
        transaction: TransactionHandle,
    ) -> Result<(), IndexedDbError> {
        if self.ready(transaction)?.mode != TransactionMode::VersionChange {
            return Err(IndexedDbError::InvalidState(
                "schema changes require a versionchange transaction".into(),
            ));
        }
        Ok(())
    }

    pub fn create_object_store(
        &mut self,
        transaction: TransactionHandle,
        name: &str,
        options: ObjectStoreOptions,
    ) -> Result<(), IndexedDbError> {
        self.versionchange(transaction)?;
        if options
            .key_path
            .as_ref()
            .is_some_and(|path| !path.is_valid())
        {
            return Err(IndexedDbError::Syntax("The key path is not valid.".into()));
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM object_stores WHERE name=?1)",
            [name],
            |row| row.get(0),
        )?;
        if exists {
            return Err(IndexedDbError::Constraint(format!(
                "object store `{name}` already exists"
            )));
        }
        validate_object_store_options(options.key_path.as_ref(), options.auto_increment).map_err(
            |_| {
                IndexedDbError::InvalidAccess(
                    "autoIncrement cannot use an empty or sequence key path".into(),
                )
            },
        )?;
        self.request(transaction, |owner| {
            let id = owner.next_id("next_store_id")?;
            let key_path = options
                .key_path
                .as_ref()
                .map(crate::KeyPath::encode_value)
                .transpose()?;
            owner.connection.execute(
                "INSERT INTO object_stores VALUES(?1, ?2, ?3, ?4, 0)",
                rusqlite::params![id, name, key_path, options.auto_increment],
            )?;
            owner.adjust_usage(i128::from(store_usage(name, &options)))
        })
    }

    pub fn delete_object_store(
        &mut self,
        transaction: TransactionHandle,
        name: &str,
    ) -> Result<(), IndexedDbError> {
        self.versionchange(transaction)?;
        let store = self.store_id(transaction, name)?;
        self.request(transaction, |owner| {
            let metadata = read_metadata(&owner.connection)?;
            let metadata = metadata.stores.get(name).ok_or_else(|| corruption("missing store metadata"))?;
            let definitions = store_usage(name, &metadata.options) + metadata.indexes.iter().map(|(name, index)| index_usage(name, &index.options)).sum::<u64>();
            owner.remove_store_records(store)?;
            owner.connection.execute("DELETE FROM index_entries WHERE index_id IN (SELECT index_id FROM indexes WHERE store_id=?1)", [store])?;
            owner.connection.execute("DELETE FROM indexes WHERE store_id=?1", [store])?;
            owner.connection.execute("DELETE FROM object_stores WHERE store_id=?1", [store])?;
            owner.adjust_usage(-i128::from(definitions))
        })
    }

    /// Backfill is subsequent ordered internal work in the same versionchange
    /// transaction. No committed metadata is published until that work ends.
    pub fn create_index(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        name: &str,
        options: IndexOptions,
    ) -> Result<IndexMetadata, IndexedDbError> {
        self.versionchange(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM indexes WHERE store_id=?1 AND name=?2)",
            rusqlite::params![store, name],
            |row| row.get(0),
        )?;
        if exists {
            return Err(IndexedDbError::Constraint(format!(
                "index `{name}` already exists"
            )));
        }
        if !options.key_path.is_valid() {
            return Err(IndexedDbError::Syntax("The key path is not valid.".into()));
        }
        validate_index_options(&options.key_path, options.multi_entry).map_err(|_| {
            IndexedDbError::InvalidAccess(
                "multiEntry indexes cannot use a sequence key path".into(),
            )
        })?;
        self.request(transaction, |owner| {
            let id = owner.next_id("next_index_id")?;
            owner.connection.execute(
                "INSERT INTO indexes VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id,
                    store,
                    name,
                    options.key_path.encode_value()?,
                    options.unique,
                    options.multi_entry
                ],
            )?;
            owner.adjust_usage(i128::from(index_usage(name, &options)))?;
            Ok(IndexMetadata { id, options })
        })
    }

    pub fn delete_index(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        name: &str,
    ) -> Result<(), IndexedDbError> {
        self.versionchange(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        self.request(transaction, |owner| {
            let metadata = read_metadata(&owner.connection)?;
            let index = metadata
                .stores
                .get(store_name)
                .and_then(|store| store.indexes.get(name))
                .ok_or_else(|| IndexedDbError::NotFound(format!("index `{name}` was not found")))?;
            owner
                .connection
                .execute("DELETE FROM index_entries WHERE index_id=?1", [index.id])?;
            owner.connection.execute(
                "DELETE FROM indexes WHERE store_id=?1 AND index_id=?2",
                [store, index.id],
            )?;
            owner.adjust_usage(-i128::from(index_usage(name, &index.options)))
        })
    }
}
