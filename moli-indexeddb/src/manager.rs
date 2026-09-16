use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    DatabaseHandle, DatabaseInfo, DatabaseNameAndVersion, IndexInfo, IndexOptions, IndexedDbError,
    IndexedDbQuotaCheck, IndexedDbValue, Key, ObjectStoreInfo, ObjectStoreOptions, OpenDisposition,
    OpenOptions, OpenResult, RequestOutcome, TransactionHandle, TransactionMode,
    persistence::IndexedDbPersistenceBackend,
    state::{
        DatabaseData, DatabaseHandleState, IndexData, IndexedDbManager, ObjectStoreData,
        TransactionLifecycle, TransactionState,
    },
    transaction::{ensure_writeable, resolve_key, transaction_store, transaction_store_mut},
    usage::{database_usage_bytes, origin_usage_bytes, sum_usage},
};

impl IndexedDbManager {
    /// Shared across every renderer and worker using this storage partition.
    pub fn connection_request_queues(&self) -> std::sync::Arc<crate::ConnectionRequestQueues> {
        self.connection_requests.clone()
    }

    pub fn new_in_memory() -> Self {
        Self {
            connection_requests: Default::default(),
            connection_notifications: Default::default(),
            backend: IndexedDbPersistenceBackend::InMemory,
            origins: BTreeMap::new(),
            databases: BTreeMap::new(),
            transactions: BTreeMap::new(),
            next_database_handle: AtomicU64::new(1),
            next_transaction_handle: AtomicU64::new(1),
        }
    }

    pub fn new(storage_root: impl Into<PathBuf>) -> Result<Self, IndexedDbError> {
        let storage_root = storage_root.into();
        fs::create_dir_all(&storage_root)
            .map_err(|err| IndexedDbError::Io(format!("failed to create storage root: {err}")))?;
        Ok(Self {
            connection_requests: Default::default(),
            connection_notifications: Default::default(),
            backend: IndexedDbPersistenceBackend::JsonFiles { storage_root },
            origins: BTreeMap::new(),
            databases: BTreeMap::new(),
            transactions: BTreeMap::new(),
            next_database_handle: AtomicU64::new(1),
            next_transaction_handle: AtomicU64::new(1),
        })
    }

    pub fn open(&mut self, options: OpenOptions) -> Result<OpenResult, IndexedDbError> {
        let origin = options.origin;
        let db_name = options.name;
        self.ensure_origin_loaded(&origin)?;

        let maybe_existing = self
            .origins
            .get(&origin)
            .and_then(|state| state.databases.get(&db_name))
            .cloned();

        let Some(existing) = maybe_existing else {
            let new_version = options.version.unwrap_or(1);
            if new_version == 0 {
                return Err(IndexedDbError::Version(
                    "database version must be greater than zero".to_owned(),
                ));
            }
            let database = self.allocate_database_handle(origin.clone(), db_name.clone());
            let tx = self.allocate_upgrade_transaction(
                database,
                &origin,
                &db_name,
                DatabaseData {
                    version: new_version,
                    stores: BTreeMap::new(),
                },
            );
            return Ok(OpenResult {
                database,
                disposition: OpenDisposition::UpgradeNeeded {
                    old_version: 0,
                    new_version,
                },
                upgrade_transaction: Some(tx),
            });
        };

        let requested_version = options.version.unwrap_or(existing.version);
        if requested_version == 0 {
            return Err(IndexedDbError::Version(
                "database version must be greater than zero".to_owned(),
            ));
        }
        if requested_version < existing.version {
            return Err(IndexedDbError::Version(format!(
                "requested version {requested_version} is lower than existing version {}",
                existing.version
            )));
        }
        let database = self.allocate_database_handle(origin.clone(), db_name.clone());
        if requested_version == existing.version {
            return Ok(OpenResult {
                database,
                disposition: OpenDisposition::Existing,
                upgrade_transaction: None,
            });
        }

        let mut upgraded = existing;
        upgraded.version = requested_version;
        let tx = self.allocate_upgrade_transaction(database, &origin, &db_name, upgraded);
        Ok(OpenResult {
            database,
            disposition: OpenDisposition::UpgradeNeeded {
                old_version: self
                    .origins
                    .get(&origin)
                    .and_then(|state| state.databases.get(&db_name))
                    .map(|db| db.version)
                    .unwrap_or(0),
                new_version: requested_version,
            },
            upgrade_transaction: Some(tx),
        })
    }

