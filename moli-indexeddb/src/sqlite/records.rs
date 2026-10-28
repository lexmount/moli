use super::*;
use crate::{
    IndexedDbValue,
    transaction::MAX_AUTO_INCREMENT_KEY,
    usage::{indexed_db_value_usage_bytes, key_usage_bytes},
};

impl Database {
    pub fn prepare_write(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: Option<Key>,
        add_only: bool,
    ) -> Result<PreparedWrite, IndexedDbError> {
        self.writable(transaction)?;
        let store_id = self.store_id(transaction, store_name)?;
        let id = self.next_request;
        self.next_request = id
            .checked_add(1)
            .ok_or_else(|| IndexedDbError::InvalidState("write request token exhausted".into()))?;
        // The SAVEPOINT includes both generated and explicit-key high-water updates.
        self.open_request()?;
        let outcome = (|| {
            let (auto_increment, generator): (bool, i64) = self.connection.query_row(
                "SELECT auto_increment, generator FROM object_stores WHERE store_id=?1",
                [store_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let (key, next_generator) = match key {
                Some(key) => {
                    key.encode_ordered()?;
                    let mut next = generator as u64;
                    if auto_increment && let Key::Number(number) = &key {
                        next = next.max(
                            number
                                .value()
                                .floor()
                                .clamp(0.0, MAX_AUTO_INCREMENT_KEY as f64)
                                as u64,
                        );
                    }
                    (key, next)
                }
                None if auto_increment
                    && generator >= 0
                    && (generator as u64) < MAX_AUTO_INCREMENT_KEY =>
                {
                    let next = generator as u64 + 1;
                    (Key::from(next as i64), next)
                }
                None if auto_increment => {
                    return Err(IndexedDbError::Constraint(
                        "auto_increment key generator exceeded 2^53".into(),
                    ));
                }
                None => {
                    return Err(IndexedDbError::InvalidState(
                        "a key is required when auto_increment is disabled".into(),
                    ));
                }
            };
            if auto_increment {
                self.connection.execute(
                    "UPDATE object_stores SET generator=?1 WHERE store_id=?2",
                    [sql_integer(next_generator)?, store_id],
                )?;
            }
            if add_only {
                let exists: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM records WHERE store_id=?1 AND key_sort=?2)",
                    rusqlite::params![store_id, key.encode_ordered()?],
                    |row| row.get(0),
                )?;
                if exists {
                    return Err(IndexedDbError::Constraint(
                        "record already exists for key".into(),
                    ));
                }
            }
            Ok(key)
        })();
        match outcome {
            Ok(key) => {
                self.active.as_mut().expect("validated transaction").pending = Some(PendingWrite {
                    id,
                    store_id,
                    key: key.clone(),
                });
                Ok(PreparedWrite { id, key })
            }
            Err(error) => self.finish_request(Err(error)),
        }
    }

    pub fn cancel_write(
        &mut self,
        transaction: TransactionHandle,
        request: u64,
    ) -> Result<(), IndexedDbError> {
        self.pending_write(transaction, request)?;
        self.active.as_mut().expect("validated transaction").pending = None;
        self.rollback_request()
    }

    pub fn finish_write(
        &mut self,
        transaction: TransactionHandle,
        request: u64,
        value: IndexedDbValue,
        indexes: Vec<IndexKeys>,
        quota: Option<u64>,
    ) -> Result<Key, IndexedDbError> {
        self.pending_write(transaction, request)?;
        let pending = self
            .active
            .as_mut()
            .expect("validated transaction")
            .pending
            .take()
            .expect("validated request");
        let outcome = self.write_prepared(pending, value, indexes, quota);
        self.finish_request(outcome)
    }

