use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct AudioStats {
    inner: Arc<AudioStatsInner>,
}

#[derive(Debug)]
struct AudioStatsInner {
    callback_count: AtomicU64,
    render_duration_ns_total: AtomicU64,
    callback_budget_ns_total: AtomicU64,
    underrun_count: AtomicU64,
    peak_load_ppm: AtomicU64,
    underrun_duration_ns_total: AtomicU64,
    underrun_events_total: AtomicU64,
    previous_underrun: AtomicBool,
    latest_latency_ns: AtomicU64,
    latency_sum_ns: AtomicU64,
    latency_count: AtomicU64,
    latency_min_ns: AtomicU64,
    latency_max_ns: AtomicU64,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct AudioStatsSnapshot {
    pub callback_count: u64,
    pub render_duration_ns_total: u64,
    pub callback_budget_ns_total: u64,
    pub underrun_count: u64,
    pub underrun_duration_ns_total: u64,
    pub underrun_events_total: u64,
    pub latency_sum_ns: u64,
    pub latency_count: u64,
    pub latency_min_ns: u64,
    pub latency_max_ns: u64,
}

impl Default for AudioStats {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioStats {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(AudioStatsInner {
                callback_count: AtomicU64::new(0),
                render_duration_ns_total: AtomicU64::new(0),
                callback_budget_ns_total: AtomicU64::new(0),
                underrun_count: AtomicU64::new(0),
                peak_load_ppm: AtomicU64::new(0),
                underrun_duration_ns_total: AtomicU64::new(0),
                underrun_events_total: AtomicU64::new(0),
                previous_underrun: AtomicBool::new(false),
                latest_latency_ns: AtomicU64::new(0),
                latency_sum_ns: AtomicU64::new(0),
                latency_count: AtomicU64::new(0),
                latency_min_ns: AtomicU64::new(u64::MAX),
                latency_max_ns: AtomicU64::new(0),
            }),
        }
    }

    pub(crate) fn record_render_callback(&self, render_duration_ns: u64, callback_budget_ns: u64) {
        self.inner.callback_count.fetch_add(1, Ordering::Relaxed);
        self.inner
            .render_duration_ns_total
            .fetch_add(render_duration_ns, Ordering::Relaxed);
        self.inner
            .callback_budget_ns_total
            .fetch_add(callback_budget_ns, Ordering::Relaxed);
        let load_ppm = render_duration_ns
            .saturating_mul(1_000_000)
            .checked_div(callback_budget_ns)
            .unwrap_or(0);
        self.inner
            .peak_load_ppm
            .fetch_max(load_ppm, Ordering::Relaxed);

        let underrun_duration_ns = render_duration_ns.saturating_sub(callback_budget_ns);
        let underrun = underrun_duration_ns > 0;
        if underrun {
            self.inner.underrun_count.fetch_add(1, Ordering::Relaxed);
            self.inner
                .underrun_duration_ns_total
                .fetch_add(underrun_duration_ns, Ordering::Relaxed);
            if !self.inner.previous_underrun.swap(true, Ordering::Relaxed) {
                self.inner
                    .underrun_events_total
                    .fetch_add(1, Ordering::Relaxed);
            }
        } else {
            self.inner.previous_underrun.store(false, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_latency_seconds(&self, latency: f64) {
        if !latency.is_finite() || latency < 0. {
            return;
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let latency_ns = (latency * 1_000_000_000.).round() as u64;
        self.record_latency_ns(latency_ns);
    }

    pub(crate) fn record_latency_ns(&self, latency_ns: u64) {
        self.inner
            .latest_latency_ns
            .store(latency_ns, Ordering::Relaxed);
        self.inner
            .latency_sum_ns
            .fetch_add(latency_ns, Ordering::Relaxed);
        self.inner.latency_count.fetch_add(1, Ordering::Relaxed);
        self.inner
            .latency_min_ns
            .fetch_min(latency_ns, Ordering::Relaxed);
        self.inner
            .latency_max_ns
            .fetch_max(latency_ns, Ordering::Relaxed);
    }

    pub(crate) fn reset_latency(&self) {
        let current = self.inner.latest_latency_ns.load(Ordering::Relaxed);
        self.inner.latency_sum_ns.store(current, Ordering::Relaxed);
        self.inner.latency_count.store(1, Ordering::Relaxed);
        self.inner.latency_min_ns.store(current, Ordering::Relaxed);
        self.inner.latency_max_ns.store(current, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> AudioStatsSnapshot {
        let latency_min_ns = self.inner.latency_min_ns.load(Ordering::Relaxed);
        AudioStatsSnapshot {
            callback_count: self.inner.callback_count.load(Ordering::Relaxed),
            render_duration_ns_total: self.inner.render_duration_ns_total.load(Ordering::Relaxed),
            callback_budget_ns_total: self.inner.callback_budget_ns_total.load(Ordering::Relaxed),
            underrun_count: self.inner.underrun_count.load(Ordering::Relaxed),
            underrun_duration_ns_total: self
                .inner
                .underrun_duration_ns_total
                .load(Ordering::Relaxed),
            underrun_events_total: self.inner.underrun_events_total.load(Ordering::Relaxed),
            latency_sum_ns: self.inner.latency_sum_ns.load(Ordering::Relaxed),
            latency_count: self.inner.latency_count.load(Ordering::Relaxed),
            latency_min_ns: if latency_min_ns == u64::MAX {
                0
            } else {
                latency_min_ns
            },
            latency_max_ns: self.inner.latency_max_ns.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn take_peak_load(&self) -> f64 {
        self.inner.peak_load_ppm.swap(0, Ordering::Relaxed) as f64 / 1_000_000.
    }
}

impl AudioStatsSnapshot {
    pub(crate) fn underrun_duration_seconds(&self) -> f64 {
        ns_to_seconds(self.underrun_duration_ns_total)
    }

    pub(crate) fn average_latency_seconds(&self) -> f64 {
        self.latency_sum_ns
            .checked_div(self.latency_count)
            .map_or(0., ns_to_seconds)
    }

    pub(crate) fn minimum_latency_seconds(&self) -> f64 {
        ns_to_seconds(self.latency_min_ns)
    }

    pub(crate) fn maximum_latency_seconds(&self) -> f64 {
        ns_to_seconds(self.latency_max_ns)
    }
}

fn ns_to_seconds(ns: u64) -> f64 {
    ns as f64 / 1_000_000_000.
}

#[cfg(test)]
mod tests {
    use super::AudioStats;

    #[test]
    fn underrun_duration_tracks_missed_deadline_time() {
        let stats = AudioStats::new();

        stats.record_render_callback(3_000_000, 2_000_000);
        stats.record_render_callback(2_500_000, 2_000_000);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.underrun_count, 2);
        assert_eq!(snapshot.underrun_events_total, 1);
        assert_eq!(snapshot.underrun_duration_ns_total, 1_500_000);
        assert_eq!(snapshot.underrun_duration_seconds(), 0.0015);
    }

    #[test]
    fn underrun_events_count_continuous_sequences() {
        let stats = AudioStats::new();

        stats.record_render_callback(3_000_000, 2_000_000);
        stats.record_render_callback(1_000_000, 2_000_000);
        stats.record_render_callback(3_000_000, 2_000_000);

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.underrun_count, 2);
        assert_eq!(snapshot.underrun_events_total, 2);
        assert_eq!(snapshot.underrun_duration_ns_total, 2_000_000);
    }
}
