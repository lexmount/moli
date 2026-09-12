use parking_lot::Mutex;
use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

/// Facts about transmission of a request body, delivered on the fetch runtime
/// thread. Consumers must enqueue work rather than enter a script engine here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadEvent {
    Progress { loaded: u64, total: u64 },
    Complete { loaded: u64, total: u64 },
}

/// A single body upload, shared by redirect and authentication retries. Byte
/// counts never go backwards and completion is reported only once.
#[derive(Clone)]
pub struct UploadObserver(Arc<UploadObserverInner>);

struct UploadObserverInner {
    total: u64,
    callback: Box<dyn Fn(UploadEvent) + Send + Sync>,
    state: Mutex<UploadState>,
}

struct UploadState {
    loaded: u64,
    last_progress: Instant,
    complete: bool,
}

impl fmt::Debug for UploadObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadObserver")
            .field("total", &self.0.total)
            .finish_non_exhaustive()
    }
}

impl UploadObserver {
    pub fn new(total: u64, callback: impl Fn(UploadEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(UploadObserverInner {
            total,
            callback: Box::new(callback),
            state: Mutex::new(UploadState {
                loaded: 0,
                last_progress: Instant::now(),
                complete: false,
            }),
        }))
    }

    pub(crate) fn request_headers_sent(&self) {
        // There is no positive curl upload count for a present, empty body.
        // Its request headers, unlike a response, establish that it was sent.
        if self.0.total == 0 {
            self.observe(0);
        }
    }

    pub(crate) fn bytes_sent(&self, loaded: u64) {
        if loaded > 0 {
            self.observe(loaded);
        }
    }

    fn observe(&self, loaded: u64) {
        let total = self.0.total;
        let (progress, completion) = {
            let mut state = self.0.state.lock();
            if state.complete {
                return;
            }
            let loaded = loaded.min(total);
            let progress = if loaded > state.loaded {
                state.loaded = loaded;
                let now = Instant::now();
                if now.duration_since(state.last_progress) >= Duration::from_millis(50) {
                    state.last_progress = now;
                    Some(UploadEvent::Progress { loaded, total })
                } else {
                    None
                }
            } else {
                None
            };
            let completion = (loaded == total).then(|| {
                state.complete = true;
                UploadEvent::Complete { loaded, total }
            });
            (progress, completion)
        };
        // Keep the byte-progress and end-of-body tasks distinct. In particular,
        // a consumer can abort while processing the former before the latter.
        for event in progress.into_iter().chain(completion) {
            (self.0.callback)(event);
        }
    }
}
