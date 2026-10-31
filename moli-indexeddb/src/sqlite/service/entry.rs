//! Operations on one database, independent of worker routing.

use super::*;

impl Entry {
    pub(super) fn metadata(
        &mut self,
        config: ServiceConfig,
    ) -> Result<CommandResult, IndexedDbError> {
        self.check_live()?;
        self.idle_since = None;
        if self.initialized
            && let Some(metadata) = self.state.metadata.read().as_ref()
        {
            return Ok(CommandResult::Metadata(metadata.clone()));
        }
        self.connect(config)?;
        Ok(CommandResult::Metadata(self.publish()))
    }

    fn check_live(&self) -> Result<(), IndexedDbError> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB database is retired".into(),
            ));
        }
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        Ok(())
    }

    pub(super) fn connect(&mut self, config: ServiceConfig) -> Result<(), IndexedDbError> {
        self.check_live()?;
        self.idle_since = None;
        let entry = &*self;
        if entry.database.is_some() {
            return Ok(());
        }
        let entry = self;
        let result = (|| {
            let (connection, create) = match &entry.location {
                Location::Memory if !entry.initialized => {
                    (moli_sqlite::open_private_memory(config.connection), true)
                }
                Location::Memory => {
                    return Err(IndexedDbError::InvalidState(
                        "private memory database cannot be recreated".into(),
                    ));
                }
                Location::Disk { path, mode } => {
                    if let Some(catalog) = &entry.catalog
                        && !entry.prepared
                    {
                        catalog.prepare(path, &entry.identity)?;
                        entry.prepared = true;
                    }
                    let mode = if entry.initialized {
                        OpenMode::Existing
                    } else {
                        *mode
                    };
                    (
                        moli_sqlite::open_disk(path, mode, config.connection),
                        matches!(mode, OpenMode::Create),
                    )
                }
            };
            let mut connection =
                connection.map_err(|error| IndexedDbError::Io(error.to_string()))?;
            if create {
                Database::initialize(&mut connection, &entry.identity)?;
                // Loading schema can fail after the creating schema was
                // committed. A retry must open that disk generation, not try
                // to create a second schema over it. Unpublished private memory
                // is disposable until its first successful open.
                if matches!(entry.location, Location::Disk { .. }) {
                    entry.initialized = true;
                }
            }
            // SQL initialization can succeed before filesystem publication
            // fails. Resume that publication without recreating the schema.
            if entry.state.metadata.read().is_none()
                && let (Some(catalog), Location::Disk { path, .. }) =
                    (&entry.catalog, &entry.location)
            {
                catalog.finish_preparation(path, &connection)?;
            }
            Database::open_shared(connection, entry.identity.clone())
        })();
        match result {
            Ok(mut database) => {
                if let Some(previous) = entry.state.metadata.read().as_ref() {
                    if previous.as_ref() != database.committed().as_ref() {
                        let error = corruption("IndexedDB changed outside its database owner");
                        entry.failure = Some(error.clone());
                        *entry.state.failure.lock() = Some(error.clone());
                        return Err(error);
                    }
                    // Reopening a file does not change its published schema.
                    database.committed = previous.clone();
                }
                database.blobs = Some(entry.blobs.clone());
                entry.database = Some(database);
                entry.initialized = true;
                entry.publish();
                Ok(())
            }
            Err(error) => {
                // A missing known generation is data loss, not temporary FD
                // pressure. Keep it terminal and never replace it with a new DB.
                let missing_existing = match &entry.location {
                    Location::Disk { path, mode }
                        if entry.initialized || matches!(mode, OpenMode::Existing) =>
                    {
                        std::fs::symlink_metadata(path)
                            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                    }
                    _ => false,
                };
                if missing_existing
                    || !matches!(error, IndexedDbError::Resource(_) | IndexedDbError::Io(_))
                {
                    entry.failure = Some(error.clone());
                    *entry.state.failure.lock() = Some(error.clone());
                }
                Err(error)
            }
        }
    }

    pub(super) fn run_command(
        &mut self,
        id: DatabaseId,
        command: Command,
        config: ServiceConfig,
        connection: Option<crate::DatabaseHandle>,
        preflight: Result<(), IndexedDbError>,
    ) -> Result<CommandResult, IndexedDbError> {
        self.connect(config)?;
        let entry = self;
        let database = entry.database.as_mut().expect("connected owner");
        if let Err(error) = preflight {
            match command {
                Command::Write { transaction, .. } => {
                    database.ready(transaction)?;
                }
                Command::FinishWrite {
                    transaction,
                    request,
                    ..
                } => {
                    database.cancel_write(transaction, request)?;
                }
                Command::Commit { transaction, .. } => {
                    database.abort(transaction)?;
                }
                _ => {
                    return Err(IndexedDbError::InvalidState(
                        "unsupported SQL preflight rejection".into(),
                    ));
                }
            }
            return Err(error);
        }
        match command {
            Command::Begin {
                transaction,
                mode,
                durability,
                stores,
                version,
                admission,
            } => {
                if let Some(connection) = connection
                    && !entry.state.connections.lock().contains_key(&connection)
                {
                    return Err(IndexedDbError::InvalidState(
                        "SQL connection is closed".into(),
                    ));
                }
                if mode != TransactionMode::VersionChange {
                    let key = queue_key(id);
                    let Some(request) = admission.as_ref() else {
                        return Err(IndexedDbError::InvalidState(
                            "regular SQL transactions require admission".into(),
                        ));
                    };
                    if !request.belongs_to(&entry.admissions)
                        || !request.matches_key(&key.0, &key.1)
                        || request.mode() != mode
                        || request
                            .store_names()
                            .into_iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            != stores
                        || !request.claim_start()
                    {
                        return Err(IndexedDbError::InvalidState(
                            "invalid or unready transaction admission".into(),
                        ));
                    }
                } else if admission.is_some() {
                    return Err(IndexedDbError::InvalidState(
                        "upgrade transaction cannot reuse regular admission".into(),
                    ));
                }
                match database.begin_with_durability(transaction, mode, stores, version, durability)
                {
                    Ok(()) => {
                        entry.transactions.insert(
                            transaction,
                            StartedTransaction {
                                connection: connection.or_else(|| {
                                    admission.as_ref().map(TransactionRequestHandle::database)
                                }),
                                admission,
                            },
                        );
                        Ok(CommandResult::Unit)
                    }
                    Err(error) => {
                        if let Some(admission) = admission {
                            admission.finish();
                        }
                        Err(error)
                    }
                }
            }
            Command::CreateStore {
                transaction,
                name,
                options,
            } => database
                .create_object_store(transaction, &name, options)
                .map(|_| CommandResult::Unit),
            Command::DeleteStore { transaction, name } => database
                .delete_object_store(transaction, &name)
                .map(|_| CommandResult::Unit),
            Command::RenameStore {
                transaction,
                name,
                new_name,
            } => database
                .rename_object_store(transaction, &name, &new_name)
                .map(|_| CommandResult::Unit),
            Command::CreateIndex {
                transaction,
                store,
                name,
                options,
            } => database
                .create_index(transaction, &store, &name, options)
                .map(|_| CommandResult::Unit),
            Command::DeleteIndex {
                transaction,
                store,
                name,
            } => database
                .delete_index(transaction, &store, &name)
                .map(|_| CommandResult::Unit),
            Command::RenameIndex {
                transaction,
                store,
                name,
                new_name,
            } => database
                .rename_index(transaction, &store, &name, &new_name)
                .map(|_| CommandResult::Unit),
            Command::PrepareWrite {
                transaction,
                store,
                add_only,
            } => database
                .prepare_write(transaction, &store, None, add_only)
                .map(CommandResult::Prepared),
            Command::Write {
                transaction,
                store,
                key,
                value,
                indexes,
                add_only,
                quota,
            } => database
                .write(
                    transaction,
                    &store,
                    RecordWrite {
                        key,
                        value,
                        indexes,
                        add_only,
                        quota,
                    },
                )
                .map(CommandResult::Key),
            Command::FinishWrite {
                transaction,
                request,
                value,
                indexes,
                quota,
            } => database
                .finish_write(transaction, request, value, indexes, quota)
                .map(CommandResult::Key),
            Command::CancelWrite {
                transaction,
                request,
            } => database
                .cancel_write(transaction, request)
                .map(|_| CommandResult::Unit),
            Command::BackfillIndex {
                transaction,
                store,
                name,
                primary_key,
                keys,
            } => database
                .backfill_index_record(transaction, &store, &name, &primary_key, keys)
                .map(|_| CommandResult::Unit),
            Command::Query {
                transaction,
                store,
                query,
            } => database
                .query(transaction, &store, &query)
                .map(CommandResult::Records),
            Command::QueryPage {
                transaction,
                store,
                query,
            } => database
                .query_page(transaction, &store, &query)
                .map(CommandResult::RecordPage),
            Command::Count {
                transaction,
                store,
                source,
                range,
            } => database
                .query_count(transaction, &store, &source, &range)
                .map(CommandResult::Count),
            Command::Delete {
                transaction,
                store,
                range,
            } => database
                .delete_range(transaction, &store, &range)
                .map(|_| CommandResult::Unit),
            Command::Commit { transaction, quota } => {
                let result = database
                    .commit_with_quota(transaction, quota)
                    .map(|_| CommandResult::Unit);
                if result.is_err() && database.is_transaction_active() {
                    let _ = database.abort(transaction);
                }
                result
            }
            Command::Abort { transaction } => {
                database.abort(transaction).map(|_| CommandResult::Unit)
            }
            Command::Close | Command::DeleteDatabase => {
                unreachable!("close handled without reopening")
            }
        }
    }
}
