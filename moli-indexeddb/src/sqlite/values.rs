use rusqlite::types::ValueRef;

use super::*;
use crate::{
    IndexedDbExternalObject as External, IndexedDbFileSystemHandleBucket as Bucket,
    IndexedDbFileSystemHandleKind as HandleKind, IndexedDbValue,
};

struct ReadBudget(u64);

impl ReadBudget {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        self.0 = self.0.checked_sub(bytes).ok_or_else(|| {
            IndexedDbError::Io("IndexedDB result exceeds its reserved byte budget".into())
        })?;
        Ok(())
    }
}

impl Database {
    pub(super) fn write_value(
        &self,
        record: i64,
        value: &IndexedDbValue,
    ) -> Result<(), IndexedDbError> {
        self.write_chunks("record_chunks", "record_id", record, value.wire_bytes())?;
        for (ordinal, object) in value.external_objects().iter().enumerate() {
            let id = self.next_id("next_object_id")?;
            match object {
                External::Blob { bytes, mime_type } => {
                    self.connection.execute("INSERT INTO external_objects(object_id, kind, size, mime_type) VALUES(?1, 'blob', ?2, ?3)", rusqlite::params![id, sql_integer(bytes.len() as u64)?, mime_type])?;
                    self.write_chunks("blob_chunks", "object_id", id, bytes)?;
                }
                External::File {
                    bytes,
                    mime_type,
                    name,
                    last_modified,
                } => {
                    self.connection.execute("INSERT INTO external_objects(object_id, kind, size, mime_type, file_name, last_modified_f64) VALUES(?1, 'file', ?2, ?3, ?4, ?5)", rusqlite::params![id, sql_integer(bytes.len() as u64)?, mime_type, name, last_modified.to_bits().to_be_bytes().as_slice()])?;
                    self.write_chunks("blob_chunks", "object_id", id, bytes)?;
                }
                External::FileSystemHandle { kind, bucket, path } => {
                    let kind = match kind {
                        HandleKind::File => "file",
                        HandleKind::Directory => "directory",
                    };
                    let (bucket_kind, bucket_id) = match bucket {
                        Bucket::Default => ("default", None),
                        Bucket::Named { bucket_id } => ("named", Some(sql_integer(*bucket_id)?)),
                    };
                    if bucket_id == Some(0) {
                        return Err(IndexedDbError::Data(
                            "named bucket identity must be positive".into(),
                        ));
                    }
                    self.connection.execute("INSERT INTO external_objects(object_id, kind, size, handle_kind, handle_bucket_kind, handle_bucket_id) VALUES(?1, 'file-system-handle', 0, ?2, ?3, ?4)", rusqlite::params![id, kind, bucket_kind, bucket_id])?;
                    let mut insert = self
                        .connection
                        .prepare_cached("INSERT INTO handle_components VALUES(?1, ?2, ?3)")?;
                    for (ordinal, component) in path.iter().enumerate() {
                        insert.execute(rusqlite::params![
                            id,
                            sql_integer(ordinal as u64)?,
                            component
                        ])?;
                    }
                }
            }
            self.connection.execute(
                "INSERT INTO record_objects VALUES(?1, ?2, ?3)",
                rusqlite::params![record, sql_integer(ordinal as u64)?, id],
            )?;
        }
        Ok(())
    }

    fn write_chunks(
        &self,
        table: &'static str,
        column: &'static str,
        id: i64,
        bytes: &[u8],
    ) -> Result<(), IndexedDbError> {
        let mut insert = self.connection.prepare_cached(&format!(
            "INSERT INTO {table}({column}, chunk_no, bytes) VALUES(?1, ?2, ?3)"
        ))?;
        for (ordinal, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
            insert.execute(rusqlite::params![id, sql_integer(ordinal as u64)?, chunk])?;
        }
        Ok(())
    }

    /// The delivered value owns its final bytes; it does not borrow the SQLite
    /// connection or retain a record after deletion/rollback/database close.
    pub(super) fn read_value(
        &self,
        record: i64,
        wire_size: u64,
        max_bytes: u64,
    ) -> Result<IndexedDbValue, IndexedDbError> {
        let mut budget = ReadBudget(max_bytes);
        let wire =
            self.read_chunks("record_chunks", "record_id", record, wire_size, &mut budget)?;
        let mut objects = Vec::new();
        let mut mapping = self.connection.prepare_cached(
            "SELECT ordinal, object_id FROM record_objects WHERE record_id=?1 ORDER BY ordinal",
        )?;
        let mut rows = mapping.query([record])?;
        while let Some(row) = rows.next()? {
            let ordinal: i64 = row.get(0)?;
            let id: i64 = row.get(1)?;
            if ordinal != objects.len() as i64 || id <= 0 {
                return Err(corruption("invalid IndexedDB external object slot"));
            }
            budget.charge(std::mem::size_of::<External>() as u64)?;
            let object = self.read_object(id, &mut budget)?;
            objects.try_reserve(1).map_err(|_| {
                IndexedDbError::Io("unable to allocate IndexedDB external objects".into())
            })?;
            objects.push(object);
        }
        Ok(IndexedDbValue::new(wire, objects))
    }

