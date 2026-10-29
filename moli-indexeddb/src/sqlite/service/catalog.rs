//! The logical namespace uses the service's existing registration map. There
//! is no separately updated catalog database and no legacy JSON discovery.

use super::*;
use std::sync::mpsc;
mod layout;
#[cfg(test)]
pub(super) use layout::Boundary;
pub(super) use layout::DiskCatalog;
use layout::io_error;

pub struct CatalogSnapshot {
    databases: Vec<Arc<DatabaseMetadata>>,
    _reservation: MetadataReservation,
}

impl std::ops::Deref for CatalogSnapshot {
    type Target = [Arc<DatabaseMetadata>];
    fn deref(&self) -> &Self::Target {
        &self.databases
    }
}

impl Registry {
    pub(super) fn reap_closed(&mut self) {
        self.entries.retain(|_, client| {
            !client.state.closed.load(Ordering::Acquire)
                || client.state.failure.lock().is_some()
                || (client.catalog.is_some() && !client.state.deleted.load(Ordering::Acquire))
        });
    }
}

impl Service {
    /// Startup only: the caller holds the profile lock and has not opened a
    /// V8 realm. Filesystem recovery can block here; every SQLite connection,
    /// including header inspection, still opens and closes on a SQL worker.
    pub fn on_disk(root: PathBuf, config: ServiceConfig) -> Result<Self, IndexedDbError> {
        let mut service =
            Self::new(config).map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        let catalog = Arc::new(DiskCatalog::open(root)?);
        service.client.catalog = Some(catalog.clone());
        catalog.visit(|path, preparing| {
            let Some(header) = service.inspect(path, preparing)? else {
                return Ok(());
            };
            catalog.verify(&header.identity, &header.path)?;
            let Header {
                identity,
                path,
                _reservation: reservation,
            } = header;
            let mut registry = service.registry.lock();
            if registry
                .entries
                .keys()
                .any(|(owner, name)| owner == &identity.owner_key && name == &identity.name)
            {
                return Err(corruption("multiple generations for one IndexedDB name"));
            }
            let (sender, receiver) = mpsc::channel();
            service
                .register_client(
                    &mut registry,
                    (identity.owner_key, identity.name),
                    identity.generation,
                    Location::Disk {
                        path,
                        mode: OpenMode::Existing,
                    },
                    Some(catalog.clone()),
                    move |reply| {
                        let _ = sender.send(reply);
                    },
                )
                .map_err(dispatch_error)?;
            drop(registry);
            drop(reservation);
            receiver
                .recv()
                .map_err(|_| corruption("IndexedDB recovery worker stopped"))?
                .result?;
            Ok(())
        })?;
        catalog.cleanup_trash();
        Ok(service)
    }
}

impl ServiceClient {
    /// Resolve a logical name without requiring callers to choose a file or a
    /// generation. The callback is native-only and runs on the database worker.
    pub fn resolve(
        &self,
        owner_key: Vec<u8>,
        name: String,
        completion: impl FnOnce(Completion) + Send + 'static,
    ) -> Result<DatabaseClient, DispatchError> {
        if owner_key.is_empty() {
            return Err(DispatchError::Storage(IndexedDbError::Data(
                "empty IndexedDB owner identity".into(),
            )));
        }
        let mut registry = self.registry.lock();
        if registry.closed {
            return Err(DispatchError::Closed);
        }
        registry.reap_closed();
        let key = (owner_key, name);
        registry
            .check_owner(&key.0)
            .map_err(DispatchError::Storage)?;
        if let Some(client) = registry.entries.get(&key) {
            if let Some(error) = client.state.failure.lock().clone() {
                return Err(DispatchError::Storage(error));
            }
            if client.state.deleting.load(Ordering::Acquire) {
                return Err(DispatchError::Closed);
            }
            if !client.is_closed() {
                let client = client.clone();
                client.submit_metadata(completion)?;
                return Ok(client);
            }
            // Closing a disk owner is not deletion. Reopen the known file;
            // never manufacture a replacement for a missing generation.
            let generation = client.identity.generation;
            let location = match &client.location {
                Location::Disk { path, .. } => Location::Disk {
                    path: path.clone(),
                    mode: OpenMode::Existing,
                },
                Location::Memory => unreachable!("closed memory registrations were reaped"),
            };
            return self.register_client(
                &mut registry,
                key,
                generation,
                location,
                self.catalog.clone(),
                completion,
            );
        }
        let mut generation = [0; 16];
        moli_crypto::fill_secure_random(&mut generation)
            .map_err(|error| DispatchError::Storage(IndexedDbError::Io(error.to_string())))?;
        let location = self
            .catalog
            .as_ref()
            .map_or(Location::Memory, |catalog| Location::Disk {
                path: catalog.path(&key.0, &key.1, generation),
                mode: OpenMode::Create,
            });
        self.register_client(
            &mut registry,
            key,
            generation,
            location,
            self.catalog.clone(),
            completion,
        )
    }

