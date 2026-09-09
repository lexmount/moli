use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde_json::{Value, json};

use crate::devtools_runtime::{
    AutomationEvent, BrowserDownloadProgressEvent, BrowserDownloadWillBeginEvent,
    DevToolsCommandResult, DevToolsError, DevToolsErrorKind, DevToolsFetchRequestId,
    DevToolsFrameId, DevToolsLoaderId, DevToolsNetworkDataBytesType, DevToolsNetworkInterceptId,
    DevToolsNetworkResourceType, DevToolsRealmId, DevToolsRemoteValue, DevToolsRequestId,
    DevToolsScriptException, DevToolsScriptResult, DevToolsStackTrace, DevToolsTargetId,
    DevToolsTargetInfo, NavigationFrameEvent, NavigationFrameEventKind, NetworkAuthChallengeEvent,
    NetworkRedirectResponseEvent, NetworkRequestEvent, PageFileChooserOpenedEvent,
    PageJavaScriptDialogOpeningEvent, PageLifecycleEvent, RuntimeConsoleEvent,
    RuntimeExecutionContextEvent, SameDocumentNavigationEvent, ScriptExceptionEvent,
};
use moli_cookie_jar::{
    CookiePriority, StoredCookie, StoredCookieAccess, StoredCookieAccessSemantics,
    StoredCookieBrowserContextValueSource, StoredCookieEffectiveSameSite,
    StoredCookieExclusionReason, StoredCookiePartitionKey, StoredCookieQueryReport,
    StoredCookieRequestSameSiteContext, StoredCookieSameSite,
    StoredCookieSameSiteContextDowngradeType, StoredCookieSameSiteHttpMethod,
    StoredCookieSameSiteRedirectType, StoredCookieScopeSemantics, StoredCookieSiteContextBasis,
    StoredCookieSourceScheme, StoredCookieStorageAccessStatus, StoredCookieWarningReason,
};

use crate::conn::{BackgroundProtocolEvent, build_command_success_response, build_event};
use crate::domains::observable_output::log_entry_event;

#[derive(Debug, Default)]
pub(crate) struct CommandOutputPlan {
    outputs: Vec<CommandOutput>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    renderer_output_boundary: Option<Box<RendererOutputBoundary>>,
}

#[derive(Debug)]
struct RendererOutputBoundary {
    output_index: usize,
    cursor: moli_core::RendererOutputFence,
}

#[derive(Debug)]
enum CommandOutput {
    Command(CommandResponseOutput),
    CommandWithoutSession(CommandResponseOutput),
    OwnerEvent(CommandOwnerEvent),
    BackgroundEvent(BackgroundProtocolEvent),
}

#[derive(Debug)]
enum CommandResponseOutput {
    Success(CommandResponseResult),
    Error {
        code: i32,
        message: String,
        data: Option<Value>,
    },
}

#[derive(Debug)]
enum CommandResponseResult {
    Empty,
    Json(Value),
}

#[derive(Debug)]
enum CommandOwnerEvent {
    InspectorTargetCrashed {
        session_id: Option<String>,
    },
    PageLifecycle {
        session_id: Option<String>,
        name: String,
        frame_id: String,
        loader_id: String,
        timestamp: f64,
    },
}

impl CommandOutputPlan {
    pub(crate) fn success() -> Self {
        Self::from_devtools_result(DevToolsCommandResult::Empty)
    }

    pub(crate) fn error(code: i32, message: impl Into<String>) -> Self {
        let mut plan = Self::default();
        plan.push_error(code, message);
        plan
    }

    pub(crate) fn error_without_session(code: i32, message: impl Into<String>) -> Self {
        let mut plan = Self::default();
        plan.push_error_without_session(code, message);
        plan
    }

    pub(crate) fn result(value: Value) -> Self {
        let mut plan = Self::default();
        plan.push_result(value);
        plan
    }

    pub(crate) fn from_devtools_result(result: DevToolsCommandResult) -> Self {
        Self::result(Self::devtools_result_payload(result))
    }

    pub(crate) fn devtools_result_payload(result: DevToolsCommandResult) -> Value {
        cdp_result_payload_from_devtools_result(result)
    }

    pub(crate) fn from_devtools_error(error: DevToolsError) -> Self {
        Self::error(cdp_error_code_from_devtools_error(&error), error.message)
    }

    pub(crate) fn push_success(&mut self) {
        self.outputs
            .push(CommandOutput::Command(CommandResponseOutput::Success(
                CommandResponseResult::Empty,
            )));
    }

    pub(crate) fn push_result(&mut self, value: Value) {
        self.outputs
            .push(CommandOutput::Command(CommandResponseOutput::Success(
                CommandResponseResult::Json(value),
            )));
    }

    pub(crate) fn push_error(&mut self, code: i32, message: impl Into<String>) {
        self.push_error_with_data(code, message, None);
    }

    pub(crate) fn push_error_with_data(
        &mut self,
        code: i32,
        message: impl Into<String>,
        data: Option<Value>,
    ) {
        self.outputs
            .push(CommandOutput::Command(CommandResponseOutput::Error {
                code,
                message: message.into(),
                data,
            }));
    }

    pub(crate) fn push_error_without_session(&mut self, code: i32, message: impl Into<String>) {
        self.outputs.push(CommandOutput::CommandWithoutSession(
            CommandResponseOutput::Error {
                code,
                message: message.into(),
                data: None,
            },
        ));
    }

    pub(crate) fn extend(&mut self, other: CommandOutputPlan) {
        let output_offset = self.outputs.len();
        let CommandOutputPlan {
            outputs,
            post_response_events,
            renderer_output_predecessor,
            renderer_output_boundary,
        } = other;
        if let Some(boundary) = renderer_output_boundary {
            assert!(
                self.renderer_output_boundary.is_none(),
                "one command output plan cannot contain multiple renderer insertion boundaries"
            );
            self.renderer_output_boundary = Some(Box::new(RendererOutputBoundary {
                output_index: output_offset
                    .checked_add(boundary.output_index)
                    .expect("command output boundary index exhausted"),
                cursor: boundary.cursor,
            }));
        }
        self.outputs.extend(outputs);
        self.post_response_events.extend(post_response_events);
        if let Some(predecessor) = renderer_output_predecessor {
            self.set_renderer_output_predecessor(predecessor);
        }
    }

    /// Converts a successfully completed nested command into the side effects
    /// that precede its enclosing command's response.
    ///
    /// Composite commands such as `Page.createIsolatedWorld` may first perform
    /// an internal initial navigation. That navigation has no independently
    /// visible command response, but its protocol events, exact renderer
    /// insertion boundary, and causal predecessor must retain their order.
    /// Flattening through `into_background_events()` would either manufacture
    /// an `id: null` response or lose the independently transported renderer
    /// boundary.
    pub(crate) fn into_composite_command_prefix(self) -> Self {
        let CommandOutputPlan {
            outputs,
            post_response_events,
            renderer_output_predecessor,
            renderer_output_boundary,
        } = self;
        let boundary_index = renderer_output_boundary
            .as_ref()
            .map(|boundary| boundary.output_index);
        let mut retained_before_boundary = 0usize;
        let mut retained_outputs = Vec::with_capacity(outputs.len() + post_response_events.len());
        for (index, output) in outputs.into_iter().enumerate() {
            let retain = matches!(
                output,
                CommandOutput::OwnerEvent(_) | CommandOutput::BackgroundEvent(_)
            );
            if retain {
                if boundary_index.is_some_and(|boundary| index < boundary) {
                    retained_before_boundary = retained_before_boundary
                        .checked_add(1)
                        .expect("composite command output index exhausted");
                }
                retained_outputs.push(output);
            }
        }
        retained_outputs.extend(
            post_response_events
                .into_iter()
                .map(CommandOutput::BackgroundEvent),
        );
        Self {
            outputs: retained_outputs,
            post_response_events: Vec::new(),
            renderer_output_predecessor,
            renderer_output_boundary: renderer_output_boundary.map(|boundary| {
                Box::new(RendererOutputBoundary {
                    output_index: retained_before_boundary,
                    cursor: boundary.cursor,
                })
            }),
        }
    }

    pub(crate) fn set_renderer_output_predecessor(
        &mut self,
        predecessor: moli_core::RendererOutputFence,
    ) {
        predecessor.merge_into_same_stream_tail(&mut self.renderer_output_predecessor);
    }

    pub(crate) fn take_renderer_output_predecessor(
        &mut self,
    ) -> Option<moli_core::RendererOutputFence> {
        self.renderer_output_predecessor.take()
    }

    /// Inserts one exact renderer publication at the current protocol-output
    /// position.
    ///
    /// This is distinct from [`Self::set_renderer_output_predecessor`].
    /// A command predecessor is causal output that Chromium flushes before the
    /// command response. An insertion boundary belongs to an independent
    /// renderer event, such as a main-Document commit, whose observable
    /// position is after the navigation response metadata but before DCL.
    pub(crate) fn insert_renderer_output_boundary(
        &mut self,
        cursor: moli_core::RendererOutputFence,
    ) {
        assert!(
            self.renderer_output_boundary.is_none(),
            "one command output plan cannot insert multiple renderer boundaries"
        );
        self.renderer_output_boundary = Some(Box::new(RendererOutputBoundary {
            output_index: self.outputs.len(),
            cursor,
        }));
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    pub(crate) fn push_inspector_target_crashed(&mut self, session_id: Option<&str>) {
        self.outputs.push(CommandOutput::OwnerEvent(
            CommandOwnerEvent::InspectorTargetCrashed {
                session_id: session_id.map(ToOwned::to_owned),
            },
        ));
    }

    pub(crate) fn push_page_lifecycle_event(
        &mut self,
        session_id: Option<&str>,
        name: &str,
        frame_id: &str,
        loader_id: &str,
        timestamp: f64,
    ) {
        self.outputs.push(CommandOutput::OwnerEvent(
            CommandOwnerEvent::PageLifecycle {
                session_id: session_id.map(ToOwned::to_owned),
                name: name.to_owned(),
                frame_id: frame_id.to_owned(),
                loader_id: loader_id.to_owned(),
                timestamp,
            },
        ));
    }

    pub(crate) fn push_background_event(&mut self, event: BackgroundProtocolEvent) {
        self.outputs.push(CommandOutput::BackgroundEvent(event));
    }

    pub(crate) fn extend_background_events(
        &mut self,
        events: impl IntoIterator<Item = BackgroundProtocolEvent>,
    ) {
        self.outputs
            .extend(events.into_iter().map(CommandOutput::BackgroundEvent));
    }

    pub(crate) fn extend_post_response_events(
        &mut self,
        events: impl IntoIterator<Item = BackgroundProtocolEvent>,
    ) {
        self.post_response_events.extend(events);
    }

    pub(crate) fn command_status(&self) -> Option<Result<(), DevToolsError>> {
        let mut status = None;
        for output in &self.outputs {
            match output {
                CommandOutput::Command(command) | CommandOutput::CommandWithoutSession(command) => {
                    record_command_status(&mut status, command.status());
                }
                CommandOutput::OwnerEvent(_) | CommandOutput::BackgroundEvent(_) => {}
            }
        }
        status
    }

    pub(crate) fn push_runtime_inspector_protocol_response(
        &mut self,
        message: Value,
        command_id: Option<u64>,
    ) -> bool {
        let Some(response) =
            CommandResponseOutput::from_runtime_inspector_protocol_message(message, command_id)
        else {
            return false;
        };
        self.outputs.push(CommandOutput::Command(response));
        true
    }

    #[cfg(test)]
    pub(crate) fn emit_into(
        self,
        out: &mut Vec<Value>,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) {
        out.extend(
            self.into_background_events(command_id, session_id)
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message),
        );
    }

    pub(crate) fn into_command_status_and_background_events(
        self,
    ) -> (
        Option<Result<(), DevToolsError>>,
        Vec<BackgroundProtocolEvent>,
    ) {
        let mut status = None;
        let mut out = Vec::new();
        for output in self.outputs {
            match output {
                CommandOutput::Command(command) => {
                    record_command_status(&mut status, command.status());
                }
                CommandOutput::CommandWithoutSession(command) => {
                    record_command_status(&mut status, command.status());
                }
                CommandOutput::OwnerEvent(event) => out.push(event.into_background_event()),
                CommandOutput::BackgroundEvent(event) => out.push(event),
            }
        }
        out.extend(self.post_response_events);
        (status, out)
    }

