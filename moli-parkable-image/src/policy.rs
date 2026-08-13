use std::time::Duration;

pub const DEFAULT_MIN_SIZE_TO_PARK: usize = 1024;
pub const DEFAULT_PARKING_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParkableImagePolicy {
    pub min_size_to_park: usize,
    pub parking_delay: Duration,
}

impl Default for ParkableImagePolicy {
    fn default() -> Self {
        Self {
            min_size_to_park: DEFAULT_MIN_SIZE_TO_PARK,
            parking_delay: DEFAULT_PARKING_DELAY,
        }
    }
}
