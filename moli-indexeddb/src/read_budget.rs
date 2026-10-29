use crate::IndexedDbError;

/// Charge before allocating a result. A refund only returns a previously
/// charged temporary allocation; retained output remains charged until delivery.
pub trait ReadBudget {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError>;
    fn refund(&mut self, bytes: u64);
}

impl ReadBudget for moli_sqlite::TaskPermit {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        if bytes == 0 {
            return Ok(());
        }
        let bytes = usize::try_from(bytes)
            .ok()
            .and_then(|bytes| self.reserved_bytes().checked_add(bytes))
            .ok_or_else(|| IndexedDbError::Resource("IndexedDB allocation size overflow".into()))?;
        self.try_resize(bytes)
            .map_err(|error| IndexedDbError::Resource(error.to_string()))
    }

    fn refund(&mut self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let bytes = usize::try_from(bytes).expect("refund fits its allocation");
        let remaining = self
            .reserved_bytes()
            .checked_sub(bytes)
            .expect("reserved allocation refund");
        self.try_resize(remaining)
            .expect("shrinking a reservation also works after shutdown");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use moli_sqlite::{Capacity, Executor, ExecutorConfig, TaskKind};

    #[test]
    fn task_budget_failure_keeps_its_reservation_and_refund_works_after_shutdown() {
        let mut executor = Executor::new(ExecutorConfig {
            workers: 1,
            data: Capacity {
                tasks: 1,
                bytes: 128,
            },
            ..Default::default()
        })
        .unwrap();
        let handle = executor.handle();
        let mut permit = handle.try_reserve(TaskKind::Data, 32).unwrap();
        permit.charge(96).unwrap();
        assert_eq!(permit.reserved_bytes(), 128);
        assert!(matches!(permit.charge(1), Err(IndexedDbError::Resource(_))));
        assert_eq!(permit.reserved_bytes(), 128);
        executor.shutdown().unwrap();
        permit.refund(96);
        assert_eq!(permit.reserved_bytes(), 32);
        assert_eq!(handle.usage().data_bytes, 32);
        drop(permit);
        assert_eq!(handle.usage().data_bytes, 0);
        assert_eq!(handle.usage().data_tasks, 0);
    }
}
