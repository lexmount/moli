use crate::IndexedDbError;

/// Charge before allocating a result. A refund only returns a previously
/// charged temporary allocation; retained output remains charged until delivery.
pub(crate) trait ReadBudget {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError>;
    fn refund(&mut self, bytes: u64);
}

impl ReadBudget for u64 {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        *self = self.checked_sub(bytes).ok_or_else(|| {
            IndexedDbError::Io("IndexedDB result exceeds its reserved byte budget".into())
        })?;
        Ok(())
    }

    fn refund(&mut self, bytes: u64) {
        *self += bytes;
    }
}
