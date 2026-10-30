use super::*;

impl Database {
    /// Resolve the accepted name on the SQL owner, after preceding CREATEs.
    /// Records and generators continue to address the same store_id.
    pub fn rename_object_store(
        &mut self,
        transaction: TransactionHandle,
        name: &str,
        new_name: &str,
    ) -> Result<(), IndexedDbError> {
        self.versionchange(transaction)?;
        let store = self.store_id(transaction, name)?;
        if name == new_name {
            return Ok(());
        }
        self.request(transaction, |owner| {
            owner.connection.execute(
                "UPDATE object_stores SET name=?1 WHERE store_id=?2",
                rusqlite::params![new_name, store],
            )?;
            owner.adjust_usage(i128::from(new_name.len() as u64) - i128::from(name.len() as u64))
        })
    }

    /// Index entries retain index_id; only the schema label changes.
    pub fn rename_index(
        &mut self,
        transaction: TransactionHandle,
        store_name: &str,
        name: &str,
        new_name: &str,
    ) -> Result<(), IndexedDbError> {
        self.versionchange(transaction)?;
        let store = self.store_id(transaction, store_name)?;
        let index: i64 = self
            .connection
            .query_row(
                "SELECT index_id FROM indexes WHERE store_id=?1 AND name=?2",
                rusqlite::params![store, name],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| IndexedDbError::NotFound(format!("index `{name}` was not found")))?;
        if name == new_name {
            return Ok(());
        }
        self.request(transaction, |owner| {
            owner.connection.execute(
                "UPDATE indexes SET name=?1 WHERE index_id=?2 AND store_id=?3",
                rusqlite::params![new_name, index, store],
            )?;
            owner.adjust_usage(i128::from(new_name.len() as u64) - i128::from(name.len() as u64))
        })
    }
}