    fn pending_write(
        &self,
        transaction: TransactionHandle,
        request: u64,
    ) -> Result<(), IndexedDbError> {
        if self
            .transaction(transaction)?
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != request)
        {
            return Err(IndexedDbError::InvalidState(
                "stale IndexedDB write completion".into(),
            ));
        }
        Ok(())
    }

    fn write_prepared(
        &mut self,
        pending: PendingWrite,
        value: IndexedDbValue,
        indexes: Vec<IndexKeys>,
        quota: Option<u64>,
    ) -> Result<Key, IndexedDbError> {
        let sort = pending.key.encode_ordered()?;
        let old: Option<(i64, Vec<u8>, i64)> = self
            .connection
            .query_row(
                "SELECT record_id, key_value, usage FROM records WHERE store_id=?1 AND key_sort=?2",
                rusqlite::params![pending.store_id, sort],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (record, key_value, old_usage) = if let Some((record, key, usage)) = old {
            self.remove_record_contents(record)?;
            (record, key, usage)
        } else {
            (
                self.next_id("next_record_id")?,
                pending.key.encode_value()?,
                0,
            )
        };
        // Equivalent keys retain the original record representative, including -0.
        let primary_key = Key::decode_value(&key_value)?;
        let usage = sql_integer(
            key_usage_bytes(&primary_key).saturating_add(indexed_db_value_usage_bytes(&value)),
        )?;
        self.connection.execute(
            "INSERT INTO records(record_id, store_id, key_sort, key_value, wire_size, usage) VALUES(?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(record_id) DO UPDATE SET wire_size=excluded.wire_size, usage=excluded.usage",
            rusqlite::params![record, pending.store_id, sort, key_value, sql_integer(value.wire_bytes().len() as u64)?, usage],
        )?;
        self.write_value(record, &value)?;
        self.write_indexes(pending.store_id, record, &sort, indexes)?;
        self.adjust_usage(i128::from(usage) - i128::from(old_usage))?;
        if let Some(quota) = quota {
            let requested: i64 = self.connection.query_row(
                "SELECT usage FROM database_metadata WHERE singleton=1",
                [],
                |row| row.get(0),
            )?;
            if requested as u64 > quota {
                return Err(IndexedDbError::QuotaExceeded {
                    quota,
                    requested: requested as u64,
                });
            }
        }
        Ok(pending.key)
    }

    fn write_indexes(
        &self,
        store: i64,
        record: i64,
        primary_sort: &[u8],
        indexes: Vec<IndexKeys>,
    ) -> Result<(), IndexedDbError> {
        let definitions = self.index_definitions(store)?;
        let mut supplied = BTreeMap::new();
        for index in indexes {
            if supplied.insert(index.name, index.keys).is_some() {
                return Err(IndexedDbError::Data(
                    "duplicate index-key declaration".into(),
                ));
            }
        }
        if supplied.len() != definitions.len() || supplied.keys().ne(definitions.keys()) {
            return Err(IndexedDbError::InvalidState(
                "write must supply keys for every current index".into(),
            ));
        }
        for (name, (id, unique, multi_entry)) in definitions {
            self.insert_index_keys(
                id,
                unique,
                multi_entry,
                record,
                primary_sort,
                supplied.remove(&name).expect("validated index keys"),
            )?;
        }
        Ok(())
    }

    fn index_definitions(
        &self,
        store: i64,
    ) -> Result<BTreeMap<String, (i64, bool, bool)>, IndexedDbError> {
        let mut statement = self.connection.prepare_cached("SELECT name, index_id, is_unique, multi_entry FROM indexes WHERE store_id=?1 ORDER BY name")?;
        Ok(statement
            .query_map([store], |row| {
                Ok((row.get(0)?, (row.get(1)?, row.get(2)?, row.get(3)?)))
            })?
            .collect::<Result<_, _>>()?)
    }

    fn insert_index_keys(
        &self,
        index: i64,
        unique: bool,
        multi_entry: bool,
        record: i64,
        primary: &[u8],
        keys: Vec<Key>,
    ) -> Result<(), IndexedDbError> {
        if !multi_entry && keys.len() > 1 {
            return Err(IndexedDbError::Data(
                "a non-multiEntry index can have at most one key".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO index_entries VALUES(?1, ?2, ?3, ?4, ?5, ?6)")?;
        for key in keys {
            let sort = key.encode_ordered()?;
            if seen.insert(sort.clone()) {
                insert.execute(rusqlite::params![
                    index,
                    sort,
                    key.encode_value()?,
                    primary,
                    record,
                    unique
                ])?;
            }
        }
        Ok(())
    }

    /// A batch obtained and decoded by the versionchange owner. References and
    /// unique flags come from this database, never from the caller's payload.
    pub fn backfill_index_record(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        index_name: &str,
        primary_key: &Key,
        keys: Vec<Key>,
    ) -> Result<(), IndexedDbError> {
        self.versionchange(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        self.request(transaction, |owner| {
            let (index, unique, multi_entry) = owner
                .index_definitions(store)?
                .remove(index_name)
                .ok_or_else(|| {
                IndexedDbError::NotFound(format!("index `{index_name}` was not found"))
            })?;
            let primary = primary_key.encode_ordered()?;
            let record: i64 = owner.connection.query_row(
                "SELECT record_id FROM records WHERE store_id=?1 AND key_sort=?2",
                rusqlite::params![store, primary],
                |row| row.get(0),
            )?;
            owner.connection.execute(
                "DELETE FROM index_entries WHERE index_id=?1 AND record_id=?2",
                [index, record],
            )?;
            owner.insert_index_keys(index, unique, multi_entry, record, &primary, keys)
        })
    }

    pub fn get(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
        key: &Key,
        max_bytes: u64,
    ) -> Result<Option<IndexedDbValue>, IndexedDbError> {
        self.ready(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        let record: Option<(i64, i64)> = self
            .connection
            .query_row(
                "SELECT record_id, wire_size FROM records WHERE store_id=?1 AND key_sort=?2",
                rusqlite::params![store, key.encode_ordered()?],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        record
            .map(|(record, wire)| {
                if wire < 0 {
                    return Err(corruption("negative wire byte length"));
                }
                self.read_value(record, wire as u64, max_bytes)
            })
            .transpose()
    }

    pub fn get_key(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
        key: &Key,
    ) -> Result<Option<Key>, IndexedDbError> {
        self.ready(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        let value: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT key_value FROM records WHERE store_id=?1 AND key_sort=?2",
                rusqlite::params![store, key.encode_ordered()?],
                |row| row.get(0),
            )
            .optional()?;
        value.map(|value| Key::decode_value(&value)).transpose()
    }

    pub fn count(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<u64, IndexedDbError> {
        self.ready(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM records WHERE store_id=?1",
            [store],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    pub fn delete(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        key: &Key,
    ) -> Result<(), IndexedDbError> {
        self.writable(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        let sort = key.encode_ordered()?;
        self.request(transaction, |owner| {
            let old: Option<(i64, i64)> = owner
                .connection
                .query_row(
                    "SELECT record_id, usage FROM records WHERE store_id=?1 AND key_sort=?2",
                    rusqlite::params![store, sort],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((record, usage)) = old {
                owner.remove_record_contents(record)?;
                owner
                    .connection
                    .execute("DELETE FROM records WHERE record_id=?1", [record])?;
                owner.adjust_usage(-i128::from(usage))?;
            }
            Ok(())
        })
    }

    pub fn clear(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
    ) -> Result<(), IndexedDbError> {
        self.writable(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        self.request(transaction, |owner| owner.remove_store_records(store))
    }
}
