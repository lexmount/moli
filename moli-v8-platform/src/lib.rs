//! V8 platform foreground-task routing shared by Moli V8 users.
//!
//! V8 calls `PlatformImpl` from arbitrary background threads when foreground
//! work becomes ready for an isolate. This crate owns the isolate-to-runtime
//! registry and the RAII registration token used to keep that registry in sync
//! with V8 isolate lifetimes.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crossbeam_utils::atomic::AtomicCell;
use parking_lot::Mutex;
use tracing::trace;

const MAX_V8_BACKGROUND_WORKER_THREADS: usize = 8;

mod cpu_tracing;
mod environment;

pub use environment::{
    ProcessEnvironmentChange, ProcessEnvironmentNotifications, ProcessEnvironmentNotifier,
    ProcessEnvironmentOwner,
};

pub use cpu_tracing::{
    PendingV8CpuTraceStart, PendingV8CpuTraceStop, V8CpuProfileSegment, V8CpuTraceConfiguration,
    V8CpuTraceResult, V8CpuTraceSession, V8CpuTraceStartError, V8CpuTraceStartStatus,
    start_v8_cpu_trace,
};

#[derive(Clone)]
pub struct V8ForegroundTaskWake {
    dispatch: Arc<dyn Fn(V8ForegroundTask) + Send + Sync + 'static>,
}

impl V8ForegroundTaskWake {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            dispatch: Arc::new(move |task| {
                if task.run() {
                    wake();
                }
            }),
        }
    }

    /// Transfers foreground work to an isolate owner that will enter the
    /// isolate before calling [`V8ForegroundTask::run`].
    pub fn queued(dispatch: impl Fn(V8ForegroundTask) + Send + Sync + 'static) -> Self {
        Self {
            dispatch: Arc::new(dispatch),
        }
    }

    fn send(&self, task: V8ForegroundTask) {
        (self.dispatch)(task);
    }
}

impl fmt::Debug for V8ForegroundTaskWake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V8ForegroundTaskWake")
            .finish_non_exhaustive()
    }
}

enum V8ForegroundTaskKind {
    Task(v8::Task),
    IdleTask(v8::IdleTask),
    Callback(Box<dyn FnOnce() + Send + 'static>),
}

/// A foreground task tied to one live isolate registration generation.
///
/// Queued page owners execute this only after entering their own isolate. The
/// generation check also makes a task already transferred out of the platform
/// a no-op after unregister.
pub struct V8ForegroundTask {
    kind: V8ForegroundTaskKind,
    generation: IsolateRegistrationGeneration,
}

impl V8ForegroundTask {
    fn task(task: v8::Task, generation: IsolateRegistrationGeneration) -> Self {
        Self {
            kind: V8ForegroundTaskKind::Task(task),
            generation,
        }
    }

    fn idle_task(task: v8::IdleTask, generation: IsolateRegistrationGeneration) -> Self {
        Self {
            kind: V8ForegroundTaskKind::IdleTask(task),
            generation,
        }
    }

    fn callback(
        callback: impl FnOnce() + Send + 'static,
        generation: IsolateRegistrationGeneration,
    ) -> Self {
        Self {
            kind: V8ForegroundTaskKind::Callback(Box::new(callback)),
            generation,
        }
    }

    /// Runs the task if its exact isolate registration is still active.
    /// Returns whether V8 work was executed.
    pub fn run(self) -> bool {
        if !self.generation.is_active() {
            return false;
        }
        match self.kind {
            V8ForegroundTaskKind::Task(task) => task.run(),
            V8ForegroundTaskKind::IdleTask(task) => task.run(0.0),
            V8ForegroundTaskKind::Callback(callback) => callback(),
        }
        true
    }
}

impl fmt::Debug for V8ForegroundTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V8ForegroundTask")
            .field("active", &self.generation.is_active())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct IsolateRuntimeRegistration {
    handle: tokio::runtime::Handle,
    wake: V8ForegroundTaskWake,
    generation: IsolateRegistrationGeneration,
    environment_notifications: ProcessEnvironmentNotifier,
}

#[derive(Clone)]
struct IsolateRegistrationGeneration {
    id: u64,
    active: Arc<AtomicBool>,
}

impl IsolateRegistrationGeneration {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    fn id(&self) -> u64 {
        self.id
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.active.store(false, Ordering::Release);
    }