    pub fn delete_database(&mut self, origin: &str, name: &str) -> Result<(), IndexedDbError> {
        self.ensure_origin_loaded(origin)?;
        if self
            .databases
            .values()
            .any(|db| db.origin == origin && db.name == name)
        {
            return Err(IndexedDbError::InvalidState(format!(
                "database `{name}` for origin `{origin}` is still open"
            )));
        }
        if let Some(origin_state) = self.origins.get_mut(origin) {
            origin_state.databases.remove(name);
        }
        self.persist_origin(origin)
    }

    pub fn clear_origin(&mut self, origin: &str) -> Result<(), IndexedDbError> {
        self.ensure_origin_loaded(origin)?;
        let handles: Vec<_> = self
            .databases
            .iter()
            .filter_map(|(handle, db)| (db.origin == origin).then_some(*handle))
            .collect();
        for handle in handles {
            self.force_close_database(handle)?;
        }
        self.transactions.retain(|_, tx| tx.origin != origin);
        self.origins.remove(origin);
        self.remove_persisted_origin(origin)
    }

    /// Move one complete storage owner to a new opaque owner key.
    ///
    /// This is intended for browser-owned storage identity migrations. It is
    /// deliberately stricter than a merge: active handles are rejected and a
    /// non-empty, different destination is treated as corruption. Repeating a
    /// migration after the destination was persisted but before the source was
    /// removed is safe.
    pub fn migrate_origin(
        &mut self,
        source: &str,
        destination: &str,
    ) -> Result<(), IndexedDbError> {
        if source == destination {
            return Ok(());
        }
        if self.databases.values().any(|database| {
            !database.closed && (database.origin == source || database.origin == destination)
        }) || self
            .transactions
            .values()
            .any(|transaction| transaction.origin == source || transaction.origin == destination)
        {
            return Err(IndexedDbError::InvalidState(format!(
                "cannot migrate IndexedDB owner `{source}` to `{destination}` while it has active handles"
            )));
        }

        self.ensure_origin_loaded(source)?;
        self.ensure_origin_loaded(destination)?;
        let source_state = self
            .origins
            .get(source)
            .cloned()
            .expect("loaded IndexedDB source owner should exist");
        let destination_state = self
            .origins
            .get(destination)
            .cloned()
            .expect("loaded IndexedDB destination owner should exist");

        if source_state.databases.is_empty() {
            self.origins.remove(source);
            return self.remove_persisted_origin(source);
        }
        if !destination_state.databases.is_empty() && destination_state != source_state {
            return Err(IndexedDbError::Corruption(format!(
                "cannot migrate IndexedDB owner `{source}` to non-empty owner `{destination}`"
            )));
        }

        self.origins.insert(destination.to_owned(), source_state);
        self.persist_origin(destination)?;
        self.origins.remove(source);
        self.remove_persisted_origin(source)
    }

    pub fn clear_origins_with_prefix(&mut self, origin_prefix: &str) -> Result<(), IndexedDbError> {
        self.databases
            .retain(|_, db| !db.origin.starts_with(origin_prefix));
        self.transactions
            .retain(|_, tx| !tx.origin.starts_with(origin_prefix));
        self.origins
            .retain(|origin, _| !origin.starts_with(origin_prefix));
        self.remove_persisted_origins_with_prefix(origin_prefix)
    }

