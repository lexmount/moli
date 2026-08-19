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

pub(crate) enum WorkerInspectorTask {
    DispatchProtocolMessage {
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>, String>>,
    },
    AttachSession {
        inspector_session_id: Option<String>,
    },
    DetachSession {
        inspector_session_id: Option<String>,
    },
    RunIfWaitingForDebugger,
}

impl WorkerInspectorTask {
    fn fail(self, message: &str) {
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
                    "message": message,
                },
            }));
        }
        let _ = response_tx.send(Err(message.to_owned()));
    }
}

struct WorkerInspectorTaskRunnerState {
    interrupting_tasks: VecDeque<WorkerInspectorTask>,
    dont_interrupting_tasks: VecDeque<WorkerInspectorTask>,
    disposed: bool,
    isolate_ready: bool,
    pause_loop_active: bool,
    quit_pause_loop: bool,
}

struct WorkerInspectorTaskRunnerShared {
    state: Mutex<WorkerInspectorTaskRunnerState>,
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
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("worker Inspector route ID exhausted");
        Self {
            shared: Arc::new(WorkerInspectorTaskRunnerShared {
                state: Mutex::new(WorkerInspectorTaskRunnerState {
                    interrupting_tasks: VecDeque::new(),
                    dont_interrupting_tasks: VecDeque::new(),
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

    pub(crate) fn append_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>, String>>,
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
                inspector_session_id,
                raw_json,
                deferred_response,
                response_tx,
            },
        )
    }

    pub(crate) fn append_attach(&self, inspector_session_id: Option<String>) -> bool {
        self.append(
            WorkerInspectorTaskMode::Interrupt,
            WorkerInspectorTask::AttachSession {
                inspector_session_id,
            },
        )
    }

    pub(crate) fn append_detach(&self, inspector_session_id: Option<String>) -> bool {
        self.append(
            WorkerInspectorTaskMode::Interrupt,
            WorkerInspectorTask::DetachSession {
                inspector_session_id,
            },
        )
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
            task.fail("Worker Inspector task runner is disposed");
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
            self.dispose("Worker Inspector owner is unavailable");
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

    pub(crate) fn interrupt_callback_started(&self) {
        self.shared.interrupt_armed.store(false, Ordering::Release);
    }

    pub(crate) fn request_interrupt_if_needed(&self) {
        let should_request = {
            let state = self.shared.state.lock();
            !state.disposed && state.isolate_ready && !state.interrupting_tasks.is_empty()
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

    pub(crate) fn dispose(&self, message: &str) {
        let (interrupting_tasks, dont_interrupting_tasks) = {
            let mut state = self.shared.state.lock();
            if state.disposed {
                return;
            }
            state.disposed = true;
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
            task.fail(message);
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
    ) -> oneshot::Receiver<Result<Vec<crate::runtime::RendererRuntimeInspectorMessage>, String>>
    {
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
    fn dispose_rejects_queued_and_late_protocol_tasks() {
        let (runner, _wake_rx) = runner();
        let mut queued = append_protocol(&runner, 1, "Runtime.evaluate");
        runner.dispose("worker teardown");
        assert_eq!(
            queued
                .try_recv()
                .expect("queued task should be canceled")
                .expect_err("queued task must not dispatch"),
            "worker teardown"
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
                .expect_err("late task must not dispatch"),
            "Worker Inspector task runner is disposed"
        );
    }
}
