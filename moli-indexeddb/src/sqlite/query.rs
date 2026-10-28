use rusqlite::{params_from_iter, types::Value};

use super::{values::ReadBudget, *};
use crate::{CursorDirection, KeyRange, RecordSnapshot};

#[derive(Debug, Clone, Default)]
pub enum QuerySource {
    #[default]
    ObjectStore,
    Index(String),
}

#[derive(Debug, Clone)]
pub struct Seek {
    pub key: Key,
    /// `None` seeks by index key alone; `Some` seeks by the complete tuple.
    pub primary_key: Option<Key>,
    pub inclusive: bool,
}

#[derive(Debug, Clone)]
pub struct Query {
    pub source: QuerySource,
    pub range: KeyRange,
    pub direction: CursorDirection,
    pub seek: Option<Seek>,
    pub skip: u32,
    pub limit: u32,
    pub keys_only: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            source: QuerySource::ObjectStore,
            range: KeyRange::default(),
            direction: CursorDirection::Next,
            seek: None,
            skip: 0,
            limit: 1,
            keys_only: false,
        }
    }
}

struct Selection {
    sql: String,
    parameters: Vec<Value>,
    key: &'static str,
    primary: Option<&'static str>,
    index_unique: Option<bool>,
}

impl Selection {
    fn range(&mut self, range: &KeyRange) -> Result<(), IndexedDbError> {
        range.validate()?;
        if let Some(key) = &range.lower {
            self.sql.push_str(&format!(
                " AND {} {} ?",
                self.key,
                if range.lower_open { ">" } else { ">=" }
            ));
            self.parameters.push(Value::Blob(key.encode_ordered()?));
        }
        if let Some(key) = &range.upper {
            self.sql.push_str(&format!(
                " AND {} {} ?",
                self.key,
                if range.upper_open { "<" } else { "<=" }
            ));
            self.parameters.push(Value::Blob(key.encode_ordered()?));
        }
        Ok(())
    }

    fn seek(&mut self, seek: &Seek, direction: CursorDirection) -> Result<(), IndexedDbError> {
        let comparison = match (direction.is_reverse(), seek.inclusive) {
            (false, false) => ">",
            (false, true) => ">=",
            (true, false) => "<",
            (true, true) => "<=",
        };
        if self.primary.is_none()
            && seek
                .primary_key
                .as_ref()
                .is_some_and(|primary| primary != &seek.key)
        {
            return Err(IndexedDbError::Data(
                "object-store seek keys must agree".into(),
            ));
        }
        if let (Some(primary_column), Some(primary_key)) = (self.primary, &seek.primary_key)
            && !direction.is_unique()
        {
            self.sql.push_str(&format!(
                " AND ({}, {primary_column}) {comparison} (?, ?)",
                self.key
            ));
            self.parameters
                .push(Value::Blob(seek.key.encode_ordered()?));
            self.parameters
                .push(Value::Blob(primary_key.encode_ordered()?));
        } else {
            self.sql
                .push_str(&format!(" AND {} {comparison} ?", self.key));
            self.parameters
                .push(Value::Blob(seek.key.encode_ordered()?));
        }
        Ok(())
    }
}

