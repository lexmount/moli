use parking_lot::Mutex;
use std::{
    fmt,
    sync::{Arc, Weak},
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
    callback: UploadCallback,
    state: Mutex<UploadState>,
}

enum UploadCallback {
    Transmission(Box<dyn Fn(UploadEvent) + Send + Sync>),
    Browser(Box<dyn Fn(UploadReport) + Send + Sync>),
}

const BROWSER_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy)]
enum BrowserSchedule {
    Dormant,
    Running(Instant),
    Finished,
}

struct UploadState {
    loaded: u64,
    last_progress: Instant,
    complete: bool,
    browser: Option<BrowserSchedule>,
    reported: u64,
    waiting_for_ack: bool,
    generation: u64,
}

/// A browser upload notification, acknowledged when its script owner applies
/// or discards it. Dropping a stale report cannot release a newer report, and
/// a queued report does not retain its request.
pub struct UploadReport {
    pub event: UploadEvent,
    observer: Weak<UploadObserverInner>,
    generation: u64,
}

impl fmt::Debug for UploadReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadReport")
            .field("event", &self.event)
            .finish_non_exhaustive()
    }
}

impl Drop for UploadReport {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.upgrade() {
            let mut state = observer.state.lock();
            if state.generation == self.generation {
                state.waiting_for_ack = false;
            }
        }
    }
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
            callback: UploadCallback::Transmission(Box::new(callback)),
            state: Mutex::new(UploadState {
                loaded: 0,
                last_progress: Instant::now(),
                complete: false,
                browser: None,
                reported: 0,
                waiting_for_ack: false,
                generation: 0,
            }),
        }))
    }

    /// Browser upload reports use the network service's periodic cadence,
    /// owner acknowledgements and final response-start flush. Transmission
    /// itself continues independently of reporting or the script owner.
    pub fn new_browser(
        total: u64,
        callback: impl Fn(UploadReport) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(UploadObserverInner {
            total,
            callback: UploadCallback::Browser(Box::new(callback)),
            state: Mutex::new(UploadState {
                loaded: 0,
                last_progress: Instant::now(),
                complete: false,
                browser: Some(BrowserSchedule::Dormant),
                reported: 0,
                waiting_for_ack: false,
                generation: 0,
            }),
        }))
    }

    pub(crate) fn begin_transfer(&self) {
        let mut state = self.0.state.lock();
        if state.browser.is_some() && !state.complete {
            state.browser = Some(BrowserSchedule::Running(
                Instant::now() + BROWSER_PROGRESS_INTERVAL,
            ));
            state.waiting_for_ack = false;
            state.generation = state.generation.wrapping_add(1);
        }
    }

    pub(crate) fn report_if_due(&self) {
        let now = Instant::now();
        let report = {
            let mut state = self.0.state.lock();
            let Some(BrowserSchedule::Running(deadline)) = state.browser else {
                return;
            };
            if now < deadline {
                return;
            }
            state.browser = Some(BrowserSchedule::Running(now + BROWSER_PROGRESS_INTERVAL));
            self.prepare_browser_report(&mut state, now, false)
        };
        self.publish_browser_report(report);
    }

    pub(crate) fn finish_reporting(&self) {
        let report = {
            let mut state = self.0.state.lock();
            if !matches!(state.browser, Some(BrowserSchedule::Running(_))) {
                return;
            }
            let report = self.prepare_browser_report(&mut state, Instant::now(), true);
            state.browser = Some(BrowserSchedule::Finished);
            report
        };
        self.publish_browser_report(report);
    }

    fn prepare_browser_report(
        &self,
        state: &mut UploadState,
        now: Instant,
        final_report: bool,
    ) -> Option<(UploadEvent, u64)> {
        if final_report {
            state.waiting_for_ack = false;
        }
        let total = self.0.total;
        if state.waiting_for_ack || total == 0 || state.loaded <= state.reported {
            return None;
        }
        let loaded = state.loaded;
        let finished = loaded == total;
        if !finished
            && state.reported != 0
            && loaded - state.reported <= total / 200
            && now.duration_since(state.last_progress) <= Duration::from_secs(1)
        {
            return None;
        }
        state.reported = loaded;
        state.last_progress = now;
        state.complete = finished;
        state.waiting_for_ack = true;
        state.generation = state.generation.wrapping_add(1);
        if finished {
            state.browser = Some(BrowserSchedule::Finished);
        }
        let event = if finished {
            UploadEvent::Complete { loaded, total }
        } else {
            UploadEvent::Progress { loaded, total }
        };
        Some((event, state.generation))
    }

    fn publish_browser_report(&self, report: Option<(UploadEvent, u64)>) {
        if let Some((event, generation)) = report
            && let UploadCallback::Browser(callback) = &self.0.callback
        {
            callback(UploadReport {
                event,
                observer: Arc::downgrade(&self.0),
                generation,
            });
        }
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
            if let Some(schedule) = &state.browser {
                if !matches!(schedule, BrowserSchedule::Finished) {
                    state.loaded = state.loaded.max(loaded);
                }
                return;
            }
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
            if let UploadCallback::Transmission(callback) = &self.0.callback {
                callback(event);
            }
        }
    }
}