    fn is_same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.active, &other.active)
    }
}

type IsolateRegistry = HashMap<usize, IsolateRuntimeRegistration>;

const _: () = {
    assert!(
        std::mem::size_of::<v8::UnsafeRawIsolatePtr>()
            == std::mem::size_of::<*mut std::ffi::c_void>()
    );
    assert!(
        std::mem::align_of::<v8::UnsafeRawIsolatePtr>()
            == std::mem::align_of::<*mut std::ffi::c_void>()
    );
};

/// Global registry mapping V8 isolate raw pointers to Tokio runtime handles.
///
/// `PlatformImpl` is invoked from arbitrary V8 background threads, so we need
/// a `Send + Sync` way to look up the correct runtime. The registry stores
/// address keys derived from V8 isolate pointers because raw pointers are not
/// `Send`/`Sync`. The address is only the live-registration routing key:
/// `OwnedIsolate::drop` notifies the V8 platform of isolate shutdown before
/// disposal, while `IsolateRegistrationGeneration` distinguishes and cancels
/// work that was already transferred to Tokio before unregister.
static ISOLATE_RUNTIME_REGISTRY: LazyLock<Mutex<IsolateRegistry>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn registry_map() -> parking_lot::MutexGuard<'static, IsolateRegistry> {
    ISOLATE_RUNTIME_REGISTRY.lock()
}

fn unsafe_raw_isolate_addr(isolate_ptr: v8::UnsafeRawIsolatePtr) -> usize {
    // SAFETY: the local v8 crate declares `UnsafeRawIsolatePtr` as a
    // repr(transparent) raw isolate pointer wrapper, but exposes no address
    // accessor. The const assertions above make incompatible pointer-sized
    // layout changes fail at compile time. We only read the pointer value as an
    // address key; ownership and lifetime remain with the `OwnedIsolate`.
    let raw: *mut std::ffi::c_void = unsafe { std::mem::transmute(isolate_ptr) };
    raw.addr()
}

fn unsafe_raw_isolate_ptr_from_addr(isolate_addr: usize) -> v8::UnsafeRawIsolatePtr {
    let raw = std::ptr::with_exposed_provenance_mut::<std::ffi::c_void>(isolate_addr);
    // SAFETY: inverse of `unsafe_raw_isolate_addr`. Callers only reconstruct a
    // pointer while the exact registration generation remains active.
    unsafe { std::mem::transmute(raw) }
}

#[derive(Clone)]
struct RegisteredIsolateOwner {
    isolate_addr: usize,
    registration: IsolateRuntimeRegistration,
}

fn registered_isolate_owners() -> Vec<RegisteredIsolateOwner> {
    registry_map()
        .iter()
        .map(|(&isolate_addr, registration)| RegisteredIsolateOwner {
            isolate_addr,
            registration: registration.clone(),
        })
        .collect()
}

fn registered_isolate_owner(
    isolate_addr: usize,
    registration_id: u64,
) -> Option<RegisteredIsolateOwner> {
    let registration = registry_map().get(&isolate_addr)?.clone();
    (registration.generation.id() == registration_id).then_some(RegisteredIsolateOwner {
        isolate_addr,
        registration,
    })
}

fn dispatch_isolate_owner_callback(
    owner: RegisteredIsolateOwner,
    callback: impl FnOnce() + Send + 'static,
) {
    let RegisteredIsolateOwner {
        isolate_addr,
        registration:
            IsolateRuntimeRegistration {
                handle,
                wake,
                generation,
                ..
            },
    } = owner;
    handle.spawn(async move {
        if !generation.is_active() {
            trace!(
                isolate = isolate_addr,
                "dropping owner callback: isolate generation is inactive"
            );
            return;
        }
        wake.send(V8ForegroundTask::callback(callback, generation));
    });
}

