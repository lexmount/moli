use rusqlite::types::ValueRef;

use super::*;
use crate::{
    IndexedDbExternalObject as External, IndexedDbFileSystemHandleBucket as Bucket,
    IndexedDbFileSystemHandleKind as HandleKind, IndexedDbValue, read_budget::ReadBudget,
};

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
        mut max_bytes: u64,
    ) -> Result<IndexedDbValue, IndexedDbError> {
        self.read_value_budgeted(record, wire_size, &mut max_bytes)
    }

    pub(super) fn read_value_budgeted(
        &self,
        record: i64,
        wire_size: u64,
        budget: &mut dyn ReadBudget,
    ) -> Result<IndexedDbValue, IndexedDbError> {
        let wire = self.read_chunks("record_chunks", "record_id", record, wire_size, budget)?;
        let mut objects = Vec::new();
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM record_objects WHERE record_id=?1",
            [record],
            |row| row.get(0),
        )?;
        let count = usize::try_from(count)
            .map_err(|_| corruption("external object count exceeds address space"))?;
        budget.charge((count as u64).saturating_mul(std::mem::size_of::<External>() as u64))?;
        objects.try_reserve_exact(count).map_err(|_| {
            IndexedDbError::Io("unable to allocate IndexedDB external objects".into())
        })?;
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
            if objects.len() == count {
                return Err(corruption("external object count changed during read"));
            }
            let object = self.read_object(id, budget)?;
            objects.push(object);
        }
        Ok(IndexedDbValue::new(wire, objects))
    }

    fn read_object(
        &self,
        id: i64,
        budget: &mut dyn ReadBudget,
    ) -> Result<External, IndexedDbError> {
        let mut statement = self.connection.prepare_cached("SELECT kind, size, mime_type, file_name, last_modified_f64, handle_kind, handle_bucket_kind, handle_bucket_id FROM external_objects WHERE object_id=?1")?;
        let mut rows = statement.query([id])?;
        let row = rows
            .next()?
            .ok_or_else(|| corruption("missing IndexedDB external object"))?;
        let kind = row
            .get_ref(0)?
            .as_str()
            .map_err(|_| corruption("external object kind is not TEXT"))?;
        let size: i64 = row.get(1)?;
        if size < 0 {
            return Err(corruption("negative external object size"));
        }
        match kind {
            "blob" | "file" => {
                let mime_type = read_text(row, 2, budget)?;
                let bytes =
                    self.read_chunks("blob_chunks", "object_id", id, size as u64, budget)?;
                if kind == "blob" {
                    return Ok(External::Blob { bytes, mime_type });
                }
                let name = read_text(row, 3, budget)?;
                let bits = row
                    .get_ref(4)?
                    .as_blob()
                    .map_err(|_| corruption("invalid File lastModified bits"))?;
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
                let kind = match row
                    .get_ref(5)?
                    .as_str()
                    .map_err(|_| corruption("invalid filesystem handle kind"))?
                {
                    "file" => HandleKind::File,
                    "directory" => HandleKind::Directory,
                    _ => return Err(corruption("invalid filesystem handle kind")),
                };
                let bucket = match (
                    row.get_ref(6)?
                        .as_str()
                        .map_err(|_| corruption("invalid filesystem handle bucket"))?,
                    row.get::<_, Option<i64>>(7)?,
                ) {
                    ("default", None) => Bucket::Default,
                    ("named", Some(id)) if id > 0 => Bucket::Named {
                        bucket_id: id as u64,
                    },
                    _ => return Err(corruption("invalid filesystem handle bucket")),
                };
                let mut path = Vec::new();
                let count: i64 = self.connection.query_row(
                    "SELECT count(*) FROM handle_components WHERE object_id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                let count = usize::try_from(count)
                    .map_err(|_| corruption("filesystem path count exceeds address space"))?;
                budget
                    .charge((count as u64).saturating_mul(std::mem::size_of::<String>() as u64))?;
                path.try_reserve_exact(count).map_err(|_| {
                    IndexedDbError::Io("unable to allocate filesystem handle path".into())
                })?;
                let mut statement = self.connection.prepare_cached("SELECT ordinal, name FROM handle_components WHERE object_id=?1 ORDER BY ordinal")?;
                let mut rows = statement.query([id])?;
                while let Some(row) = rows.next()? {
                    if row.get::<_, i64>(0)? != path.len() as i64 {
                        return Err(corruption("filesystem handle path is not contiguous"));
                    }
                    let component = read_text(row, 1, budget)?;
                    if path.len() == count {
                        return Err(corruption("filesystem path count changed during read"));
                    }
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
        budget: &mut dyn ReadBudget,
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
        self.remove_selected_records(
            " FROM records r WHERE r.store_id=?",
            &[rusqlite::types::Value::Integer(store)],
        )
    }

    pub(super) fn remove_selected_records(
        &self,
        selection: &str,
        parameters: &[rusqlite::types::Value],
    ) -> Result<(), IndexedDbError> {
        let usage: i64 = self.connection.query_row(
            &format!("SELECT coalesce(sum(r.usage), 0){selection}"),
            rusqlite::params_from_iter(parameters.iter()),
            |row| row.get(0),
        )?;
        let records = format!("SELECT r.record_id{selection}");
        for table in ["blob_chunks", "handle_components", "external_objects"] {
            self.connection.execute(&format!("DELETE FROM {table} WHERE object_id IN (SELECT object_id FROM record_objects WHERE record_id IN ({records}))"), rusqlite::params_from_iter(parameters.iter()))?;
        }
        for table in ["record_objects", "record_chunks", "index_entries"] {
            self.connection.execute(
                &format!("DELETE FROM {table} WHERE record_id IN ({records})"),
                rusqlite::params_from_iter(parameters.iter()),
            )?;
        }
        self.connection.execute(
            &format!("DELETE FROM records WHERE record_id IN ({records})"),
            rusqlite::params_from_iter(parameters.iter()),
        )?;
        self.adjust_usage(-i128::from(usage))
    }
}

fn read_text(
    row: &rusqlite::Row<'_>,
    column: usize,
    budget: &mut dyn ReadBudget,
) -> Result<String, IndexedDbError> {
    let ValueRef::Text(bytes) = row.get_ref(column)? else {
        return Err(corruption("external object string is not TEXT"));
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|_| corruption("external object string is invalid UTF-8"))?;
    budget.charge(bytes.len() as u64)?;
    let mut output = String::new();
    output
        .try_reserve_exact(bytes.len())
        .map_err(|_| IndexedDbError::Io("unable to allocate external object string".into()))?;
    output.push_str(text);
    Ok(output)
}