    pub fn origin_usage_bytes(&mut self, origin: &str) -> Result<u64, IndexedDbError> {
        self.ensure_origin_loaded(origin)?;
        Ok(self
            .origins
            .get(origin)
            .map(origin_usage_bytes)
            .unwrap_or(0))
    }

    pub fn origins_with_prefix_usage_bytes(
        &mut self,
        origin_prefix: &str,
    ) -> Result<u64, IndexedDbError> {
        let mut total = self
            .origins
            .iter()
            .filter(|(origin, _)| origin.starts_with(origin_prefix))
            .map(|(_, state)| origin_usage_bytes(state))
            .fold(0u64, |total, usage| total.saturating_add(usage));
        for (origin, state) in self.persisted_origins_with_prefix(origin_prefix)? {
            if !self.origins.contains_key(&origin) {
                total = total.saturating_add(origin_usage_bytes(&state));
            }
        }
        Ok(total)
    }

    pub(crate) fn committed_origin_usage_except_database(
        &self,
        origin: &str,
        db_name: &str,
    ) -> u64 {
        self.origins
            .get(origin)
            .map(|state| {
                sum_usage(
                    state
                        .databases
                        .iter()
                        .filter(|(name, _)| name.as_str() != db_name)
                        .map(|(name, database)| database_usage_bytes(name, database)),
                )
            })
            .unwrap_or(0)
    }

    pub fn close_database(&mut self, handle: DatabaseHandle) -> Result<(), IndexedDbError> {
        let database = self.databases.get_mut(&handle).ok_or_else(|| {
            IndexedDbError::InvalidState(format!("database handle {:?} is invalid", handle))
        })?;
        database.closed = true;
        self.connection_notifications.mark_close_pending(handle);
        self.finish_pending_database_close(handle);
        Ok(())
    }

    /// Realm teardown aborts every transaction owned by this connection before
    /// releasing it and waking requests in other event loops.
    pub fn force_close_database(&mut self, handle: DatabaseHandle) -> Result<(), IndexedDbError> {
        self.transactions
            .retain(|_, transaction| transaction.database != handle);
        self.databases.remove(&handle).ok_or_else(|| {
            IndexedDbError::InvalidState(format!("database handle {:?} is invalid", handle))
        })?;
        self.connection_notifications.unregister(handle);
        Ok(())
    }

    pub fn connection_notifications(&self) -> std::sync::Arc<crate::ConnectionNotifications> {
        self.connection_notifications.clone()
    }

    fn finish_pending_database_close(&mut self, handle: DatabaseHandle) {
        if self
            .databases
            .get(&handle)
            .is_some_and(|database| database.closed)
            && !self
                .transactions
                .values()
                .any(|transaction| transaction.database == handle)
        {
            self.databases.remove(&handle);
            self.connection_notifications.unregister(handle);
        }
    }

    pub fn database_version(
        &mut self,
        origin: &str,
        name: &str,
    ) -> Result<Option<u64>, IndexedDbError> {
        self.ensure_origin_loaded(origin)?;
        Ok(self
            .origins
            .get(origin)
            .and_then(|state| state.databases.get(name))
            .map(|database| database.version))
    }

