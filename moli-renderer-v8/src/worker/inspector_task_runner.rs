use crate::runtime::WorkerInspectorError;
use anyhow::Result;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    ffi::c_void,
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use moli_protocol_cdp::CdpInspectorTaskMode;
use moli_v8_platform::{ProcessEnvironmentNotifications, ProcessEnvironmentNotifier};
use parking_lot::{Condvar, Mutex};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender};

use super::handle::WorkerMessage;

static NEXT_WORKER_INSPECTOR_ROUTE_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static WORKER_INSPECTOR_EXECUTORS: RefCell<HashMap<u64, Weak<dyn WorkerInspectorInterruptExecutor>>> =
        RefCell::new(HashMap::new());
}

pub(crate) trait WorkerInspectorInterruptExecutor {
    fn dispatch_interrupt(&self, isolate: v8::UnsafeRawIsolatePtr);
}

pub(crate) fn register_worker_inspector_executor(
    route_id: u64,
    executor: &Rc<dyn WorkerInspectorInterruptExecutor>,
) {
    let previous = WORKER_INSPECTOR_EXECUTORS.with(|executors| {
        executors
            .borrow_mut()
            .insert(route_id, Rc::downgrade(executor))
    });
    assert!(
        previous.is_none(),
        "worker Inspector executor route IDs must be unique"
    );
}

pub(crate) fn unregister_worker_inspector_executor(route_id: u64) {
    let _ = WORKER_INSPECTOR_EXECUTORS.try_with(|executors| {
        executors.borrow_mut().remove(&route_id);
    });
}

fn worker_inspector_executor(route_id: u64) -> Option<Rc<dyn WorkerInspectorInterruptExecutor>> {
    WORKER_INSPECTOR_EXECUTORS
        .try_with(|executors| executors.borrow().get(&route_id).and_then(Weak::upgrade))
        .ok()
        .flatten()
}

struct WorkerInspectorInterruptTarget {
    route_id: u64,
}

unsafe extern "C" fn dispatch_worker_inspector_interrupt(
    isolate: v8::UnsafeRawIsolatePtr,
    data: *mut c_void,
) {
    // SAFETY: every accepted V8 interrupt owns exactly one strong reference
    // created with `Arc::into_raw`. V8 invokes an accepted callback at most
    // once, so this callback is the unique consumer of that reference.
    let callback_target = unsafe { Arc::from_raw(data.cast::<WorkerInspectorInterruptTarget>()) };
    let Some(executor) = worker_inspector_executor(callback_target.route_id) else {
        return;
    };
    executor.dispatch_interrupt(isolate);
}

pub(crate) type WorkerInspectorTaskMode = CdpInspectorTaskMode;

/// One native session incarnation. Wire ids may be reused after attach; tasks
/// and channels retain this token so retired work cannot address its successor.
#[derive(Clone)]
pub(crate) struct WorkerInspectorSessionLifetime(Arc<WorkerInspectorSessionLifetimeState>);

struct WorkerInspectorSessionLifetimeState {
    active: AtomicBool,
    native_created: AtomicBool,
    detachment: Mutex<WorkerInspectorSessionCompletion>,
    retirement: Mutex<WorkerInspectorSessionCompletion>,
}

#[derive(Default)]
struct WorkerInspectorSessionCompletion {
    result: Option<std::result::Result<(), WorkerInspectorError>>,
    waiters: Vec<oneshot::Sender<Result<()>>>,
}

impl WorkerInspectorSessionCompletion {
    fn complete(&mut self, result: std::result::Result<(), WorkerInspectorError>) {
        if self.result.is_some() {
            return;
        }
        self.result = Some(result);
        for waiter in self.waiters.drain(..) {
            let _ = waiter.send(result.map_err(Into::into));
        }
    }