/// Register an isolate and notify its owning loop after foreground tasks run.
///
/// Captures the current Tokio runtime handle. Registration must happen from
/// inside the isolate owner's runtime so foreground tasks have a correct
/// execution target.
fn register_isolate_with_wake(
    isolate_ptr: v8::UnsafeRawIsolatePtr,
    wake: V8ForegroundTaskWake,
    environment_notifications: ProcessEnvironmentNotifier,
) -> IsolateRegistrationGeneration {
    let isolate_key = unsafe_raw_isolate_addr(isolate_ptr);
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        panic!("V8 isolate registration requires a Tokio runtime; isolate={isolate_key}");
    };
    let generation = IsolateRegistrationGeneration::new();
    let mut guard = registry_map();
    if let Some(previous) = guard.insert(
        isolate_key,
        IsolateRuntimeRegistration {
            handle,
            wake,
            generation: generation.clone(),
            environment_notifications,
        },
    ) {
        // A raw isolate address may be reused after disposal. Invalidate queued
        // work from the previous generation before publishing the replacement.
        previous.generation.cancel();
        previous.environment_notifications.close();
    }
    drop(guard);
    trace!(isolate = isolate_key, "registered isolate with V8 platform");
    cpu_tracing::isolate_registered(isolate_key, generation.clone());
    generation
}

fn lookup_registration(isolate_ptr: *mut std::ffi::c_void) -> Option<IsolateRuntimeRegistration> {
    let isolate_key = isolate_ptr.addr();
    let guard = registry_map();
    guard.get(&isolate_key).cloned()
}

pub struct V8PlatformIsolateRegistration {
    isolate_ptr: AtomicCell<v8::UnsafeRawIsolatePtr>,
    generation: IsolateRegistrationGeneration,
}

impl V8PlatformIsolateRegistration {
    /// Register an isolate and its owner task routes. The environment mailbox
    /// belongs to this exact isolate; its wake must schedule owner work even
    /// while paused, never synchronously enter V8. Unregister closes the mailbox.
    pub fn register(
        isolate: &mut v8::OwnedIsolate,
        wake: V8ForegroundTaskWake,
        environment_notifications: ProcessEnvironmentNotifier,
    ) -> Self {
        // SAFETY: `OwnedIsolate::as_raw_isolate_ptr` returns the V8 isolate
        // pointer used by platform foreground-task callbacks. Ownership remains
        // with `OwnedIsolate`; this registration stores only the address value.
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        let generation = register_isolate_with_wake(isolate_ptr, wake, environment_notifications);
        // Bootstrap may have primed caches before registration. Register first,
        // then reset once: changes before registration are read from ICU, and
        // changes racing this reset already have an owner notification queued.
        ProcessEnvironmentChange::LocaleChanged.notify_isolate(isolate);
        Self {
            isolate_ptr: AtomicCell::new(isolate_ptr),
            generation,
        }
    }

    pub fn unregister(&self) {
        let isolate_ptr = self.isolate_ptr.load();
        if !isolate_ptr.is_null() {
            cpu_tracing::isolate_unregistering(
                unsafe_raw_isolate_addr(isolate_ptr),
                self.generation.clone(),
            );
        }
        // Queued Tokio tasks retain a clone of this generation token. Cancel it
        // before removing the registry entry so already-posted foreground work
        // is dropped instead of entering a disposed isolate.
        self.generation.cancel();
        let isolate_ptr = self.isolate_ptr.swap(v8::UnsafeRawIsolatePtr::null());
        if !isolate_ptr.is_null() {
            let isolate_key = unsafe_raw_isolate_addr(isolate_ptr);
            let mut registry = registry_map();
            if registry.get(&isolate_key).is_some_and(|registration| {
                registration.generation.is_same_generation(&self.generation)
            }) && let Some(registration) = registry.remove(&isolate_key)
            {
                registration.environment_notifications.close();
            }
            trace!(
                isolate = isolate_key,
                "unregistered isolate from V8 platform"
            );
        }
    }
}

impl Drop for V8PlatformIsolateRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// Custom V8 platform implementation.
///
/// Background thread pool management is still handled by the underlying
/// `DefaultPlatform` created by `v8::new_custom_platform`. This impl only
/// controls how foreground tasks are dispatched.
pub struct MoliPlatformImpl;