    /// Only committed, live databases are visible. Arc snapshots retain their
    /// own schema budget; this lease covers the result vector without copying
    /// names or definitions. No filesystem or SQL access occurs here.
    pub fn databases(&self, owner_key: &[u8]) -> Result<CatalogSnapshot, IndexedDbError> {
        let registry = self.registry.lock();
        if registry.closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        registry.check_owner(owner_key)?;
        let mut reservation = self.metadata.reservation();
        let mut databases = Vec::new();
        for ((owner, _), client) in &registry.entries {
            if owner != owner_key {
                continue;
            }
            let Some(metadata) = client.catalog_metadata()? else {
                continue;
            };
            if databases.len() == databases.capacity() {
                let capacity = databases.len().max(1).checked_mul(2).ok_or_else(|| {
                    IndexedDbError::Resource("IndexedDB catalog size overflow".into())
                })?;
                reservation.try_resize(
                    capacity
                        .checked_mul(std::mem::size_of::<Arc<DatabaseMetadata>>())
                        .ok_or_else(|| {
                            IndexedDbError::Resource("IndexedDB catalog size overflow".into())
                        })?,
                )?;
                databases.reserve_exact(capacity - databases.len());
            }
            databases.push(metadata);
        }
        databases.sort_unstable_by(|left, right| {
            left.name.encode_utf16().cmp(right.name.encode_utf16())
        });
        Ok(CatalogSnapshot {
            databases,
            _reservation: reservation,
        })
    }

    pub fn database_version(
        &self,
        owner_key: &[u8],
        name: &str,
    ) -> Result<Option<u64>, IndexedDbError> {
        let registry = self.registry.lock();
        if registry.closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        registry.check_owner(owner_key)?;
        let client = registry
            .entries
            .iter()
            .find_map(|((owner, stored), client)| {
                (owner == owner_key && stored == name).then_some(client)
            });
        let Some(client) = client else {
            return Ok(None);
        };
        Ok(client.catalog_metadata()?.map(|metadata| metadata.version))
    }

    /// The shared committed view used by bucket quota coordination. Creating
    /// and durably deleted generations contribute no usage; storage failures
    /// remain errors rather than looking like an empty owner.
    pub fn owner_usage_bytes(&self, owner_key: &[u8]) -> Result<u64, IndexedDbError> {
        let registry = self.registry.lock();
        if registry.closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        registry.check_owner(owner_key)?;
        let mut usage = 0_u64;
        for ((owner, _), client) in &registry.entries {
            if owner == owner_key
                && let Some(metadata) = client.catalog_metadata()?
            {
                usage = usage
                    .checked_add(metadata.usage)
                    .ok_or_else(|| corruption("IndexedDB aggregate usage overflow"))?;
            }
        }
        Ok(usage)
    }

    pub fn owners_with_prefix_usage_bytes(&self, prefix: &[u8]) -> Result<u64, IndexedDbError> {
        let registry = self.registry.lock();
        if registry.closed {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB service is closed".into(),
            ));
        }
        if registry
            .clearing
            .keys()
            .any(|scope| scope.overlaps_prefix(prefix))
        {
            return Err(IndexedDbError::InvalidState(
                "IndexedDB owner is being cleared".into(),
            ));
        }
        let mut total = 0_u64;
        for ((owner, _), client) in &registry.entries {
            if owner.starts_with(prefix)
                && let Some(metadata) = client.catalog_metadata()?
            {
                total = total
                    .checked_add(metadata.usage)
                    .ok_or_else(|| IndexedDbError::Corruption("IndexedDB usage overflow".into()))?;
            }
        }
        Ok(total)
    }
}