    fn subscribe(&mut self) -> oneshot::Receiver<Result<()>> {
        let (tx, rx) = oneshot::channel();
        if let Some(result) = self.result {
            let _ = tx.send(result.map_err(Into::into));
        } else {
            self.waiters.push(tx);
        }
        rx
    }
}

impl WorkerInspectorSessionLifetime {
    fn new() -> Self {
        Self(Arc::new(WorkerInspectorSessionLifetimeState {
            active: AtomicBool::new(true),
            native_created: AtomicBool::new(false),
            detachment: Mutex::new(WorkerInspectorSessionCompletion::default()),
            retirement: Mutex::new(WorkerInspectorSessionCompletion::default()),
        }))
    }

    pub(crate) fn is_active(&self) -> bool {
        self.0.active.load(Ordering::Acquire)
    }
    pub(crate) fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn mark_native_created(&self) {
        self.0.native_created.store(true, Ordering::Release);
    }
    pub(crate) fn native_was_created(&self) -> bool {
        self.0.native_created.load(Ordering::Acquire)
    }

    pub(crate) fn complete_retirement(
        &self,
        result: std::result::Result<(), WorkerInspectorError>,
    ) {
        self.0.retirement.lock().complete(result);
    }

    #[cfg(test)]
    pub(crate) fn retirement(&self) -> oneshot::Receiver<Result<()>> {
        self.0.retirement.lock().subscribe()
    }

    pub(crate) fn complete_detachment(
        &self,
        result: std::result::Result<(), WorkerInspectorError>,
    ) {
        self.0.detachment.lock().complete(result);
    }

    fn detachment(&self) -> oneshot::Receiver<Result<()>> {
        self.0.detachment.lock().subscribe()
    }
}

pub(crate) enum WorkerInspectorTask {
    EnvironmentInvalidation(moli_v8_platform::ProcessEnvironmentInvalidation),
    DispatchProtocolMessage {
        lifetime: WorkerInspectorSessionLifetime,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>>>,
    },
    AttachSession {
        lifetime: WorkerInspectorSessionLifetime,
        inspector_session_id: Option<String>,
    },
    DetachSession {
        lifetime: WorkerInspectorSessionLifetime,
        inspector_session_id: Option<String>,
    },
    RunIfWaitingForDebugger,
}

impl WorkerInspectorTask {
    fn fail(self, error: WorkerInspectorError) {
        if let Self::DetachSession { lifetime, .. } = &self {
            lifetime.complete_detachment(Err(error));
            lifetime.complete_retirement(Err(error));
        }
        let Self::DispatchProtocolMessage {
            deferred_response,
            response_tx,
            ..
        } = self
        else {
            return;
        };
        if let Some(response) = deferred_response {
            let call_id = response.call_id();
            let _ = response.send(json!({
                "id": call_id,
                "error": {
                    "code": -32000,
                    "message": error.to_string(),
                },
            }));
        }
        let _ = response_tx.send(Err(error.into()));
    }
}

struct WorkerInspectorTaskRunnerState {
    interrupting_tasks: VecDeque<WorkerInspectorTask>,
    dont_interrupting_tasks: VecDeque<WorkerInspectorTask>,
    sessions: HashMap<String, WorkerInspectorSessionLifetime>,
    disposed: bool,
    isolate_ready: bool,
    pause_loop_active: bool,
    quit_pause_loop: bool,
}