    pub(crate) fn into_background_events(
        self,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) -> Vec<BackgroundProtocolEvent> {
        let (mut out, renderer_output_boundary, mut after_boundary, post_response_events) =
            self.into_renderer_fenced_background_and_post_response_events(command_id, session_id);
        assert!(
            renderer_output_boundary.is_none(),
            "a renderer insertion boundary must be consumed by an ordered command owner"
        );
        out.append(&mut after_boundary);
        out.extend(post_response_events);
        out
    }

    pub(crate) fn into_renderer_fenced_background_and_post_response_events(
        self,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
        Vec<BackgroundProtocolEvent>,
        Vec<BackgroundProtocolEvent>,
    ) {
        let boundary_index = self
            .renderer_output_boundary
            .as_ref()
            .map(|boundary| boundary.output_index);
        let mut before_boundary = Vec::new();
        let mut after_boundary = Vec::new();
        for (index, output) in self.outputs.into_iter().enumerate() {
            let out = if boundary_index.is_some_and(|boundary| index >= boundary) {
                &mut after_boundary
            } else {
                &mut before_boundary
            };
            match output {
                CommandOutput::Command(command) => {
                    out.push(command.into_background_event(command_id, session_id));
                }
                CommandOutput::CommandWithoutSession(command) => {
                    out.push(command.into_background_event(command_id, None));
                }
                CommandOutput::OwnerEvent(event) => out.push(event.into_background_event()),
                CommandOutput::BackgroundEvent(event) => out.push(event),
            }
        }
        (
            before_boundary,
            self.renderer_output_boundary
                .map(|boundary| boundary.cursor),
            after_boundary,
            self.post_response_events,
        )
    }

    pub(crate) fn into_runtime_inspector_response_and_background_events(
        self,
        command_id: u64,
        session_id: Option<&str>,
    ) -> (Option<Value>, Vec<BackgroundProtocolEvent>) {
        let mut response = None;
        let mut out = Vec::new();
        for output in self.outputs {
            match output {
                CommandOutput::Command(command) => record_runtime_inspector_response(
                    &mut response,
                    command.into_protocol_message(Some(command_id), session_id),
                ),
                CommandOutput::CommandWithoutSession(command) => record_runtime_inspector_response(
                    &mut response,
                    command.into_protocol_message(Some(command_id), None),
                ),
                CommandOutput::OwnerEvent(event) => out.push(event.into_background_event()),
                CommandOutput::BackgroundEvent(event) => out.push(event),
            }
        }
        out.extend(self.post_response_events);
        (response, out)
    }
}

#[derive(Default)]
pub(crate) struct CommandOutputBuffer {
    plan: CommandOutputPlan,
}

#[derive(Default)]
pub(crate) struct BackgroundProtocolEventBuffer {
    events: Vec<BackgroundProtocolEvent>,
}

impl CommandOutputBuffer {
    pub(crate) fn extend_background_events_after_messages(
        &mut self,
        events: impl IntoIterator<Item = BackgroundProtocolEvent>,
    ) {
        for event in events {
            self.plan.push_background_event(event);
        }
    }

    pub(crate) fn push_result_after_messages(&mut self, value: Value) {
        self.plan.push_result(value);
    }

    pub(crate) fn push_error_after_messages(&mut self, code: i32, message: impl Into<String>) {
        self.plan.push_error(code, message);
    }

    #[cfg(test)]
    pub(crate) fn insert_renderer_output_boundary_after_messages(
        &mut self,
        cursor: moli_core::RendererOutputFence,
    ) {
        self.plan.insert_renderer_output_boundary(cursor);
    }

    pub(crate) fn into_plan(self) -> CommandOutputPlan {
        self.plan
    }
}

impl BackgroundProtocolEventBuffer {
    pub(crate) fn extend_background_events(
        &mut self,
        events: impl IntoIterator<Item = BackgroundProtocolEvent>,
    ) {
        self.events.extend(events);
    }

    pub(crate) fn into_events(self) -> Vec<BackgroundProtocolEvent> {
        self.events
    }
}

fn record_command_status(
    current: &mut Option<Result<(), DevToolsError>>,
    next: Result<(), DevToolsError>,
) {
    if current.is_none() {
        *current = Some(next);
    } else {
        tracing::warn!("command output plan produced multiple command responses");
    }
}

fn record_runtime_inspector_response(current: &mut Option<Value>, next: Value) {
    if current.is_none() {
        *current = Some(next);
    } else {
        tracing::warn!("command output plan produced multiple runtime inspector responses");
    }
}

pub(crate) fn protocol_message_background_event(message: Value) -> BackgroundProtocolEvent {
    protocol_message_background_event_for_target(message, None)
}

/// Freezes the Page target that owned an already-produced protocol message.
///
/// Renderer Inspector messages are converted into automation sidecars while
/// the exact Page attachment is still available. The sidecar can be consumed
/// later, after an implicit owner route has been restored, so it must not leave
/// its target empty and let BiDi infer it from the then-active tab.
pub(crate) fn protocol_message_background_event_for_target(
    message: Value,
    target_id: Option<&str>,
) -> BackgroundProtocolEvent {
    if let Some(mut event) = automation_event_from_protocol_message(&message) {
        if let Some(target_id) = target_id {
            qualify_automation_event_target(&mut event, DevToolsTargetId::from(target_id));
        }
        let session_id = message.get("sessionId").and_then(Value::as_str);
        let method = message.get("method").and_then(Value::as_str);
        return match (method, event) {
            (Some("Runtime.consoleAPICalled"), AutomationEvent::RuntimeConsoleApiCalled(event)) => {
                BackgroundProtocolEvent::runtime_console_api_called(session_id, event)
            }
            (Some("Runtime.exceptionThrown"), AutomationEvent::ScriptException(event)) => {
                BackgroundProtocolEvent::runtime_exception_thrown(session_id, event)
            }
            (_, event) => BackgroundProtocolEvent::immediate_automation_event(message, event),
        };
    }
    BackgroundProtocolEvent::immediate(message)
}

fn qualify_automation_event_target(event: &mut AutomationEvent, target_id: DevToolsTargetId) {
    match event {
        AutomationEvent::RuntimeConsoleApiCalled(event) => event.target_id = Some(target_id),
        AutomationEvent::ScriptException(event) => event.target_id = Some(target_id),
        AutomationEvent::LogEntryAdded(event) => event.target_id = Some(target_id),
        _ => {}
    }
}

fn automation_event_from_protocol_message(message: &Value) -> Option<AutomationEvent> {
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params")?;
    match method {
        "Page.javascriptDialogOpening" => page_javascript_dialog_opening_event_from_params(params),
        "Page.fileChooserOpened" => page_file_chooser_opened_event_from_params(params),
        "Browser.downloadWillBegin" => browser_download_will_begin_event_from_params(params),
        "Browser.downloadProgress" => browser_download_progress_event_from_params(params),
        "Page.frameScheduledNavigation" => {
            navigation_frame_event_from_params(params, NavigationFrameEventKind::Scheduled)
        }
        "Page.frameRequestedNavigation" => {
            navigation_frame_event_from_params(params, NavigationFrameEventKind::Requested)
        }
        "Page.frameStartedNavigating" => {
            navigation_frame_event_from_params(params, NavigationFrameEventKind::StartedNavigating)
        }
        "Page.frameStartedLoading" => {
            navigation_frame_event_from_params(params, NavigationFrameEventKind::StartedLoading)
        }
        "Page.frameClearedScheduledNavigation" => {
            navigation_frame_event_from_params(params, NavigationFrameEventKind::ClearedScheduled)
        }
        "Page.frameNavigated" => frame_navigated_event_from_params(params),
        "Page.frameStoppedLoading" => {
            navigation_frame_event_from_params(params, NavigationFrameEventKind::StoppedLoading)
        }
        "Page.navigatedWithinDocument" => same_document_navigation_event_from_params(params),
        "Page.lifecycleEvent" => page_lifecycle_event_from_params(params),
        "Console.messageAdded" => console_message_added_event_from_params(params),
        "Log.entryAdded" => log_entry_added_event_from_params(params),
        "Runtime.consoleAPICalled" => runtime_console_api_called_event_from_params(params),
        "Runtime.exceptionThrown" => runtime_exception_thrown_event_from_params(params),
        "Network.requestWillBeSent" => {
            network_automation_event_from_params(params, AutomationEvent::NetworkBeforeRequestSent)
        }
        "Network.responseReceived" => {
            network_automation_event_from_params(params, AutomationEvent::NetworkResponseStarted)
        }
        "Network.loadingFinished" => {
            network_automation_event_from_params(params, AutomationEvent::NetworkResponseCompleted)
        }
        "Network.loadingFailed" => {
            network_automation_event_from_params(params, AutomationEvent::NetworkFetchError)
        }
        "Fetch.requestPaused" => {
            network_automation_event_from_params(params, AutomationEvent::RequestPaused)
        }
        "Fetch.authRequired" => {
            network_automation_event_from_params(params, AutomationEvent::NetworkAuthRequired)
        }
        _ => None,
    }
}

fn page_javascript_dialog_opening_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let message = params.get("message").and_then(Value::as_str)?;
    let dialog_type = params.get("type").and_then(Value::as_str)?;
    let has_browser_handler = params
        .get("hasBrowserHandler")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let default_prompt = params
        .get("defaultPrompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let frame_id = params
        .get("frameId")
        .and_then(Value::as_str)
        .map(DevToolsFrameId::from);
    Some(AutomationEvent::PageJavaScriptDialogOpening(
        PageJavaScriptDialogOpeningEvent {
            frame_id,
            url,
            message: message.to_owned(),
            dialog_type: dialog_type.to_owned(),
            has_browser_handler,
            default_prompt,
        },
    ))
}

fn page_file_chooser_opened_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let frame_id = params.get("frameId").and_then(Value::as_str)?;
    let mode = params.get("mode").and_then(Value::as_str)?;
    let backend_node_id = params
        .get("backendNodeId")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())?;
    Some(AutomationEvent::PageFileChooserOpened(
        PageFileChooserOpenedEvent {
            frame_id: DevToolsFrameId::from(frame_id),
            mode: mode.to_owned(),
            backend_node_id,
            element_shared_id: None,
        },
    ))
}

fn browser_download_will_begin_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let frame_id = params.get("frameId").and_then(Value::as_str)?;
    let guid = params.get("guid").and_then(Value::as_str)?;
    let url = params.get("url").and_then(Value::as_str)?;
    let suggested_filename = params.get("suggestedFilename").and_then(Value::as_str)?;
    Some(AutomationEvent::BrowserDownloadWillBegin(
        BrowserDownloadWillBeginEvent {
            frame_id: DevToolsFrameId::from(frame_id),
            guid: guid.to_owned(),
            url: url.to_owned(),
            suggested_filename: suggested_filename.to_owned(),
        },
    ))
}

fn browser_download_progress_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let guid = params.get("guid").and_then(Value::as_str)?;
    let state = params.get("state").and_then(Value::as_str)?;
    let received_bytes = params.get("receivedBytes").and_then(Value::as_u64)?;
    let total_bytes = params.get("totalBytes").and_then(Value::as_u64)?;
    let file_path = params
        .get("filePath")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(AutomationEvent::BrowserDownloadProgress(
        BrowserDownloadProgressEvent {
            guid: guid.to_owned(),
            state: state.to_owned(),
            received_bytes,
            total_bytes,
            file_path,
        },
    ))
}

fn navigation_frame_event_from_params(
    params: &Value,
    kind: NavigationFrameEventKind,
) -> Option<AutomationEvent> {
    let frame_id = params.get("frameId").and_then(Value::as_str)?;
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let loader_id = optional_loader_id(params.get("loaderId").and_then(Value::as_str));
    Some(AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from(frame_id),
        frame_id: DevToolsFrameId::from(frame_id),
        parent_frame_id: None,
        loader_id,
        url,
        kind,
        frame_name: None,
        security_origin: None,
        secure_context_type: None,
    }))
}

