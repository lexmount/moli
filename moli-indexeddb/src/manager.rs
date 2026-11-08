use crate::{
    IndexedDbError,
    sqlite::service::{Service, ServiceClient},
};

/// Owns the partition's SQLite workers. Request execution belongs to their
/// database clients; this handle exposes only committed usage and shutdown.
pub struct IndexedDbManager {
    service: Service,
}

impl IndexedDbManager {
    pub fn with_sqlite_service(service: Service) -> Self {
        Self { service }
    }

    pub fn sqlite_service(&self) -> ServiceClient {
        self.service.client()
    }

    pub fn shutdown_sqlite(&mut self) -> Result<(), IndexedDbError> {
        self.service.shutdown()
    }

    pub fn origin_usage_bytes(&self, origin: &str) -> Result<u64, IndexedDbError> {
        self.service.owner_usage_bytes(origin.as_bytes())
    }
}