impl Service {
    fn inspect(&self, path: PathBuf, preparing: bool) -> Result<Option<Header>, IndexedDbError> {
        let permit = self
            .executor
            .handle()
            .try_reserve(TaskKind::Data, path.capacity().saturating_add(1024))
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        let options = self.config.connection;
        let budget = self.metadata.clone();
        let catalog = self.catalog.clone().expect("disk catalog");
        let (sender, receiver) = mpsc::channel();
        permit
            .submit(0, move |permit| {
                let result = inspect_header(path, preparing, options, &budget, &catalog);
                // Closing the probe precedes normal registration of this file.
                drop(permit);
                let _ = sender.send(result);
            })
            .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
        receiver
            .recv()
            .map_err(|_| corruption("IndexedDB inspection worker stopped"))?
    }
}

impl DatabaseClient {
    fn catalog_metadata(&self) -> Result<Option<Arc<DatabaseMetadata>>, IndexedDbError> {
        if self.state.deleted.load(Ordering::Acquire) {
            return Ok(None);
        }
        if let Some(error) = self.state.failure.lock().clone() {
            return Err(error);
        }
        if self.is_closed()
            && self.catalog.is_none()
            && !self.state.deleting.load(Ordering::Acquire)
        {
            return Ok(None);
        }
        Ok(self.committed().filter(|metadata| metadata.version != 0))
    }
}

fn dispatch_error(error: DispatchError) -> IndexedDbError {
    match error {
        DispatchError::Storage(error) => error,
        error => IndexedDbError::Resource(error.to_string()),
    }
}

struct Header {
    identity: DatabaseIdentity,
    path: PathBuf,
    _reservation: MetadataReservation,
}

fn inspect_header(
    path: PathBuf,
    preparing: bool,
    options: ConnectionOptions,
    budget: &MetadataBudget,
    catalog: &DiskCatalog,
) -> Result<Option<Header>, IndexedDbError> {
    if preparing
        && !path
            .try_exists()
            .map_err(|error| io_error("inspect generation", &path, error))?
    {
        catalog.discard_preparing(&path)?;
        return Ok(None);
    }
    let connection = moli_sqlite::open_disk(&path, OpenMode::Existing, options)
        .map_err(|error| IndexedDbError::Io(error.to_string()))?;
    if preparing {
        let has_metadata: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='database_metadata' AND type='table')", [], |row| row.get(0))?;
        if has_metadata {
            let version: i64 = connection.query_row(
                "SELECT version FROM database_metadata WHERE singleton=1",
                [],
                |row| row.get(0),
            )?;
            if version != 0 {
                return Err(corruption("published IndexedDB has a preparation marker"));
            }
        }
        if !has_metadata {
            connection
                .close()
                .map_err(|(_, error)| IndexedDbError::from(error))?;
            catalog.discard_preparing(&path)?;
            return Ok(None);
        }
    }
    moli_sqlite::require_schema_version(&connection, SCHEMA_VERSION)
        .map_err(|error| corruption(&error.to_string()))?;
    let mut reservation = budget.reservation();
    let mut statement = connection.prepare("SELECT i.owner_key, m.name, m.generation FROM database_info i JOIN database_metadata m ON m.singleton=i.singleton WHERE i.singleton=1 AND i.backend='indexeddb' AND i.identity_format=1")?;
    let mut rows = statement.query([])?;
    let row = rows
        .next()?
        .ok_or_else(|| corruption("missing IndexedDB identity"))?;
    let owner = row
        .get_ref(0)?
        .as_blob()
        .map_err(|_| corruption("invalid IndexedDB owner"))?;
    let name = row
        .get_ref(1)?
        .as_str()
        .map_err(|_| corruption("invalid IndexedDB name"))?;
    let generation: [u8; 16] = row
        .get_ref(2)?
        .as_blob()
        .map_err(|_| corruption("invalid IndexedDB generation"))?
        .try_into()
        .map_err(|_| corruption("invalid IndexedDB generation length"))?;
    reservation.try_resize(
        std::mem::size_of::<Header>()
            .saturating_add(path.capacity())
            .saturating_add(owner.len())
            .saturating_add(name.len()),
    )?;
    let identity = DatabaseIdentity {
        owner_key: owner.to_vec(),
        name: name.to_owned(),
        generation,
    };
    drop(rows);
    drop(statement);
    connection
        .close()
        .map_err(|(_, error)| IndexedDbError::from(error))?;
    if preparing {
        catalog.verify(&identity, &path)?;
        catalog.discard_preparing(&path)?;
        return Ok(None);
    }
    Ok(Some(Header {
        identity,
        path,
        _reservation: reservation,
    }))
}