impl v8::PlatformImpl for MoliPlatformImpl {
    fn post_task(&self, isolate_ptr: *mut std::ffi::c_void, task: v8::Task) {
        if let Some(registration) = lookup_registration(isolate_ptr) {
            let isolate_key = isolate_ptr.addr();
            let IsolateRuntimeRegistration {
                handle,
                wake,
                generation,
                ..
            } = registration;
            handle.spawn(async move {
                if !generation.is_active() {
                    trace!(
                        isolate = isolate_key,
                        "dropping queued foreground task: isolate generation is inactive"
                    );
                    return;
                }
                wake.send(V8ForegroundTask::task(task, generation));
            });
        } else {
            // Isolate not registered (e.g. during V8 init, in unit tests, or
            // after shutdown). Dropping the task is safe: the v8 crate
            // guarantees `Task` can be dropped without running. Running inline
            // is not safe because V8 may call post_task from a GC background
            // thread where re-entering the isolate would violate invariants.
            trace!(
                isolate = isolate_ptr.addr(),
                "dropping foreground task: no runtime handle registered"
            );
        }
    }

    fn post_non_nestable_task(&self, isolate_ptr: *mut std::ffi::c_void, task: v8::Task) {
        self.post_task(isolate_ptr, task);
    }

    fn post_delayed_task(
        &self,
        isolate_ptr: *mut std::ffi::c_void,
        task: v8::Task,
        delay_in_seconds: f64,
    ) {
        if let Some(registration) = lookup_registration(isolate_ptr) {
            let isolate_key = isolate_ptr.addr();
            let IsolateRuntimeRegistration {
                handle,
                wake,
                generation,
                ..
            } = registration;
            let delay = Duration::from_secs_f64(delay_in_seconds);
            handle.spawn(async move {
                tokio::time::sleep(delay).await;
                if !generation.is_active() {
                    trace!(
                        isolate = isolate_key,
                        "dropping delayed foreground task: isolate generation is inactive"
                    );
                    return;
                }
                wake.send(V8ForegroundTask::task(task, generation));
            });
        } else {
            trace!(
                isolate = isolate_ptr.addr(),
                "dropping delayed foreground task: no runtime handle registered"
            );
        }
    }

    fn post_non_nestable_delayed_task(
        &self,
        isolate_ptr: *mut std::ffi::c_void,
        task: v8::Task,
        delay_in_seconds: f64,
    ) {
        self.post_delayed_task(isolate_ptr, task, delay_in_seconds);
    }

    fn post_idle_task(&self, isolate_ptr: *mut std::ffi::c_void, task: v8::IdleTask) {
        if let Some(registration) = lookup_registration(isolate_ptr) {
            let isolate_key = isolate_ptr.addr();
            let IsolateRuntimeRegistration {
                handle,
                wake,
                generation,
                ..
            } = registration;
            handle.spawn(async move {
                if !generation.is_active() {
                    trace!(
                        isolate = isolate_key,
                        "dropping idle foreground task: isolate generation is inactive"
                    );
                    return;
                }
                wake.send(V8ForegroundTask::idle_task(task, generation));
            });
        } else {
            trace!(
                isolate = isolate_ptr.addr(),
                "dropping idle foreground task: no runtime handle registered"
            );
        }
    }
}

/// Returns the V8 background-worker budget for the CPU quota visible to this
/// process. V8 background jobs may block or perform CPU-heavy GC and compile
/// work, so they keep their own pool; the explicit cap prevents the default
/// platform from scaling that pool to a large host's full processor count.
fn v8_background_worker_thread_count() -> u32 {
    std::thread::available_parallelism()
        .map(v8_background_worker_thread_count_for_parallelism)
        .unwrap_or(1)
}

fn v8_background_worker_thread_count_for_parallelism(parallelism: std::num::NonZeroUsize) -> u32 {
    u32::try_from(parallelism.get().min(MAX_V8_BACKGROUND_WORKER_THREADS))
        .expect("the capped V8 background worker count must fit in u32")
}