fn frame_navigated_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let frame = params.get("frame")?;
    let frame_id = frame.get("id").and_then(Value::as_str)?;
    let url = frame
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let loader_id = optional_loader_id(frame.get("loaderId").and_then(Value::as_str));
    let parent_frame_id = frame
        .get("parentId")
        .and_then(Value::as_str)
        .map(DevToolsFrameId::from);
    let frame_name = frame.get("name").and_then(Value::as_str).map(str::to_owned);
    let security_origin = frame
        .get("securityOrigin")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let secure_context_type = frame
        .get("secureContextType")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from(frame_id),
        frame_id: DevToolsFrameId::from(frame_id),
        parent_frame_id,
        loader_id,
        url,
        kind: NavigationFrameEventKind::Navigated,
        frame_name,
        security_origin,
        secure_context_type,
    }))
}

fn page_lifecycle_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let frame_id = params.get("frameId").and_then(Value::as_str)?;
    let loader_id = params.get("loaderId").and_then(Value::as_str)?;
    let name = params.get("name").and_then(Value::as_str)?;
    let timestamp = params.get("timestamp").and_then(Value::as_f64)?;
    Some(AutomationEvent::PageLifecycle(PageLifecycleEvent {
        target_id: DevToolsTargetId::from(frame_id),
        frame_id: DevToolsFrameId::from(frame_id),
        loader_id: DevToolsLoaderId::from(loader_id),
        name: name.to_owned(),
        timestamp,
    }))
}

fn log_entry_added_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let entry = params.get("entry")?;
    let source = entry
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let level = entry
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = entry.get("text").and_then(Value::as_str)?;
    let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
    let timestamp = entry
        .get("timestamp")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    let network_request_id = entry.get("networkRequestId").and_then(Value::as_str);
    Some(AutomationEvent::LogEntryAdded(log_entry_event(
        source,
        level,
        text,
        url,
        timestamp,
        network_request_id,
    )))
}

fn console_message_added_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let message = params.get("message")?;
    let console_type = message
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or("log")
        .to_owned();
    let text = message.get("text").and_then(Value::as_str)?;
    Some(AutomationEvent::RuntimeConsoleApiCalled(
        RuntimeConsoleEvent {
            target_id: None,
            console_type,
            text: text.to_owned(),
            args: vec![json!({
                "type": "string",
                "value": text,
            })],
            stack: None,
            stack_trace: None,
            execution_context_id: None,
            timestamp: None,
        },
    ))
}

fn runtime_console_api_called_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let console_type = params
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("log")
        .to_owned();
    let args = params
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let text = runtime_console_text_from_cdp_args(&args);
    Some(AutomationEvent::RuntimeConsoleApiCalled(
        RuntimeConsoleEvent {
            target_id: None,
            console_type,
            text,
            args,
            stack: None,
            stack_trace: params
                .get("stackTrace")
                .and_then(DevToolsStackTrace::from_cdp_value),
            execution_context_id: params.get("executionContextId").and_then(Value::as_i64),
            timestamp: params.get("timestamp").and_then(Value::as_f64),
        },
    ))
}