    pub fn databases(
        &mut self,
        origin: &str,
    ) -> Result<Vec<DatabaseNameAndVersion>, IndexedDbError> {
        self.ensure_origin_loaded(origin)?;
        Ok(self
            .origins
            .get(origin)
            .map(|state| {
                state
                    .databases
                    .iter()
                    .map(|(name, database)| DatabaseNameAndVersion {
                        name: name.clone(),
                        version: database.version,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn database_info(&self, handle: DatabaseHandle) -> Result<DatabaseInfo, IndexedDbError> {
        let db = self.database_state(handle)?;
        if db.closed {
            return Err(IndexedDbError::InvalidState(
                "database handle is closed".to_owned(),
            ));
        }
        let current = self.database_data(&db.origin, &db.name)?;
        Ok(DatabaseInfo {
            name: db.name.clone(),
            version: current.version,
            object_store_names: current.stores.keys().cloned().collect(),
        })
    }

    pub fn object_store_info(
        &self,
        database: DatabaseHandle,
        store_name: &str,
    ) -> Result<ObjectStoreInfo, IndexedDbError> {
        let db = self.database_state(database)?;
        if db.closed {
            return Err(IndexedDbError::InvalidState(
                "database handle is closed".to_owned(),
            ));
        }
        let store = self
            .database_data(&db.origin, &db.name)?
            .stores
            .get(store_name)
            .ok_or_else(|| {
                IndexedDbError::NotFound(format!("object store `{store_name}` was not found"))
            })?;
        Ok(ObjectStoreInfo {
            name: store_name.to_owned(),
            key_path: store.key_path.clone(),
            auto_increment: store.auto_increment,
            index_names: store.indexes.keys().cloned().collect(),
        })
    }

    pub fn begin_transaction(
        &mut self,
        database: DatabaseHandle,
        store_names: &[String],
        mode: TransactionMode,
    ) -> Result<TransactionHandle, IndexedDbError> {
        if mode == TransactionMode::VersionChange {
            return Err(IndexedDbError::InvalidState(
                "versionchange transactions are created by open()".to_owned(),
            ));
        }

        let db = self.database_state(database)?.clone();
        if db.closed {
            return Err(IndexedDbError::InvalidState(
                "database handle is closed".to_owned(),
            ));
        }
        let current = self.database_data(&db.origin, &db.name)?.clone();
        let store_set = store_names.iter().cloned().collect::<BTreeSet<_>>();
        self.ensure_store_names_exist(&current, &store_set)?;

        if mode == TransactionMode::ReadWrite
            && self.transactions.values().any(|tx| {
                tx.state == TransactionLifecycle::Active
                    && tx.mode == TransactionMode::ReadWrite
                    && tx.origin == db.origin
                    && tx.db_name == db.name
            })
        {
            return Err(IndexedDbError::InvalidState(
                "concurrent readwrite transactions are not supported in the MVP backend".to_owned(),
            ));
        }

        let handle = self.allocate_transaction_handle();
        self.transactions.insert(
            handle,
            TransactionState {
                database,
                origin: db.origin,
                db_name: db.name,
                mode,
                stores: store_set,
                state: TransactionLifecycle::Active,
                working_copy: current,
            },
        );
        Ok(handle)
    }

    pub fn create_object_store(
        &mut self,
        transaction: TransactionHandle,
        name: &str,
        options: ObjectStoreOptions,
    ) -> Result<(), IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        if tx.mode != TransactionMode::VersionChange {
            return Err(IndexedDbError::InvalidState(
                "create_object_store requires a versionchange transaction".to_owned(),
            ));
        }
        if tx.working_copy.stores.contains_key(name) {
            return Err(IndexedDbError::Constraint(format!(
                "object store `{name}` already exists"
            )));
        }
        tx.working_copy.stores.insert(
            name.to_owned(),
            ObjectStoreData {
                key_path: options.key_path,
                auto_increment: options.auto_increment,
                auto_increment_counter: 0,
                indexes: BTreeMap::new(),
                records: BTreeMap::new(),
            },
        );
        tx.stores.insert(name.to_owned());
        Ok(())
    }

    pub fn delete_object_store(
        &mut self,
        transaction: TransactionHandle,
        name: &str,
    ) -> Result<(), IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        if tx.mode != TransactionMode::VersionChange {
            return Err(IndexedDbError::InvalidState(
                "delete_object_store requires a versionchange transaction".to_owned(),
            ));
        }
        if tx.working_copy.stores.remove(name).is_none() {
            return Err(IndexedDbError::NotFound(format!(
                "object store `{name}` was not found"
            )));
        }
        tx.stores.remove(name);
        Ok(())
    }

    pub fn create_index(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        index_name: &str,
        options: IndexOptions,
    ) -> Result<IndexInfo, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        if tx.mode != TransactionMode::VersionChange {
            return Err(IndexedDbError::InvalidState(
                "create_index requires a versionchange transaction".to_owned(),
            ));
        }
        let store = transaction_store_mut(tx, store_name)?;
        if store.indexes.contains_key(index_name) {
            return Err(IndexedDbError::Constraint(format!(
                "index `{index_name}` already exists"
            )));
        }
        if options.multi_entry && options.key_path.is_sequence() {
            return Err(IndexedDbError::InvalidState(
                "multiEntry indexes cannot use a sequence key_path".to_owned(),
            ));
        }
        let info = IndexInfo {
            name: index_name.to_owned(),
            key_path: options.key_path.clone(),
            unique: options.unique,
            multi_entry: options.multi_entry,
        };
        store.indexes.insert(
            index_name.to_owned(),
            IndexData {
                key_path: options.key_path,
                unique: options.unique,
                multi_entry: options.multi_entry,
            },
        );
        Ok(info)
    }

    pub fn delete_index(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        index_name: &str,
    ) -> Result<(), IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        if tx.mode != TransactionMode::VersionChange {
            return Err(IndexedDbError::InvalidState(
                "delete_index requires a versionchange transaction".to_owned(),
            ));
        }
        let store = transaction_store_mut(tx, store_name)?;
        if store.indexes.remove(index_name).is_none() {
            return Err(IndexedDbError::NotFound(format!(
                "index `{index_name}` was not found"
            )));
        }
        Ok(())
    }

    pub fn index_info(
        &self,
        database: DatabaseHandle,
        store_name: &str,
        index_name: &str,
    ) -> Result<IndexInfo, IndexedDbError> {
        let db = self.database_state(database)?;
        if db.closed {
            return Err(IndexedDbError::InvalidState(
                "database handle is closed".to_owned(),
            ));
        }
        let store = self
            .database_data(&db.origin, &db.name)?
            .stores
            .get(store_name)
            .ok_or_else(|| {
                IndexedDbError::NotFound(format!("object store `{store_name}` was not found"))
            })?;
        let index = store.indexes.get(index_name).ok_or_else(|| {
            IndexedDbError::NotFound(format!("index `{index_name}` was not found"))
        })?;
        Ok(IndexInfo {
            name: index_name.to_owned(),
            key_path: index.key_path.clone(),
            unique: index.unique,
            multi_entry: index.multi_entry,
        })
    }

    pub fn get(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: &Key,
    ) -> Result<RequestOutcome, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        let store = transaction_store(tx, store_name)?;
        Ok(RequestOutcome::Value(store.records.get(key).cloned()))
    }

    pub fn get_all(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<RequestOutcome, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        let store = transaction_store(tx, store_name)?;
        Ok(RequestOutcome::Values(
            store.records.values().cloned().collect(),
        ))
    }

    pub fn get_key(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: &Key,
    ) -> Result<RequestOutcome, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        let store = transaction_store(tx, store_name)?;
        Ok(RequestOutcome::Key(
            store.records.contains_key(key).then(|| key.clone()),
        ))
    }

