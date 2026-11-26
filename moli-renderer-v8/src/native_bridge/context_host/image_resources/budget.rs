use std::sync::Arc;

use parking_lot::Mutex;

pub(super) const MAX_QUEUED_IMAGE_ENCODED_BYTES: usize = 256 * 1024 * 1024;
pub(super) const MAX_RETAINED_IMAGE_DECODED_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Default)]
pub(super) struct SharedImageResourceBudget {
    state: Arc<Mutex<ImageResourceBudgetState>>,
}

#[derive(Default)]
struct ImageResourceBudgetState {
    encoded_bytes: usize,
    decoded_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageResourceBudgetError {
    EncodedByteLimit,
    DecodedByteLimit,
}

impl SharedImageResourceBudget {
    pub(super) fn reserve_encoded(
        &self,
        encoded_bytes: usize,
    ) -> Result<ImageEncodedBytesPermit, ImageResourceBudgetError> {
        let mut state = self.state.lock();
        let next_encoded = state
            .encoded_bytes
            .checked_add(encoded_bytes)
            .ok_or(ImageResourceBudgetError::EncodedByteLimit)?;
        if next_encoded > MAX_QUEUED_IMAGE_ENCODED_BYTES {
            return Err(ImageResourceBudgetError::EncodedByteLimit);
        }
        state.encoded_bytes = next_encoded;
        Ok(ImageEncodedBytesPermit {
            budget: self.clone(),
            encoded_bytes,
        })
    }

    pub(super) fn reserve_decoded(
        &self,
        decoded_bytes: usize,
    ) -> Result<ImageDecodedBytesPermit, ImageResourceBudgetError> {
        let mut state = self.state.lock();
        let next_decoded = state
            .decoded_bytes
            .checked_add(decoded_bytes)
            .ok_or(ImageResourceBudgetError::DecodedByteLimit)?;
        if next_decoded > MAX_RETAINED_IMAGE_DECODED_BYTES {
            return Err(ImageResourceBudgetError::DecodedByteLimit);
        }
        state.decoded_bytes = next_decoded;
        Ok(ImageDecodedBytesPermit {
            budget: self.clone(),
            decoded_bytes,
        })
    }

    #[cfg(test)]
    pub(super) fn counters(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.encoded_bytes, state.decoded_bytes)
    }
}

pub(super) struct ImageEncodedBytesPermit {
    budget: SharedImageResourceBudget,
    encoded_bytes: usize,
}

impl Drop for ImageEncodedBytesPermit {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock();
        state.encoded_bytes = state.encoded_bytes.saturating_sub(self.encoded_bytes);
    }
}

pub(super) struct ImageDecodedBytesPermit {
    budget: SharedImageResourceBudget,
    decoded_bytes: usize,
}

impl Drop for ImageDecodedBytesPermit {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock();
        state.decoded_bytes = state.decoded_bytes.saturating_sub(self.decoded_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_image_charge_lives_exactly_as_long_as_its_permit() {
        let budget = SharedImageResourceBudget::default();
        let encoded = budget
            .reserve_encoded(MAX_QUEUED_IMAGE_ENCODED_BYTES)
            .expect("the exact encoded-byte bound is admitted");
        assert!(matches!(
            budget.reserve_encoded(1),
            Err(ImageResourceBudgetError::EncodedByteLimit)
        ));
        drop(encoded);
        assert_eq!(budget.counters(), (0, 0));
    }

    #[test]
    fn decoded_resource_charge_lives_exactly_as_long_as_its_permit() {
        let budget = SharedImageResourceBudget::default();
        let permit = budget
            .reserve_decoded(MAX_RETAINED_IMAGE_DECODED_BYTES)
            .expect("the exact decoded-byte bound is admitted");
        assert!(matches!(
            budget.reserve_decoded(1),
            Err(ImageResourceBudgetError::DecodedByteLimit)
        ));
        assert_eq!(budget.counters(), (0, MAX_RETAINED_IMAGE_DECODED_BYTES));
        drop(permit);
        assert_eq!(budget.counters(), (0, 0));
    }
}
