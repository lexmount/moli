use super::*;

impl Database {
    /// Resolve the accepted name on the SQL owner, after preceding CREATEs.
    /// Records and generators continue to address the same store_id.
    pub fn rename_object_store(
        &mut self,
        transaction: TransactionHandle,
        name: impl Into<crate::IndexedDbName>,
        new_name: impl Into<crate::IndexedDbName>,
    ) -> Result<(), IndexedDbError> {
        let name = name.into();
        let new_name = new_name.into();

        self.versionchange(transaction)?;
        let store = self.store_id(transaction, &name)?;
        if name == new_name {
            return Ok(());
        }
        self.request(transaction, |owner| {
            owner.connection.execute(
                "UPDATE object_stores SET name=?1 WHERE store_id=?2",
                rusqlite::params![names::encode(&new_name)?, store],
            )?;
            owner.adjust_usage(i128::from(new_name.usage_bytes()) - i128::from(name.usage_bytes()))
        })
    }

    /// Index entries retain index_id; only the schema label changes.
    pub fn rename_index(
        &mut self,
        transaction: TransactionHandle,
        store_name: impl Into<crate::IndexedDbName>,
        name: impl Into<crate::IndexedDbName>,
        new_name: impl Into<crate::IndexedDbName>,
    ) -> Result<(), IndexedDbError> {
        let store_name = store_name.into();
        let name = name.into();
        let new_name = new_name.into();

        self.versionchange(transaction)?;
        let store = self.store_id(transaction, &store_name)?;
        let index: i64 = self
            .connection
            .query_row(
                "SELECT index_id FROM indexes WHERE store_id=?1 AND name=?2",
                rusqlite::params![store, names::encode(&name)?],
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
                rusqlite::params![names::encode(&new_name)?, index, store],
            )?;
            owner.adjust_usage(i128::from(new_name.usage_bytes()) - i128::from(name.usage_bytes()))
        })
    }
}
