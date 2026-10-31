//! Active LocalWindows retain their localStorage area; dirty batches retain it
//! independently until commit. Storage wrappers need no native IDs or finalizers.
use super::{JsContextHost, WindowExecutionContextOwner};
use moli_web_storage::{WebStorageAreaLease, WebStorageMutationError};

pub(super) struct WindowStorageArea {
    key: String,
    _lease: WebStorageAreaLease,
}

impl JsContextHost {
    pub(crate) fn ensure_local_storage_area(
        &mut self,
        owner: WindowExecutionContextOwner,
        key: &str,
    ) -> Result<(), WebStorageMutationError> {
        if self
            .local_storage_areas
            .get(&owner)
            .is_some_and(|area| area.key == key)
        {
            return Ok(());
        }
        if let Some(lease) = self.web_storage_store.lease_area(key)? {
            self.local_storage_areas.insert(
                owner,
                WindowStorageArea {
                    key: key.to_owned(),
                    _lease: lease,
                },
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn local_storage_area_count_for_test(&self) -> usize {
        self.local_storage_areas.len()
    }
}
