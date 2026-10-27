use std::collections::{HashSet, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, monotonic_timestamp_seconds,
};

pub(crate) const DEFAULT_TRACE_BUFFER_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TRACE_BUFFER_BYTES: usize = 64 * 1024 * 1024;
const MAX_TRACE_EVENTS: usize = 100_000;
const REPORT_EVENT_CHUNK_BYTES: usize = 512 * 1024;
const TRACE_THREAD_ID_MASK: u64 = (1_u64 << 53) - 1;
const V8_CPU_PROFILE_CATEGORY: &str = "disabled-by-default-v8.cpu_profiler";
const CPU_PROFILE_NODE_CHUNK_SIZE: usize = 10;
const CPU_PROFILE_SAMPLE_CHUNK_SIZE: usize = 100;

pub(crate) const SUPPORTED_CATEGORIES: &[&str] = &[
    "__metadata",
    "devtools.timeline",
    "disabled-by-default-devtools.timeline",
    V8_CPU_PROFILE_CATEGORY,
    "moli.devtools",
    "loading",
    "v8.execute",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceTransferMode {
    ReportEvents,
    ReturnAsStream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceRecordMode {
    UntilFull,
    Continuously,
}

#[derive(Debug)]
pub(crate) struct TraceConfiguration {
    pub(crate) transfer_mode: TraceTransferMode,
    pub(crate) record_mode: TraceRecordMode,
    pub(crate) included_categories: Vec<String>,
    pub(crate) excluded_categories: Vec<String>,
    pub(crate) max_buffer_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TraceFrameSnapshot {
    pub(crate) frame_id: String,
    pub(crate) url: String,
}

pub(crate) enum TraceStart {
    Complete,
    Pending(PendingTraceStart),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceStartCompletion {
    Started,
    StoppedBeforeStart,
    ResponseHandledByEnd,
}

#[derive(Clone, Debug)]
pub(crate) struct TraceStartCommand {
    command_id: Option<u64>,
    session_id: Option<String>,
}

impl TraceStartCommand {
    pub(crate) fn new(command_id: Option<u64>, session_id: Option<&str>) -> Self {
        Self {
            command_id,
            session_id: session_id.map(str::to_owned),
        }
    }

    pub(crate) fn into_parts(self) -> (Option<u64>, Option<String>) {
        (self.command_id, self.session_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TraceStartResponseState {
    Pending,
    ClaimedByStart,
    ClaimedByEnd,
    Cancelled,
}

#[derive(Debug)]
struct TraceStartResponseCoordinator {
    command: TraceStartCommand,
    state: Mutex<TraceStartResponseState>,
}

impl TraceStartResponseCoordinator {
    fn new(command: TraceStartCommand) -> Self {
        Self {
            command,
            state: Mutex::new(TraceStartResponseState::Pending),
        }
    }

    fn complete_from_data_source(
        &self,
        completion: TraceDataSourceStartCompletion,
    ) -> TraceStartCompletion {
        let mut state = self.state.lock();
        match *state {
            TraceStartResponseState::Pending => match completion {
                TraceDataSourceStartCompletion::Started => {
                    *state = TraceStartResponseState::ClaimedByStart;
                    TraceStartCompletion::Started
                }
                TraceDataSourceStartCompletion::StoppedBeforeStart => {
                    *state = TraceStartResponseState::Cancelled;
                    TraceStartCompletion::StoppedBeforeStart
                }
            },
            TraceStartResponseState::ClaimedByStart => TraceStartCompletion::Started,
            TraceStartResponseState::ClaimedByEnd => TraceStartCompletion::ResponseHandledByEnd,
            TraceStartResponseState::Cancelled => TraceStartCompletion::StoppedBeforeStart,
        }
    }

    fn claim_for_end(&self) -> Option<TraceStartCommand> {
        let mut state = self.state.lock();
        if *state != TraceStartResponseState::Pending {
            return None;
        }
        *state = TraceStartResponseState::ClaimedByEnd;
        Some(self.command.clone())
    }

    fn cancel(&self) {
        let mut state = self.state.lock();
        if *state == TraceStartResponseState::Pending {
            *state = TraceStartResponseState::Cancelled;
        }
    }
}

pub(crate) struct PendingTraceStart {
    data_sources: Vec<PendingTraceDataSourceStart>,
    response: Arc<TraceStartResponseCoordinator>,
}

enum PendingTraceDataSourceStart {
    V8Cpu(moli_v8_platform::PendingV8CpuTraceStart),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TraceDataSourceStartCompletion {
    Started,
    StoppedBeforeStart,
}

impl PendingTraceDataSourceStart {
    fn status(&self) -> Option<TraceDataSourceStartCompletion> {
        match self {
            Self::V8Cpu(pending) => pending.status().map(|status| match status {
                moli_v8_platform::V8CpuTraceStartStatus::Started => {
                    TraceDataSourceStartCompletion::Started
                }
                moli_v8_platform::V8CpuTraceStartStatus::StoppedBeforeStart => {
                    TraceDataSourceStartCompletion::StoppedBeforeStart
                }
            }),
        }
    }

    async fn wait(self) -> TraceDataSourceStartCompletion {
        match self {
            Self::V8Cpu(pending) => match pending.wait().await {
                moli_v8_platform::V8CpuTraceStartStatus::Started => {
                    TraceDataSourceStartCompletion::Started
                }
                moli_v8_platform::V8CpuTraceStartStatus::StoppedBeforeStart => {
                    TraceDataSourceStartCompletion::StoppedBeforeStart
                }
            },
        }
    }
}

impl PendingTraceStart {
    fn into_trace_start(self) -> TraceStart {
        if self
            .data_sources
            .iter()
            .all(|pending| pending.status() == Some(TraceDataSourceStartCompletion::Started))
        {
            let completion = self
                .response
                .complete_from_data_source(TraceDataSourceStartCompletion::Started);
            debug_assert_eq!(completion, TraceStartCompletion::Started);
            TraceStart::Complete
        } else {
            TraceStart::Pending(self)
        }
    }

    pub(crate) async fn wait(self) -> TraceStartCompletion {
        let mut completion = TraceDataSourceStartCompletion::Started;
        for pending in self.data_sources {
            if pending.wait().await == TraceDataSourceStartCompletion::StoppedBeforeStart {
                completion = TraceDataSourceStartCompletion::StoppedBeforeStart;
                break;
            }
        }
        self.response.complete_from_data_source(completion)
    }
}

#[derive(Debug)]
struct TraceCategoryFilter {
    included: HashSet<String>,
    excluded: HashSet<String>,
}

impl TraceCategoryFilter {
    fn new(included: Vec<String>, excluded: Vec<String>) -> Self {
        Self {
            included: included.into_iter().collect(),
            excluded: excluded.into_iter().collect(),
        }
    }

    fn allows(&self, category: &str) -> bool {
        if self
            .included
            .iter()
            .any(|pattern| category_pattern_matches(pattern, category))
        {
            return true;
        }
        if self
            .excluded
            .iter()
            .any(|pattern| category_pattern_matches(pattern, category))
        {
            return false;
        }
        self.included.is_empty()
    }

    fn explicitly_allows_disabled_category(&self, category: &str) -> bool {
        self.included.contains(category)
    }
}

#[derive(Debug)]
struct ActiveTrace {
    owner: CommandOwnerScope,
    start_response: Arc<TraceStartResponseCoordinator>,
    interrupted_start_response: Option<TraceStartCommand>,
    transfer_mode: TraceTransferMode,
    record_mode: TraceRecordMode,
    category_filter: TraceCategoryFilter,
    max_buffer_bytes: usize,
    events: VecDeque<Value>,
    event_bytes: usize,
    data_loss_occurred: bool,
    cpu_trace_session: Option<moli_v8_platform::V8CpuTraceSession>,
}

impl ActiveTrace {
    fn new(
        owner: CommandOwnerScope,
        start_command: TraceStartCommand,
        configuration: TraceConfiguration,
        frame: Option<TraceFrameSnapshot>,
    ) -> Result<(Self, PendingTraceStart), moli_v8_platform::V8CpuTraceStartError> {
        let start_response = Arc::new(TraceStartResponseCoordinator::new(start_command));
        let category_filter = TraceCategoryFilter::new(
            configuration.included_categories,
            configuration.excluded_categories,
        );
        let mut pending_data_sources = Vec::new();
        let cpu_trace_session = if category_filter
            .explicitly_allows_disabled_category(V8_CPU_PROFILE_CATEGORY)
        {
            let session = moli_v8_platform::start_v8_cpu_trace(
                moli_v8_platform::V8CpuTraceConfiguration::bounded_for_trace_buffer(
                    configuration.max_buffer_bytes,
                ),
            )?;
            pending_data_sources.push(PendingTraceDataSourceStart::V8Cpu(session.start_barrier()));
            Some(session)
        } else {
            None
        };
        let mut trace = Self {
            owner,
            start_response: Arc::clone(&start_response),
            interrupted_start_response: None,
            transfer_mode: configuration.transfer_mode,
            record_mode: configuration.record_mode,
            category_filter,
            max_buffer_bytes: configuration.max_buffer_bytes,
            events: VecDeque::new(),
            event_bytes: 0,
            data_loss_occurred: false,
            cpu_trace_session,
        };
        trace.push_unfiltered(metadata_event("process_name", json!({ "name": "moli" })));
        trace.push_unfiltered(metadata_event(
            "thread_name",
            json!({ "name": "cdp-owner" }),
        ));
        if trace
            .category_filter
            .allows("disabled-by-default-devtools.timeline")
        {
            trace.push_unfiltered(tracing_started_event(frame));
        }
        Ok((
            trace,
            PendingTraceStart {
                data_sources: pending_data_sources,
                response: start_response,
            },
        ))
    }

    fn push_for_category(&mut self, category: &str, event: Value) {
        if self.category_filter.allows(category) {
            self.push_unfiltered(event);
        }
    }

    fn push_unfiltered(&mut self, event: Value) {
        let event_bytes = serde_json::to_vec(&event).map_or(0, |encoded| encoded.len());
        if event_bytes == 0 || event_bytes > self.max_buffer_bytes {
            self.data_loss_occurred = true;
            return;
        }

        let would_overflow = |trace: &Self| {
            trace.events.len() >= MAX_TRACE_EVENTS
                || trace.event_bytes.saturating_add(event_bytes) > trace.max_buffer_bytes
        };
        if self.record_mode == TraceRecordMode::Continuously {
            while would_overflow(self) {
                let Some(removed) = self.events.pop_front() else {
                    break;
                };
                self.event_bytes = self.event_bytes.saturating_sub(
                    serde_json::to_vec(&removed).map_or(0, |encoded| encoded.len()),
                );
                self.data_loss_occurred = true;
            }
        }
        if would_overflow(self) {
            self.data_loss_occurred = true;
            return;
        }

        self.event_bytes = self.event_bytes.saturating_add(event_bytes);
        self.events.push_back(event);
    }

    fn into_completed(self) -> CompletedTrace {
        debug_assert!(self.cpu_trace_session.is_none());
        CompletedTrace {
            transfer_mode: self.transfer_mode,
            events: self.events.into_iter().collect(),
            data_loss_occurred: self.data_loss_occurred,
            interrupted_start_response: self.interrupted_start_response,
        }
    }

    fn append_cpu_trace_result(&mut self, result: moli_v8_platform::V8CpuTraceResult) {
        self.data_loss_occurred |= result.data_loss_occurred();
        for (profile_index, profile) in result.profiles().iter().enumerate() {
            match cpu_profile_trace_events(result.trace_id(), profile_index, profile) {
                Ok(events) => {
                    for event in events {
                        self.push_unfiltered(event);
                    }
                }
                Err(()) => self.data_loss_occurred = true,
            }
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct TracingState {
    active: Option<ActiveTrace>,
}

pub(crate) struct PendingTraceCancel {
    pending_cpu_trace: Option<moli_v8_platform::PendingV8CpuTraceStop>,
}

impl PendingTraceCancel {
    pub(crate) async fn wait(self) {
        if let Some(pending_cpu_trace) = self.pending_cpu_trace {
            let _ = pending_cpu_trace.wait().await;
        }
    }
}

impl TracingState {
    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn start(
        &mut self,
        owner: CommandOwnerScope,
        start_command: TraceStartCommand,
        configuration: TraceConfiguration,
        frame: Option<TraceFrameSnapshot>,
    ) -> Result<TraceStart, moli_v8_platform::V8CpuTraceStartError> {
        if self.active.is_some() {
            return Err(moli_v8_platform::V8CpuTraceStartError::AlreadyActive);
        }
        let (active, pending_start) = ActiveTrace::new(owner, start_command, configuration, frame)?;
        self.active = Some(active);
        Ok(pending_start.into_trace_start())
    }

    pub(crate) fn finish(&mut self, owner: &CommandOwnerScope) -> Option<TraceFinish> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| &active.owner != owner)
        {
            return None;
        }
        let mut active = self.active.take()?;
        active.interrupted_start_response = active.start_response.claim_for_end();
        match active.cpu_trace_session.take() {
            Some(session) => Some(TraceFinish::Pending(Box::new(PendingCompletedTrace {
                active,
                pending_cpu_trace: session.stop(),
            }))),
            None => Some(TraceFinish::Complete(active.into_completed())),
        }
    }

    pub(crate) fn cancel(&mut self, owner: &CommandOwnerScope) -> Option<PendingTraceCancel> {
        if !self
            .active
            .as_ref()
            .is_some_and(|active| &active.owner == owner)
        {
            return None;
        }
        let mut active = self
            .active
            .take()
            .expect("matching active trace must still be present");
        active.start_response.cancel();
        let pending_cpu_trace = active
            .cpu_trace_session
            .take()
            .map(moli_v8_platform::V8CpuTraceSession::cancel);
        drop(active);
        Some(PendingTraceCancel { pending_cpu_trace })
    }

    pub(crate) fn record_command(
        &mut self,
        method: &str,
        session_id: Option<&str>,
        frame: Option<&TraceFrameSnapshot>,
    ) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let category = category_for_protocol_method(method);
        active.push_for_category(
            category,
            protocol_activity_event("MoliDevToolsCommand", category, method, session_id, frame),
        );
    }

    pub(crate) fn record_protocol_event(
        &mut self,
        method: &str,
        session_id: Option<&str>,
        frame: Option<&TraceFrameSnapshot>,
    ) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let category = category_for_protocol_method(method);
        active.push_for_category(
            category,
            protocol_activity_event("MoliProtocolEvent", category, method, session_id, frame),
        );
    }

    pub(crate) fn record_clock_sync_marker(&mut self, sync_id: &str) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        active.push_unfiltered(json!({
            "args": { "sync_id": sync_id },
            "cat": "__metadata",
            "name": "clock_sync",
            "ph": "c",
            "pid": std::process::id(),
            "tid": current_trace_thread_id(),
            "ts": trace_timestamp_micros(),
        }));
        true
    }

    pub(crate) fn diagnostics(&self) -> Value {
        let Some(active) = self.active.as_ref() else {
            return json!({ "active": false });
        };
        json!({
            "active": true,
            "eventCount": active.events.len(),
            "eventBytes": active.event_bytes,
            "maxBufferBytes": active.max_buffer_bytes,
            "dataLossOccurred": active.data_loss_occurred,
        })
    }
}

pub(crate) enum TraceFinish {
    Complete(CompletedTrace),
    Pending(Box<PendingCompletedTrace>),
}

pub(crate) struct PendingCompletedTrace {
    active: ActiveTrace,
    pending_cpu_trace: moli_v8_platform::PendingV8CpuTraceStop,
}

impl PendingCompletedTrace {
    pub(crate) async fn wait(mut self) -> CompletedTrace {
        let cpu_trace = self.pending_cpu_trace.wait().await;
        self.active.append_cpu_trace_result(cpu_trace);
        self.active.into_completed()
    }
}

#[derive(Debug)]
pub(crate) struct CompletedTrace {
    transfer_mode: TraceTransferMode,
    events: Vec<Value>,
    data_loss_occurred: bool,
    interrupted_start_response: Option<TraceStartCommand>,
}

pub(crate) enum CompletedTraceOutput {
    ReportEvents {
        chunks: Vec<Vec<Value>>,
        data_loss_occurred: bool,
    },
    ReturnAsStream {
        bytes: Vec<u8>,
        data_loss_occurred: bool,
    },
}

impl CompletedTrace {
    pub(crate) fn take_interrupted_start_response(&mut self) -> Option<TraceStartCommand> {
        self.interrupted_start_response.take()
    }

    pub(crate) fn into_output(self) -> Result<CompletedTraceOutput, serde_json::Error> {
        match self.transfer_mode {
            TraceTransferMode::ReportEvents => Ok(CompletedTraceOutput::ReportEvents {
                chunks: report_event_chunks(self.events),
                data_loss_occurred: self.data_loss_occurred,
            }),
            TraceTransferMode::ReturnAsStream => {
                let bytes = serde_json::to_vec(&json!({
                    "traceEvents": self.events,
                    "metadata": {
                        "product": "moli",
                        "processId": std::process::id(),
                    },
                }))?;
                Ok(CompletedTraceOutput::ReturnAsStream {
                    bytes,
                    data_loss_occurred: self.data_loss_occurred,
                })
            }
        }
    }
}

fn cpu_profile_trace_events(
    trace_id: u64,
    profile_index: usize,
    profile: &moli_v8_platform::V8CpuProfileSegment,
) -> Result<Vec<Value>, ()> {
    cpu_profile_trace_events_from_json(
        trace_id,
        profile_index,
        profile.isolate_id(),
        profile.sample_count(),
        profile.profile_json(),
    )
}

fn cpu_profile_trace_events_from_json(
    trace_id: u64,
    profile_index: usize,
    isolate_id: u64,
    sample_count: u32,
    profile_json: &[u8],
) -> Result<Vec<Value>, ()> {
    let Value::Object(mut serialized) =
        serde_json::from_slice::<Value>(profile_json).map_err(|_| ())?
    else {
        return Err(());
    };
    let start_time = serialized.remove("startTime").ok_or(())?;
    let end_time = serialized.remove("endTime").ok_or(())?;
    let Value::Array(mut nodes) = serialized.remove("nodes").ok_or(())? else {
        return Err(());
    };
    let Value::Array(samples) = serialized.remove("samples").ok_or(())? else {
        return Err(());
    };
    let Value::Array(time_deltas) = serialized.remove("timeDeltas").ok_or(())? else {
        return Err(());
    };
    if nodes.is_empty()
        || samples.len() != time_deltas.len()
        || samples.len() != sample_count as usize
    {
        return Err(());
    }

    let mut parent_by_node = std::collections::HashMap::<u64, u64>::new();
    for node in &nodes {
        let object = node.as_object().ok_or(())?;
        let parent_id = object.get("id").and_then(Value::as_u64).ok_or(())?;
        if let Some(children) = object.get("children") {
            for child_id in children.as_array().ok_or(())? {
                let child_id = child_id.as_u64().ok_or(())?;
                if parent_by_node.insert(child_id, parent_id).is_some() {
                    return Err(());
                }
            }
        }
    }
    for node in &mut nodes {
        let object = node.as_object_mut().ok_or(())?;
        let node_id = object.get("id").and_then(Value::as_u64).ok_or(())?;
        object.remove("children");
        object.remove("hitCount");
        object.remove("positionTicks");
        if let Some(parent) = parent_by_node.get(&node_id) {
            object.insert("parent".to_owned(), json!(parent));
        }
        if let Some(call_frame) = object.get_mut("callFrame").and_then(Value::as_object_mut)
            && let Some(script_id) = call_frame.get("scriptId").and_then(Value::as_str)
            && let Ok(script_id) = script_id.parse::<u64>()
        {
            call_frame.insert("scriptId".to_owned(), json!(script_id));
        }
    }

    let profile_wire_id = format!(
        "0x{:x}",
        trace_id
            .wrapping_mul(0x1_0000)
            .wrapping_add(profile_index as u64 + 1)
    );
    let thread_id = isolate_id & TRACE_THREAD_ID_MASK;
    let common = |name: &str, timestamp: Value, data: Value| {
        json!({
            "args": { "data": data },
            "cat": V8_CPU_PROFILE_CATEGORY,
            "id": profile_wire_id,
            "name": name,
            "ph": "P",
            "pid": std::process::id(),
            "tid": thread_id,
            "ts": timestamp,
        })
    };

    let mut events = vec![common(
        "Profile",
        start_time.clone(),
        json!({ "source": "Internal", "startTime": start_time }),
    )];
    for node_chunk in nodes.chunks(CPU_PROFILE_NODE_CHUNK_SIZE) {
        events.push(common(
            "ProfileChunk",
            end_time.clone(),
            json!({
                "cpuProfile": { "nodes": node_chunk },
                "source": "Internal",
            }),
        ));
    }
    for (sample_chunk, delta_chunk) in samples
        .chunks(CPU_PROFILE_SAMPLE_CHUNK_SIZE)
        .zip(time_deltas.chunks(CPU_PROFILE_SAMPLE_CHUNK_SIZE))
    {
        events.push(common(
            "ProfileChunk",
            end_time.clone(),
            json!({
                "cpuProfile": { "samples": sample_chunk },
                "source": "Internal",
                "timeDeltas": delta_chunk,
            }),
        ));
    }
    events.push(common(
        "ProfileChunk",
        end_time.clone(),
        json!({ "endTime": end_time, "source": "Internal" }),
    ));
    Ok(events)
}

impl CdpConnection {
    pub(crate) fn tracing_owner_scope(&self, session_id: Option<&str>) -> CommandOwnerScope {
        CommandOwnerScope::capture(self, session_id)
    }

    pub(crate) fn tracing_frame_snapshot(
        &self,
        session_id: Option<&str>,
    ) -> Option<TraceFrameSnapshot> {
        let (frame_id, url, _, _) = self.target_session_owner_frame_tree_identity(session_id)?;
        Some(TraceFrameSnapshot {
            frame_id,
            url: strip_url_fragment(&url),
        })
    }

    pub(crate) fn record_tracing_command(&mut self, method: &str, session_id: Option<&str>) {
        if !self.tracing_state.is_active() {
            return;
        }
        let frame = self.tracing_frame_snapshot(session_id);
        self.tracing_state
            .record_command(method, session_id, frame.as_ref());
    }

    pub(crate) fn record_tracing_protocol_events(&mut self, events: &[BackgroundProtocolEvent]) {
        if !self.tracing_state.is_active() {
            return;
        }
        let records = events
            .iter()
            .filter_map(|event| {
                let message = event.clone().into_protocol_message();
                let method = message.get("method")?.as_str()?.to_owned();
                let session_id = message
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let frame = self.tracing_frame_snapshot(session_id.as_deref());
                Some((method, session_id, frame))
            })
            .collect::<Vec<_>>();
        for (method, session_id, frame) in records {
            self.tracing_state.record_protocol_event(
                &method,
                session_id.as_deref(),
                frame.as_ref(),
            );
        }
    }

    pub(crate) fn cancel_tracing_for_session_owner(&mut self, session_id: Option<&str>) -> bool {
        let owner = self.tracing_owner_scope(session_id);
        self.tracing_state.cancel(&owner).is_some()
    }

    #[cfg(test)]
    pub(crate) async fn cancel_tracing_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        let Some(pending) = self.start_cancel_tracing_for_session_owner(session_id) else {
            return false;
        };
        pending.wait().await;
        true
    }

    pub(crate) fn start_cancel_tracing_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<PendingTraceCancel> {
        let owner = self.tracing_owner_scope(session_id);
        self.tracing_state.cancel(&owner)
    }
}

fn report_event_chunks(events: Vec<Value>) -> Vec<Vec<Value>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_bytes = 0_usize;
    for event in events {
        let event_bytes = serde_json::to_vec(&event).map_or(0, |encoded| encoded.len());
        if !current.is_empty()
            && current_bytes.saturating_add(event_bytes) > REPORT_EVENT_CHUNK_BYTES
        {
            chunks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes = current_bytes.saturating_add(event_bytes);
        current.push(event);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn category_pattern_matches(pattern: &str, category: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    pattern
        .strip_suffix('*')
        .map_or(pattern == category, |prefix| category.starts_with(prefix))
}

fn category_for_protocol_method(method: &str) -> &'static str {
    match method.split_once('.').map(|(domain, _)| domain) {
        Some("Page" | "DOM" | "Input") => "devtools.timeline",
        Some("Runtime" | "Debugger" | "Profiler" | "HeapProfiler") => "v8.execute",
        Some("Network" | "Fetch") => "loading",
        _ => "moli.devtools",
    }
}

fn metadata_event(name: &str, args: Value) -> Value {
    json!({
        "args": args,
        "cat": "__metadata",
        "name": name,
        "ph": "M",
        "pid": std::process::id(),
        "tid": current_trace_thread_id(),
        "ts": trace_timestamp_micros(),
    })
}

fn tracing_started_event(frame: Option<TraceFrameSnapshot>) -> Value {
    let frames = frame
        .map(|frame| {
            vec![json!({
                "frame": frame.frame_id,
                "url": frame.url,
                "name": "",
                "isOutermostMainFrame": true,
                "isInPrimaryMainFrame": true,
                "processId": std::process::id(),
            })]
        })
        .unwrap_or_default();
    json!({
        "args": {
            "data": {
                "persistentIds": true,
                "frames": frames,
            },
        },
        "cat": "disabled-by-default-devtools.timeline",
        "name": "TracingStartedInBrowser",
        "ph": "I",
        "pid": std::process::id(),
        "s": "t",
        "tid": current_trace_thread_id(),
        "ts": trace_timestamp_micros(),
    })
}

fn protocol_activity_event(
    name: &str,
    category: &str,
    method: &str,
    session_id: Option<&str>,
    frame: Option<&TraceFrameSnapshot>,
) -> Value {
    let mut args = Map::new();
    args.insert("method".to_owned(), json!(method));
    if let Some(session_id) = session_id {
        args.insert("sessionId".to_owned(), json!(session_id));
    }
    if let Some(frame) = frame {
        args.insert("frameId".to_owned(), json!(&frame.frame_id));
    }
    json!({
        "args": Value::Object(args),
        "cat": category,
        "name": name,
        "ph": "I",
        "pid": std::process::id(),
        "s": "t",
        "tid": current_trace_thread_id(),
        "ts": trace_timestamp_micros(),
    })
}

fn trace_timestamp_micros() -> u64 {
    let micros = monotonic_timestamp_seconds() * 1_000_000.0;
    if micros.is_finite() && micros > 0.0 {
        micros.round().min(u64::MAX as f64) as u64
    } else {
        0
    }
}

fn current_trace_thread_id() -> u64 {
    let mut hasher = DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    hasher.finish() & TRACE_THREAD_ID_MASK
}

fn strip_url_fragment(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.split_once('#').map_or(url, |(base, _)| base).to_owned();
    };
    parsed.set_fragment(None);
    parsed.into()
}

#[cfg(test)]
mod cpu_profile_tests {
    use super::*;

    #[test]
    fn cpu_profiler_requires_explicit_disabled_category_selection() {
        let default_filter = TraceCategoryFilter::new(Vec::new(), Vec::new());
        assert!(default_filter.allows(V8_CPU_PROFILE_CATEGORY));
        assert!(
            !default_filter.explicitly_allows_disabled_category(V8_CPU_PROFILE_CATEGORY),
            "default category selection must not activate disabled-by-default sampling"
        );

        let broad_filter = TraceCategoryFilter::new(vec!["*".to_owned()], Vec::new());
        assert!(!broad_filter.explicitly_allows_disabled_category(V8_CPU_PROFILE_CATEGORY));

        let wildcard_filter = TraceCategoryFilter::new(
            vec!["disabled-by-default-v8.cpu_profiler*".to_owned()],
            vec!["*".to_owned()],
        );
        assert!(
            !wildcard_filter.explicitly_allows_disabled_category(V8_CPU_PROFILE_CATEGORY),
            "Chromium does not expand wildcards when enabling disabled data sources"
        );

        let profiler_filter = TraceCategoryFilter::new(
            vec![V8_CPU_PROFILE_CATEGORY.to_owned()],
            vec!["*".to_owned()],
        );
        assert!(profiler_filter.explicitly_allows_disabled_category(V8_CPU_PROFILE_CATEGORY));
    }

    #[test]
    fn serialized_v8_profile_is_converted_to_chromium_trace_chunks() {
        let profile = json!({
            "nodes": [
                {
                    "id": 1,
                    "callFrame": {
                        "functionName": "(root)",
                        "scriptId": "0",
                        "url": "",
                        "lineNumber": -1,
                        "columnNumber": -1
                    },
                    "hitCount": 0,
                    "children": [2]
                },
                {
                    "id": 2,
                    "callFrame": {
                        "functionName": "namedHotFunction",
                        "scriptId": "7",
                        "url": "https://example.test/hot.js",
                        "lineNumber": 3,
                        "columnNumber": 4
                    },
                    "hitCount": 2
                }
            ],
            "startTime": 1000,
            "endTime": 1300,
            "samples": [2, 2],
            "timeDeltas": [100, 200]
        });
        let events = cpu_profile_trace_events_from_json(
            4,
            2,
            19,
            2,
            &serde_json::to_vec(&profile).expect("profile JSON"),
        )
        .expect("valid V8 profile should convert");

        assert_eq!(events[0]["name"], "Profile");
        assert_eq!(events[0]["args"]["data"]["source"], "Internal");
        assert_eq!(events[0]["args"]["data"]["startTime"], 1000);
        assert_eq!(events[0]["tid"], 19);
        let chunks = events
            .iter()
            .filter(|event| event["name"] == "ProfileChunk")
            .collect::<Vec<_>>();
        let nodes = chunks
            .iter()
            .flat_map(|event| {
                event["args"]["data"]["cpuProfile"]["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .collect::<Vec<_>>();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[1]["parent"], 1);
        assert_eq!(nodes[1]["callFrame"]["scriptId"], 7);
        assert!(nodes.iter().all(|node| node.get("children").is_none()));
        assert!(nodes.iter().all(|node| node.get("hitCount").is_none()));
        let sample_chunk = chunks
            .iter()
            .find(|event| event["args"]["data"]["cpuProfile"]["samples"].is_array())
            .expect("sample chunk");
        assert_eq!(
            sample_chunk["args"]["data"]["cpuProfile"]["samples"],
            json!([2, 2])
        );
        assert_eq!(
            sample_chunk["args"]["data"]["timeDeltas"],
            json!([100, 200])
        );
        assert_eq!(chunks.last().unwrap()["args"]["data"]["endTime"], 1300);
    }

    #[test]
    fn malformed_v8_profile_is_rejected_instead_of_emitting_invalid_samples() {
        let profile = json!({
            "nodes": [{ "id": 1, "callFrame": { "functionName": "(root)" } }],
            "startTime": 1000,
            "endTime": 1300,
            "samples": [1, 1],
            "timeDeltas": [100]
        });
        assert!(
            cpu_profile_trace_events_from_json(
                1,
                0,
                1,
                2,
                &serde_json::to_vec(&profile).expect("profile JSON"),
            )
            .is_err()
        );
    }
}
