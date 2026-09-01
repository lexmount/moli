use serde::Deserialize;
use serde_json::json;

use crate::conn::{BackgroundProtocolEvent, CdpConnection, Cmd, build_event};
use crate::domains::actions::TracingAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) use state::TracingState;
use state::{
    CompletedTraceOutput, DEFAULT_TRACE_BUFFER_BYTES, MAX_TRACE_BUFFER_BYTES,
    PendingCompletedTrace, PendingTraceStart, SUPPORTED_CATEGORIES, TraceConfiguration,
    TraceFinish, TraceRecordMode, TraceStart, TraceStartCommand, TraceStartCompletion,
    TraceTransferMode,
};

mod state;

#[cfg(test)]
mod tests;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TracingStartParams {
    #[serde(default)]
    categories: Option<String>,
    #[serde(default)]
    options: Option<String>,
    #[serde(default)]
    buffer_usage_reporting_interval: Option<f64>,
    #[serde(default)]
    transfer_mode: Option<String>,
    #[serde(default)]
    stream_format: Option<String>,
    #[serde(default)]
    stream_compression: Option<String>,
    #[serde(default)]
    trace_config: Option<TracingTraceConfigParams>,
    #[serde(default)]
    perfetto_config: Option<String>,
    #[serde(default)]
    tracing_backend: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TracingTraceConfigParams {
    #[serde(default)]
    record_mode: Option<String>,
    #[serde(default)]
    trace_buffer_size_in_kb: Option<f64>,
    #[serde(default)]
    enable_sampling: Option<bool>,
    #[serde(default)]
    enable_systrace: Option<bool>,
    #[serde(default)]
    enable_argument_filter: Option<bool>,
    #[serde(default)]
    included_categories: Vec<String>,
    #[serde(default)]
    excluded_categories: Vec<String>,
    #[serde(default)]
    synthetic_delays: Vec<String>,
    #[serde(default)]
    memory_dump_config: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordClockSyncMarkerParams {
    sync_id: String,
}

pub(crate) struct PendingTracingCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    pending: PendingTracingOperation,
}

pub(crate) struct CompletedTracingCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    completed: CompletedTracingOperation,
}

enum PendingTracingOperation {
    Start(PendingTraceStart),
    End(Box<PendingCompletedTrace>),
}

enum CompletedTracingOperation {
    Start(TraceStartCompletion),
    End(state::CompletedTrace),
}

pub(crate) enum TracingCommandTaskStep {
    Pending(Box<PendingTracingCommandDispatch>),
    Complete(CommandOutputPlan),
}

impl PendingTracingCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedTracingCommandDispatch {
        let completed = match self.pending {
            PendingTracingOperation::Start(pending) => {
                CompletedTracingOperation::Start(pending.wait().await)
            }
            PendingTracingOperation::End(pending) => {
                CompletedTracingOperation::End(pending.wait().await)
            }
        };
        CompletedTracingCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            completed,
        }
    }
}

impl CompletedTracingCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

pub(crate) fn try_start_tracing_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TracingCommandTaskStep {
    match cmd.parse_action::<TracingAction>() {
        Some(TracingAction::Start) => start_command(conn, cmd),
        Some(TracingAction::End) => end_command(conn, cmd),
        Some(TracingAction::GetCategories) => {
            TracingCommandTaskStep::Complete(get_categories_command())
        }
        Some(TracingAction::RecordClockSyncMarker) => {
            TracingCommandTaskStep::Complete(record_clock_sync_marker_command(conn, cmd))
        }
        None => TracingCommandTaskStep::Complete(CommandOutputPlan::error(-32601, "UnknownMethod")),
    }
}

fn end_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> TracingCommandTaskStep {
    let owner = conn.tracing_owner_scope(cmd.session_id);
    let Some(finish) = conn.tracing_state.finish(&owner) else {
        return TracingCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "Tracing is not started",
        ));
    };
    match finish {
        TraceFinish::Complete(completed) => TracingCommandTaskStep::Complete(
            completed_trace_output_plan(conn, completed, cmd.session_id),
        ),
        TraceFinish::Pending(pending) => {
            TracingCommandTaskStep::Pending(Box::new(PendingTracingCommandDispatch {
                command_id: cmd.id,
                session_id: cmd.session_id.map(str::to_owned),
                pending: PendingTracingOperation::End(pending),
            }))
        }
    }
}

