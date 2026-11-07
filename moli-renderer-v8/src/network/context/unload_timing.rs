use std::{sync::Arc, time::Instant};

use parking_lot::Mutex;

use crate::context_bootstrap::RuntimeMessageSourceSecurity;

/// One retiring Document's own unload event, with its immutable native origin.
/// Descendant events and lifecycle authority are deliberately not retained.
#[derive(Clone, Debug)]
pub(crate) struct DocumentUnloadTiming {
    origin: RuntimeMessageSourceSecurity,
    start: f64,
    end: f64,
}

impl DocumentUnloadTiming {
    pub(crate) fn finish(
        origin: RuntimeMessageSourceSecurity,
        started: Instant,
        start: f64,
    ) -> Self {
        Self {
            origin,
            start,
            end: start + started.elapsed().as_secs_f64() * 1000.0,
        }
    }

    pub(crate) fn for_origin(&self, origin: &RuntimeMessageSourceSecurity) -> Option<(f64, f64)> {
        let same_origin = match (self.origin.serialized_origin(), origin.serialized_origin()) {
            (Some("null"), Some("null")) => matches!(
                (self.origin.opaque_origin(), origin.opaque_origin()),
                (Some(left), Some(right)) if left == right
            ),
            (Some(left), Some(right)) => left == right,
            _ => false,
        };
        same_origin.then_some((self.start, self.end))
    }
}

#[derive(Default)]
struct DocumentUnloadRecords {
    own: Option<DocumentUnloadTiming>,
    previous: Option<DocumentUnloadTiming>,
}

#[derive(Clone, Default)]
pub(super) struct DocumentUnloadTimings(Arc<Mutex<DocumentUnloadRecords>>);

impl DocumentUnloadTimings {
    pub(super) fn record(&self, timing: DocumentUnloadTiming) {
        self.0.lock().own.get_or_insert(timing);
    }

    pub(super) fn own(&self) -> Option<DocumentUnloadTiming> {
        self.0.lock().own.clone()
    }

    pub(super) fn set_previous(&self, timing: Option<DocumentUnloadTiming>) {
        self.0.lock().previous = timing;
    }

    pub(super) fn previous(&self) -> Option<DocumentUnloadTiming> {
        self.0.lock().previous.clone()
    }
}
