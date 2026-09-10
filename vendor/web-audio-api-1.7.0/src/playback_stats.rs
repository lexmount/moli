use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::context::{AudioContextState, BaseAudioContext, ConcreteBaseAudioContext};
use crate::stats::AudioStats;

const UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// Snapshot of [`AudioPlaybackStats`] values.
#[derive(Clone, Debug, Default)]
pub struct AudioPlaybackStatsSnapshot {
    /// Total duration of underrun frames in seconds.
    pub underrun_duration: f64,
    /// Number of underrun events.
    pub underrun_events: u64,
    /// Total playback duration in seconds.
    pub total_duration: f64,
    /// Average output latency in seconds since the last latency reset.
    pub average_latency: f64,
    /// Minimum output latency in seconds since the last latency reset.
    pub minimum_latency: f64,
    /// Maximum output latency in seconds since the last latency reset.
    pub maximum_latency: f64,
}

#[derive(Debug, Default)]
struct ExposedStats {
    values: AudioPlaybackStatsSnapshot,
    last_update: Option<Instant>,
}

/// Playback statistics for an [`AudioContext`](crate::context::AudioContext).
///
/// The underrun counters are currently based on render callbacks that miss their realtime
/// deadline. Backends that expose device-level underrun counters can feed more precise values into
/// the shared stats layer in the future.
#[derive(Clone, Debug)]
pub struct AudioPlaybackStats {
    context: ConcreteBaseAudioContext,
    stats: AudioStats,
    exposed: Arc<Mutex<ExposedStats>>,
}

impl AudioPlaybackStats {
    pub(crate) fn new(context: ConcreteBaseAudioContext, stats: AudioStats) -> Self {
        let instance = Self {
            context,
            stats,
            exposed: Arc::new(Mutex::new(ExposedStats::default())),
        };
        instance.stats.reset_latency();
        instance
    }

    /// Total duration of underrun frames in seconds.
    #[must_use]
    pub fn underrun_duration(&self) -> f64 {
        self.current().underrun_duration
    }

    /// Number of underrun events.
    #[must_use]
    pub fn underrun_events(&self) -> u64 {
        self.current().underrun_events
    }

    /// Total playback duration in seconds.
    #[must_use]
    pub fn total_duration(&self) -> f64 {
        self.current().total_duration
    }

    /// Average output latency in seconds since the last latency reset.
    #[must_use]
    pub fn average_latency(&self) -> f64 {
        self.current().average_latency
    }

    /// Minimum output latency in seconds since the last latency reset.
    #[must_use]
    pub fn minimum_latency(&self) -> f64 {
        self.current().minimum_latency
    }

    /// Maximum output latency in seconds since the last latency reset.
    #[must_use]
    pub fn maximum_latency(&self) -> f64 {
        self.current().maximum_latency
    }

    /// Reset the tracked latency interval.
    pub fn reset_latency(&self) {
        self.stats.reset_latency();
        self.refresh();
    }

    /// Return the currently exposed values as a plain Rust snapshot.
    #[must_use]
    pub fn to_json(&self) -> AudioPlaybackStatsSnapshot {
        self.current()
    }

    fn current(&self) -> AudioPlaybackStatsSnapshot {
        let mut exposed = self.exposed.lock().unwrap();
        let should_update = self.context.state() == AudioContextState::Running
            && exposed
                .last_update
                .is_none_or(|last_update| last_update.elapsed() >= UPDATE_INTERVAL);
        if should_update {
            exposed.values = self.read_current_values();
            exposed.last_update = Some(Instant::now());
        }
        exposed.values.clone()
    }

    fn refresh(&self) {
        let mut exposed = self.exposed.lock().unwrap();
        exposed.values = self.read_current_values();
        exposed.last_update = Some(Instant::now());
    }

    fn read_current_values(&self) -> AudioPlaybackStatsSnapshot {
        let snapshot = self.stats.snapshot();
        let underrun_duration = snapshot.underrun_duration_seconds();

        AudioPlaybackStatsSnapshot {
            underrun_duration,
            underrun_events: snapshot.underrun_events_total,
            total_duration: underrun_duration + self.context.current_time(),
            average_latency: snapshot.average_latency_seconds(),
            minimum_latency: snapshot.minimum_latency_seconds(),
            maximum_latency: snapshot.maximum_latency_seconds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::context::{AudioContext, AudioContextOptions};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn test_same_instance() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        let stats1 = context.playback_stats();
        let stats2 = context.playback_stats();
        let stats3 = stats2.clone();

        assert!(Arc::ptr_eq(&stats1.exposed, &stats2.exposed));
        assert!(Arc::ptr_eq(&stats1.exposed, &stats3.exposed));
    }

    #[test]
    fn test_playback_stats() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);
        let stats = context.playback_stats();

        std::thread::sleep(Duration::from_millis(50));

        let snapshot = stats.to_json();
        assert!(snapshot.total_duration > 0.);
        assert!(snapshot.underrun_duration >= 0.);
        assert_eq!(stats.underrun_events(), snapshot.underrun_events);
        assert!(stats.average_latency().is_finite());
        assert!(stats.minimum_latency().is_finite());
        assert!(stats.maximum_latency().is_finite());

        stats.reset_latency();
        assert_eq!(stats.average_latency(), 0.);
        assert_eq!(stats.minimum_latency(), 0.);
        assert_eq!(stats.maximum_latency(), 0.);
    }

    #[test]
    fn test_playback_stats_do_not_update_when_closed() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);
        let stats = context.playback_stats();

        std::thread::sleep(Duration::from_millis(50));
        let running_total_duration = stats.total_duration();

        context.close_sync();
        std::thread::sleep(Duration::from_secs(2));

        assert_eq!(stats.total_duration(), running_total_duration);
    }
}