    pub fn get_all_keys(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<RequestOutcome, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        let store = transaction_store(tx, store_name)?;
        Ok(RequestOutcome::Keys(
            store.records.keys().cloned().collect(),
        ))
    }

    pub fn count(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<RequestOutcome, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        let store = transaction_store(tx, store_name)?;
        Ok(RequestOutcome::Count(store.records.len() as u64))
    }

    pub fn entries(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<Vec<(Key, IndexedDbValue)>, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        let store = transaction_store(tx, store_name)?;
        Ok(store
            .records
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }

    pub fn generate_key(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<Key, IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        ensure_writeable(tx)?;
        let store = transaction_store_mut(tx, store_name)?;
        resolve_key(store, None)
    }

    pub fn put(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: Option<Key>,
        value: impl Into<IndexedDbValue>,
    ) -> Result<Key, IndexedDbError> {
        self.write_record(transaction, store_name, key, value.into(), false, None)
    }

    pub fn put_with_quota(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: Option<Key>,
        value: impl Into<IndexedDbValue>,
        quota: IndexedDbQuotaCheck,
    ) -> Result<Key, IndexedDbError> {
        self.write_record(
            transaction,
            store_name,
            key,
            value.into(),
            false,
            Some(quota),
        )
    }

    pub fn add(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: Option<Key>,
        value: impl Into<IndexedDbValue>,
    ) -> Result<Key, IndexedDbError> {
        self.write_record(transaction, store_name, key, value.into(), true, None)
    }

    pub fn add_with_quota(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: Option<Key>,
        value: impl Into<IndexedDbValue>,
        quota: IndexedDbQuotaCheck,
    ) -> Result<Key, IndexedDbError> {
        self.write_record(
            transaction,
            store_name,
            key,
            value.into(),
            true,
            Some(quota),
        )
    }

    fn write_record(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: Option<Key>,
        value: IndexedDbValue,
        add_only: bool,
        quota: Option<IndexedDbQuotaCheck>,
    ) -> Result<Key, IndexedDbError> {
        let (origin, db_name, working_copy_usage, resolved_key, previous_store) = {
            let tx = self.active_transaction_mut(transaction)?;
            ensure_writeable(tx)?;
            let db_name = tx.db_name.clone();
            let origin = tx.origin.clone();
            let store = transaction_store_mut(tx, store_name)?;
            let previous_store = quota.map(|_| store.clone());
            let resolved_key = resolve_key(store, key)?;
            if add_only && store.records.contains_key(&resolved_key) {
                return Err(IndexedDbError::Constraint(
                    "record already exists for key".to_owned(),
                ));
            }
            store.records.insert(resolved_key.clone(), value);
            let working_copy_usage = database_usage_bytes(&db_name, &tx.working_copy);
            (
                origin,
                db_name,
                working_copy_usage,
                resolved_key,
                previous_store,
            )
        };

        if let Some(quota) = quota {
            let requested = quota
                .non_indexed_db_usage
                .saturating_add(self.committed_origin_usage_except_database(&origin, &db_name))
                .saturating_add(working_copy_usage);
            if requested > quota.quota {
                if let Some(previous_store) = previous_store {
                    let tx = self.active_transaction_mut(transaction)?;
                    if let Some(store) = tx.working_copy.stores.get_mut(store_name) {
                        *store = previous_store;
                    }
                }
                return Err(IndexedDbError::QuotaExceeded {
                    quota: quota.quota,
                    requested,
                });
            }
        }

        Ok(resolved_key)
    }

    pub fn delete(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: &Key,
    ) -> Result<(), IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        ensure_writeable(tx)?;
        let store = transaction_store_mut(tx, store_name)?;
        store.records.remove(key);
        Ok(())
    }

