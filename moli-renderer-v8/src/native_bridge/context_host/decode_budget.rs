use std::sync::Arc;

use parking_lot::Mutex;

pub(in crate::native_bridge::context_host) const MAX_QUEUED_DECODE_JOBS: usize = 64;
pub(in crate::native_bridge::context_host) const MAX_QUEUED_ENCODED_BYTES: usize =
    256 * 1024 * 1024;
pub(in crate::native_bridge::context_host) const MAX_RETAINED_DECODED_BYTES: usize =
    256 * 1024 * 1024;

#[derive(Clone, Default)]
pub(in crate::native_bridge::context_host) struct SharedDecodeBudget {
    state: Arc<Mutex<DecodeBudgetState>>,
}

#[derive(Default)]
struct DecodeBudgetState {
    jobs: usize,
    encoded_bytes: usize,
    decoded_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::context_host) enum DecodeBudgetError {
    JobLimit,
    EncodedByteLimit,
    DecodedByteLimit,
}

impl SharedDecodeBudget {
    pub(in crate::native_bridge::context_host) fn admit_job(
        &self,
        encoded_bytes: usize,
    ) -> Result<DecodeJobPermit, DecodeBudgetError> {
        let mut state = self.state.lock();
        if state.jobs >= MAX_QUEUED_DECODE_JOBS {
            return Err(DecodeBudgetError::JobLimit);
        }
        let next_encoded = state
            .encoded_bytes
            .checked_add(encoded_bytes)
            .ok_or(DecodeBudgetError::EncodedByteLimit)?;
        if next_encoded > MAX_QUEUED_ENCODED_BYTES {
            return Err(DecodeBudgetError::EncodedByteLimit);
        }
        state.jobs += 1;
        state.encoded_bytes = next_encoded;
        Ok(DecodeJobPermit {
            budget: self.clone(),
            encoded_bytes,
        })
    }

    pub(in crate::native_bridge::context_host) fn reserve_decoded(
        &self,
        decoded_bytes: usize,
    ) -> Result<DecodedBytesPermit, DecodeBudgetError> {
        let mut state = self.state.lock();
        let next_decoded = state
            .decoded_bytes
            .checked_add(decoded_bytes)
            .ok_or(DecodeBudgetError::DecodedByteLimit)?;
        if next_decoded > MAX_RETAINED_DECODED_BYTES {
            return Err(DecodeBudgetError::DecodedByteLimit);
        }
        state.decoded_bytes = next_decoded;
        Ok(DecodedBytesPermit {
            budget: self.clone(),
            decoded_bytes,
        })
    }

    #[cfg(test)]
    pub(in crate::native_bridge::context_host) fn counters(&self) -> (usize, usize, usize) {
        let state = self.state.lock();
        (state.jobs, state.encoded_bytes, state.decoded_bytes)
    }
}

pub(in crate::native_bridge::context_host) struct DecodeJobPermit {
    budget: SharedDecodeBudget,
    encoded_bytes: usize,
}

impl DecodeJobPermit {
    pub(in crate::native_bridge::context_host) fn release_encoded_bytes(&mut self) {
        if self.encoded_bytes == 0 {
            return;
        }
        let mut state = self.budget.state.lock();
        state.encoded_bytes = state.encoded_bytes.saturating_sub(self.encoded_bytes);
        self.encoded_bytes = 0;
    }
}

impl Drop for DecodeJobPermit {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock();
        state.jobs = state.jobs.saturating_sub(1);
        state.encoded_bytes = state.encoded_bytes.saturating_sub(self.encoded_bytes);
    }
}

pub(in crate::native_bridge::context_host) struct DecodedBytesPermit {
    budget: SharedDecodeBudget,
    decoded_bytes: usize,
}

impl DecodedBytesPermit {
    pub(in crate::native_bridge::context_host) fn resize(
        &mut self,
        bytes: usize,
    ) -> Result<(), DecodeBudgetError> {
        let mut state = self.budget.state.lock();
        let next = state
            .decoded_bytes
            .checked_sub(self.decoded_bytes)
            .and_then(|retained| retained.checked_add(bytes))
            .ok_or(DecodeBudgetError::DecodedByteLimit)?;
        if next > MAX_RETAINED_DECODED_BYTES {
            return Err(DecodeBudgetError::DecodedByteLimit);
        }
        state.decoded_bytes = next;
        self.decoded_bytes = bytes;
        Ok(())
    }
}

impl Drop for DecodedBytesPermit {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock();
        state.decoded_bytes = state.decoded_bytes.saturating_sub(self.decoded_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_admission_is_bounded_and_drop_releases_every_charge() {
        let budget = SharedDecodeBudget::default();
        let permits = (0..MAX_QUEUED_DECODE_JOBS)
            .map(|_| budget.admit_job(0).expect("job within the bound"))
            .collect::<Vec<_>>();
        assert!(matches!(
            budget.admit_job(0),
            Err(DecodeBudgetError::JobLimit)
        ));
        assert_eq!(budget.counters(), (MAX_QUEUED_DECODE_JOBS, 0, 0));
        drop(permits);

        let encoded = budget
            .admit_job(MAX_QUEUED_ENCODED_BYTES)
            .expect("the exact encoded-byte bound is admitted");
        assert!(matches!(
            budget.admit_job(1),
            Err(DecodeBudgetError::EncodedByteLimit)
        ));
        drop(encoded);
        assert_eq!(budget.counters(), (0, 0, 0));
    }

    #[test]
    fn decoded_resource_charge_lives_exactly_as_long_as_its_permit() {
        let budget = SharedDecodeBudget::default();
        let permit = budget
            .reserve_decoded(MAX_RETAINED_DECODED_BYTES)
            .expect("the exact decoded-byte bound is admitted");
        assert!(matches!(
            budget.reserve_decoded(1),
            Err(DecodeBudgetError::DecodedByteLimit)
        ));
        assert_eq!(budget.counters(), (0, 0, MAX_RETAINED_DECODED_BYTES));
        drop(permit);
        assert_eq!(budget.counters(), (0, 0, 0));
    }

    #[test]
    fn growing_a_retained_resource_is_atomic_and_can_release_capacity() {
        let budget = SharedDecodeBudget::default();
        let mut first = budget.reserve_decoded(16).unwrap();
        let second = budget
            .reserve_decoded(MAX_RETAINED_DECODED_BYTES - 16)
            .unwrap();
        assert_eq!(first.resize(17), Err(DecodeBudgetError::DecodedByteLimit));
        assert_eq!(budget.counters().2, MAX_RETAINED_DECODED_BYTES);
        first.resize(8).unwrap();
        let third = budget.reserve_decoded(8).unwrap();
        drop((first, second, third));
        assert_eq!(budget.counters(), (0, 0, 0));
    }
}
