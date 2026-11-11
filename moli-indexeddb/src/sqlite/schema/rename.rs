use super::*;

impl Database {
    pub fn rename_object_store(
        &mut self,
        transaction: TransactionHandle,
        store: StoreId,
        new_name: impl Into<crate::IndexedDbName>,
    ) -> Result<(), IndexedDbError> {
        let new_name = new_name.into();
        self.versionchange(transaction)?;
        self.validate_store(transaction, store)?;
        let name: Vec<u8> = self.connection.query_row(
            "SELECT name FROM object_stores WHERE store_id=?1",
            [store],
            |row| row.get(0),
        )?;
        let name = names::decode(&name)?;
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

    pub fn rename_index(
        &mut self,
        transaction: TransactionHandle,
        store: StoreId,
        index: IndexId,
        new_name: impl Into<crate::IndexedDbName>,
    ) -> Result<(), IndexedDbError> {
        let new_name = new_name.into();
        self.versionchange(transaction)?;
        self.validate_store(transaction, store)?;
        let name: Vec<u8> = self
            .connection
            .query_row(
                "SELECT name FROM indexes WHERE store_id=?1 AND index_id=?2",
                rusqlite::params![store, index],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| IndexedDbError::NotFound(format!("index {index} was not found")))?;
        let name = names::decode(&name)?;
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
