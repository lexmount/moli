//! Worker monotonic clock. Performance API uses the shared realm implementation.

use super::*;

fn unix_epoch_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub(super) fn monotonic_unix_epoch_millis() -> f64 {
    static BASE: OnceLock<(f64, Instant)> = OnceLock::new();
    let (epoch_millis, instant) = BASE.get_or_init(|| (unix_epoch_millis(), Instant::now()));
    epoch_millis + instant.elapsed().as_secs_f64() * 1000.0
}
