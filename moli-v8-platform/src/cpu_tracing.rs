use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::Mutex;
use tokio::sync::Notify;
use tracing::error;

use super::{
    IsolateRegistrationGeneration, RegisteredIsolateOwner, dispatch_isolate_owner_callback,
    registered_isolate_owner, registered_isolate_owners, unsafe_raw_isolate_ptr_from_addr,
};

const CHROMIUM_CPU_PROFILE_SAMPLING_INTERVAL_US: i32 = 100;
const MAX_PROFILED_ISOLATES: usize = 256;
const MAX_PROFILE_BYTES_PER_ISOLATE: usize = 4 * 1024 * 1024;
const MAX_SAMPLES_PER_ISOLATE: u32 = 100_000;
const MIN_SAMPLES_PER_ISOLATE: u32 = 1_000;

static ACTIVE_CPU_TRACE: LazyLock<Mutex<Option<Arc<CpuTraceRun>>>> =
    LazyLock::new(|| Mutex::new(None));
static PROFILE_SERIALIZATION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static NEXT_CPU_TRACE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct V8CpuTraceConfiguration {
    max_total_profile_bytes: usize,
    max_profile_bytes_per_isolate: usize,
    max_samples_per_isolate: u32,
}

impl V8CpuTraceConfiguration {
    pub fn bounded_for_trace_buffer(max_trace_buffer_bytes: usize) -> Self {
        let max_total_profile_bytes = max_trace_buffer_bytes.max(1);
        let max_profile_bytes_per_isolate =
            max_total_profile_bytes.min(MAX_PROFILE_BYTES_PER_ISOLATE);
        let sample_budget = max_total_profile_bytes / 32;
        let max_samples_per_isolate = u32::try_from(sample_budget)
            .unwrap_or(u32::MAX)
            .clamp(MIN_SAMPLES_PER_ISOLATE, MAX_SAMPLES_PER_ISOLATE);
        Self {
            max_total_profile_bytes,
            max_profile_bytes_per_isolate,
            max_samples_per_isolate,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct IsolateTraceKey {
    isolate_addr: usize,
    registration_id: u64,
}

impl IsolateTraceKey {
    fn new(isolate_addr: usize, generation: &IsolateRegistrationGeneration) -> Self {
        Self {
            isolate_addr,
            registration_id: generation.id(),
        }
    }
}

#[derive(Debug)]
enum ParticipantState {
    StartPending,
    StartInProgress,
    Running(v8::CpuTraceProfiler),
    StopRequested(v8::CpuTraceProfiler),
    StopInProgress,
    Complete,
}

#[derive(Debug)]
struct CpuTraceRunState {
    stopping: bool,
    discard_output: bool,
    participants: HashMap<IsolateTraceKey, ParticipantState>,
    profiles: Vec<V8CpuProfileSegment>,
    profile_bytes: usize,
    data_loss_occurred: bool,
    result_taken: bool,
}

#[derive(Debug)]
struct CpuTraceRun {
    id: u64,
    configuration: V8CpuTraceConfiguration,
    state: Mutex<CpuTraceRunState>,
    state_changed: Notify,
}

#[derive(Debug)]
pub struct V8CpuProfileSegment {
    isolate_id: u64,
    profile_json: Vec<u8>,
    sample_count: u32,
}

impl V8CpuProfileSegment {
    pub fn isolate_id(&self) -> u64 {
        self.isolate_id
    }

    pub fn profile_json(&self) -> &[u8] {
        &self.profile_json
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

#[derive(Debug)]
pub struct V8CpuTraceResult {
    trace_id: u64,
    profiles: Vec<V8CpuProfileSegment>,
    data_loss_occurred: bool,
}

impl V8CpuTraceResult {
    pub fn trace_id(&self) -> u64 {
        self.trace_id
    }

    pub fn profiles(&self) -> &[V8CpuProfileSegment] {
        &self.profiles
    }

    pub fn data_loss_occurred(&self) -> bool {
        self.data_loss_occurred
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum V8CpuTraceStartError {
    AlreadyActive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum V8CpuTraceStartStatus {
    Started,
    StoppedBeforeStart,
}

#[derive(Debug)]
pub struct V8CpuTraceSession {
    run: Option<Arc<CpuTraceRun>>,
    start_participants: Arc<[IsolateTraceKey]>,
}

#[derive(Debug)]
pub struct PendingV8CpuTraceStart {
    run: Arc<CpuTraceRun>,
    participants: Arc<[IsolateTraceKey]>,
}

#[derive(Debug)]
pub struct PendingV8CpuTraceStop {
    run: Arc<CpuTraceRun>,
}

impl V8CpuTraceSession {
    pub fn start_barrier(&self) -> PendingV8CpuTraceStart {
        PendingV8CpuTraceStart {
            run: Arc::clone(
                self.run
                    .as_ref()
                    .expect("CPU trace session must be active while waiting for start"),
            ),
            participants: Arc::clone(&self.start_participants),
        }
    }

    pub fn stop(mut self) -> PendingV8CpuTraceStop {
        let run = self
            .run
            .take()
            .expect("CPU trace session can only stop once");
        run.request_stop(false);
        PendingV8CpuTraceStop { run }
    }

    pub fn cancel(mut self) -> PendingV8CpuTraceStop {
        let run = self
            .run
            .take()
            .expect("CPU trace session can only be cancelled once");
        run.request_stop(true);
        PendingV8CpuTraceStop { run }
    }
}

impl Drop for V8CpuTraceSession {
    fn drop(&mut self) {
        if let Some(run) = self.run.take() {
            run.request_stop(true);
        }
    }
}

impl PendingV8CpuTraceStart {
    pub fn status(&self) -> Option<V8CpuTraceStartStatus> {
        self.run.start_status(&self.participants)
    }

    pub async fn wait(self) -> V8CpuTraceStartStatus {
        loop {
            let notified = self.run.state_changed.notified();
            if let Some(status) = self.status() {
                return status;
            }
            notified.await;
        }
    }
}

impl PendingV8CpuTraceStop {
    pub async fn wait(self) -> V8CpuTraceResult {
        loop {
            let notified = self.run.state_changed.notified();
            if let Some(result) = self.run.take_result_if_complete() {
                return result;
            }
            notified.await;
        }
    }
}

pub fn start_v8_cpu_trace(
    configuration: V8CpuTraceConfiguration,
) -> Result<V8CpuTraceSession, V8CpuTraceStartError> {
    let run = {
        let mut active = ACTIVE_CPU_TRACE.lock();
        if active.is_some() {
            return Err(V8CpuTraceStartError::AlreadyActive);
        }
        let run = Arc::new(CpuTraceRun {
            id: NEXT_CPU_TRACE_ID.fetch_add(1, Ordering::Relaxed),
            configuration,
            state: Mutex::new(CpuTraceRunState {
                stopping: false,
                discard_output: false,
                participants: HashMap::new(),
                profiles: Vec::new(),
                profile_bytes: 0,
                data_loss_occurred: false,
                result_taken: false,
            }),
            state_changed: Notify::new(),
        });
        *active = Some(Arc::clone(&run));
        run
    };

    let owners = registered_isolate_owners();
    let start_participants = owners
        .iter()
        .map(|owner| IsolateTraceKey::new(owner.isolate_addr, &owner.registration.generation))
        .collect::<Vec<_>>()
        .into();
    for owner in owners {
        schedule_existing_isolate_start(&run, owner);
    }
    Ok(V8CpuTraceSession {
        run: Some(run),
        start_participants,
    })
}

pub(super) fn isolate_registered(isolate_addr: usize, generation: IsolateRegistrationGeneration) {
    let Some(run) = ACTIVE_CPU_TRACE.lock().as_ref().cloned() else {
        return;
    };
    let key = IsolateTraceKey::new(isolate_addr, &generation);
    if run.register_participant(key) {
        run.start_participant_on_owner(key);
    }
}

pub(super) fn isolate_unregistering(
    isolate_addr: usize,
    generation: IsolateRegistrationGeneration,
) {
    let Some(run) = ACTIVE_CPU_TRACE.lock().as_ref().cloned() else {
        return;
    };
    run.stop_participant_on_owner(IsolateTraceKey::new(isolate_addr, &generation), true);
}

fn schedule_existing_isolate_start(run: &Arc<CpuTraceRun>, owner: RegisteredIsolateOwner) {
    let key = IsolateTraceKey::new(owner.isolate_addr, &owner.registration.generation);
    if !run.register_participant(key) {
        return;
    }
    let callback = ParticipantOwnerCallback::new(Arc::clone(run), key, CallbackAction::Start);
    dispatch_isolate_owner_callback(owner, move || callback.run());
}

#[derive(Clone, Copy, Debug)]
enum CallbackAction {
    Start,
    Stop,
}

struct ParticipantOwnerCallback {
    run: Arc<CpuTraceRun>,
    key: IsolateTraceKey,
    action: CallbackAction,
    armed: bool,
}

impl ParticipantOwnerCallback {
    fn new(run: Arc<CpuTraceRun>, key: IsolateTraceKey, action: CallbackAction) -> Self {
        Self {
            run,
            key,
            action,
            armed: true,
        }
    }

    fn run(mut self) {
        self.armed = false;
        match self.action {
            CallbackAction::Start => self.run.start_participant_on_owner(self.key),
            CallbackAction::Stop => self.run.stop_participant_on_owner(self.key, false),
        }
    }
}

impl Drop for ParticipantOwnerCallback {
    fn drop(&mut self) {
        if self.armed {
            self.run.abandon_participant(self.key, self.action);
        }
    }
}

impl CpuTraceRun {
    fn start_status(&self, participants: &[IsolateTraceKey]) -> Option<V8CpuTraceStartStatus> {
        let state = self.state.lock();
        if state.stopping {
            return Some(V8CpuTraceStartStatus::StoppedBeforeStart);
        }
        participants
            .iter()
            .all(|key| {
                !matches!(
                    state.participants.get(key),
                    Some(ParticipantState::StartPending | ParticipantState::StartInProgress)
                )
            })
            .then_some(V8CpuTraceStartStatus::Started)
    }

    fn register_participant(&self, key: IsolateTraceKey) -> bool {
        let mut state = self.state.lock();
        if state.stopping || state.participants.contains_key(&key) {
            return false;
        }
        if state.participants.len() >= MAX_PROFILED_ISOLATES {
            state.data_loss_occurred = true;
            return false;
        }
        state
            .participants
            .insert(key, ParticipantState::StartPending);
        true
    }

    fn start_participant_on_owner(self: &Arc<Self>, key: IsolateTraceKey) {
        {
            let mut state = self.state.lock();
            let Some(participant) = state.participants.get_mut(&key) else {
                return;
            };
            if !matches!(participant, ParticipantState::StartPending) {
                return;
            }
            *participant = ParticipantState::StartInProgress;
        }

        let isolate = unsafe_raw_isolate_ptr_from_addr(key.isolate_addr);
        // SAFETY: this method is called only by the exact registration's owner
        // callback, or synchronously from registration while that isolate is
        // current. The generation remains live until unregister returns.
        let profiler = unsafe {
            v8::CpuTraceProfiler::start_on_current_isolate(
                isolate,
                CHROMIUM_CPU_PROFILE_SAMPLING_INTERVAL_US,
                self.configuration.max_samples_per_isolate,
            )
        };

        let mut stop_immediately = None;
        {
            let mut state = self.state.lock();
            let stopping = state.stopping;
            let Some(participant) = state.participants.get_mut(&key) else {
                if let Some(profiler) = profiler {
                    // SAFETY: the same owner callback still has the isolate
                    // entered, so an orphaned start can be cancelled here.
                    let _ = unsafe { profiler.cancel_on_current_isolate() };
                }
                return;
            };
            match profiler {
                Some(profiler) if stopping => {
                    *participant = ParticipantState::StopInProgress;
                    stop_immediately = Some(profiler);
                }
                Some(profiler) => *participant = ParticipantState::Running(profiler),
                None => {
                    *participant = ParticipantState::Complete;
                    state.data_loss_occurred = true;
                }
            }
        }
        if let Some(profiler) = stop_immediately {
            self.finish_profiler_on_owner(key, profiler);
        } else {
            self.notify_state_changed();
        }
    }

    fn request_stop(self: &Arc<Self>, discard_output: bool) {
        let keys_to_stop = {
            let mut state = self.state.lock();
            state.stopping = true;
            state.discard_output |= discard_output;
            let mut keys = Vec::new();
            for (&key, participant) in &mut state.participants {
                if matches!(participant, ParticipantState::Running(_)) {
                    let previous = std::mem::replace(participant, ParticipantState::Complete);
                    let ParticipantState::Running(profiler) = previous else {
                        unreachable!();
                    };
                    *participant = ParticipantState::StopRequested(profiler);
                    keys.push(key);
                }
            }
            keys
        };

        for key in keys_to_stop {
            if let Some(owner) = registered_isolate_owner(key.isolate_addr, key.registration_id) {
                let callback =
                    ParticipantOwnerCallback::new(Arc::clone(self), key, CallbackAction::Stop);
                dispatch_isolate_owner_callback(owner, move || callback.run());
            } else {
                self.abandon_participant(key, CallbackAction::Stop);
            }
        }
        self.notify_state_changed();
    }

    fn stop_participant_on_owner(self: &Arc<Self>, key: IsolateTraceKey, retiring: bool) {
        let profiler = {
            let mut state = self.state.lock();
            let Some(participant) = state.participants.get_mut(&key) else {
                return;
            };
            let previous = std::mem::replace(participant, ParticipantState::Complete);
            match previous {
                ParticipantState::Running(profiler) | ParticipantState::StopRequested(profiler) => {
                    *participant = ParticipantState::StopInProgress;
                    Some(profiler)
                }
                ParticipantState::StartPending => {
                    if !state.discard_output {
                        state.data_loss_occurred = true;
                    }
                    None
                }
                ParticipantState::StartInProgress | ParticipantState::StopInProgress => {
                    error!(
                        isolate = key.isolate_addr,
                        registration = key.registration_id,
                        retiring,
                        "V8 CPU trace participant changed during owner callback"
                    );
                    state.data_loss_occurred = true;
                    None
                }
                ParticipantState::Complete => None,
            }
        };
        if let Some(profiler) = profiler {
            self.finish_profiler_on_owner(key, profiler);
        } else {
            self.notify_state_changed();
        }
    }

    fn finish_profiler_on_owner(
        self: &Arc<Self>,
        key: IsolateTraceKey,
        profiler: v8::CpuTraceProfiler,
    ) {
        let discard_output = self.state.lock().discard_output;
        if discard_output {
            // SAFETY: caller is the exact isolate owner with the isolate entered.
            let cancelled = unsafe { profiler.cancel_on_current_isolate() };
            self.complete_participant(key, None, !cancelled);
            return;
        }

        // CPU profile serialization is synchronous on an isolate owner. One
        // process-wide lock bounds transient output memory even when several
        // worker owners stop concurrently.
        let _serialization = PROFILE_SERIALIZATION_LOCK.lock();
        // SAFETY: caller is the exact isolate owner with the isolate entered.
        let profile = unsafe {
            profiler.stop_on_current_isolate(self.configuration.max_profile_bytes_per_isolate)
        };
        match profile {
            Ok(profile) => {
                let data_loss = profile.sample_limit_reached;
                self.complete_participant(
                    key,
                    Some(V8CpuProfileSegment {
                        isolate_id: key.registration_id,
                        profile_json: profile.bytes,
                        sample_count: profile.sample_count,
                    }),
                    data_loss,
                );
            }
            Err(_) => self.complete_participant(key, None, true),
        }
    }

    fn complete_participant(
        self: &Arc<Self>,
        key: IsolateTraceKey,
        profile: Option<V8CpuProfileSegment>,
        data_loss: bool,
    ) {
        {
            let mut state = self.state.lock();
            let discard_output = state.discard_output;
            if let Some(participant) = state.participants.get_mut(&key) {
                *participant = ParticipantState::Complete;
            }
            state.data_loss_occurred |= data_loss;
            if !discard_output && let Some(profile) = profile {
                let next_bytes = state
                    .profile_bytes
                    .saturating_add(profile.profile_json.len());
                if next_bytes > self.configuration.max_total_profile_bytes {
                    state.data_loss_occurred = true;
                } else {
                    state.profile_bytes = next_bytes;
                    state.profiles.push(profile);
                }
            }
        }
        self.notify_state_changed();
    }

    fn abandon_participant(self: &Arc<Self>, key: IsolateTraceKey, action: CallbackAction) {
        {
            let mut state = self.state.lock();
            let Some(participant) = state.participants.get_mut(&key) else {
                return;
            };
            let previous = std::mem::replace(participant, ParticipantState::Complete);
            if matches!(previous, ParticipantState::Complete) {
                return;
            }
            if matches!(
                previous,
                ParticipantState::Running(_) | ParticipantState::StopRequested(_)
            ) {
                error!(
                    isolate = key.isolate_addr,
                    registration = key.registration_id,
                    ?action,
                    "V8 CPU profiler owner callback was abandoned after native start"
                );
            }
            state.data_loss_occurred = true;
        }
        self.notify_state_changed();
    }

    fn is_complete(&self) -> bool {
        let state = self.state.lock();
        state.stopping
            && state
                .participants
                .values()
                .all(|participant| matches!(participant, ParticipantState::Complete))
    }

    fn notify_state_changed(self: &Arc<Self>) {
        if self.is_complete() {
            let mut active = ACTIVE_CPU_TRACE.lock();
            if active.as_ref().is_some_and(|run| Arc::ptr_eq(run, self)) {
                *active = None;
            }
        }
        self.state_changed.notify_waiters();
    }

    fn take_result_if_complete(&self) -> Option<V8CpuTraceResult> {
        let mut state = self.state.lock();
        if !state.stopping
            || !state
                .participants
                .values()
                .all(|participant| matches!(participant, ParticipantState::Complete))
            || state.result_taken
        {
            return None;
        }
        state.result_taken = true;
        Some(V8CpuTraceResult {
            trace_id: self.id,
            profiles: std::mem::take(&mut state.profiles),
            data_loss_occurred: state.data_loss_occurred,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::{Future, poll_fn},
        pin::pin,
        task::Poll,
    };

    use serde_json::Value;

    use super::*;
    use crate::{V8ForegroundTask, V8ForegroundTaskWake, V8PlatformIsolateRegistration};

    async fn drive_owner_tasks_until<F>(
        receiver: &mut tokio::sync::mpsc::UnboundedReceiver<V8ForegroundTask>,
        future: F,
    ) -> (F::Output, usize)
    where
        F: Future,
    {
        let mut future = pin!(future);
        let mut tasks_run = 0;
        loop {
            let next = poll_fn(|cx| {
                if let Poll::Ready(output) = future.as_mut().poll(cx) {
                    return Poll::Ready(Ok(output));
                }
                receiver
                    .poll_recv(cx)
                    .map(|task| Err(task.expect("isolate owner task route should remain open")))
            })
            .await;
            match next {
                Ok(output) => return (output, tasks_run),
                Err(task) => {
                    assert!(task.run(), "live isolate owner task should execute");
                    tasks_run += 1;
                }
            }
        }
    }

    fn run_hot_function(isolate: &mut v8::OwnedIsolate, function_name: &str) {
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = v8::String::new(
            scope,
            &format!(
                "function {function_name}() {{ let n = 0; for (let i = 0; i < 20000000; i++) n += i % 7; return n; }} {function_name}();"
            ),
        )
        .expect("profile test source");
        let script =
            v8::Script::compile(scope, source, None).expect("profile test script should compile");
        script.run(scope).expect("profile test script should run");
    }

    #[test]
    fn real_isolate_profile_is_sampled_serialized_and_disposed() {
        moli_v8_init::ensure_v8_initialized(crate::create_platform);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let (owner_task_tx, mut owner_task_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut isolate = v8::Isolate::new(Default::default());
            let registration = V8PlatformIsolateRegistration::register(
                &mut isolate,
                V8ForegroundTaskWake::queued(move |task| {
                    let _ = owner_task_tx.send(task);
                }),
                |_| {},
            );

            let session = start_v8_cpu_trace(V8CpuTraceConfiguration {
                max_total_profile_bytes: 4 * 1024 * 1024,
                max_profile_bytes_per_isolate: 4 * 1024 * 1024,
                max_samples_per_isolate: 50_000,
            })
            .expect("CPU trace should start");
            let (start_status, start_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                drive_owner_tasks_until(&mut owner_task_rx, session.start_barrier().wait()),
            )
            .await
            .expect("CPU profiler start barrier should complete");
            assert_eq!(start_status, V8CpuTraceStartStatus::Started);
            assert!(
                start_tasks_run > 0,
                "CPU trace start must wait for the existing isolate owner callback"
            );

            run_hot_function(&mut isolate, "moliCpuTraceHotFunction");

            let (result, stop_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                drive_owner_tasks_until(&mut owner_task_rx, session.stop().wait()),
            )
            .await
            .expect("CPU trace stop should finish");
            assert!(stop_tasks_run > 0, "CPU trace stop must run on the owner");
            assert_eq!(result.profiles().len(), 1);
            let profile = &result.profiles()[0];
            assert!(profile.sample_count() > 0, "profile must contain samples");
            let serialized: Value = serde_json::from_slice(profile.profile_json())
                .expect("V8 CPU profile should be valid JSON");
            assert_eq!(
                serialized["samples"].as_array().map(Vec::len),
                serialized["timeDeltas"].as_array().map(Vec::len),
                "serialized samples and timeDeltas must stay aligned"
            );
            assert!(serialized["nodes"].as_array().is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|node| node["callFrame"]["functionName"] == "moliCpuTraceHotFunction")
            }));

            let sample_limited = start_v8_cpu_trace(V8CpuTraceConfiguration {
                max_total_profile_bytes: 4 * 1024 * 1024,
                max_profile_bytes_per_isolate: 4 * 1024 * 1024,
                max_samples_per_isolate: 5,
            })
            .expect("sample-limited CPU trace should start");
            let (start_status, start_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                drive_owner_tasks_until(&mut owner_task_rx, sample_limited.start_barrier().wait()),
            )
            .await
            .expect("sample-limited profiler start barrier should complete");
            assert_eq!(start_status, V8CpuTraceStartStatus::Started);
            assert!(start_tasks_run > 0);
            run_hot_function(&mut isolate, "moliCpuTraceSampleLimited");
            let (sample_limited, stop_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                drive_owner_tasks_until(&mut owner_task_rx, sample_limited.stop().wait()),
            )
            .await
            .expect("sample-limited CPU trace stop should finish");
            assert!(stop_tasks_run > 0);
            assert!(sample_limited.data_loss_occurred());
            assert_eq!(sample_limited.profiles().len(), 1);
            assert_eq!(sample_limited.profiles()[0].sample_count(), 5);

            let output_limited = start_v8_cpu_trace(V8CpuTraceConfiguration {
                max_total_profile_bytes: 64,
                max_profile_bytes_per_isolate: 64,
                max_samples_per_isolate: 1_000,
            })
            .expect("output-limited CPU trace should start");
            let (start_status, start_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                drive_owner_tasks_until(&mut owner_task_rx, output_limited.start_barrier().wait()),
            )
            .await
            .expect("output-limited profiler start barrier should complete");
            assert_eq!(start_status, V8CpuTraceStartStatus::Started);
            assert!(start_tasks_run > 0);
            run_hot_function(&mut isolate, "moliCpuTraceOutputLimited");
            let (output_limited, stop_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                drive_owner_tasks_until(&mut owner_task_rx, output_limited.stop().wait()),
            )
            .await
            .expect("output-limited CPU trace stop should finish");
            assert!(stop_tasks_run > 0);
            assert!(output_limited.data_loss_occurred());
            assert!(output_limited.profiles().is_empty());

            let stopped_during_start = start_v8_cpu_trace(V8CpuTraceConfiguration {
                max_total_profile_bytes: 4 * 1024 * 1024,
                max_profile_bytes_per_isolate: 4 * 1024 * 1024,
                max_samples_per_isolate: 1_000,
            })
            .expect("stopped-during-start CPU trace should start");
            let start_barrier = stopped_during_start.start_barrier();
            assert_eq!(
                start_barrier.status(),
                None,
                "queued existing-isolate start must keep the barrier pending"
            );
            let pending_stop = stopped_during_start.stop();
            assert_eq!(
                start_barrier.wait().await,
                V8CpuTraceStartStatus::StoppedBeforeStart
            );
            let (_, stop_tasks_run) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                drive_owner_tasks_until(&mut owner_task_rx, pending_stop.wait()),
            )
            .await
            .expect("stopped-during-start CPU trace stop should finish");
            assert!(stop_tasks_run > 0);

            registration.unregister();
            drop(isolate);
        });
    }
}