    pub fn clear(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<(), IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        ensure_writeable(tx)?;
        let store = transaction_store_mut(tx, store_name)?;
        store.records.clear();
        Ok(())
    }

    pub fn commit_transaction(
        &mut self,
        transaction: TransactionHandle,
    ) -> Result<(), IndexedDbError> {
        let (database, origin, db_name, working_copy) = {
            let tx = self.active_transaction_mut(transaction)?;
            tx.state = TransactionLifecycle::Committed;
            (
                tx.database,
                tx.origin.clone(),
                tx.db_name.clone(),
                tx.working_copy.clone(),
            )
        };
        let origin_state = self.origins.get_mut(&origin).ok_or_else(|| {
            IndexedDbError::NotFound(format!("origin `{origin}` was not found during commit"))
        })?;
        origin_state.databases.insert(db_name, working_copy);
        self.transactions.remove(&transaction);
        let result = self.persist_origin(&origin);
        self.finish_pending_database_close(database);
        result
    }

    pub fn commit_transaction_with_quota(
        &mut self,
        transaction: TransactionHandle,
        quota: IndexedDbQuotaCheck,
    ) -> Result<(), IndexedDbError> {
        let (database, origin, db_name, working_copy_usage) = {
            let tx = self.active_transaction_mut(transaction)?;
            (
                tx.database,
                tx.origin.clone(),
                tx.db_name.clone(),
                database_usage_bytes(&tx.db_name, &tx.working_copy),
            )
        };
        let requested = quota
            .non_indexed_db_usage
            .saturating_add(self.committed_origin_usage_except_database(&origin, &db_name))
            .saturating_add(working_copy_usage);
        if requested > quota.quota {
            self.transactions.remove(&transaction);
            self.finish_pending_database_close(database);
            return Err(IndexedDbError::QuotaExceeded {
                quota: quota.quota,
                requested,
            });
        }
        self.commit_transaction(transaction)
    }