struct WorkerInspectorTaskRunnerShared {
    state: Mutex<WorkerInspectorTaskRunnerState>,
    environment_notifications: ProcessEnvironmentNotifications,
    // Tracks an outstanding fallback WorkerMessage::RunEnvironmentNotification,
    // not whether the mailbox contains an invalidation. Only consumption of
    // that owner message disarms it; interrupt/pause consumption must not,
    // otherwise repeated interrupts can flood the ordinary Worker channel.
    environment_owner_wake_armed: AtomicBool,
    pause_work: Condvar,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
    isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    interrupt_target: Arc<WorkerInspectorInterruptTarget>,
    interrupt_armed: AtomicBool,
    resume_requested: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct WorkerInspectorTaskRunner {
    shared: Arc<WorkerInspectorTaskRunnerShared>,
}

impl WorkerInspectorTaskRunner {
    pub(crate) fn new(
        wake_tx: mpsc::UnboundedSender<WorkerMessage>,
        isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    ) -> Self {
        let route_id = NEXT_WORKER_INSPECTOR_ROUTE_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("worker Inspector route ID exhausted");
        Self {
            shared: Arc::new(WorkerInspectorTaskRunnerShared {
                environment_notifications: ProcessEnvironmentNotifications::default(),
                environment_owner_wake_armed: AtomicBool::new(false),
                state: Mutex::new(WorkerInspectorTaskRunnerState {
                    interrupting_tasks: VecDeque::new(),
                    dont_interrupting_tasks: VecDeque::new(),
                    sessions: HashMap::new(),
                    disposed: false,
                    isolate_ready: false,
                    pause_loop_active: false,
                    quit_pause_loop: false,
                }),
                pause_work: Condvar::new(),
                wake_tx,
                isolate_handle,
                interrupt_target: Arc::new(WorkerInspectorInterruptTarget { route_id }),
                interrupt_armed: AtomicBool::new(false),
                resume_requested: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn route_id(&self) -> u64 {
        self.shared.interrupt_target.route_id
    }

    /// Share owner/interrupt/pause delivery without appending a task per change.
    /// The controller publishes the mailbox before calling this wake, unlocked.
    pub(crate) fn environment_notifier(&self) -> ProcessEnvironmentNotifier {
        let runner = self.clone();
        self.shared.environment_notifications.notifier(move || {
            runner.wake_environment();
        })
    }

    fn wake_environment(&self) {
        if !self.shared.environment_notifications.has_pending() {
            return;
        }
        if !self
            .shared
            .environment_owner_wake_armed
            .swap(true, Ordering::AcqRel)
            && self
                .shared
                .wake_tx
                .send(WorkerMessage::RunEnvironmentNotification)
                .is_err()
        {
            self.dispose(WorkerInspectorError::EnvironmentUnavailable);
            return;
        }
        self.request_interrupt_if_needed();
        self.shared.pause_work.notify_one();
    }

    /// Called only when consuming the fallback owner message. Disarm before
    /// taking the mailbox so a racing publication can arm the next owner wake.
    pub(crate) fn claim_environment_for_owner(
        &self,
    ) -> Option<moli_v8_platform::ProcessEnvironmentInvalidation> {
        self.shared
            .environment_owner_wake_armed
            .store(false, Ordering::Release);
        self.claim_environment_invalidation()
    }

    fn claim_environment_task(&self) -> Option<WorkerInspectorTask> {
        self.claim_environment_invalidation()
            .map(WorkerInspectorTask::EnvironmentInvalidation)
    }

    pub(crate) fn claim_environment_invalidation(
        &self,
    ) -> Option<moli_v8_platform::ProcessEnvironmentInvalidation> {
        self.shared.environment_notifications.take()
    }

    pub(crate) fn append_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>>>,
    ) -> bool {
        let mode = serde_json::from_str::<serde_json::Value>(&raw_json)
            .ok()
            .and_then(|value| {
                value
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .map(CdpInspectorTaskMode::for_method)
            })
            .unwrap_or(WorkerInspectorTaskMode::Interrupt);
        self.append(
            mode,
            WorkerInspectorTask::DispatchProtocolMessage {
                lifetime: self.session_lifetime(inspector_session_id.as_deref()),
                inspector_session_id,
                raw_json,
                deferred_response,
                response_tx,
            },
        )
    }

    pub(crate) fn session_lifetime(&self, id: Option<&str>) -> WorkerInspectorSessionLifetime {
        self.shared
            .state
            .lock()
            .sessions
            .entry(id.unwrap_or_default().to_owned())
            .or_insert_with(WorkerInspectorSessionLifetime::new)
            .clone()
    }

    pub(crate) fn attach_lifetime(&self, id: Option<&str>) -> WorkerInspectorSessionLifetime {
        let mut state = self.shared.state.lock();
        let lifetime = state
            .sessions
            .entry(id.unwrap_or_default().to_owned())
            .or_insert_with(WorkerInspectorSessionLifetime::new);
        if !lifetime.is_active() {
            *lifetime = WorkerInspectorSessionLifetime::new();
        }
        lifetime.clone()
    }

    pub(crate) fn retire_lifetime(&self, id: Option<&str>) -> WorkerInspectorSessionLifetime {
        let mut state = self.shared.state.lock();
        let lifetime = state
            .sessions
            .entry(id.unwrap_or_default().to_owned())
            .or_insert_with(WorkerInspectorSessionLifetime::new)
            .clone();
        lifetime.0.active.store(false, Ordering::Release);
        lifetime
    }

    pub(crate) fn append_attach(&self, inspector_session_id: Option<String>) -> bool {
        self.append(
            WorkerInspectorTaskMode::Interrupt,
            WorkerInspectorTask::AttachSession {
                lifetime: self.attach_lifetime(inspector_session_id.as_deref()),
                inspector_session_id,
            },
        )
    }

    pub(crate) fn append_detach(
        &self,
        inspector_session_id: Option<String>,
    ) -> oneshot::Receiver<Result<()>> {
        let lifetime = self.retire_lifetime(inspector_session_id.as_deref());
        // A synchronous Inspector dispatch can retain its native session on
        // a pause stack owned by another debugger. The frontend waits until
        // its agent is stopped, not until that unrelated pause is resumed.
        let receipt = lifetime.detachment();
        self.append(
            WorkerInspectorTaskMode::Interrupt,
            WorkerInspectorTask::DetachSession {
                lifetime,
                inspector_session_id,
            },
        );
        receipt
    }

    pub(crate) fn append_run_if_waiting_for_debugger(&self) -> bool {
        self.append(
            WorkerInspectorTaskMode::Interrupt,
            WorkerInspectorTask::RunIfWaitingForDebugger,
        )
    }

    fn append(&self, mode: WorkerInspectorTaskMode, task: WorkerInspectorTask) -> bool {
        let mut state = self.shared.state.lock();
        if state.disposed {
            drop(state);
            task.fail(WorkerInspectorError::Disposed);
            return false;
        }
        match mode {
            WorkerInspectorTaskMode::Interrupt => state.interrupting_tasks.push_back(task),
            WorkerInspectorTaskMode::DontInterrupt => {
                state.dont_interrupting_tasks.push_back(task);
            }
        }
        drop(state);

        if self
            .shared
            .wake_tx
            .send(WorkerMessage::RunInspectorTask(mode))
            .is_err()
        {
            self.dispose(WorkerInspectorError::OwnerUnavailable);
            return false;
        }
        if mode == WorkerInspectorTaskMode::Interrupt {
            self.request_interrupt_if_needed();
        }
        self.shared.pause_work.notify_one();
        true
    }

    pub(crate) fn activate_isolate(&self) {
        let mut state = self.shared.state.lock();
        if state.disposed {
            return;
        }
        state.isolate_ready = true;
        drop(state);
        self.request_interrupt_if_needed();
    }

    pub(crate) fn claim_task(&self, mode: WorkerInspectorTaskMode) -> Option<WorkerInspectorTask> {
        let mut state = self.shared.state.lock();
        match mode {
            WorkerInspectorTaskMode::Interrupt => state.interrupting_tasks.pop_front(),
            WorkerInspectorTaskMode::DontInterrupt => state.dont_interrupting_tasks.pop_front(),
        }
    }

    /// An interrupt can service the mailbox first. An ordinary command wake
    /// must claim its command instead: using it up on environment work would
    /// strand that command after an idle Worker consumed both wake messages.
    pub(crate) fn claim_interrupt_task(&self) -> Option<WorkerInspectorTask> {
        self.claim_environment_task()
            .or_else(|| self.claim_task(WorkerInspectorTaskMode::Interrupt))
    }

    pub(crate) fn interrupt_callback_started(&self) {
        self.shared.interrupt_armed.store(false, Ordering::Release);
    }

    pub(crate) fn request_interrupt_if_needed(&self) {
        let should_request = {
            let state = self.shared.state.lock();
            !state.disposed
                && state.isolate_ready
                && (self.shared.environment_notifications.has_pending()
                    || !state.interrupting_tasks.is_empty())
        };
        if !should_request
            || self
                .shared
                .interrupt_armed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }

        let callback_target = Arc::into_raw(Arc::clone(&self.shared.interrupt_target));
        let callback_data = callback_target.cast_mut().cast::<c_void>();
        let accepted = self
            .shared
            .isolate_handle
            .lock()
            .as_ref()
            .is_some_and(|handle| {
                handle.request_interrupt(dispatch_worker_inspector_interrupt, callback_data)
            });
        if !accepted {
            // SAFETY: V8 rejected the request, so no callback can consume the
            // strong reference created immediately above.
            unsafe { drop(Arc::from_raw(callback_target)) };
            self.shared.interrupt_armed.store(false, Ordering::Release);
        }
    }

    pub(crate) fn begin_pause_loop(&self) -> bool {
        let mut state = self.shared.state.lock();
        if state.disposed || state.pause_loop_active {
            return false;
        }
        state.pause_loop_active = true;
        state.quit_pause_loop = false;
        true
    }

    pub(crate) fn wait_for_pause_task(&self) -> Option<WorkerInspectorTask> {
        let mut state = self.shared.state.lock();
        loop {
            if state.disposed || state.quit_pause_loop {
                return None;
            }
            if let Some(task) = self.claim_environment_task() {
                return Some(task);
            }
            if let Some(task) = state.interrupting_tasks.pop_front() {
                return Some(task);
            }
            self.shared.pause_work.wait(&mut state);
        }
    }

    pub(crate) fn finish_pause_loop(&self) {
        let mut state = self.shared.state.lock();
        state.pause_loop_active = false;
        state.quit_pause_loop = false;
    }

    pub(crate) fn request_quit_pause_loop(&self) {
        self.shared.state.lock().quit_pause_loop = true;
        self.shared.pause_work.notify_all();
    }

    pub(crate) fn request_resume(&self) {
        self.shared.resume_requested.store(true, Ordering::Release);
    }

    pub(crate) fn take_resume_requested(&self) -> bool {
        self.shared.resume_requested.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn dispose(&self, error: WorkerInspectorError) {
        let (interrupting_tasks, dont_interrupting_tasks) = {
            let mut state = self.shared.state.lock();
            if state.disposed {
                return;
            }
            state.disposed = true;
            for lifetime in state.sessions.values() {
                lifetime.0.active.store(false, Ordering::Release);
            }
            self.shared.environment_notifications.close();
            state.isolate_ready = false;
            state.quit_pause_loop = true;
            (
                state.interrupting_tasks.drain(..).collect::<Vec<_>>(),
                state.dont_interrupting_tasks.drain(..).collect::<Vec<_>>(),
            )
        };
        self.shared.pause_work.notify_all();
        for task in interrupting_tasks
            .into_iter()
            .chain(dont_interrupting_tasks)
        {
            task.fail(error);
        }
    }
}

impl std::fmt::Debug for WorkerInspectorTaskRunner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.state.lock();
        formatter
            .debug_struct("WorkerInspectorTaskRunner")
            .field("route_id", &self.route_id())
            .field("interrupting_tasks", &state.interrupting_tasks.len())
            .field(
                "dont_interrupting_tasks",
                &state.dont_interrupting_tasks.len(),
            )
            .field("disposed", &state.disposed)
            .field("isolate_ready", &state.isolate_ready)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::WorkerInspectorError;
    use anyhow::Result;
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tokio::sync::{mpsc, oneshot};

    use super::{WorkerInspectorTask, WorkerInspectorTaskMode, WorkerInspectorTaskRunner};

    fn runner() -> (
        WorkerInspectorTaskRunner,
        mpsc::UnboundedReceiver<super::WorkerMessage>,
    ) {
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        (
            WorkerInspectorTaskRunner::new(wake_tx, Arc::new(Mutex::new(None))),
            wake_rx,
        )
    }

    fn append_protocol(
        runner: &WorkerInspectorTaskRunner,
        id: i64,
        method: &str,
    ) -> oneshot::Receiver<Result<Vec<crate::runtime::RendererRuntimeInspectorMessage>>> {
        let (response_tx, response_rx) = oneshot::channel();
        assert!(runner.append_protocol_message(
            Some("SID-worker-runner".to_owned()),
            serde_json::json!({ "id": id, "method": method }).to_string(),
            None,
            response_tx,
        ));
        response_rx
    }

    #[test]
    fn chromium_worker_inspector_dont_interrupt_catalog_is_exact() {
        for method in [
            "Debugger.evaluateOnCallFrame",
            "Runtime.evaluate",
            "Runtime.callFunctionOn",
            "Runtime.getProperties",
            "Runtime.runScript",
        ] {
            assert_eq!(
                WorkerInspectorTaskMode::for_method(method),
                WorkerInspectorTaskMode::DontInterrupt,
                "{method} should use Chromium's non-interrupting worker path"
            );
        }
        for method in [
            "Debugger.pause",
            "Debugger.resume",
            "Debugger.stepInto",
            "Runtime.enable",
            "Runtime.terminateExecution",
            "Inspector.disable",
        ] {
            assert_eq!(
                WorkerInspectorTaskMode::for_method(method),
                WorkerInspectorTaskMode::Interrupt,
                "{method} should interrupt a busy worker"
            );
        }
    }

    #[test]
    fn interrupt_task_overtakes_earlier_non_interrupting_owner_task() {
        let (runner, mut wake_rx) = runner();
        let _evaluate = append_protocol(&runner, 1, "Runtime.evaluate");
        let _terminate = append_protocol(&runner, 2, "Runtime.terminateExecution");

        assert!(matches!(
            wake_rx.try_recv(),
            Ok(super::WorkerMessage::RunInspectorTask(
                WorkerInspectorTaskMode::DontInterrupt
            ))
        ));
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(super::WorkerMessage::RunInspectorTask(
                WorkerInspectorTaskMode::Interrupt
            ))
        ));
        assert!(matches!(
            runner.claim_task(WorkerInspectorTaskMode::Interrupt),
            Some(WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. })
                if raw_json.contains("Runtime.terminateExecution")
        ));
        assert!(matches!(
            runner.claim_task(WorkerInspectorTaskMode::DontInterrupt),
            Some(WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. })
                if raw_json.contains("Runtime.evaluate")
        ));
        assert!(
            runner
                .claim_task(WorkerInspectorTaskMode::Interrupt)
                .is_none()
        );
        assert!(
            runner
                .claim_task(WorkerInspectorTaskMode::DontInterrupt)
                .is_none()
        );
    }

    #[test]
    fn pause_loop_claims_only_interrupting_tasks() {
        let (runner, _wake_rx) = runner();
        let _evaluate = append_protocol(&runner, 1, "Runtime.evaluate");
        let _terminate = append_protocol(&runner, 2, "Runtime.terminateExecution");

        assert!(runner.begin_pause_loop());
        let interrupt = runner
            .wait_for_pause_task()
            .expect("pause loop should claim the interrupting task");
        runner.request_quit_pause_loop();
        assert!(runner.wait_for_pause_task().is_none());
        runner.finish_pause_loop();

        assert!(matches!(
            interrupt,
            WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. }
                if raw_json.contains("Runtime.terminateExecution")
        ));
        let ordinary = runner
            .claim_task(WorkerInspectorTaskMode::DontInterrupt)
            .expect("ordinary owner runner must retain DontInterrupt work");
        assert!(matches!(
            ordinary,
            WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. }
                if raw_json.contains("Runtime.evaluate")
        ));
    }

    #[test]
    fn owner_fallback_and_interrupt_claim_one_fifo_exactly_once() {
        let (runner, _wake_rx) = runner();
        let _first = append_protocol(&runner, 1, "Debugger.pause");
        let _second = append_protocol(&runner, 2, "Debugger.resume");

        let first = runner
            .claim_task(WorkerInspectorTaskMode::Interrupt)
            .expect("owner fallback should claim first task");
        let second = runner
            .claim_task(WorkerInspectorTaskMode::Interrupt)
            .expect("interrupt callback should claim second task");
        assert!(
            runner
                .claim_task(WorkerInspectorTaskMode::Interrupt)
                .is_none()
        );
        assert!(matches!(
            first,
            WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. }
                if raw_json.contains("Debugger.pause")
        ));
        assert!(matches!(
            second,
            WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. }
                if raw_json.contains("Debugger.resume")
        ));
    }

    #[test]
    fn environment_bursts_and_interrupt_consumption_keep_the_owner_wake_bounded() {
        use moli_v8_platform::ProcessEnvironmentInvalidation::{DateTime, LocaleAndDateTime};

        let (runner, mut wake_rx) = runner();
        let notifier = runner.environment_notifier();
        for _ in 0..10_000 {
            notifier.notify(DateTime);
            notifier.notify(LocaleAndDateTime);
        }
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(super::WorkerMessage::RunEnvironmentNotification)
        ));
        assert!(wake_rx.try_recv().is_err());
        assert!(runner.shared.state.lock().interrupting_tasks.is_empty());

        // Simulate JS repeatedly accepting V8 interrupts while the ordinary
        // owner loop has not handled the already-posted fallback wake.
        for _ in 0..1_000 {
            notifier.notify(LocaleAndDateTime);
            assert!(matches!(
                runner.claim_interrupt_task(),
                Some(WorkerInspectorTask::EnvironmentInvalidation(
                    LocaleAndDateTime
                ))
            ));
        }
        assert!(wake_rx.try_recv().is_err());
        assert!(runner.claim_environment_for_owner().is_none());
        notifier.notify(DateTime);
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(super::WorkerMessage::RunEnvironmentNotification)
        ));
        assert!(matches!(
            runner.claim_environment_for_owner(),
            Some(DateTime)
        ));
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn pending_environment_does_not_consume_an_ordinary_command_wake() {
        use moli_v8_platform::ProcessEnvironmentInvalidation::LocaleAndDateTime;
        let (runner, mut wake_rx) = runner();
        let _command = append_protocol(&runner, 1, "Debugger.resume");
        runner.environment_notifier().notify(LocaleAndDateTime);
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(super::WorkerMessage::RunInspectorTask(
                WorkerInspectorTaskMode::Interrupt
            ))
        ));
        assert!(matches!(
            runner.claim_task(WorkerInspectorTaskMode::Interrupt),
            Some(WorkerInspectorTask::DispatchProtocolMessage { .. })
        ));
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(super::WorkerMessage::RunEnvironmentNotification)
        ));
        assert_eq!(
            runner.claim_environment_for_owner(),
            Some(LocaleAndDateTime)
        );
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn environment_notifications_merge_before_pause_commands_and_are_discarded_on_disposal() {
        use moli_v8_platform::ProcessEnvironmentInvalidation::{DateTime, LocaleAndDateTime};

        let (runner, _wake_rx) = runner();
        let notifier = runner.environment_notifier();
        let _first = append_protocol(&runner, 1, "Debugger.pause");
        notifier.notify(LocaleAndDateTime);
        let _second = append_protocol(&runner, 2, "Debugger.resume");
        notifier.notify(DateTime);
        assert!(runner.begin_pause_loop());
        // The pause claims one merged invalidation ahead of queued commands,
        // without a session or ordinary Worker-loop turn.
        assert!(matches!(
            runner.wait_for_pause_task(),
            Some(WorkerInspectorTask::EnvironmentInvalidation(
                LocaleAndDateTime
            ))
        ));
        assert!(matches!(
            runner.wait_for_pause_task(),
            Some(WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. })
                if raw_json.contains("Debugger.pause")
        ));
        assert!(matches!(runner.wait_for_pause_task(),
            Some(WorkerInspectorTask::DispatchProtocolMessage { raw_json, .. })
                if raw_json.contains("Debugger.resume")));
        notifier.notify(DateTime);
        runner.dispose(WorkerInspectorError::Closed);
        notifier.notify(LocaleAndDateTime);
        assert!(
            runner
                .claim_task(WorkerInspectorTaskMode::Interrupt)
                .is_none()
        );
        assert!(runner.wait_for_pause_task().is_none());
    }

    #[test]
    fn dispose_rejects_queued_and_late_protocol_tasks() {
        let (runner, _wake_rx) = runner();
        let mut queued = append_protocol(&runner, 1, "Runtime.evaluate");
        runner.dispose(WorkerInspectorError::Closed);
        assert_eq!(
            queued
                .try_recv()
                .expect("queued task should be canceled")
                .expect_err("queued task must not dispatch")
                .downcast::<WorkerInspectorError>()
                .unwrap(),
            WorkerInspectorError::Closed
        );

        let (response_tx, mut late) = oneshot::channel();
        assert!(!runner.append_protocol_message(
            None,
            serde_json::json!({ "id": 2, "method": "Debugger.pause" }).to_string(),
            None,
            response_tx,
        ));
        assert_eq!(
            late.try_recv()
                .expect("late task should be rejected")
                .expect_err("late task must not dispatch")
                .downcast::<WorkerInspectorError>()
                .unwrap(),
            WorkerInspectorError::Disposed
        );
    }
    #[test]
    fn queued_execution_keeps_its_original_session_incarnation_across_detach_attach() {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = WorkerInspectorTaskRunner::new(wake_tx, Arc::new(Mutex::new(None)));
        let (old_tx, _old_rx) = oneshot::channel();
        assert!(runner.append_protocol_message(
            Some("reused".into()),
            r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"'old'"}}"#.into(),
            None,
            old_tx
        ));
        let mut retired = runner.append_detach(Some("reused".into()));
        assert!(runner.append_attach(Some("reused".into())));
        let (new_tx, _new_rx) = oneshot::channel();
        assert!(runner.append_protocol_message(
            Some("reused".into()),
            r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"'new'"}}"#.into(),
            None,
            new_tx
        ));
        let Some(WorkerInspectorTask::DispatchProtocolMessage { lifetime: old, .. }) =
            runner.claim_task(WorkerInspectorTaskMode::DontInterrupt)
        else {
            panic!("old execution task")
        };
        let Some(WorkerInspectorTask::DispatchProtocolMessage { lifetime: new, .. }) =
            runner.claim_task(WorkerInspectorTaskMode::DontInterrupt)
        else {
            panic!("new execution task")
        };
        assert!(!old.is_active());
        assert!(new.is_active());
        assert!(!old.matches(&new));
        assert!(
            matches!(retired.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "enqueueing detach has not stopped the native debugger agent"
        );
        let Some(WorkerInspectorTask::DetachSession { lifetime, .. }) =
            runner.claim_task(WorkerInspectorTaskMode::Interrupt)
        else {
            panic!("detach owner task")
        };
        lifetime.complete_detachment(Ok(()));
        retired.try_recv().unwrap().unwrap();
    }
}