fn start_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> TracingCommandTaskStep {
    let params = match cmd.get_params::<TracingStartParams>() {
        Ok(Some(params)) => params,
        Ok(None) => TracingStartParams::default(),
        Err(_) => return TracingCommandTaskStep::Complete(invalid_parameters()),
    };
    if conn.tracing_state.is_active() {
        return TracingCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "Tracing has already been started (possibly in another tab).",
        ));
    }
    let configuration = match trace_configuration(params) {
        Ok(configuration) => configuration,
        Err(message) => {
            return TracingCommandTaskStep::Complete(CommandOutputPlan::error(-32602, message));
        }
    };
    let owner = conn.tracing_owner_scope(cmd.session_id);
    let frame = conn.tracing_frame_snapshot(cmd.session_id);
    let start = match conn.tracing_state.start(
        owner,
        TraceStartCommand::new(cmd.id, cmd.session_id),
        configuration,
        frame,
    ) {
        Ok(start) => start,
        Err(_) => {
            return TracingCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "Tracing has already been started (possibly in another tab).",
            ));
        }
    };
    match start {
        TraceStart::Complete => TracingCommandTaskStep::Complete(CommandOutputPlan::success()),
        TraceStart::Pending(pending) => {
            TracingCommandTaskStep::Pending(Box::new(PendingTracingCommandDispatch {
                command_id: cmd.id,
                session_id: cmd.session_id.map(str::to_owned),
                pending: PendingTracingOperation::Start(pending),
            }))
        }
    }
}

pub(crate) fn complete_pending_tracing_command(
    conn: &mut CdpConnection,
    completed: CompletedTracingCommandDispatch,
) -> CommandOutputPlan {
    let CompletedTracingCommandDispatch {
        session_id,
        completed,
        ..
    } = completed;
    match completed {
        CompletedTracingOperation::Start(TraceStartCompletion::Started) => {
            CommandOutputPlan::success()
        }
        CompletedTracingOperation::Start(TraceStartCompletion::StoppedBeforeStart) => {
            CommandOutputPlan::error(
                -32000,
                "Tracing was stopped before start has been completed.",
            )
        }
        CompletedTracingOperation::Start(TraceStartCompletion::ResponseHandledByEnd) => {
            CommandOutputPlan::default()
        }
        CompletedTracingOperation::End(completed) => {
            completed_trace_output_plan(conn, completed, session_id.as_deref())
        }
    }
}

fn completed_trace_output_plan(
    conn: &mut CdpConnection,
    mut completed: state::CompletedTrace,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let interrupted_start_response = completed.take_interrupted_start_response();
    let output = match completed.into_output() {
        Ok(output) => output,
        Err(_) => {
            let mut plan = CommandOutputPlan::error(-32000, "Tracing failed");
            push_interrupted_start_error(&mut plan, interrupted_start_response);
            return plan;
        }
    };

    // Chromium acknowledges Tracing.end before flushing trace payload events.
    let mut plan = CommandOutputPlan::default();
    plan.push_success();
    push_interrupted_start_error(&mut plan, interrupted_start_response);
    match output {
        CompletedTraceOutput::ReportEvents {
            chunks,
            data_loss_occurred,
        } => {
            for chunk in chunks {
                plan.push_background_event(BackgroundProtocolEvent::immediate(build_event(
                    "Tracing.dataCollected",
                    json!({ "value": chunk }),
                    session_id,
                )));
            }
            plan.push_background_event(BackgroundProtocolEvent::immediate(build_event(
                "Tracing.tracingComplete",
                json!({ "dataLossOccurred": data_loss_occurred }),
                session_id,
            )));
        }
        CompletedTraceOutput::ReturnAsStream {
            bytes,
            data_loss_occurred,
        } => {
            let stream = conn.open_global_io_stream(bytes);
            plan.push_background_event(BackgroundProtocolEvent::immediate(build_event(
                "Tracing.tracingComplete",
                json!({
                    "dataLossOccurred": data_loss_occurred,
                    "stream": stream,
                    "traceFormat": "json",
                    "streamCompression": "none",
                }),
                session_id,
            )));
        }
    }
    plan
}

fn push_interrupted_start_error(
    plan: &mut CommandOutputPlan,
    interrupted_start_response: Option<TraceStartCommand>,
) {
    if let Some(start_command) = interrupted_start_response {
        let (command_id, start_session_id) = start_command.into_parts();
        plan.push_background_event(BackgroundProtocolEvent::command_error(
            command_id,
            start_session_id.as_deref(),
            -32000,
            "Tracing was stopped before start has been completed.".to_owned(),
            None,
        ));
    }
}

fn get_categories_command() -> CommandOutputPlan {
    CommandOutputPlan::result(json!({ "categories": SUPPORTED_CATEGORIES }))
}

fn record_clock_sync_marker_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params = match cmd.get_params::<RecordClockSyncMarkerParams>() {
        Ok(Some(params)) => params,
        _ => return invalid_parameters(),
    };
    if !conn.tracing_state.record_clock_sync_marker(&params.sync_id) {
        return CommandOutputPlan::error(-32000, "Tracing is not started");
    }
    CommandOutputPlan::success()
}