pub fn create_platform() -> v8::SharedRef<v8::Platform> {
    v8::new_custom_platform(
        v8_background_worker_thread_count(),
        false,
        false,
        MoliPlatformImpl,
    )
    .make_shared()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;

    fn fake_isolate_ptr(raw: *mut std::ffi::c_void) -> v8::UnsafeRawIsolatePtr {
        // SAFETY: test-only mirror of `unsafe_raw_isolate_addr`; the fake
        // pointer is never dereferenced and is used only as a registry key.
        unsafe { std::mem::transmute(raw) }
    }

    #[test]
    fn v8_worker_thread_budget_tracks_small_hosts_and_caps_large_hosts() {
        assert_eq!(
            v8_background_worker_thread_count_for_parallelism(NonZeroUsize::new(1).unwrap()),
            1
        );
        assert_eq!(
            v8_background_worker_thread_count_for_parallelism(NonZeroUsize::new(6).unwrap()),
            6
        );
        assert_eq!(
            v8_background_worker_thread_count_for_parallelism(NonZeroUsize::new(8).unwrap()),
            8
        );
        assert_eq!(
            v8_background_worker_thread_count_for_parallelism(NonZeroUsize::new(32).unwrap()),
            8
        );
    }

    #[test]
    #[should_panic(expected = "V8 isolate registration requires a Tokio runtime")]
    fn register_isolate_with_wake_panics_without_tokio_runtime() {
        let _ = register_isolate_with_wake(
            v8::UnsafeRawIsolatePtr::null(),
            V8ForegroundTaskWake::new(|| {}),
            ProcessEnvironmentNotifications::default().notifier(|| {}),
        );
    }

    #[test]
    fn unregister_invalidates_cloned_registration_for_already_queued_work() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime should build");
        let mut marker = 0_u8;
        let raw = (&mut marker as *mut u8).cast::<std::ffi::c_void>();
        let isolate_ptr = fake_isolate_ptr(raw);

        runtime.block_on(async {
            let notifications = ProcessEnvironmentNotifications::default();
            let notifier = notifications.notifier(|| {});
            let generation = register_isolate_with_wake(
                isolate_ptr,
                V8ForegroundTaskWake::new(|| {}),
                notifier.clone(),
            );
            let registration = lookup_registration(raw)
                .expect("registered isolate should expose its runtime registration");
            let owner = V8PlatformIsolateRegistration {
                isolate_ptr: AtomicCell::new(isolate_ptr),
                generation,
            };

            owner.unregister();
            notifier.notify(ProcessEnvironmentChange::LocaleChanged);
            assert_eq!(
                notifications.take(),
                None,
                "unregister closes retained mailbox clones"
            );

            assert!(
                !registration.generation.is_active(),
                "queued registration clones must observe isolate unregister"
            );
            assert!(lookup_registration(raw).is_none());
        });
    }

    #[test]
    fn stale_registration_drop_does_not_remove_replacement_generation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime should build");
        let mut marker = 0_u8;
        let raw = (&mut marker as *mut u8).cast::<std::ffi::c_void>();
        let isolate_ptr = fake_isolate_ptr(raw);

        runtime.block_on(async {
            let first_generation = register_isolate_with_wake(
                isolate_ptr,
                V8ForegroundTaskWake::new(|| {}),
                ProcessEnvironmentNotifications::default().notifier(|| {}),
            );
            let first = V8PlatformIsolateRegistration {
                isolate_ptr: AtomicCell::new(isolate_ptr),
                generation: first_generation.clone(),
            };

            let second_generation = register_isolate_with_wake(
                isolate_ptr,
                V8ForegroundTaskWake::new(|| {}),
                ProcessEnvironmentNotifications::default().notifier(|| {}),
            );
            let second = V8PlatformIsolateRegistration {
                isolate_ptr: AtomicCell::new(isolate_ptr),
                generation: second_generation.clone(),
            };

            assert!(!first_generation.is_active());
            assert!(second_generation.is_active());
            drop(first);
            let registration = lookup_registration(raw)
                .expect("stale registration drop must preserve the replacement");
            assert!(
                registration
                    .generation
                    .is_same_generation(&second_generation)
            );

            second.unregister();
            assert!(lookup_registration(raw).is_none());
        });
    }
}
