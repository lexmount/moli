//! Process-stable timeout ownership for synchronous V8 execution.
//!
//! JavaScript execution can hold the renderer owner thread indefinitely, so a
//! timeout cannot depend on that owner, its Tokio runtime, or a V8 foreground
//! task being polled. The service below owns one process-lifetime thread and
//! accepts short-lived, exact-isolate registrations from script, timer, and
//! lifecycle task bodies.
//!
//! A registration is removed when its guard is disarmed or dropped. Expiry and
//! disarm serialize on the registration itself, which guarantees that a
//! cancellation can never race ahead of a later `terminate_execution()` call.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

#[cfg(not(test))]
pub(crate) const SCRIPT_TURN_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(test)]
// Keep runaway-script tests substantially faster than production without
// terminating finite debug-build checkpoints merely because workspace
// nextest is concurrently running other V8-heavy processes.
pub(crate) const SCRIPT_TURN_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum V8ExecutionWatchdogKind {
    ScriptTurn,
    TimerCallback,
    LifecycleEvent,
}

#[cfg(test)]
impl V8ExecutionWatchdogKind {
    const fn override_index(self) -> usize {
        match self {
            Self::ScriptTurn => 0,
            Self::TimerCallback => 1,
            Self::LifecycleEvent => 2,
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static V8_EXECUTION_WATCHDOG_TIMEOUT_OVERRIDES:
        std::cell::Cell<[Option<Duration>; 3]> =
            const { std::cell::Cell::new([None, None, None]) };
}

#[cfg(test)]
fn v8_execution_watchdog_timeout(kind: V8ExecutionWatchdogKind, fallback: Duration) -> Duration {
    V8_EXECUTION_WATCHDOG_TIMEOUT_OVERRIDES
        .with(|overrides| overrides.get()[kind.override_index()].unwrap_or(fallback))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum V8ExecutionWatchdogOutcome {
    Completed,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct V8ExecutionWatchdogRegistrationKey {
    deadline: Instant,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum V8ExecutionWatchdogRegistrationState {
    Armed,
    Completed,
    TimedOut,
}

struct V8ExecutionWatchdogRegistration {
    kind: V8ExecutionWatchdogKind,
    isolate: v8::IsolateHandle,
    state: Mutex<V8ExecutionWatchdogRegistrationState>,
}

impl V8ExecutionWatchdogRegistration {
    fn expire_if_armed(&self) {
        let mut state = self.state.lock();
        if *state != V8ExecutionWatchdogRegistrationState::Armed {
            return;
        }

        // Keep the registration lock across the termination request. A
        // concurrent disarm therefore either wins before this call and makes
        // it unreachable, or observes TimedOut only after termination has
        // already been delivered and can safely cancel it.
        let target_was_live = self.isolate.terminate_execution();
        *state = V8ExecutionWatchdogRegistrationState::TimedOut;
        tracing::warn!(
            target: "moli_v8_watchdog",
            kind = ?self.kind,
            target_was_live,
            "V8 execution watchdog deadline elapsed"
        );
    }

    fn disarm(&self) -> V8ExecutionWatchdogOutcome {
        let mut state = self.state.lock();
        match *state {
            V8ExecutionWatchdogRegistrationState::Armed => {
                *state = V8ExecutionWatchdogRegistrationState::Completed;
                V8ExecutionWatchdogOutcome::Completed
            }
            V8ExecutionWatchdogRegistrationState::TimedOut => {
                // `expire_if_armed()` holds this same lock until its
                // termination request has completed. Cancellation therefore
                // cannot be followed by a late termination from this
                // registration.
                let _ = self.isolate.cancel_terminate_execution();
                V8ExecutionWatchdogOutcome::TimedOut
            }
            V8ExecutionWatchdogRegistrationState::Completed => {
                V8ExecutionWatchdogOutcome::Completed
            }
        }
    }
}

#[derive(Default)]
struct V8ExecutionWatchdogServiceState {
    next_sequence: u64,
    registrations:
        BTreeMap<V8ExecutionWatchdogRegistrationKey, Arc<V8ExecutionWatchdogRegistration>>,
}

#[derive(Default)]
struct V8ExecutionWatchdogServiceShared {
    state: Mutex<V8ExecutionWatchdogServiceState>,
    changed: Condvar,
}

struct V8ExecutionWatchdogService {
    shared: Arc<V8ExecutionWatchdogServiceShared>,
}

impl V8ExecutionWatchdogService {
    fn global() -> &'static Self {
        static SERVICE: OnceLock<V8ExecutionWatchdogService> = OnceLock::new();
        SERVICE.get_or_init(Self::start)
    }

    fn start() -> Self {
        let shared = Arc::new(V8ExecutionWatchdogServiceShared::default());
        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("lm-v8-watchdog".to_owned())
            .spawn(move || run_v8_execution_watchdog_service(worker_shared))
            .expect("the process V8 execution watchdog service must start");
        #[cfg(test)]
        V8_EXECUTION_WATCHDOG_SERVICE_THREAD_STARTS
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self { shared }
    }

    fn register(
        &self,
        kind: V8ExecutionWatchdogKind,
        isolate: v8::IsolateHandle,
        timeout: Duration,
    ) -> (
        V8ExecutionWatchdogRegistrationKey,
        Arc<V8ExecutionWatchdogRegistration>,
    ) {
        let deadline = Instant::now()
            .checked_add(timeout)
            .expect("a V8 execution watchdog deadline must fit in Instant");
        let registration = Arc::new(V8ExecutionWatchdogRegistration {
            kind,
            isolate,
            state: Mutex::new(V8ExecutionWatchdogRegistrationState::Armed),
        });
        let key = {
            let mut state = self.shared.state.lock();
            let sequence = state.next_sequence;
            state.next_sequence = state
                .next_sequence
                .checked_add(1)
                .expect("V8 execution watchdog registration sequence exhausted");
            let key = V8ExecutionWatchdogRegistrationKey { deadline, sequence };
            let previous = state.registrations.insert(key, Arc::clone(&registration));
            assert!(
                previous.is_none(),
                "a V8 execution watchdog registration key must be unique"
            );
            key
        };
        self.shared.changed.notify_one();
        (key, registration)
    }

    fn remove(&self, key: V8ExecutionWatchdogRegistrationKey) {
        let removed = self.shared.state.lock().registrations.remove(&key);
        if removed.is_some() {
            // The removed registration may have been the earliest deadline.
            // Wake the worker so it can immediately recompute its next wait.
            self.shared.changed.notify_one();
        }
    }
}

fn run_v8_execution_watchdog_service(shared: Arc<V8ExecutionWatchdogServiceShared>) {
    loop {
        let registration = {
            let mut state = shared.state.lock();
            loop {
                let Some((&key, _)) = state.registrations.first_key_value() else {
                    shared.changed.wait(&mut state);
                    continue;
                };
                let now = Instant::now();
                if key.deadline > now {
                    shared.changed.wait_for(&mut state, key.deadline - now);
                    continue;
                }
                break state.registrations.remove(&key);
            }
        };

        if let Some(registration) = registration {
            registration.expire_if_armed();
        }
    }
}

pub(crate) struct V8ExecutionWatchdog {
    key: V8ExecutionWatchdogRegistrationKey,
    registration: Option<Arc<V8ExecutionWatchdogRegistration>>,
    timeout: Duration,
}

impl V8ExecutionWatchdog {
    pub(crate) fn arm(
        kind: V8ExecutionWatchdogKind,
        isolate: v8::IsolateHandle,
        timeout: Duration,
    ) -> Self {
        #[cfg(test)]
        let timeout = v8_execution_watchdog_timeout(kind, timeout);
        let (key, registration) =
            V8ExecutionWatchdogService::global().register(kind, isolate, timeout);
        Self {
            key,
            registration: Some(registration),
            timeout,
        }
    }

    #[cfg(test)]
    pub(crate) fn override_timeout_for_test(
        kind: V8ExecutionWatchdogKind,
        timeout: Duration,
    ) -> V8ExecutionWatchdogTimeoutOverride {
        assert!(!timeout.is_zero(), "watchdog timeout must be non-zero");
        let previous = V8_EXECUTION_WATCHDOG_TIMEOUT_OVERRIDES.with(|overrides| {
            let mut values = overrides.get();
            let previous = values[kind.override_index()].replace(timeout);
            overrides.set(values);
            previous
        });
        V8ExecutionWatchdogTimeoutOverride { kind, previous }
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(crate) fn disarm(mut self) -> V8ExecutionWatchdogOutcome {
        self.disarm_inner()
    }

    fn disarm_inner(&mut self) -> V8ExecutionWatchdogOutcome {
        let Some(registration) = self.registration.take() else {
            return V8ExecutionWatchdogOutcome::Completed;
        };
        V8ExecutionWatchdogService::global().remove(self.key);
        registration.disarm()
    }
}

impl Drop for V8ExecutionWatchdog {
    fn drop(&mut self) {
        let _ = self.disarm_inner();
    }
}

#[cfg(test)]
pub(crate) struct V8ExecutionWatchdogTimeoutOverride {
    kind: V8ExecutionWatchdogKind,
    previous: Option<Duration>,
}

#[cfg(test)]
impl Drop for V8ExecutionWatchdogTimeoutOverride {
    fn drop(&mut self) {
        V8_EXECUTION_WATCHDOG_TIMEOUT_OVERRIDES.with(|overrides| {
            let mut values = overrides.get();
            values[self.kind.override_index()] = self.previous;
            overrides.set(values);
        });
    }
}

#[cfg(test)]
static V8_EXECUTION_WATCHDOG_SERVICE_THREAD_STARTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_override_is_kind_scoped_and_restored() {
        let fallback = Duration::from_secs(8);
        assert_eq!(
            v8_execution_watchdog_timeout(V8ExecutionWatchdogKind::ScriptTurn, fallback),
            fallback
        );

        {
            let _outer = V8ExecutionWatchdog::override_timeout_for_test(
                V8ExecutionWatchdogKind::ScriptTurn,
                Duration::from_millis(500),
            );
            assert_eq!(
                v8_execution_watchdog_timeout(V8ExecutionWatchdogKind::ScriptTurn, fallback),
                Duration::from_millis(500)
            );
            assert_eq!(
                v8_execution_watchdog_timeout(V8ExecutionWatchdogKind::TimerCallback, fallback),
                fallback
            );
            {
                let _inner = V8ExecutionWatchdog::override_timeout_for_test(
                    V8ExecutionWatchdogKind::ScriptTurn,
                    Duration::from_millis(250),
                );
                assert_eq!(
                    v8_execution_watchdog_timeout(V8ExecutionWatchdogKind::ScriptTurn, fallback),
                    Duration::from_millis(250)
                );
            }
            assert_eq!(
                v8_execution_watchdog_timeout(V8ExecutionWatchdogKind::ScriptTurn, fallback),
                Duration::from_millis(500)
            );
        }

        assert_eq!(
            v8_execution_watchdog_timeout(V8ExecutionWatchdogKind::ScriptTurn, fallback),
            fallback
        );
    }

    fn evaluate_script(isolate: &mut v8::OwnedIsolate, source: &str) -> Option<String> {
        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = v8::String::new(scope, source)?;
        let script = v8::Script::compile(scope, source, None)?;
        let value = script.run(scope)?;
        Some(value.to_string(scope)?.to_rust_string_lossy(scope))
    }

    #[test]
    fn repeated_registrations_share_one_process_service_thread() {
        crate::ensure_v8_for_test();
        let isolate = v8::Isolate::new(Default::default());
        let handle = isolate.thread_safe_handle();

        for _ in 0..256 {
            let watchdog = V8ExecutionWatchdog::arm(
                V8ExecutionWatchdogKind::ScriptTurn,
                handle.clone(),
                Duration::from_secs(1),
            );
            assert_eq!(watchdog.disarm(), V8ExecutionWatchdogOutcome::Completed);
        }

        assert_eq!(
            V8_EXECUTION_WATCHDOG_SERVICE_THREAD_STARTS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "every V8 execution registration must reuse the process service"
        );
    }

    #[test]
    fn dropped_registration_cannot_late_terminate_its_exact_isolate() {
        crate::ensure_v8_for_test();

        let mut cancelled_isolate = v8::Isolate::new(Default::default());
        let cancelled_watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::ScriptTurn,
            cancelled_isolate.thread_safe_handle(),
            Duration::from_secs(1),
        );
        drop(cancelled_watchdog);

        // A second, deliberately non-terminating isolate advances the stable
        // service beyond the cancelled registration's former deadline without
        // relying on a sleep. Each registration retains its exact isolate
        // handle, so the second deadline must terminate only this isolate.
        let mut timed_out_isolate = v8::Isolate::new(Default::default());
        let timed_out_watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::ScriptTurn,
            timed_out_isolate.thread_safe_handle(),
            Duration::from_millis(1_250),
        );
        assert!(
            evaluate_script(&mut timed_out_isolate, "for (;;) {}").is_none(),
            "the deliberately non-terminating script must be interrupted"
        );
        assert_eq!(
            timed_out_watchdog.disarm(),
            V8ExecutionWatchdogOutcome::TimedOut
        );
        // `OwnedIsolate` instances enter V8 in stack order. Leave the inner
        // isolate before re-entering the older one below.
        drop(timed_out_isolate);

        assert_eq!(
            evaluate_script(&mut cancelled_isolate, "1 + 1").as_deref(),
            Some("2"),
            "a dropped registration must not leave a late termination request on its isolate"
        );
    }
}