fn trace_configuration(params: TracingStartParams) -> Result<TraceConfiguration, &'static str> {
    if params.trace_config.is_some() && (params.categories.is_some() || params.options.is_some()) {
        return Err(
            "Either trace config (preferred), or categories+options should be specified, but not both.",
        );
    }
    if params.perfetto_config.is_some() {
        return Err("Perfetto trace configuration is not supported.");
    }
    if !matches!(
        params.tracing_backend.as_deref(),
        None | Some("auto" | "chrome")
    ) {
        return Err("Unsupported value for tracing_backend parameter.");
    }
    if params
        .buffer_usage_reporting_interval
        .is_some_and(|interval| interval != 0.0)
    {
        return Err("Periodic tracing buffer usage reports are not supported.");
    }

    let transfer_mode = match params.transfer_mode.as_deref().unwrap_or("ReportEvents") {
        "ReportEvents" => TraceTransferMode::ReportEvents,
        "ReturnAsStream" => TraceTransferMode::ReturnAsStream,
        _ => return Err("Invalid transferMode."),
    };
    let stream_format = params.stream_format.as_deref().unwrap_or("json");
    if stream_format == "proto" && transfer_mode != TraceTransferMode::ReturnAsStream {
        return Err("Proto format is only supported when using stream transfer mode.");
    }
    if !matches!(stream_format, "json" | "proto") {
        return Err("Invalid streamFormat.");
    }
    if stream_format == "proto" {
        return Err("Proto trace streams are not supported.");
    }
    let stream_compression = params.stream_compression.as_deref().unwrap_or("none");
    if !matches!(stream_compression, "none" | "gzip") {
        return Err("Invalid streamCompression.");
    }
    if transfer_mode == TraceTransferMode::ReturnAsStream && stream_compression == "gzip" {
        return Err("Gzip trace streams are not supported.");
    }

    let mut included_categories = Vec::new();
    let mut excluded_categories = Vec::new();
    let mut record_mode = TraceRecordMode::UntilFull;
    let mut max_buffer_bytes = DEFAULT_TRACE_BUFFER_BYTES;
    if let Some(categories) = params.categories {
        for category in categories
            .split(',')
            .map(str::trim)
            .filter(|category| !category.is_empty())
        {
            if let Some(excluded) = category.strip_prefix('-') {
                excluded_categories.push(excluded.to_owned());
            } else {
                included_categories.push(category.to_owned());
            }
        }
    }
    if let Some(options) = params.options {
        record_mode = parse_deprecated_record_mode(&options)?;
    }
    if let Some(trace_config) = params.trace_config {
        // Chromium accepts the legacy enableSampling field, but its current
        // TraceConfig conversion does not use it. Sampling sources are selected
        // by trace categories instead, so preserve that wire compatibility.
        let _enable_sampling = trace_config.enable_sampling;
        if trace_config.enable_systrace == Some(true)
            || trace_config.enable_argument_filter == Some(true)
            || !trace_config.synthetic_delays.is_empty()
            || trace_config.memory_dump_config.is_some()
        {
            return Err("The requested traceConfig feature is not supported.");
        }
        record_mode = parse_record_mode(trace_config.record_mode.as_deref())?;
        max_buffer_bytes = trace_buffer_bytes(trace_config.trace_buffer_size_in_kb)?;
        included_categories = trace_config.included_categories;
        excluded_categories = trace_config.excluded_categories;
    }

    Ok(TraceConfiguration {
        transfer_mode,
        record_mode,
        included_categories,
        excluded_categories,
        max_buffer_bytes,
    })
}

fn parse_record_mode(value: Option<&str>) -> Result<TraceRecordMode, &'static str> {
    match value.unwrap_or("recordUntilFull") {
        "recordUntilFull" | "recordAsMuchAsPossible" => Ok(TraceRecordMode::UntilFull),
        "recordContinuously" => Ok(TraceRecordMode::Continuously),
        "echoToConsole" => Err("echoToConsole trace mode is not supported."),
        _ => Err("Invalid trace recordMode."),
    }
}

fn parse_deprecated_record_mode(options: &str) -> Result<TraceRecordMode, &'static str> {
    let mut mode = TraceRecordMode::UntilFull;
    for option in options
        .split(',')
        .map(str::trim)
        .filter(|option| !option.is_empty())
    {
        mode = match option {
            "record-until-full" | "record-as-much-as-possible" => TraceRecordMode::UntilFull,
            "record-continuously" => TraceRecordMode::Continuously,
            "trace-to-console" => return Err("trace-to-console mode is not supported."),
            _ => return Err("Invalid tracing options."),
        };
    }
    Ok(mode)
}

fn trace_buffer_bytes(value: Option<f64>) -> Result<usize, &'static str> {
    let Some(kilobytes) = value else {
        return Ok(DEFAULT_TRACE_BUFFER_BYTES);
    };
    if !kilobytes.is_finite() || kilobytes < 0.0 {
        return Err("Invalid traceBufferSizeInKb.");
    }
    if kilobytes == 0.0 {
        return Ok(DEFAULT_TRACE_BUFFER_BYTES);
    }
    let bytes = (kilobytes * 1024.0).round();
    Ok((bytes as usize).clamp(1, MAX_TRACE_BUFFER_BYTES))
}

fn invalid_parameters() -> CommandOutputPlan {
    CommandOutputPlan::error(-32602, "Invalid parameters")
}