    fn read_object(&self, id: i64, budget: &mut ReadBudget) -> Result<External, IndexedDbError> {
        let mut statement = self.connection.prepare_cached("SELECT kind, size, mime_type, file_name, last_modified_f64, handle_kind, handle_bucket_kind, handle_bucket_id FROM external_objects WHERE object_id=?1")?;
        let mut rows = statement.query([id])?;
        let row = rows
            .next()?
            .ok_or_else(|| corruption("missing IndexedDB external object"))?;
        let kind: String = row.get(0)?;
        let size: i64 = row.get(1)?;
        if size < 0 {
            return Err(corruption("negative external object size"));
        }
        match kind.as_str() {
            "blob" | "file" => {
                let mime_type: String = row.get(2)?;
                budget.charge(mime_type.len() as u64)?;
                let bytes =
                    self.read_chunks("blob_chunks", "object_id", id, size as u64, budget)?;
                if kind == "blob" {
                    return Ok(External::Blob { bytes, mime_type });
                }
                let name: String = row.get(3)?;
                budget.charge(name.len() as u64 + 8)?;
                let bits: Vec<u8> = row.get(4)?;
                let bits: [u8; 8] = bits
                    .try_into()
                    .map_err(|_| corruption("invalid File lastModified bits"))?;
                Ok(External::File {
                    bytes,
                    mime_type,
                    name,
                    last_modified: f64::from_bits(u64::from_be_bytes(bits)),
                })
            }
            "file-system-handle" => {
                if size != 0 {
                    return Err(corruption("filesystem handle contains a body"));
                }
                let kind = match row.get::<_, String>(5)?.as_str() {
                    "file" => HandleKind::File,
                    "directory" => HandleKind::Directory,
                    _ => return Err(corruption("invalid filesystem handle kind")),
                };
                let bucket = match (
                    row.get::<_, String>(6)?.as_str(),
                    row.get::<_, Option<i64>>(7)?,
                ) {
                    ("default", None) => Bucket::Default,
                    ("named", Some(id)) if id > 0 => {
                        budget.charge(8)?;
                        Bucket::Named {
                            bucket_id: id as u64,
                        }
                    }
                    _ => return Err(corruption("invalid filesystem handle bucket")),
                };
                let mut path = Vec::new();
                let mut statement = self.connection.prepare_cached("SELECT ordinal, name FROM handle_components WHERE object_id=?1 ORDER BY ordinal")?;
                let mut rows = statement.query([id])?;
                while let Some(row) = rows.next()? {
                    if row.get::<_, i64>(0)? != path.len() as i64 {
                        return Err(corruption("filesystem handle path is not contiguous"));
                    }
                    let component: String = row.get(1)?;
                    budget.charge(component.len() as u64 + std::mem::size_of::<String>() as u64)?;
                    path.push(component);
                }
                Ok(External::FileSystemHandle { kind, bucket, path })
            }
            _ => Err(corruption("unknown IndexedDB external object kind")),
        }
    }

    fn read_chunks(
        &self,
        table: &'static str,
        column: &'static str,
        id: i64,
        expected: u64,
        budget: &mut ReadBudget,
    ) -> Result<Vec<u8>, IndexedDbError> {
        budget.charge(expected)?;
        let expected = usize::try_from(expected)
            .map_err(|_| corruption("IndexedDB body length exceeds address space"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(expected)
            .map_err(|_| IndexedDbError::Io("unable to allocate IndexedDB result".into()))?;
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT chunk_no, bytes FROM {table} WHERE {column}=?1 ORDER BY chunk_no"
        ))?;
        let mut rows = statement.query([id])?;
        let mut ordinal = 0_i64;
        while let Some(row) = rows.next()? {
            if row.get::<_, i64>(0)? != ordinal {
                return Err(corruption("IndexedDB chunks are not contiguous"));
            }
            let ValueRef::Blob(chunk) = row.get_ref(1)? else {
                return Err(corruption("IndexedDB chunk is not a BLOB"));
            };
            if chunk.is_empty()
                || chunk.len() > CHUNK_BYTES
                || chunk.len() > expected.saturating_sub(bytes.len())
            {
                return Err(corruption("IndexedDB chunk length mismatch"));
            }
            bytes.extend_from_slice(chunk);
            ordinal += 1;
        }
        if bytes.len() != expected {
            return Err(corruption("IndexedDB body is truncated"));
        }
        Ok(bytes)
    }

    pub(super) fn remove_record_contents(&self, record: i64) -> Result<(), IndexedDbError> {
        for table in ["blob_chunks", "handle_components", "external_objects"] {
            self.connection.execute(&format!("DELETE FROM {table} WHERE object_id IN (SELECT object_id FROM record_objects WHERE record_id=?1)"), [record])?;
        }
        self.connection
            .execute("DELETE FROM record_objects WHERE record_id=?1", [record])?;
        self.connection
            .execute("DELETE FROM record_chunks WHERE record_id=?1", [record])?;
        self.connection
            .execute("DELETE FROM index_entries WHERE record_id=?1", [record])?;
        Ok(())
    }

    pub(super) fn remove_store_records(&self, store: i64) -> Result<(), IndexedDbError> {
        let usage: i64 = self.connection.query_row(
            "SELECT coalesce(sum(usage), 0) FROM records WHERE store_id=?1",
            [store],
            |row| row.get(0),
        )?;
        for table in ["blob_chunks", "handle_components", "external_objects"] {
            self.connection.execute(&format!("DELETE FROM {table} WHERE object_id IN (SELECT object_id FROM record_objects WHERE record_id IN (SELECT record_id FROM records WHERE store_id=?1))"), [store])?;
        }
        for table in ["record_objects", "record_chunks", "index_entries"] {
            self.connection.execute(&format!("DELETE FROM {table} WHERE record_id IN (SELECT record_id FROM records WHERE store_id=?1)"), [store])?;
        }
        self.connection
            .execute("DELETE FROM records WHERE store_id=?1", [store])?;
        self.adjust_usage(-i128::from(usage))
    }
}
