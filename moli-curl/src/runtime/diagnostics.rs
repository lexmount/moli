//! Optional owner-thread counters. Disabled diagnostics do not read the clock
//! per turn or per poll. Reports contain counts only, never connection contents.
use std::time::{Duration, Instant};

#[derive(Default, Debug)]
pub(crate) struct Counters {
    pub turns: u64,
    pub progressed_turns: u64,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub read_frames: u64,
    pub written_frames: u64,
    pub read_again: u64,
    pub write_again: u64,
    pub receive_allocations: u64,
    pub polls: u64,
    pub zero_timeout_polls: u64,
    pub requested_wait_ns: u128,
    pub actual_wait_ns: u128,
    pub max_wait_ns: u128,
    pub fast_no_io_polls: u64,
    fast_no_io_streak: u64,
    pub max_fast_no_io_streak: u64,
}

impl Counters {
    fn polled(&mut self, requested: Duration, actual: Duration, progressed: bool) {
        self.polls += 1;
        self.zero_timeout_polls += u64::from(requested.is_zero());
        self.requested_wait_ns += requested.as_nanos();
        self.actual_wait_ns += actual.as_nanos();
        self.max_wait_ns = self.max_wait_ns.max(actual.as_nanos());
        // This is a diagnostic hint, not a spin detector: command/handshake
        // wakeups can legitimately return early without WebSocket I/O progress.
        if !progressed && !requested.is_zero() && actual < Duration::from_micros(100) {
            self.fast_no_io_polls += 1;
            self.fast_no_io_streak += 1;
            self.max_fast_no_io_streak = self.max_fast_no_io_streak.max(self.fast_no_io_streak);
        } else {
            self.fast_no_io_streak = 0;
        }
    }
}

struct Window {
    started: Instant,
    counters: Counters,
}

pub(crate) struct Diagnostics(Option<Window>);

impl Diagnostics {
    pub(crate) fn from_env() -> Self {
        Self::new(
            std::env::var("MOLI_CURL_WEBSOCKET_DIAGNOSTICS").is_ok_and(|value| {
                let value = value.trim();
                !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
            }),
        )
    }

    fn new(enabled: bool) -> Self {
        Self(enabled.then(|| Window {
            started: Instant::now(),
            counters: Counters::default(),
        }))
    }

    pub(crate) fn counters(&mut self) -> Option<&mut Counters> {
        self.0.as_mut().map(|window| &mut window.counters)
    }

    pub(crate) fn poll_start(&self) -> Option<Instant> {
        self.0.as_ref().map(|_| Instant::now())
    }

    pub(crate) fn polled(&mut self, start: Option<Instant>, timeout: Duration, progressed: bool) {
        if let Some(start) = start {
            self.0
                .as_mut()
                .expect("enabled poll measurement")
                .counters
                .polled(timeout, start.elapsed(), progressed);
        }
    }

    pub(crate) fn report(&mut self, sessions: usize, finished: bool) {
        let Some(window) = &mut self.0 else {
            return;
        };
        let now = Instant::now();
        let elapsed = now.duration_since(window.started);
        if !finished && elapsed < Duration::from_secs(1) {
            return;
        }
        let c = &window.counters;
        tracing::info!(
            target: "moli_curl_websocket",
            owner_thread = ?std::thread::current().id(),
            window_ms = elapsed.as_millis(), sessions, finished,
            turns = c.turns, progressed_turns = c.progressed_turns,
            read_bytes = c.read_bytes, written_bytes = c.written_bytes,
            read_frames = c.read_frames, written_frames = c.written_frames,
            read_again = c.read_again, write_again = c.write_again,
            receive_allocations = c.receive_allocations,
            polls = c.polls, zero_timeout_polls = c.zero_timeout_polls,
            requested_wait_ns = c.requested_wait_ns, actual_wait_ns = c.actual_wait_ns,
            max_wait_ns = c.max_wait_ns, fast_no_io_polls = c.fast_no_io_polls,
            max_fast_no_io_streak = c.max_fast_no_io_streak,
            "WebSocket owner work"
        );
        let streak = c.fast_no_io_streak;
        window.counters = Counters {
            fast_no_io_streak: streak,
            max_fast_no_io_streak: streak,
            ..Counters::default()
        };
        window.started = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn productive_zero_timeout_polls_are_distinct_from_fast_no_io_wakeups() {
        let mut counts = Counters::default();
        counts.polled(Duration::ZERO, Duration::ZERO, true);
        counts.polled(Duration::ZERO, Duration::ZERO, false);
        assert_eq!(counts.zero_timeout_polls, 2);
        assert_eq!(counts.fast_no_io_polls, 0);
        for _ in 0..3 {
            counts.polled(Duration::from_secs(1), Duration::from_micros(10), false);
        }
        assert_eq!(counts.fast_no_io_polls, 3);
        assert_eq!(counts.max_fast_no_io_streak, 3);
        counts.polled(Duration::from_secs(1), Duration::from_secs(1), false);
        counts.polled(Duration::from_secs(1), Duration::ZERO, false);
        assert_eq!(counts.fast_no_io_streak, 1);
        assert_eq!(counts.max_fast_no_io_streak, 3);
        assert_eq!(counts.polls, 7);
        assert_eq!(counts.requested_wait_ns, 5_000_000_000);
        assert_eq!(counts.actual_wait_ns, 1_000_030_000);
    }

    #[test]
    fn reports_reset_window_totals_but_preserve_a_consecutive_streak() {
        use parking_lot::Mutex;
        use std::{io::Write, sync::Arc};
        struct Writer(Arc<Mutex<Vec<u8>>>);
        impl Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || Writer(writer.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let mut diagnostics = Diagnostics::new(true);
            let counts = diagnostics.counters().unwrap();
            counts.turns = 4;
            counts.read_bytes = 17;
            for _ in 0..2 {
                counts.polled(Duration::from_secs(1), Duration::ZERO, false);
            }
            diagnostics.report(3, true);
            let counts = diagnostics.counters().unwrap();
            assert_eq!((counts.turns, counts.read_bytes, counts.polls), (0, 0, 0));
            counts.polled(Duration::from_secs(1), Duration::ZERO, false);
            assert_eq!(counts.max_fast_no_io_streak, 3);
            diagnostics.report(0, true);
            // A disabled owner produces neither counters nor reports.
            let mut disabled = Diagnostics::new(false);
            assert!(disabled.poll_start().is_none());
            disabled.report(100, true);
        });
        let output = String::from_utf8(output.lock().clone()).unwrap();
        assert_eq!(output.lines().count(), 2);
        assert!(output.contains("turns=4"));
        assert!(output.contains("read_bytes=17"));
        assert!(output.contains("max_fast_no_io_streak=3"));
    }
}
