use std::sync::atomic::{AtomicI64, Ordering};

use super::*;

macro_rules! schema_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(i64);

        impl $name {
            pub const fn from_raw(value: i64) -> Self {
                assert!(value > 0, "schema IDs must be positive");
                Self(value)
            }

            pub const fn into_raw(self) -> i64 {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl rusqlite::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                self.0.to_sql()
            }
        }
    };
}

schema_id!(StoreId);
schema_id!(IndexId);

/// Shared by all clients of one logical database, including idle reconnects.
/// Reservations survive transaction rollback; names never participate in IDs.
#[derive(Default)]
pub(super) struct SchemaIds {
    next_store: AtomicI64,
    next_index: AtomicI64,
}

impl SchemaIds {
    pub(super) fn observe(&self, connection: &Connection) -> Result<(), IndexedDbError> {
        let (store, index, last_store, last_index): (i64, i64, i64, i64) = connection.query_row(
            "SELECT next_store_id, next_index_id, (SELECT coalesce(max(store_id), 0) FROM object_stores), (SELECT coalesce(max(index_id), 0) FROM indexes) FROM database_metadata WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if store <= 0 || index <= 0 || store <= last_store || index <= last_index {
            return Err(corruption("invalid schema ID counters"));
        }
        self.next_store.fetch_max(store, Ordering::Relaxed);
        self.next_index.fetch_max(index, Ordering::Relaxed);
        Ok(())
    }

    fn reserve(counter: &AtomicI64) -> Result<i64, IndexedDbError> {
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                if value > 0 {
                    value.checked_add(1)
                } else {
                    None
                }
            })
            .map_err(|_| IndexedDbError::Resource("schema IDs unavailable or exhausted".into()))
    }

    pub(super) fn store(&self) -> Result<StoreId, IndexedDbError> {
        Self::reserve(&self.next_store).map(StoreId)
    }

    pub(super) fn index(&self) -> Result<IndexId, IndexedDbError> {
        Self::reserve(&self.next_index).map(IndexId)
    }
}

impl Database {
    pub fn allocate_store_id(&self) -> Result<StoreId, IndexedDbError> {
        self.schema_ids.store()
    }

    pub fn allocate_index_id(&self) -> Result<IndexId, IndexedDbError> {
        self.schema_ids.index()
    }

    pub fn lookup_store_id(
        &self,
        name: impl Into<crate::IndexedDbName>,
    ) -> Result<StoreId, IndexedDbError> {
        let name = name.into();
        self.connection
            .query_row(
                "SELECT store_id FROM object_stores WHERE name=?1",
                [names::encode(&name)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(StoreId::from_raw)
            .ok_or_else(|| IndexedDbError::NotFound(format!("object store `{name}` was not found")))
    }

    pub fn lookup_index_id(
        &self,
        store: StoreId,
        name: impl Into<crate::IndexedDbName>,
    ) -> Result<IndexId, IndexedDbError> {
        let name = name.into();
        self.connection
            .query_row(
                "SELECT index_id FROM indexes WHERE store_id=?1 AND name=?2",
                rusqlite::params![store, names::encode(&name)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(IndexId::from_raw)
            .ok_or_else(|| IndexedDbError::NotFound(format!("index `{name}` was not found")))
    }

    pub(super) fn validate_store(
        &self,
        handle: TransactionHandle,
        store: StoreId,
    ) -> Result<StoreId, IndexedDbError> {
        let transaction = self.transaction(handle)?;
        if transaction.mode != TransactionMode::VersionChange
            && !transaction.stores.contains(&store)
        {
            return Err(IndexedDbError::InvalidState(format!(
                "transaction does not cover object store {store}"
            )));
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM object_stores WHERE store_id=?1)",
            [store],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(IndexedDbError::NotFound(format!(
                "object store {store} was not found"
            )));
        }
        Ok(store)
    }

    pub(super) fn persist_schema_id(
        &self,
        column: &'static str,
        id: i64,
    ) -> Result<(), IndexedDbError> {
        let next = id
            .checked_add(1)
            .ok_or_else(|| corruption("schema ID exhausted"))?;
        self.connection.execute(
            &format!("UPDATE database_metadata SET {column}=max({column}, ?1) WHERE singleton=1"),
            [next],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_id_reservations_reject_uninitialized_and_exhausted_counters() {
        let ids = SchemaIds::default();
        assert!(ids.store().is_err());
        assert!(ids.index().is_err());
        ids.next_store.store(i64::MAX - 1, Ordering::Relaxed);
        ids.next_index.store(i64::MAX - 1, Ordering::Relaxed);
        assert_eq!(ids.store().unwrap().into_raw(), i64::MAX - 1);
        assert_eq!(ids.index().unwrap().into_raw(), i64::MAX - 1);
        assert!(ids.store().is_err());
        assert!(ids.index().is_err());
        assert_eq!(ids.next_store.load(Ordering::Relaxed), i64::MAX);
        assert_eq!(ids.next_index.load(Ordering::Relaxed), i64::MAX);
    }
}