fn runtime_console_text_from_cdp_args(args: &[Value]) -> String {
    args.iter()
        .filter_map(|arg| {
            arg.get("value")
                .and_then(runtime_console_arg_value_text)
                .or_else(|| {
                    arg.get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn runtime_console_arg_value_text(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_owned());
    }
    (!value.is_null()).then(|| value.to_string())
}

fn runtime_exception_thrown_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let details = params.get("exceptionDetails")?;
    let exception_id = details.get("exceptionId").and_then(Value::as_u64);
    let exception_index = exception_id
        .and_then(|id| id.checked_sub(1))
        .and_then(|id| usize::try_from(id).ok());
    let text = details
        .pointer("/exception/description")
        .and_then(Value::as_str)
        .or_else(|| details.get("text").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned();
    let exception = DevToolsScriptException {
        exception_id,
        script_id: details
            .get("scriptId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        text,
        value: details
            .get("exception")
            .map(|value| DevToolsRemoteValue::from_cdp_remote_object(value, true, None)),
        realm: details
            .get("executionContextUniqueId")
            .and_then(Value::as_str)
            .map(DevToolsRealmId::from)
            .or_else(|| {
                details
                    .get("executionContextId")
                    .and_then(Value::as_i64)
                    .map(|id| DevToolsRealmId::from(id.to_string()))
            }),
        line_number: details.get("lineNumber").and_then(Value::as_u64),
        column_number: details.get("columnNumber").and_then(Value::as_u64),
        stack_trace: details
            .get("stackTrace")
            .and_then(DevToolsStackTrace::from_cdp_value),
    };
    Some(AutomationEvent::ScriptException(ScriptExceptionEvent {
        target_id: None,
        url: details
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned),
        execution_context_id: details.get("executionContextId").and_then(Value::as_i64),
        exception_index,
        timestamp: params.get("timestamp").and_then(Value::as_f64),
        exception: Box::new(exception),
    }))
}

fn same_document_navigation_event_from_params(params: &Value) -> Option<AutomationEvent> {
    let frame_id = params.get("frameId").and_then(Value::as_str)?;
    let url = params.get("url").and_then(Value::as_str)?;
    let navigation_type = params.get("navigationType").and_then(Value::as_str)?;
    Some(AutomationEvent::SameDocumentNavigation(
        SameDocumentNavigationEvent {
            target_id: DevToolsTargetId::from(frame_id),
            frame_id: DevToolsFrameId::from(frame_id),
            url: url.to_owned(),
            navigation_type: navigation_type.to_owned(),
        },
    ))
}

fn optional_loader_id(loader_id: Option<&str>) -> Option<DevToolsLoaderId> {
    loader_id
        .filter(|loader_id| !loader_id.is_empty())
        .map(DevToolsLoaderId::from)
}

fn network_automation_event_from_params(
    params: &Value,
    event: fn(NetworkRequestEvent) -> AutomationEvent,
) -> Option<AutomationEvent> {
    let request_id = params.get("requestId").and_then(Value::as_str)?;
    Some(event(network_request_event_from_cdp_params(
        params, request_id,
    )))
}

fn network_request_event_from_cdp_params(params: &Value, request_id: &str) -> NetworkRequestEvent {
    let frame_id = params
        .get("frameId")
        .and_then(Value::as_str)
        .map(DevToolsFrameId::from);
    let url = params
        .pointer("/request/url")
        .or_else(|| params.pointer("/response/url"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let from_cache = params
        .pointer("/response/fromDiskCache")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || params
            .pointer("/response/fromPrefetchCache")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    NetworkRequestEvent {
        target_id: frame_id
            .as_ref()
            .map(|id| DevToolsTargetId::from(id.as_str()))
            .unwrap_or_else(|| DevToolsTargetId::from("")),
        frame_id,
        request_id: DevToolsRequestId::from(request_id),
        loader_id: params
            .get("loaderId")
            .and_then(Value::as_str)
            .map(DevToolsLoaderId::from),
        url,
        document_url: params
            .get("documentURL")
            .and_then(Value::as_str)
            .map(str::to_owned),
        method: params
            .pointer("/request/method")
            .and_then(Value::as_str)
            .map(str::to_owned),
        request_headers: cdp_header_pairs_from_object(params.pointer("/request/headers")),
        request_body: params
            .pointer("/request/postData")
            .and_then(Value::as_str)
            .map(str::to_owned),
        request_initiator_type: params
            .pointer("/initiator/type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        bidi_request_initiator_type: params
            .get("__moliRequestInitiatorType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        redirect_response: redirect_response_event_from_cdp_params(params),
        redirect_has_extra_info: params
            .get("redirectHasExtraInfo")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        request_cookie_report: cookie_query_report_from_cdp_params(params),
        resource_type: params
            .get("type")
            .or_else(|| params.get("resourceType"))
            .and_then(Value::as_str)
            .and_then(DevToolsNetworkResourceType::from_cdp_type),
        timestamp: params.get("timestamp").and_then(Value::as_f64),
        wall_time: params.get("wallTime").and_then(Value::as_f64),
        status: params
            .pointer("/response/status")
            .or_else(|| params.get("responseStatusCode"))
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok()),
        status_text: params
            .pointer("/response/statusText")
            .or_else(|| params.get("responseStatusText"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        response_headers: cdp_header_pairs_from_object(params.pointer("/response/headers"))
            .into_iter()
            .chain(fetch_header_pairs_from_array(params.get("responseHeaders")))
            .collect(),
        response_mime_type: params
            .pointer("/response/mimeType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        response_protocol: params
            .pointer("/response/protocol")
            .and_then(Value::as_str)
            .map(str::to_owned),
        encoded_data_length: params
            .get("encodedDataLength")
            .or_else(|| params.pointer("/response/encodedDataLength"))
            .and_then(Value::as_u64)
            .and_then(|length| usize::try_from(length).ok()),
        from_cache,
        has_extra_info: params
            .get("hasExtraInfo")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        error_text: params
            .get("errorText")
            .and_then(Value::as_str)
            .map(str::to_owned),
        loading_failed_canceled: params
            .get("canceled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        blocked_intercepts: blocked_intercepts_from_cdp_params(params),
        fetch_request_id: params
            .get("__moliFetchRequestId")
            .and_then(Value::as_str)
            .map(DevToolsFetchRequestId::from),
        network_id: params
            .get("networkId")
            .and_then(Value::as_str)
            .map(DevToolsRequestId::from),
        auth_challenge: network_auth_challenge_from_cdp_params(params),
    }
}

fn redirect_response_event_from_cdp_params(params: &Value) -> Option<NetworkRedirectResponseEvent> {
    let response = params.get("redirectResponse")?;
    Some(NetworkRedirectResponseEvent {
        url: response.get("url")?.as_str()?.to_owned(),
        status: response
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .unwrap_or_default(),
        status_text: response
            .get("statusText")
            .and_then(Value::as_str)
            .map(str::to_owned),
        response_headers: cdp_header_pairs_from_object(response.get("headers")),
        encoded_data_length: response
            .get("encodedDataLength")
            .and_then(Value::as_u64)
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default(),
        from_cache: response
            .get("fromDiskCache")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || response
                .get("fromPrefetchCache")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        response_protocol: response
            .get("protocol")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn cdp_header_pairs_from_object(headers: Option<&Value>) -> Vec<(String, String)> {
    headers
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .map(|(name, value)| (name.clone(), cdp_header_value_to_string(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn cdp_header_value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn fetch_header_pairs_from_array(headers: Option<&Value>) -> Vec<(String, String)> {
    headers
        .and_then(Value::as_array)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|header| {
                    Some((
                        header.get("name")?.as_str()?.to_owned(),
                        header
                            .get("value")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn network_auth_challenge_from_cdp_params(params: &Value) -> Option<NetworkAuthChallengeEvent> {
    let challenge = params.get("authChallenge")?;
    Some(NetworkAuthChallengeEvent {
        origin: challenge
            .get("origin")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        source: challenge
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("Server")
            .to_owned(),
        scheme: challenge
            .get("scheme")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        realm: challenge
            .get("realm")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

fn cookie_query_report_from_cdp_params(params: &Value) -> Option<StoredCookieQueryReport> {
    let report = params.get("cookieAccessReport")?;
    Some(StoredCookieQueryReport {
        facade_status: Default::default(),
        facade_exclusion_reasons: cookie_exclusion_reasons_from_json(
            report.get("facadeExclusionReasons"),
        ),
        included_cookies: cookie_access_vec_from_json(report.get("includedCookies")),
        excluded_cookies: cookie_access_vec_from_json(report.get("excludedCookies")),
    })
}

fn cookie_access_vec_from_json(value: Option<&Value>) -> Vec<StoredCookieAccess> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(cookie_access_from_json)
        .collect()
}

fn cookie_access_from_json(value: &Value) -> Option<StoredCookieAccess> {
    Some(StoredCookieAccess {
        cookie: stored_cookie_from_cdp_json(value.get("cookie")?)?,
        exclusion_reasons: cookie_exclusion_reasons_from_json(value.get("exclusionReasons")),
        warning_reasons: cookie_warning_reasons_from_json(value.get("warningReasons")),
        effective_same_site: cookie_effective_same_site_from_json(
            value.get("effectiveSameSite").and_then(Value::as_str),
        ),
        same_site_context: cookie_request_same_site_context_from_json(
            value.get("sameSiteContext").and_then(Value::as_str),
        ),
        schemeful_same_site_context: cookie_request_same_site_context_from_json(
            value
                .get("schemefulSameSiteContext")
                .and_then(Value::as_str),
        ),
        same_site_context_downgrade_type: value
            .get("sameSiteContextDowngradeType")
            .and_then(Value::as_str)
            .and_then(cookie_same_site_context_downgrade_type_from_json),
        schemeful_same_site_context_downgrade_type: value
            .get("schemefulSameSiteContextDowngradeType")
            .and_then(Value::as_str)
            .and_then(cookie_same_site_context_downgrade_type_from_json),
        same_site_context_http_method: cookie_same_site_http_method_from_json(
            value
                .get("sameSiteContextHttpMethod")
                .and_then(Value::as_str),
        ),
        schemeful_same_site_context_http_method: cookie_same_site_http_method_from_json(
            value
                .get("schemefulSameSiteContextHttpMethod")
                .and_then(Value::as_str),
        ),
        same_site_context_redirect_type: cookie_same_site_redirect_type_from_json(
            value
                .get("sameSiteContextRedirectType")
                .and_then(Value::as_str),
        ),
        schemeful_same_site_context_redirect_type: cookie_same_site_redirect_type_from_json(
            value
                .get("schemefulSameSiteContextRedirectType")
                .and_then(Value::as_str),
        ),
        access_semantics: cookie_access_semantics_from_json(
            value.get("accessSemantics").and_then(Value::as_str),
        ),
        scope_semantics: cookie_scope_semantics_from_json(
            value.get("scopeSemantics").and_then(Value::as_str),
        ),
        is_allowed_to_access_secure_cookies: value
            .get("isAllowedToAccessSecureCookies")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        site_for_cookies_url: value
            .get("siteForCookiesUrl")
            .and_then(Value::as_str)
            .and_then(|url| url.parse().ok()),
        site_for_cookies_source: cookie_browser_context_value_source_from_json(
            value.get("siteForCookiesSource").and_then(Value::as_str),
        ),
        top_frame_origin_url: value
            .get("topFrameOriginUrl")
            .and_then(Value::as_str)
            .and_then(|url| url.parse().ok()),
        top_frame_origin_source: cookie_browser_context_value_source_from_json(
            value.get("topFrameOriginSource").and_then(Value::as_str),
        ),
        storage_access_status: cookie_storage_access_status_from_json(
            value.get("storageAccessStatus").and_then(Value::as_str),
        ),
        storage_access_status_source: cookie_browser_context_value_source_from_json(
            value
                .get("storageAccessStatusSource")
                .and_then(Value::as_str),
        ),
        site_context_basis: cookie_site_context_basis_from_json(
            value.get("siteContextBasis").and_then(Value::as_str),
        ),
    })
}

fn stored_cookie_from_cdp_json(cookie: &Value) -> Option<StoredCookie> {
    let raw_domain = cookie
        .get("domain")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(StoredCookie {
        name: cookie.get("name")?.as_str()?.to_owned(),
        value: cookie.get("value")?.as_str()?.to_owned(),
        domain: raw_domain.trim_start_matches('.').to_owned(),
        host_only: !raw_domain.starts_with('.'),
        path: cookie
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("/")
            .to_owned(),
        secure: cookie
            .get("secure")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        http_only: cookie
            .get("httpOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        expires: cookie
            .get("expires")
            .and_then(Value::as_f64)
            .and_then(offset_datetime_from_cdp_timestamp),
        same_site: stored_cookie_same_site_from_cdp(cookie.get("sameSite").and_then(Value::as_str)),
        priority: cookie
            .get("priority")
            .and_then(Value::as_str)
            .and_then(CookiePriority::parse),
        partition_key: cookie.get("partitionKey").and_then(|key| {
            Some(StoredCookiePartitionKey::site(
                key.get("topLevelSite")?.as_str()?.to_owned(),
                key.get("hasCrossSiteAncestor")?.as_bool()?,
            ))
        }),
        source_scheme: stored_cookie_source_scheme_from_cdp(
            cookie.get("sourceScheme").and_then(Value::as_str),
        ),
        source_port: cookie
            .get("sourcePort")
            .and_then(Value::as_i64)
            .and_then(|port| i32::try_from(port).ok())
            .unwrap_or(-1),
        creation_index: 0,
        last_access_index: 0,
    })
}

fn stored_cookie_same_site_from_cdp(value: Option<&str>) -> StoredCookieSameSite {
    match value {
        Some(value) if value.eq_ignore_ascii_case("none") => StoredCookieSameSite::None,
        Some(value) if value.eq_ignore_ascii_case("lax") => StoredCookieSameSite::Lax,
        Some(value) if value.eq_ignore_ascii_case("strict") => StoredCookieSameSite::Strict,
        _ => StoredCookieSameSite::Unspecified,
    }
}

fn stored_cookie_source_scheme_from_cdp(value: Option<&str>) -> StoredCookieSourceScheme {
    match value {
        Some(value) if value.eq_ignore_ascii_case("secure") => StoredCookieSourceScheme::Secure,
        Some(value) if value.eq_ignore_ascii_case("nonsecure") => {
            StoredCookieSourceScheme::NonSecure
        }
        _ => StoredCookieSourceScheme::Unset,
    }
}

fn offset_datetime_from_cdp_timestamp(value: f64) -> Option<time::OffsetDateTime> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let seconds = value.trunc() as i64;
    let nanos = (value.fract() * 1_000_000_000.0).round() as i64;
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|datetime| datetime.checked_add(time::Duration::nanoseconds(nanos)))
}

fn cookie_exclusion_reasons_from_json(value: Option<&Value>) -> Vec<StoredCookieExclusionReason> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(cookie_exclusion_reason_from_json)
        .collect()
}

fn cookie_exclusion_reason_from_json(value: &str) -> Option<StoredCookieExclusionReason> {
    Some(match value {
        "CookiesDisabled" => StoredCookieExclusionReason::CookiesDisabled,
        "StorageAccessBlocked" => StoredCookieExclusionReason::StorageAccessBlocked,
        "StoreUnavailable" => StoredCookieExclusionReason::StoreUnavailable,
        "Expired" => StoredCookieExclusionReason::Expired,
        "DomainMismatch" => StoredCookieExclusionReason::DomainMismatch,
        "PathMismatch" => StoredCookieExclusionReason::PathMismatch,
        "SecureOnly" => StoredCookieExclusionReason::SecureOnly,
        "HttpOnly" => StoredCookieExclusionReason::HttpOnly,
        "PortMismatch" => StoredCookieExclusionReason::PortMismatch,
        "SchemeMismatch" => StoredCookieExclusionReason::SchemeMismatch,
        "SameSiteStrict" => StoredCookieExclusionReason::SameSiteStrict,
        "SameSiteLax" => StoredCookieExclusionReason::SameSiteLax,
        _ => return None,
    })
}

fn cookie_warning_reasons_from_json(value: Option<&Value>) -> Vec<StoredCookieWarningReason> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(cookie_warning_reason_from_json)
        .collect()
}

fn cookie_warning_reason_from_json(value: &str) -> Option<StoredCookieWarningReason> {
    Some(match value {
        "SchemefulSameSiteContextMismatch" => {
            StoredCookieWarningReason::SchemefulSameSiteContextMismatch
        }
        "StrictLaxDowngradeStrictSameSite" => {
            StoredCookieWarningReason::StrictLaxDowngradeStrictSameSite
        }
        "StrictCrossDowngradeStrictSameSite" => {
            StoredCookieWarningReason::StrictCrossDowngradeStrictSameSite
        }
        "StrictCrossDowngradeLaxSameSite" => {
            StoredCookieWarningReason::StrictCrossDowngradeLaxSameSite
        }
        "LaxCrossDowngradeStrictSameSite" => {
            StoredCookieWarningReason::LaxCrossDowngradeStrictSameSite
        }
        "LaxCrossDowngradeLaxSameSite" => StoredCookieWarningReason::LaxCrossDowngradeLaxSameSite,
        "SameSiteContextDowngradedByRedirect" => {
            StoredCookieWarningReason::SameSiteContextDowngradedByRedirect
        }
        "SecureAccessGrantedNonCryptographic" => {
            StoredCookieWarningReason::SecureAccessGrantedNonCryptographic
        }
        _ => return None,
    })
}

fn cookie_effective_same_site_from_json(value: Option<&str>) -> StoredCookieEffectiveSameSite {
    match value {
        Some("Lax") => StoredCookieEffectiveSameSite::Lax,
        Some("Strict") => StoredCookieEffectiveSameSite::Strict,
        _ => StoredCookieEffectiveSameSite::NoRestriction,
    }
}

fn cookie_request_same_site_context_from_json(
    value: Option<&str>,
) -> StoredCookieRequestSameSiteContext {
    match value {
        Some("SameSiteStrict") => StoredCookieRequestSameSiteContext::SameSiteStrict,
        Some("SameSiteLaxMethodUnsafe") => {
            StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
        }
        Some("CrossSite") => StoredCookieRequestSameSiteContext::CrossSite,
        _ => StoredCookieRequestSameSiteContext::SameSiteLax,
    }
}

fn cookie_same_site_context_downgrade_type_from_json(
    value: &str,
) -> Option<StoredCookieSameSiteContextDowngradeType> {
    Some(match value {
        "StrictToLax" => StoredCookieSameSiteContextDowngradeType::StrictToLax,
        "StrictToCross" => StoredCookieSameSiteContextDowngradeType::StrictToCross,
        "LaxToCross" => StoredCookieSameSiteContextDowngradeType::LaxToCross,
        _ => return None,
    })
}

fn cookie_same_site_http_method_from_json(value: Option<&str>) -> StoredCookieSameSiteHttpMethod {
    match value {
        Some("GET") => StoredCookieSameSiteHttpMethod::Get,
        Some("HEAD") => StoredCookieSameSiteHttpMethod::Head,
        Some("POST") => StoredCookieSameSiteHttpMethod::Post,
        Some("PUT") => StoredCookieSameSiteHttpMethod::Put,
        Some("DELETE") => StoredCookieSameSiteHttpMethod::Delete,
        Some("CONNECT") => StoredCookieSameSiteHttpMethod::Connect,
        Some("OPTIONS") => StoredCookieSameSiteHttpMethod::Options,
        Some("TRACE") => StoredCookieSameSiteHttpMethod::Trace,
        Some("PATCH") => StoredCookieSameSiteHttpMethod::Patch,
        Some("Unknown") => StoredCookieSameSiteHttpMethod::Unknown,
        _ => StoredCookieSameSiteHttpMethod::Unset,
    }
}

fn cookie_same_site_redirect_type_from_json(
    value: Option<&str>,
) -> StoredCookieSameSiteRedirectType {
    match value {
        Some("NoRedirect") => StoredCookieSameSiteRedirectType::NoRedirect,
        Some("CrossSiteRedirect") => StoredCookieSameSiteRedirectType::CrossSiteRedirect,
        Some("PartialSameSiteRedirect") => {
            StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
        }
        Some("AllSameSiteRedirect") => StoredCookieSameSiteRedirectType::AllSameSiteRedirect,
        _ => StoredCookieSameSiteRedirectType::Unset,
    }
}

fn cookie_access_semantics_from_json(value: Option<&str>) -> StoredCookieAccessSemantics {
    match value {
        Some("NonLegacy") => StoredCookieAccessSemantics::NonLegacy,
        Some("Legacy") => StoredCookieAccessSemantics::Legacy,
        _ => StoredCookieAccessSemantics::Unknown,
    }
}

fn cookie_scope_semantics_from_json(value: Option<&str>) -> StoredCookieScopeSemantics {
    match value {
        Some("NonLegacy") => StoredCookieScopeSemantics::NonLegacy,
        Some("Legacy") => StoredCookieScopeSemantics::Legacy,
        _ => StoredCookieScopeSemantics::Unknown,
    }
}

fn cookie_browser_context_value_source_from_json(
    value: Option<&str>,
) -> StoredCookieBrowserContextValueSource {
    match value {
        Some("RequestContext") => StoredCookieBrowserContextValueSource::RequestContext,
        Some("FacadeDefault") => StoredCookieBrowserContextValueSource::FacadeDefault,
        Some("FacadeOverride") => StoredCookieBrowserContextValueSource::FacadeOverride,
        _ => StoredCookieBrowserContextValueSource::Unset,
    }
}

fn cookie_storage_access_status_from_json(value: Option<&str>) -> StoredCookieStorageAccessStatus {
    match value {
        Some("Granted") => StoredCookieStorageAccessStatus::Granted,
        _ => StoredCookieStorageAccessStatus::None,
    }
}

fn cookie_site_context_basis_from_json(value: Option<&str>) -> StoredCookieSiteContextBasis {
    match value {
        Some("SiteForCookies") => StoredCookieSiteContextBasis::SiteForCookies,
        Some("TopFrameOrigin") => StoredCookieSiteContextBasis::TopFrameOrigin,
        _ => StoredCookieSiteContextBasis::None,
    }
}

fn blocked_intercepts_from_cdp_params(params: &Value) -> Vec<DevToolsNetworkInterceptId> {
    params
        .get("__moliBlockedInterceptors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(DevToolsNetworkInterceptId::from)
        .collect()
}

fn cdp_result_payload_from_devtools_result(result: DevToolsCommandResult) -> Value {
    match result {
        DevToolsCommandResult::Empty | DevToolsCommandResult::TraverseHistory(_) => json!({}),
        DevToolsCommandResult::Navigate(result) => {
            let mut payload = json!({});
            if let Some(frame_id) = result.frame_id {
                payload["frameId"] = json!(frame_id.into_string());
            }
            if let Some(loader_id) = result.loader_id {
                payload["loaderId"] = json!(loader_id.into_string());
            }
            if let Some(error_text) = result.error_text {
                payload["errorText"] = json!(error_text);
            }
            if let Some(is_download) = result.is_download {
                payload["isDownload"] = json!(is_download);
            }
            payload
        }
        DevToolsCommandResult::GetNavigationHistory(result) => json!({
            "currentIndex": result.current_index,
            "entries": result
                .entries
                .into_iter()
                .map(|entry| {
                    json!({
                        "id": entry.id,
                        "url": entry.url,
                        "userTypedURL": entry.user_typed_url,
                        "title": entry.title,
                        "transitionType": entry.transition_type,
                    })
                })
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::GetFrameTree(result) => json!({
            "frameTree": result.frame_tree,
        }),
        DevToolsCommandResult::GetFrameTrees(result) => json!({
            "frameTrees": result
                .frame_trees
                .into_iter()
                .map(|result| result.frame_tree)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::CreateTarget(result) => json!({
            "targetId": result.target_id.into_string(),
        }),
        DevToolsCommandResult::CloseTarget(result) => json!({
            "success": result.success,
        }),
        DevToolsCommandResult::GetTargets(result) => json!({
            "targetInfos": result
                .targets
                .into_iter()
                .map(cdp_target_info)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::ServiceWorkerLogs(result) => json!({
            "entries": result
                .entries
                .into_iter()
                .map(service_worker_log_entry_json)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::ClientWindows(result) => json!({
            "clientWindows": result
                .client_windows
                .into_iter()
                .map(client_window_info_json)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::ClientWindow(result) => {
            client_window_info_json(result.client_window)
        }
        DevToolsCommandResult::CreateBrowserContext(result) => json!({
            "browserContextId": result.browser_context_id.into_string(),
        }),
        DevToolsCommandResult::GetBrowserContexts(result) => json!({
            "browserContextIds": result
                .browser_context_ids
                .into_iter()
                .map(|id| id.into_string())
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::GetTargetInfo(result) => json!({
            "targetInfo": cdp_target_info(result.target_info),
        }),
        DevToolsCommandResult::GetCookies(result) => json!({
            "cookies": result.cookies,
        }),
        DevToolsCommandResult::DeleteCookies(_) => json!({}),
        DevToolsCommandResult::SetCookies(result) => json!({
            "success": result.success,
            "cookieReports": result.cookie_reports,
        }),
        DevToolsCommandResult::AddPreloadScript(result) => json!({
            "identifier": result.script_id.into_string(),
        }),
        DevToolsCommandResult::AddNetworkIntercept(result) => json!({
            "intercept": result.intercept_id.into_string(),
        }),
        DevToolsCommandResult::AddNetworkDataCollector(result) => json!({
            "collector": result.collector_id.into_string(),
        }),
        DevToolsCommandResult::NetworkData(result) => json!({
            "bytes": {
                "type": match result.bytes_type {
                    DevToolsNetworkDataBytesType::String => "string",
                    DevToolsNetworkDataBytesType::Base64 => "base64",
                },
                "value": result.value,
            },
        }),
        DevToolsCommandResult::Realms(result) => json!({
            "realms": result
                .realms
                .into_iter()
                .map(cdp_realm_info)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::Script(result) => match *result {
            DevToolsScriptResult::Value(value) => json!({
                "result": crate::cdp_projection::remote_object_from_devtools(value),
            }),
            DevToolsScriptResult::Exception(exception) => {
                let mut exception_details = json!({
                    "text": exception.text,
                    "lineNumber": exception.line_number.unwrap_or(0),
                    "columnNumber": exception.column_number.unwrap_or(0),
                    "stackTrace": exception
                        .stack_trace
                        .map(|stack_trace| stack_trace.into_cdp_value())
                        .unwrap_or_else(|| json!({"callFrames": []})),
                    "exception": exception
                        .value
                        .map(crate::cdp_projection::remote_object_from_devtools)
                        .unwrap_or_else(|| json!({"type": "undefined"})),
                });
                if let Some(exception_id) = exception.exception_id
                    && let Some(details) = exception_details.as_object_mut()
                {
                    details.insert("exceptionId".to_owned(), json!(exception_id));
                }
                json!({
                    "result": {
                        "type": "undefined",
                    },
                    "exceptionDetails": exception_details,
                })
            }
        },
        DevToolsCommandResult::LocateNodes(result) => json!({
            "nodes": result
                .nodes
                .into_iter()
                .map(crate::cdp_projection::remote_object_from_devtools)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::DescribeNode(result) => json!({
            "node": result.node,
        }),
        DevToolsCommandResult::GetFrameOwner(result) => json!({
            "nodeId": result.node_id,
            "backendNodeId": result.backend_node_id,
        }),
        DevToolsCommandResult::QuerySelector(result) => {
            if result.multiple {
                json!({ "nodeIds": result.node_ids })
            } else {
                json!({ "nodeId": result.node_ids.first().copied().unwrap_or(0) })
            }
        }
        DevToolsCommandResult::ResolveNode(result) => json!({
            "object": result.object,
        }),
        DevToolsCommandResult::GetAttributes(result) => json!({
            "attributes": result
                .attributes
                .into_iter()
                .flat_map(|attribute| [attribute.name, attribute.value])
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::GetText(result) => json!({
            "text": result.text,
        }),
        DevToolsCommandResult::GetProperty(result) => json!({
            "value": result.value,
        }),
        DevToolsCommandResult::PushNodesByBackendIds(result) => json!({
            "nodeIds": result.node_ids,
        }),
        DevToolsCommandResult::GetOuterHtml(result) => {
            Value::Object(serde_json::Map::from_iter([(
                "outerHTML".to_owned(),
                Value::String(result.outer_html),
            )]))
        }
        DevToolsCommandResult::GetNodeForLocation(result) => {
            let mut value = json!({
                "backendNodeId": result.backend_node_id,
                "frameId": result.frame_id.as_str(),
            });
            if let Some(node_id) = result.node_id {
                value["nodeId"] = json!(node_id);
            }
            value
        }
        DevToolsCommandResult::DomGeometry(result) => {
            if let Some(model) = result.box_model {
                return json!({
                    "model": {
                        "content": model.content.points,
                        "padding": model.padding.points,
                        "border": model.border.points,
                        "margin": model.margin.points,
                        "width": model.width,
                        "height": model.height,
                    }
                });
            }
            let quads = result
                .quads
                .into_iter()
                .map(|quad| json!(quad.points))
                .collect::<Vec<_>>();
            if let (Some(width), Some(height), Some(content)) =
                (result.width, result.height, quads.first().cloned())
            {
                json!({
                    "model": {
                        "content": content,
                        "padding": content,
                        "border": content,
                        "margin": content,
                        "width": width,
                        "height": height,
                    }
                })
            } else {
                json!({ "quads": quads })
            }
        }
        DevToolsCommandResult::LayoutMetrics(result) => {
            let viewport = json!({
                "pageX": result.page_x,
                "pageY": result.page_y,
                "clientWidth": result.layout_viewport_width,
                "clientHeight": result.layout_viewport_height,
            });
            let visual = json!({
                "offsetX": 0,
                "offsetY": 0,
                "pageX": result.page_x,
                "pageY": result.page_y,
                "clientWidth": result.layout_viewport_width,
                "clientHeight": result.layout_viewport_height,
                "scale": result.device_pixel_ratio,
                "zoom": 1,
            });
            let content = json!({
                "x": 0,
                "y": 0,
                "width": result.content_width,
                "height": result.content_height,
            });
            json!({
                "layoutViewport": viewport,
                "visualViewport": visual,
                "contentSize": content,
                "cssLayoutViewport": viewport,
                "cssVisualViewport": visual,
                "cssContentSize": content,
            })
        }
        DevToolsCommandResult::JavaScriptDialog(result) => json!({
            "type": result.dialog_type,
            "message": result.message,
            "defaultPrompt": result.default_prompt,
        }),
        DevToolsCommandResult::CaptureScreenshot(result) => json!({
            "data": BASE64_STANDARD.encode(result.bytes.as_ref()),
        }),
    }
}

fn cdp_realm_info(event: RuntimeExecutionContextEvent) -> Value {
    let mut payload = json!({});
    if let Some(target_id) = event.target_id {
        payload["targetId"] = json!(target_id.into_string());
    }
    if let Some(context_id) = event.context_id {
        payload["executionContextId"] = json!(context_id);
    }
    if let Some(realm_id) = event.realm_id {
        payload["executionContextUniqueId"] = json!(realm_id.into_string());
    }
    if let Some(frame_id) = event.frame_id {
        payload["frameId"] = json!(frame_id.into_string());
    }
    if let Some(origin) = event.origin {
        payload["origin"] = json!(origin);
    }
    if let Some(name) = event.name {
        payload["name"] = json!(name);
    }
    if let Some(is_default) = event.is_default {
        payload["isDefault"] = json!(is_default);
    }
    if let Some(context_type) = event.context_type {
        payload["type"] = json!(context_type);
    }
    payload
}

fn cdp_target_info(info: DevToolsTargetInfo) -> Value {
    info.into_cdp_value()
}

fn service_worker_log_entry_json(entry: RuntimeConsoleEvent) -> Value {
    let mut payload = json!({
        "type": entry.console_type,
        "text": entry.text,
        "args": entry.args,
    });
    if let Some(target_id) = entry.target_id {
        payload["targetId"] = json!(target_id.into_string());
    }
    if let Some(stack) = entry.stack {
        payload["stack"] = json!(stack);
    }
    if let Some(stack_trace) = entry.stack_trace {
        payload["stackTrace"] = stack_trace.into_cdp_value();
    }
    if let Some(execution_context_id) = entry
        .execution_context_id
        .filter(|execution_context_id| *execution_context_id > 0)
    {
        payload["executionContextId"] = json!(execution_context_id);
    }
    if let Some(timestamp) = entry.timestamp {
        payload["timestamp"] = json!(timestamp);
    }
    payload
}

fn client_window_info_json(window: crate::devtools_runtime::DevToolsClientWindowInfo) -> Value {
    json!({
        "clientWindow": window.client_window.into_string(),
        "active": window.active,
        "state": window.state.as_bidi_value(),
        "width": window.width,
        "height": window.height,
        "x": window.x,
        "y": window.y,
    })
}

fn cdp_error_code_from_devtools_error(error: &DevToolsError) -> i32 {
    match error.kind {
        DevToolsErrorKind::InvalidArgument | DevToolsErrorKind::InvalidSelector => -32602,
        DevToolsErrorKind::NoSuchAlert => -32602,
        DevToolsErrorKind::NoSuchHandle => -32000,
        DevToolsErrorKind::NoSuchHistoryEntry => -32000,
        DevToolsErrorKind::NoSuchNode => -32000,
        DevToolsErrorKind::NoSuchNetworkCollector | DevToolsErrorKind::NoSuchNetworkData => -32000,
        DevToolsErrorKind::NoSuchRequest => -32000,
        DevToolsErrorKind::NoSuchScript => -32000,
        DevToolsErrorKind::NoSuchSession => -32001,
        DevToolsErrorKind::NoSuchTarget => -31998,
        DevToolsErrorKind::NavigationChangingDocument
        | DevToolsErrorKind::Timeout
        | DevToolsErrorKind::UnableToCaptureScreen
        | DevToolsErrorKind::UnableToSetFileInput
        | DevToolsErrorKind::Unsupported
        | DevToolsErrorKind::Internal => -32000,
    }
}

pub(crate) fn devtools_error_from_cdp_error_value(error: &Value) -> DevToolsError {
    devtools_error_from_cdp_error_parts(
        error.get("code").and_then(Value::as_i64),
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("DevToolsCommandFailed"),
    )
}

pub(crate) fn devtools_error_from_cdp_error_parts(
    code: Option<i64>,
    message: &str,
) -> DevToolsError {
    let kind = match code {
        Some(-32602) if message == "No dialog is showing" => DevToolsErrorKind::NoSuchAlert,
        Some(-32602) => DevToolsErrorKind::InvalidArgument,
        Some(-32001) => DevToolsErrorKind::NoSuchSession,
        Some(-31998) => DevToolsErrorKind::NoSuchTarget,
        Some(-32000) if message == "UnsupportedDevToolsCommand" => DevToolsErrorKind::Unsupported,
        Some(-32000)
            if message == "NoSuchHistoryEntry"
                || message == "Navigation history entry not found" =>
        {
            DevToolsErrorKind::NoSuchHistoryEntry
        }
        Some(-32000) if message == "NoSuchScript" => DevToolsErrorKind::NoSuchScript,
        Some(-32000) if message == "Navigation is changing the document" => {
            DevToolsErrorKind::NavigationChangingDocument
        }
        Some(-32000)
            if message == "Cannot find object with given id"
                || message == "Could not find object with given id" =>
        {
            DevToolsErrorKind::NoSuchHandle
        }
        Some(-32000)
            if message == "Could not find node with given id" || message == "NoSuchNode" =>
        {
            DevToolsErrorKind::NoSuchNode
        }
        Some(-32000) if message == "UnableToSetFileInput" => {
            DevToolsErrorKind::UnableToSetFileInput
        }
        Some(-32000) if message == "RequestNotFound" => DevToolsErrorKind::NoSuchRequest,
        _ => DevToolsErrorKind::Internal,
    };
    DevToolsError::new(kind, message)
}

impl CommandResponseOutput {
    fn from_runtime_inspector_protocol_message(
        mut message: Value,
        command_id: Option<u64>,
    ) -> Option<Self> {
        let command_id = command_id?;
        if message.get("id").and_then(Value::as_u64) != Some(command_id) {
            return None;
        }
        if let Some(result) = message.as_object_mut()?.remove("result") {
            return Some(Self::Success(CommandResponseResult::Json(result)));
        }
        let error = message.as_object_mut()?.remove("error")?;
        let Value::Object(mut error) = error else {
            return Some(Self::Error {
                code: -32000,
                message: "Runtime inspector command failed".to_owned(),
                data: None,
            });
        };
        let code = error
            .remove("code")
            .and_then(|code| code.as_i64())
            .and_then(|code| i32::try_from(code).ok())
            .unwrap_or(-32000);
        let message = error
            .remove("message")
            .and_then(|message| message.as_str().map(str::to_owned))
            .unwrap_or_else(|| "Runtime inspector command failed".to_owned());
        let data = error.remove("data");
        Some(Self::Error {
            code,
            message,
            data,
        })
    }

    fn status(&self) -> Result<(), DevToolsError> {
        match self {
            Self::Success(_) => Ok(()),
            Self::Error { code, message, .. } => Err(devtools_error_from_cdp_error_parts(
                Some(i64::from(*code)),
                message,
            )),
        }
    }

    fn into_protocol_message(self, command_id: Option<u64>, session_id: Option<&str>) -> Value {
        let mut message = match self {
            Self::Success(result) => {
                return build_command_success_response(
                    command_id,
                    result.into_value(),
                    session_id.map(str::to_owned),
                );
            }
            Self::Error {
                code,
                message,
                data,
            } => {
                let mut error = json!({ "code": code, "message": message });
                if let Some(data) = data {
                    error["data"] = data;
                }
                json!({ "id": command_id, "error": error })
            }
        };
        if let Some(session_id) = session_id {
            message["sessionId"] = json!(session_id);
        }
        message
    }

    fn into_background_event(
        self,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) -> BackgroundProtocolEvent {
        match self {
            Self::Success(result) => BackgroundProtocolEvent::command_success(
                command_id,
                session_id,
                result.into_value(),
            ),
            Self::Error {
                code,
                message,
                data,
            } => {
                BackgroundProtocolEvent::command_error(command_id, session_id, code, message, data)
            }
        }
    }
}

impl CommandResponseResult {
    fn into_value(self) -> Value {
        match self {
            Self::Empty => json!({}),
            Self::Json(value) => value,
        }
    }
}

impl CommandOwnerEvent {
    fn into_background_event(self) -> BackgroundProtocolEvent {
        match self {
            Self::InspectorTargetCrashed { session_id } => {
                BackgroundProtocolEvent::inspector_target_crashed(session_id.as_deref())
            }
            Self::PageLifecycle {
                session_id,
                name,
                frame_id,
                loader_id,
                timestamp,
            } => {
                let lifecycle = PageLifecycleEvent {
                    target_id: DevToolsTargetId::from(frame_id.as_str()),
                    frame_id: DevToolsFrameId::from(frame_id),
                    loader_id: DevToolsLoaderId::from(loader_id),
                    name,
                    timestamp,
                };
                BackgroundProtocolEvent::immediate_automation_event(
                    build_event(
                        "Page.lifecycleEvent",
                        json!({
                            "frameId": lifecycle.frame_id.as_str(),
                            "loaderId": lifecycle.loader_id.as_str(),
                            "name": lifecycle.name.as_str(),
                            "timestamp": lifecycle.timestamp,
                        }),
                        session_id.as_deref(),
                    ),
                    AutomationEvent::PageLifecycle(lifecycle),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::devtools_runtime::{
        AutomationEvent, DevToolsCaptureScreenshotResult, DevToolsCommandResult, DevToolsErrorKind,
        NavigationFrameEventKind,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use serde_json::json;

    use super::{
        CommandOutputPlan, devtools_error_from_cdp_error_value, protocol_message_background_event,
        protocol_message_background_event_for_target,
    };

    #[test]
    fn command_output_plan_emits_inspector_target_crashed_after_result() {
        let mut plan = CommandOutputPlan::success();
        plan.push_inspector_target_crashed(Some("SID-1"));

        let mut out = Vec::new();
        plan.emit_into(&mut out, Some(8), Some("SID-1"));

        assert_eq!(out[0], json!({"id": 8, "result": {}, "sessionId": "SID-1"}));
        assert_eq!(
            out[1],
            json!({
                "method": "Inspector.targetCrashed",
                "params": {},
                "sessionId": "SID-1"
            })
        );
    }

    #[test]
    fn command_output_plan_attaches_session_to_error_response() {
        let mut out = Vec::new();
        CommandOutputPlan::error(-32001, "Unknown sessionId").emit_into(
            &mut out,
            Some(9),
            Some("SID-missing"),
        );

        assert_eq!(
            out,
            vec![json!({
                "id": 9,
                "error": {"code": -32001, "message": "Unknown sessionId"},
                "sessionId": "SID-missing"
            })]
        );
    }

    #[test]
    fn command_output_plan_emits_non_empty_result_payload() {
        let mut out = Vec::new();
        CommandOutputPlan::result(json!({"metrics": [{"name": "Timestamp", "value": 1.25}]}))
            .emit_into(&mut out, Some(11), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 11,
                "result": {"metrics": [{"name": "Timestamp", "value": 1.25}]}
            })]
        );
    }

    #[test]
    fn command_output_plan_base64_encodes_owned_screenshot_bytes() {
        let bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(DevToolsCommandResult::CaptureScreenshot(
            DevToolsCaptureScreenshotResult {
                mime_type: "image/png".to_owned(),
                width: 800,
                height: 600,
                bytes: bytes.clone().into(),
            },
        ))
        .emit_into(&mut out, Some(12), Some("SID-page"));

        assert_eq!(out[0]["id"], json!(12));
        assert_eq!(out[0]["sessionId"], json!("SID-page"));
        let data = out[0]["result"]["data"]
            .as_str()
            .expect("captureScreenshot result should contain base64 data");
        assert_eq!(BASE64_STANDARD.decode(data).unwrap(), bytes);
        assert!(out[0]["result"].get("mimeType").is_none());
        assert!(out[0]["result"].get("width").is_none());
        assert!(out[0]["result"].get("height").is_none());
    }

    #[test]
    fn command_output_plan_serializes_devtools_navigate_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::Navigate(
                crate::devtools_runtime::DevToolsNavigateResult {
                    navigation_id: Some(crate::devtools_runtime::DevToolsNavigationId::from(
                        "NAV-1",
                    )),
                    frame_id: Some(crate::devtools_runtime::DevToolsFrameId::from("FRAME-1")),
                    loader_id: Some(crate::devtools_runtime::DevToolsLoaderId::from("LOADER-1")),
                    url: "https://example.test/".to_owned(),
                    error_text: None,
                    is_download: None,
                },
            ),
        )
        .emit_into(&mut out, Some(12), Some("SID-1"));

        assert_eq!(
            out,
            vec![json!({
                "id": 12,
                "result": {
                    "frameId": "FRAME-1",
                    "loaderId": "LOADER-1"
                },
                "sessionId": "SID-1"
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_aborted_cdp_navigate_as_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::Navigate(
                crate::devtools_runtime::DevToolsNavigateResult {
                    navigation_id: None,
                    frame_id: Some(crate::devtools_runtime::DevToolsFrameId::from("FRAME-1")),
                    loader_id: None,
                    url: "https://superseded.example.test/".to_owned(),
                    error_text: Some("net::ERR_ABORTED".to_owned()),
                    is_download: Some(false),
                },
            ),
        )
        .emit_into(&mut out, Some(13), Some("SID-1"));

        assert_eq!(
            out,
            vec![json!({
                "id": 13,
                "result": {
                    "frameId": "FRAME-1",
                    "errorText": "net::ERR_ABORTED",
                    "isDownload": false
                },
                "sessionId": "SID-1"
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_script_value_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::Script(Box::new(
                crate::devtools_runtime::DevToolsScriptResult::Value(
                    crate::devtools_runtime::DevToolsRemoteValue {
                        value: json!("Moli"),
                        handle: Some(crate::devtools_runtime::DevToolsRemoteHandleId::from(
                            "OBJ-1",
                        )),
                        shared_id: None,
                        node_id: None,
                        backend_node_id: None,
                        window_context: None,
                        realm: None,
                        remote_type: None,
                        remote_subtype: None,
                        unserializable_value: None,
                        description: None,
                        class_name: None,
                        deep_serialized_value: None,
                        node_value: None,
                    },
                ),
            )),
        )
        .emit_into(&mut out, Some(13), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 13,
                "result": {
                    "result": {
                        "type": "string",
                        "value": "Moli",
                        "objectId": "OBJ-1"
                    }
                }
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_outer_html_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::GetOuterHtml(
                crate::devtools_runtime::DevToolsGetOuterHtmlResult {
                    outer_html: "<html><body>source</body></html>".to_owned(),
                },
            ),
        )
        .emit_into(&mut out, Some(14), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 14,
                "result": {
                    "outerHTML": "<html><body>source</body></html>"
                }
            })]
        );
    }

    #[test]
    fn command_output_plan_moves_devtools_outer_html_allocation() {
        let mut outer_html = String::with_capacity(256);
        outer_html.push_str("<html><body>large result</body></html>");
        let outer_html_pointer = outer_html.as_ptr();
        let mut out = Vec::new();

        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::GetOuterHtml(
                crate::devtools_runtime::DevToolsGetOuterHtmlResult { outer_html },
            ),
        )
        .emit_into(&mut out, Some(14), None);

        assert_eq!(
            out[0]["result"]["outerHTML"]
                .as_str()
                .expect("outerHTML string")
                .as_ptr(),
            outer_html_pointer,
            "the typed DOM result must not copy its large String"
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_query_selector_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::QuerySelector(
                crate::devtools_runtime::DevToolsQuerySelectorResult {
                    node_ids: vec![7, 9],
                    multiple: true,
                },
            ),
        )
        .emit_into(&mut out, Some(15), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 15,
                "result": {
                    "nodeIds": [7, 9]
                }
            })]
        );

        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::QuerySelector(
                crate::devtools_runtime::DevToolsQuerySelectorResult {
                    node_ids: Vec::new(),
                    multiple: false,
                },
            ),
        )
        .emit_into(&mut out, Some(16), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 16,
                "result": {
                    "nodeId": 0
                }
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_get_attributes_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::GetAttributes(
                crate::devtools_runtime::DevToolsGetAttributesResult {
                    attributes: vec![
                        crate::devtools_runtime::DevToolsDomAttribute {
                            name: "id".to_owned(),
                            value: "target".to_owned(),
                        },
                        crate::devtools_runtime::DevToolsDomAttribute {
                            name: "data-kind".to_owned(),
                            value: "primary".to_owned(),
                        },
                    ],
                },
            ),
        )
        .emit_into(&mut out, Some(17), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 17,
                "result": {
                    "attributes": ["id", "target", "data-kind", "primary"]
                }
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_get_text_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::GetText(
                crate::devtools_runtime::DevToolsGetTextResult {
                    text: "element text".to_owned(),
                },
            ),
        )
        .emit_into(&mut out, Some(18), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 18,
                "result": {
                    "text": "element text"
                }
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_get_property_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::GetProperty(
                crate::devtools_runtime::DevToolsGetPropertyResult {
                    value: json!("property value"),
                },
            ),
        )
        .emit_into(&mut out, Some(19), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 19,
                "result": {
                    "value": "property value"
                }
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_create_target_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::CreateTarget(
                crate::devtools_runtime::DevToolsCreateTargetResult {
                    target_id: crate::devtools_runtime::DevToolsTargetId::from("TARGET-1"),
                },
            ),
        )
        .emit_into(&mut out, Some(16), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 16,
                "result": {"targetId": "TARGET-1"}
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_close_target_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::CloseTarget(
                crate::devtools_runtime::DevToolsCloseTargetResult { success: true },
            ),
        )
        .emit_into(&mut out, Some(17), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 17,
                "result": {"success": true}
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_add_preload_script_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::AddPreloadScript(
                crate::devtools_runtime::DevToolsAddPreloadScriptResult {
                    script_id: crate::devtools_runtime::DevToolsPreloadScriptId::from("SCRIPT-1"),
                },
            ),
        )
        .emit_into(&mut out, Some(18), None);

        assert_eq!(
            out,
            vec![json!({
                "id": 18,
                "result": {"identifier": "SCRIPT-1"}
            })]
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_script_exception_result() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::Script(Box::new(
                crate::devtools_runtime::DevToolsScriptResult::Exception(
                    crate::devtools_runtime::DevToolsScriptException {
                        exception_id: Some(42),
                        script_id: None,
                        text: "boom".to_owned(),
                        value: Some(
                            crate::devtools_runtime::DevToolsRemoteValue::from_json_value(json!(
                                "boom"
                            )),
                        ),
                        realm: None,
                        line_number: None,
                        column_number: None,
                        stack_trace: None,
                    },
                ),
            )),
        )
        .emit_into(&mut out, Some(14), None);

        assert_eq!(out[0]["id"], json!(14));
        assert_eq!(out[0]["result"]["result"]["type"], json!("undefined"));
        assert_eq!(
            out[0]["result"]["exceptionDetails"]["exceptionId"],
            json!(42)
        );
        assert_eq!(out[0]["result"]["exceptionDetails"]["text"], json!("boom"));
        assert_eq!(
            out[0]["result"]["exceptionDetails"]["exception"],
            json!({"type": "string", "value": "boom"})
        );
    }

    #[test]
    fn command_output_plan_omits_missing_devtools_script_exception_id() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_result(
            crate::devtools_runtime::DevToolsCommandResult::Script(Box::new(
                crate::devtools_runtime::DevToolsScriptResult::Exception(
                    crate::devtools_runtime::DevToolsScriptException {
                        exception_id: None,
                        script_id: None,
                        text: "boom".to_owned(),
                        value: Some(
                            crate::devtools_runtime::DevToolsRemoteValue::from_json_value(json!(
                                "boom"
                            )),
                        ),
                        realm: None,
                        line_number: None,
                        column_number: None,
                        stack_trace: None,
                    },
                ),
            )),
        )
        .emit_into(&mut out, Some(14), None);

        assert_eq!(out[0]["id"], json!(14));
        assert_eq!(out[0]["result"]["result"]["type"], json!("undefined"));
        assert!(
            out[0]["result"]["exceptionDetails"]
                .as_object()
                .expect("exceptionDetails")
                .get("exceptionId")
                .is_none()
        );
    }

    #[test]
    fn command_output_plan_serializes_devtools_error() {
        let mut out = Vec::new();
        CommandOutputPlan::from_devtools_error(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchSession,
            "Unknown sessionId",
        ))
        .emit_into(&mut out, Some(15), Some("SID-missing"));

        assert_eq!(
            out,
            vec![json!({
                "id": 15,
                "error": {"code": -32001, "message": "Unknown sessionId"},
                "sessionId": "SID-missing"
            })]
        );
    }

    #[test]
    fn command_output_plan_can_emit_error_without_session_route() {
        let mut out = Vec::new();
        CommandOutputPlan::error_without_session(-31998, "TargetNotLoaded").emit_into(
            &mut out,
            Some(16),
            Some("SID-current"),
        );

        assert_eq!(
            out,
            vec![json!({
                "id": 16,
                "error": {"code": -31998, "message": "TargetNotLoaded"}
            })]
        );
    }

    #[test]
    fn command_output_parses_navigation_changing_document_error_kind() {
        let error = devtools_error_from_cdp_error_value(&json!({
            "code": -32000,
            "message": "Navigation is changing the document"
        }));

        assert_eq!(error.kind, DevToolsErrorKind::NavigationChangingDocument);
        assert_eq!(error.message, "Navigation is changing the document");
    }

    #[test]
    fn command_output_plan_emits_page_lifecycle_event_before_result() {
        let mut plan = CommandOutputPlan::default();
        plan.push_page_lifecycle_event(
            Some("SID-page"),
            "DOMContentLoaded",
            "TID-page",
            "LOADER-page",
            12.5,
        );
        plan.push_success();

        let mut out = Vec::new();
        plan.emit_into(&mut out, Some(13), Some("SID-page"));

        assert_eq!(out[0]["method"], "Page.lifecycleEvent");
        assert_eq!(out[0]["sessionId"], "SID-page");
        assert_eq!(out[0]["params"]["name"], "DOMContentLoaded");
        assert_eq!(out[0]["params"]["frameId"], "TID-page");
        assert_eq!(out[0]["params"]["loaderId"], "LOADER-page");
        assert_eq!(out[0]["params"]["timestamp"], json!(12.5));
        assert_eq!(
            out[1],
            json!({"id": 13, "result": {}, "sessionId": "SID-page"})
        );
    }

    #[test]
    fn command_output_plan_preserves_page_lifecycle_typed_sidecar() {
        let mut plan = CommandOutputPlan::default();
        plan.push_page_lifecycle_event(
            Some("SID-page"),
            "DOMContentLoaded",
            "TID-page",
            "LOADER-page",
            12.5,
        );

        let mut events = plan.into_background_events(Some(13), Some("SID-page"));
        let (message, automation_event) = events
            .pop()
            .expect("page lifecycle event should be emitted")
            .into_parts();

        assert_eq!(message["method"], "Page.lifecycleEvent");
        assert_eq!(message["sessionId"], "SID-page");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::PageLifecycle(event))
                if event.frame_id.as_str() == "TID-page"
                    && event.loader_id.as_str() == "LOADER-page"
                    && event.name == "DOMContentLoaded"
        ));
    }

    #[test]
    fn command_output_plan_splits_runtime_inspector_response_from_background_events() {
        let mut plan = CommandOutputPlan::default();
        assert!(plan.push_runtime_inspector_protocol_response(
            json!({
                "id": 55,
                "result": {
                    "result": {"type": "number", "value": 7}
                },
                "sessionId": "STALE"
            }),
            Some(55)
        ));
        plan.push_background_event(protocol_message_background_event(json!({
            "method": "Runtime.consoleAPICalled",
            "sessionId": "SID-runtime",
            "params": {
                "type": "log",
                "args": [{"type": "string", "value": "side"}]
            }
        })));

        let (response, events) =
            plan.into_runtime_inspector_response_and_background_events(55, Some("SID-runtime"));

        assert_eq!(
            response.expect("runtime response"),
            json!({
                "id": 55,
                "result": {
                    "result": {"type": "number", "value": 7}
                },
                "sessionId": "SID-runtime"
            })
        );
        assert_eq!(events.len(), 1);
        let (message, automation_event) = events.into_iter().next().unwrap().into_parts();
        assert_eq!(message["method"], "Runtime.consoleAPICalled");
        assert_eq!(message["sessionId"], "SID-runtime");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::RuntimeConsoleApiCalled(event))
                if event.console_type == "log" && event.text == "side"
        ));
    }

    #[test]
    fn command_output_plan_splits_command_error_as_runtime_response() {
        let (response, events) = CommandOutputPlan::error(-32000, "NoSuchScript")
            .into_runtime_inspector_response_and_background_events(56, Some("SID-runtime"));

        assert_eq!(
            response.expect("runtime response"),
            json!({
                "id": 56,
                "error": {"code": -32000, "message": "NoSuchScript"},
                "sessionId": "SID-runtime"
            })
        );
        assert!(events.is_empty());
    }

    #[test]
    fn command_output_classifies_page_navigation_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Page.frameNavigated",
            "params": {
                "type": "Navigation",
                "frame": {
                    "id": "FRAME-page",
                    "parentId": "FRAME-parent",
                    "loaderId": "LOADER-page",
                    "url": "https://example.test/page",
                    "securityOrigin": "https://example.test",
                    "secureContextType": "Secure",
                    "name": "main"
                }
            },
            "sessionId": "SID-page"
        }));

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Page.frameNavigated");
        assert_eq!(message["sessionId"], "SID-page");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::NavigationFrame(event))
                if event.kind == NavigationFrameEventKind::Navigated
                    && event.frame_id.as_str() == "FRAME-page"
                    && event.parent_frame_id.as_ref().map(|id| id.as_str()) == Some("FRAME-parent")
                    && event.loader_id.as_ref().map(|id| id.as_str()) == Some("LOADER-page")
                    && event.url == "https://example.test/page"
                    && event.frame_name.as_deref() == Some("main")
                    && event.security_origin.as_deref() == Some("https://example.test")
                    && event.secure_context_type.as_deref() == Some("Secure")
        ));
    }

    #[test]
    fn command_output_classifies_same_document_navigation_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Page.navigatedWithinDocument",
            "params": {
                "frameId": "FRAME-page",
                "url": "https://example.test/page#section",
                "navigationType": "fragment"
            },
            "sessionId": "SID-page"
        }));

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Page.navigatedWithinDocument");
        assert_eq!(message["sessionId"], "SID-page");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::SameDocumentNavigation(event))
                if event.frame_id.as_str() == "FRAME-page"
                    && event.url == "https://example.test/page#section"
                    && event.navigation_type == "fragment"
        ));
    }

    #[test]
    fn command_output_classifies_javascript_dialog_opening_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Page.javascriptDialogOpening",
            "params": {
                "frameId": "FRAME-dialog",
                "url": "https://example.test/dialog",
                "message": "hello",
                "type": "prompt",
                "hasBrowserHandler": true,
                "defaultPrompt": "default"
            },
            "sessionId": "SID-dialog"
        }));

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Page.javascriptDialogOpening");
        assert_eq!(message["sessionId"], "SID-dialog");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::PageJavaScriptDialogOpening(event))
                if event.frame_id.as_ref().map(|id| id.as_str()) == Some("FRAME-dialog")
                    && event.url == "https://example.test/dialog"
                    && event.message == "hello"
                    && event.dialog_type == "prompt"
                    && event.has_browser_handler
                    && event.default_prompt == "default"
        ));
    }

    #[test]
    fn command_output_classifies_file_chooser_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Page.fileChooserOpened",
            "params": {
                "frameId": "FRAME-file",
                "backendNodeId": 77,
                "mode": "selectMultiple"
            },
            "sessionId": "SID-file"
        }));

        let (message, automation_event) = event.into_parts();
        assert_eq!(message["method"], "Page.fileChooserOpened");
        assert_eq!(message["sessionId"], "SID-file");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::PageFileChooserOpened(event))
                if event.frame_id.as_str() == "FRAME-file"
                    && event.backend_node_id == 77
                    && event.mode == "selectMultiple"
        ));
    }

    #[test]
    fn command_output_classifies_log_entry_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Log.entryAdded",
            "params": {
                "entry": {
                    "source": "javascript",
                    "level": "error",
                    "text": "observable failure",
                    "timestamp": 31.25,
                    "url": "https://example.test/app"
                }
            },
            "sessionId": "SID-log"
        }));

        let (message, automation_event) = event.into_parts();
        assert_eq!(message["method"], "Log.entryAdded");
        assert_eq!(message["sessionId"], "SID-log");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::LogEntryAdded(event))
                if event.text == "observable failure"
                    && event.level == "error"
                    && event.url.as_deref() == Some("https://example.test/app")
                    && event.timestamp == Some(31.25)
        ));
    }

    #[test]
    fn command_output_classifies_console_message_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Console.messageAdded",
            "params": {
                "message": {
                    "source": "console-api",
                    "level": "warning",
                    "text": "console domain observed",
                    "url": "https://example.test/app",
                    "line": 0,
                    "column": 0
                }
            },
            "sessionId": "SID-console"
        }));

        let (message, automation_event) = event.into_parts();
        assert_eq!(message["method"], "Console.messageAdded");
        assert_eq!(message["sessionId"], "SID-console");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::RuntimeConsoleApiCalled(event))
                if event.console_type == "warning"
                    && event.text == "console domain observed"
                    && event.execution_context_id.is_none()
                    && event.timestamp.is_none()
                    && event.args == vec![json!({
                        "type": "string",
                        "value": "console domain observed"
                    })]
        ));
    }

    #[test]
    fn command_output_classifies_runtime_console_protocol_messages() {
        let event = protocol_message_background_event_for_target(
            json!({
                "method": "Runtime.consoleAPICalled",
                "params": {
                    "type": "warning",
                    "args": [{"type": "string", "value": "runtime observed"}],
                    "executionContextId": 9,
                    "timestamp": 41.5,
                    "stackTrace": {
                        "callFrames": [{
                            "functionName": "run",
                            "url": "https://example.test/app.js",
                            "lineNumber": 2,
                            "columnNumber": 7
                        }]
                    }
                },
                "sessionId": "SID-runtime"
            }),
            Some("TID-runtime"),
        );

        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Runtime.consoleAPICalled"));
        let (message, automation_event) = event.into_parts();
        assert_eq!(message["method"], "Runtime.consoleAPICalled");
        assert_eq!(message["sessionId"], "SID-runtime");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::RuntimeConsoleApiCalled(event))
                if event.console_type == "warning"
                    && event.text == "runtime observed"
                    && event.target_id.as_ref().is_some_and(|id| id.as_str() == "TID-runtime")
                    && event.execution_context_id == Some(9)
                    && event.timestamp == Some(41.5)
                    && event.stack_trace.as_ref().is_some_and(|trace| {
                        trace.call_frames.first().is_some_and(|frame| {
                            frame.function_name == "run"
                                && frame.url == "https://example.test/app.js"
                                && frame.line_number == 2
                                && frame.column_number == 7
                        })
                    })
        ));
    }

    #[test]
    fn command_output_classifies_runtime_exception_protocol_messages() {
        let event = protocol_message_background_event(json!({
            "method": "Runtime.exceptionThrown",
            "params": {
                "timestamp": 42.5,
                "exceptionDetails": {
                    "exceptionId": 4,
                    "text": "runtime exception",
                    "lineNumber": 3,
                    "columnNumber": 8,
                    "scriptId": "SCRIPT-9",
                    "url": "https://example.test/app.js",
                    "executionContextId": 10,
                    "exception": {
                        "type": "object",
                        "subtype": "error",
                        "className": "Error",
                        "description": "runtime exception"
                    },
                    "stackTrace": {
                        "callFrames": [{
                            "functionName": "explode",
                            "scriptId": "SCRIPT-9",
                            "url": "https://example.test/app.js",
                            "lineNumber": 3,
                            "columnNumber": 8
                        }]
                    }
                }
            },
            "sessionId": "SID-runtime"
        }));

        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Runtime.exceptionThrown"));
        let (message, automation_event) = event.into_parts();
        assert_eq!(message["method"], "Runtime.exceptionThrown");
        assert_eq!(message["sessionId"], "SID-runtime");
        assert_eq!(
            message["params"]["exceptionDetails"]["scriptId"],
            "SCRIPT-9"
        );
        assert_eq!(
            message["params"]["exceptionDetails"]["stackTrace"]["callFrames"][0]["scriptId"],
            "SCRIPT-9"
        );
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::ScriptException(event))
                if event.exception.exception_id == Some(4)
                    && event.exception.script_id.as_deref() == Some("SCRIPT-9")
                    && event.exception_index == Some(3)
                    && event.exception.text == "runtime exception"
                    && event.url.as_deref() == Some("https://example.test/app.js")
                    && event.execution_context_id == Some(10)
                    && event.timestamp == Some(42.5)
                    && event.exception.stack_trace.as_ref().is_some_and(|trace| {
                        trace.call_frames.first().is_some_and(|frame| {
                            frame.function_name == "explode"
                                && frame.script_id.as_deref() == Some("SCRIPT-9")
                                && frame.url == "https://example.test/app.js"
                                && frame.line_number == 3
                                && frame.column_number == 8
                        })
                    })
        ));
    }

    #[test]
    fn command_output_classifies_network_request_protocol_fields() {
        let event = protocol_message_background_event(json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": "REQ-1",
                "loaderId": "LOADER-1",
                "documentURL": "https://example.test/start",
                "request": {
                    "url": "https://example.test/api",
                    "method": "POST",
                    "headers": {
                        "Content-Type": "application/json",
                        "X-Number": 7
                    },
                    "hasPostData": true,
                    "postData": "{\"ok\":true}"
                },
                "timestamp": 1.5,
                "wallTime": 2.5,
                "initiator": {"type": "script"},
                "type": "Fetch",
                "frameId": "FRAME-1",
                "redirectResponse": {
                    "url": "https://example.test/old",
                    "status": 302,
                    "statusText": "Found",
                    "headers": {"Location": "/api"},
                    "encodedDataLength": 13,
                    "fromDiskCache": true
                }
            },
            "sessionId": "SID-network"
        }));

        let (message, automation_event) = event.into_parts();
        assert_eq!(message["method"], "Network.requestWillBeSent");
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::NetworkBeforeRequestSent(event))
                if event.request_id.as_str() == "REQ-1"
                    && event.loader_id.as_ref().is_some_and(|id| id.as_str() == "LOADER-1")
                    && event.url == "https://example.test/api"
                    && event.method.as_deref() == Some("POST")
                    && event.request_body.as_deref() == Some("{\"ok\":true}")
                    && event.request_headers.iter().any(|(name, value)| {
                        name == "Content-Type" && value == "application/json"
                    })
                    && event.request_headers.iter().any(|(name, value)| {
                        name == "X-Number" && value == "7"
                    })
                    && event.redirect_response.as_ref().is_some_and(|redirect| {
                        redirect.url == "https://example.test/old"
                            && redirect.status == 302
                            && redirect.status_text.as_deref() == Some("Found")
                            && redirect.from_cache
                            && redirect.response_headers == vec![(
                                "Location".to_owned(),
                                "/api".to_owned()
                            )]
                    })
        ));
    }

    #[test]
    fn command_output_classifies_network_response_protocol_fields() {
        let event = protocol_message_background_event(json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": "REQ-1",
                "loaderId": "LOADER-1",
                "timestamp": 3.5,
                "type": "Fetch",
                "frameId": "FRAME-1",
                "response": {
                    "url": "https://example.test/api",
                    "status": 201,
                    "statusText": "Created",
                    "headers": {
                        "Content-Type": "application/json"
                    },
                    "mimeType": "application/json",
                    "encodedDataLength": 42,
                    "fromPrefetchCache": true
                },
                "hasExtraInfo": true
            },
            "sessionId": "SID-network"
        }));

        let (_, automation_event) = event.into_parts();
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::NetworkResponseStarted(event))
                if event.status == Some(201)
                    && event.status_text.as_deref() == Some("Created")
                    && event.encoded_data_length == Some(42)
                    && event.from_cache
                    && event.has_extra_info
                    && event.response_headers == vec![(
                        "Content-Type".to_owned(),
                        "application/json".to_owned()
                    )]
        ));
    }

    #[test]
    fn command_output_classifies_fetch_auth_and_pause_protocol_fields() {
        let auth = protocol_message_background_event(json!({
            "method": "Fetch.authRequired",
            "params": {
                "requestId": "FETCH-1",
                "frameId": "FRAME-1",
                "request": {
                    "url": "https://example.test/secure",
                    "method": "GET",
                    "headers": {"Accept": "*/*"}
                },
                "resourceType": "Document",
                "authChallenge": {
                    "origin": "https://example.test",
                    "source": "Server",
                    "scheme": "Basic",
                    "realm": "private"
                }
            }
        }));
        let (_, automation_event) = auth.into_parts();
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::NetworkAuthRequired(event))
                if event.request_id.as_str() == "FETCH-1"
                    && event.network_id.is_none()
                    && event.auth_challenge.as_ref().is_some_and(|challenge| {
                        challenge.origin == "https://example.test"
                            && challenge.scheme == "Basic"
                            && challenge.realm == "private"
                    })
                    && event.request_headers == vec![("Accept".to_owned(), "*/*".to_owned())]
        ));

        let paused = protocol_message_background_event(json!({
            "method": "Fetch.requestPaused",
            "params": {
                "requestId": "FETCH-2",
                "networkId": "REQ-2",
                "frameId": "FRAME-1",
                "request": {
                    "url": "https://example.test/secure",
                    "method": "GET",
                    "headers": {}
                },
                "resourceType": "Fetch",
                "responseStatusCode": 401,
                "responseHeaders": [
                    {"name": "WWW-Authenticate", "value": "Basic realm=\"private\""}
                ]
            }
        }));
        let (_, automation_event) = paused.into_parts();
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::RequestPaused(event))
                if event.status == Some(401)
                    && event.response_headers == vec![(
                        "WWW-Authenticate".to_owned(),
                        "Basic realm=\"private\"".to_owned()
                    )]
        ));
    }

    #[test]
    fn command_output_plan_command_response_stays_typed_until_wire_projection() {
        let mut events =
            CommandOutputPlan::success().into_background_events(Some(20), Some("SID-response"));
        assert_eq!(events[0].protocol_message_id(), Some(20));
        assert!(
            events[0].protocol_message().is_none(),
            "command responses should not be exposed as raw protocol-message internals"
        );

        let (message, automation_event) = events
            .pop()
            .expect("command response should be emitted")
            .into_parts();
        assert_eq!(
            message,
            json!({"id": 20, "result": {}, "sessionId": "SID-response"})
        );
        assert!(automation_event.is_none());
    }
}
