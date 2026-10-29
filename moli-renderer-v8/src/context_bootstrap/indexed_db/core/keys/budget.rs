use super::*;
use moli_indexeddb::ReadBudget;

// Failed conversion and temporary traversal buffers must not consume the
// request's remaining capacity. Only the returned payload survives this scope.
pub(in crate::context_bootstrap::indexed_db) struct Allocations<'a> {
    budget: &'a mut dyn ReadBudget,
    bytes: u64,
}

impl<'a> Allocations<'a> {
    pub(in crate::context_bootstrap::indexed_db) fn new(budget: &'a mut dyn ReadBudget) -> Self {
        Self { budget, bytes: 0 }
    }

    pub(in crate::context_bootstrap::indexed_db) fn keep(
        mut self,
        bytes: usize,
    ) -> Result<(), IndexedDbError> {
        let bytes = bytes as u64;
        let temporary = self.bytes.checked_sub(bytes).ok_or_else(|| {
            IndexedDbError::Resource(
                "IndexedDB conversion exceeded its allocation reservation".into(),
            )
        })?;
        self.budget.refund(temporary);
        self.bytes = 0;
        Ok(())
    }
}

impl ReadBudget for Allocations<'_> {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        self.budget.charge(bytes)?;
        self.bytes += bytes;
        Ok(())
    }
    fn refund(&mut self, bytes: u64) {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .expect("reserved conversion refund");
        self.budget.refund(bytes);
    }
}

impl Drop for Allocations<'_> {
    fn drop(&mut self) {
        self.budget.refund(self.bytes);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn reserve_vec<T>(
    budget: &mut dyn ReadBudget,
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), IndexedDbError> {
    let required = values
        .len()
        .checked_add(additional)
        .ok_or_else(|| IndexedDbError::Resource("IndexedDB allocation size overflow".into()))?;
    if required <= values.capacity() {
        return Ok(());
    }
    let capacity = required.max(values.capacity().saturating_mul(2));
    let extra = capacity
        .checked_sub(values.capacity())
        .and_then(|count| count.checked_mul(std::mem::size_of::<T>()))
        .ok_or_else(|| IndexedDbError::Resource("IndexedDB allocation size overflow".into()))?;
    budget.charge(extra as u64)?;
    values
        .try_reserve_exact(capacity - values.len())
        .map_err(|error| IndexedDbError::Resource(error.to_string()))?;
    Ok(())
}
