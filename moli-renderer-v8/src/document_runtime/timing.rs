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
#[derive(Debug)]
pub(super) struct DocumentTiming {
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
    pub(crate) fn document_timing_snapshot(&self) -> DocumentTimingSnapshot {
        DocumentTimingSnapshot {
            time_origin: self.timing.time_origin,
            lifecycle: self.timing.lifecycle,
        }
    }

    pub(crate) fn record_lifecycle_timing(
        &mut self,
        phase: DocumentLifecycleTiming,
    ) -> DocumentTimingSnapshot {
        let index = phase as usize;
        if self.timing.lifecycle[index].is_none() {
            let previous = self.timing.lifecycle[..index]
                .iter()
                .flatten()
                .copied()
                .fold(0.0, f64::max);
            self.timing.lifecycle[index] =
                Some((self.timing.started.elapsed().as_secs_f64() * 1000.0).max(previous + 0.001));
        }
        self.document_timing_snapshot()
    }
}