impl Database {
    fn selection(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
        source: &QuerySource,
    ) -> Result<Selection, IndexedDbError> {
        self.ready(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        match source {
            QuerySource::ObjectStore => Ok(Selection {
                sql: " FROM records r WHERE r.store_id=?".into(),
                parameters: vec![Value::Integer(store)],
                key: "r.key_sort",
                primary: None,
                index_unique: None,
            }),
            QuerySource::Index(name) => {
                let (id, unique): (i64, bool) = self
                    .connection
                    .query_row(
                        "SELECT index_id, is_unique FROM indexes WHERE store_id=?1 AND name=?2",
                        rusqlite::params![store, name],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?
                    .ok_or_else(|| {
                        IndexedDbError::NotFound(format!("index `{name}` was not found"))
                    })?;
                Ok(Selection {
                    sql: " FROM index_entries e LEFT JOIN records r ON r.record_id=e.record_id AND r.key_sort=e.primary_key_sort AND r.store_id=? WHERE e.index_id=?".into(),
                    parameters: vec![Value::Integer(store), Value::Integer(id)], key: "e.index_key_sort", primary: Some("e.primary_key_sort"), index_unique: Some(unique),
                })
            }
        }
    }

    fn compile_query(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
        query: &Query,
    ) -> Result<Selection, IndexedDbError> {
        let mut selection = self.selection(transaction, store_name, &query.source)?;
        selection.range(&query.range)?;
        if let Some(seek) = &query.seek {
            selection.seek(seek, query.direction)?;
        }
        if query.direction.is_unique() && selection.primary.is_some() {
            // Unique reverse traversal still chooses the smallest primary key.
            selection.sql.push_str(" AND e.primary_key_sort=(SELECT min(g.primary_key_sort) FROM index_entries g WHERE g.index_id=e.index_id AND g.index_key_sort=e.index_key_sort)");
        }
        let order = if query.direction.is_reverse() {
            "DESC"
        } else {
            "ASC"
        };
        selection
            .sql
            .push_str(&format!(" ORDER BY {} {order}", selection.key));
        if let Some(primary) = selection.primary
            && !query.direction.is_unique()
        {
            selection.sql.push_str(&format!(", {primary} {order}"));
        }
        selection.sql.push_str(" LIMIT ? OFFSET ?");
        selection.parameters.extend([
            Value::Integer(i64::from(query.limit)),
            Value::Integer(i64::from(query.skip)),
        ]);
        let wire = if query.keys_only { "0" } else { "r.wire_size" };
        let (key, unique) = if selection.primary.is_some() {
            ("e.index_key_value", "e.is_unique")
        } else {
            ("r.key_value", "0")
        };
        selection.sql = format!(
            "SELECT {key}, r.key_value, r.record_id, {wire}, {unique}{}",
            selection.sql
        );
        Ok(selection)
    }

    /// Range/limit/projection and tuple seek are evaluated by SQLite. Each row
    /// consumes the same result budget; statements never survive a JS task.
    pub fn query(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
        query: &Query,
        max_bytes: u64,
    ) -> Result<Vec<RecordSnapshot>, IndexedDbError> {
        let selection = self.compile_query(transaction, store_name, query)?;
        let mut statement = self.connection.prepare_cached(&selection.sql)?;
        let mut rows = statement.query(params_from_iter(selection.parameters.iter()))?;
        let mut budget = ReadBudget::new(max_bytes);
        let mut output: Vec<RecordSnapshot> = Vec::new();
        while let Some(row) = rows.next()? {
            if let Some(unique) = selection.index_unique
                && row.get::<_, bool>(4)? != unique
            {
                return Err(corruption(
                    "index entry uniqueness differs from its definition",
                ));
            }
            let record: Option<i64> = row.get(2)?;
            let record = record
                .filter(|id| *id > 0)
                .ok_or_else(|| corruption("index entry has no matching record"))?;
            if output.len() == output.capacity() {
                let slots = output
                    .capacity()
                    .max(4)
                    .min(query.limit as usize - output.len());
                budget.charge(
                    (slots as u64).saturating_mul(std::mem::size_of::<RecordSnapshot>() as u64),
                )?;
                output.try_reserve_exact(slots).map_err(|_| {
                    IndexedDbError::Io("unable to allocate IndexedDB query results".into())
                })?;
            }
            let key = Key::decode_value_budgeted(
                row.get_ref(0)?
                    .as_blob()
                    .map_err(|_| corruption("query key is not a BLOB"))?,
                budget.remaining(),
            )?;
            let primary_key = Key::decode_value_budgeted(
                row.get_ref(1)?
                    .as_blob()
                    .map_err(|_| corruption("primary key is not a BLOB"))?,
                budget.remaining(),
            )?;
            let value = if query.keys_only {
                None
            } else {
                let wire: i64 = row.get(3)?;
                if wire < 0 {
                    return Err(corruption("negative wire byte length"));
                }
                Some(self.read_value_budgeted(record, wire as u64, &mut budget)?)
            };
            output.push(RecordSnapshot {
                key,
                primary_key,
                value,
            });
        }
        Ok(output)
    }

    pub fn query_count(
        &self,
        transaction: TransactionHandle,
        store_name: &str,
        source: &QuerySource,
        range: &KeyRange,
    ) -> Result<u64, IndexedDbError> {
        let mut selection = self.selection(transaction, store_name, source)?;
        selection.range(range)?;
        let count: i64 = self.connection.query_row(
            &format!("SELECT count(*){}", selection.sql),
            params_from_iter(selection.parameters.iter()),
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| corruption("negative query count"))
    }

    pub fn delete_range(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        range: &KeyRange,
    ) -> Result<(), IndexedDbError> {
        self.writable(transaction)?;
        let mut selection = self.selection(transaction, store_name, &QuerySource::ObjectStore)?;
        selection.range(range)?;
        self.request(transaction, |owner| {
            owner.remove_selected_records(&selection.sql, &selection.parameters)
        })
    }
}

#[cfg(test)]
mod tests;
