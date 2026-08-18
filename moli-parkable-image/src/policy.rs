use std::time::Duration;

const DEFAULT_MIN_SIZE_TO_PARK: usize = 1024;
const DEFAULT_PARKING_DELAY: Duration = Duration::from_secs(30);
const DEFAULT_READER_RELEASE_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParkableImagePolicy {
    pub min_size_to_park: usize,
    /// How long an unused frozen image remains resident.
    pub parking_delay: Duration,
    /// Grace period after the last reader releases the encoded bytes.
    pub reader_release_delay: Duration,
}

impl Default for ParkableImagePolicy {
    fn default() -> Self {
        Self {
            min_size_to_park: DEFAULT_MIN_SIZE_TO_PARK,
            parking_delay: DEFAULT_PARKING_DELAY,
            reader_release_delay: DEFAULT_READER_RELEASE_DELAY,
        }
    }
}