    pub fn abort_transaction(
        &mut self,
        transaction: TransactionHandle,
    ) -> Result<(), IndexedDbError> {
        let tx = self.active_transaction_mut(transaction)?;
        tx.state = TransactionLifecycle::Aborted;
        let database = tx.database;
        self.transactions.remove(&transaction);
        self.finish_pending_database_close(database);
        Ok(())
    }

    fn allocate_database_handle(&mut self, origin: String, name: String) -> DatabaseHandle {
        let handle =
            DatabaseHandle::from_raw(self.next_database_handle.fetch_add(1, Ordering::Relaxed));
        self.databases.insert(
            handle,
            DatabaseHandleState {
                origin,
                name,
                closed: false,
            },
        );
        handle
    }

    fn allocate_upgrade_transaction(
        &mut self,
        database: DatabaseHandle,
        origin: &str,
        db_name: &str,
        working_copy: DatabaseData,
    ) -> TransactionHandle {
        let stores = working_copy.stores.keys().cloned().collect::<BTreeSet<_>>();
        let handle = self.allocate_transaction_handle();
        self.transactions.insert(
            handle,
            TransactionState {
                database,
                origin: origin.to_owned(),
                db_name: db_name.to_owned(),
                mode: TransactionMode::VersionChange,
                stores,
                state: TransactionLifecycle::Active,
                working_copy,
            },
        );
        handle
    }

    fn allocate_transaction_handle(&self) -> TransactionHandle {
        TransactionHandle::from_raw(self.next_transaction_handle.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn database_state(
        &self,
        handle: DatabaseHandle,
    ) -> Result<&DatabaseHandleState, IndexedDbError> {
        self.databases.get(&handle).ok_or_else(|| {
            IndexedDbError::InvalidState(format!("database handle {:?} is invalid", handle))
        })
    }

    pub(crate) fn database_data(
        &self,
        origin: &str,
        name: &str,
    ) -> Result<&DatabaseData, IndexedDbError> {
        self.origins
            .get(origin)
            .and_then(|state| state.databases.get(name))
            .ok_or_else(|| {
                IndexedDbError::NotFound(format!(
                    "database `{name}` for origin `{origin}` was not found"
                ))
            })
    }

    pub(crate) fn active_transaction_mut(
        &mut self,
        handle: TransactionHandle,
    ) -> Result<&mut TransactionState, IndexedDbError> {
        let tx = self.transactions.get_mut(&handle).ok_or_else(|| {
            IndexedDbError::TransactionInactive(format!(
                "transaction handle {:?} is invalid",
                handle
            ))
        })?;
        if tx.state != TransactionLifecycle::Active {
            return Err(IndexedDbError::TransactionInactive(
                "transaction is not active".to_owned(),
            ));
        }
        Ok(tx)
    }

    pub(crate) fn ensure_store_names_exist(
        &self,
        database: &DatabaseData,
        store_names: &BTreeSet<String>,
    ) -> Result<(), IndexedDbError> {
        for store_name in store_names {
            if !database.stores.contains_key(store_name) {
                return Err(IndexedDbError::NotFound(format!(
                    "object store `{store_name}` was not found"
                )));
            }
        }
        Ok(())
    }
}
