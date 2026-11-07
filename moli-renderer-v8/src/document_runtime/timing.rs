use std::time::Instant;

use super::DocumentRuntime;

#[derive(Clone, Copy, Debug)]
pub(crate) enum DocumentLifecycleTiming {
    DomContentLoadedStart,
    DomContentLoadedEnd,
    LoadStart,
    LoadEnd,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DocumentTimingSnapshot {
    pub(crate) time_origin: f64,
    pub(crate) lifecycle: [Option<f64>; 4],
}

/// The Document records lifecycle facts even when it has no JavaScript realm.
/// Performance objects only project these timestamps; materializing a wrapper
/// must neither restart the clock nor record/replay a lifecycle event.
#[derive(Debug, Clone)]
pub(crate) struct DocumentTiming {
    time_origin: f64,
    started: Instant,
    lifecycle: [Option<f64>; 4],
}

impl Default for DocumentTiming {
    fn default() -> Self {
        Self {
            time_origin: moli_time::unix_epoch_millis(),
            started: Instant::now(),
            lifecycle: [None; 4],
        }
    }
}

impl DocumentRuntime {
    /// Replacing the initial Document can retain the actual Window and its
    /// Performance objects. Keep that clock's anchors, but leave this new
    /// Document's lifecycle slots empty.
    pub(crate) fn retain_performance_clock_from_initial_document(&mut self, initial: &Self) {
        self.timing.time_origin = initial.timing.time_origin;
        self.timing.started = initial.timing.started;
    }

    pub(crate) fn document_timing_snapshot(&self) -> DocumentTimingSnapshot {
        self.timing.snapshot()
    }

    pub(crate) fn record_lifecycle_timing(
        &mut self,
        phase: DocumentLifecycleTiming,
    ) -> DocumentTimingSnapshot {
        self.timing.record(phase)
    }
}

impl DocumentTiming {
    pub(crate) fn from_navigation(
        navigation: &crate::network::navigation::NavigationTiming,
    ) -> Self {
        Self {
            time_origin: navigation.time_origin(),
            started: navigation.started_at(),
            lifecycle: [None; 4],
        }
    }
    pub(crate) fn from_navigation_start(started: Instant, start: std::time::SystemTime) -> Self {
        Self {
            time_origin: start
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0,
            started,
            lifecycle: [None; 4],
        }
    }
    pub(crate) fn snapshot(&self) -> DocumentTimingSnapshot {
        DocumentTimingSnapshot {
            time_origin: self.time_origin,
            lifecycle: self.lifecycle,
        }
    }

    pub(crate) fn record(&mut self, phase: DocumentLifecycleTiming) -> DocumentTimingSnapshot {
        let index = phase as usize;
        if self.lifecycle[index].is_none() {
            let previous = self.lifecycle[..index]
                .iter()
                .flatten()
                .copied()
                .fold(0.0, f64::max);
            self.lifecycle[index] =
                Some((self.started.elapsed().as_secs_f64() * 1000.0).max(previous + 0.001));
        }
        self.snapshot()
    }
}
