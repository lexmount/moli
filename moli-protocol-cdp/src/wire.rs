use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{error::Error, fmt};

/// CDP response class for a command that could not enter the typed wire
/// envelope.
///
/// Syntax/EOF failures are JSON-RPC parse errors. A syntactically valid JSON
/// value whose known CDP fields have the wrong shape is an invalid request,
/// matching Chromium's `Dispatchable::DispatchError` split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CdpCommandParseErrorKind {
    ParseError,
    InvalidRequest,
}

/// Failure to construct a typed [`ParsedCdpCommand`].
///
/// `command_id` is recovered only on the invalid path, after the normal
/// single-pass parse has failed. Valid commands therefore do not pay for a
/// second JSON parse, while a structurally invalid request can still receive
/// the response id Chromium would preserve.
#[derive(Debug)]
pub struct CdpCommandParseError {
    kind: CdpCommandParseErrorKind,
    command_id: Option<u64>,
    source: serde_json::Error,
}

impl CdpCommandParseError {
    fn from_str(json: &str, source: serde_json::Error) -> Self {
        let kind = match source.classify() {
            serde_json::error::Category::Data => CdpCommandParseErrorKind::InvalidRequest,
            serde_json::error::Category::Io
            | serde_json::error::Category::Syntax
            | serde_json::error::Category::Eof => CdpCommandParseErrorKind::ParseError,
        };
        let command_id = if kind == CdpCommandParseErrorKind::InvalidRequest {
            serde_json::from_str::<Value>(json)
                .ok()
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|message| message.get("id"))
                .and_then(Value::as_u64)
        } else {
            None
        };
        Self {
            kind,
            command_id,
            source,
        }
    }

    fn from_value(command_id: Option<u64>, source: serde_json::Error) -> Self {
        let kind = match source.classify() {
            serde_json::error::Category::Data => CdpCommandParseErrorKind::InvalidRequest,
            serde_json::error::Category::Io
            | serde_json::error::Category::Syntax
            | serde_json::error::Category::Eof => CdpCommandParseErrorKind::ParseError,
        };
        Self {
            kind,
            command_id: if kind == CdpCommandParseErrorKind::InvalidRequest {
                command_id
            } else {
                None
            },
            source,
        }
    }

    pub const fn kind(&self) -> CdpCommandParseErrorKind {
        self.kind
    }

    pub const fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub const fn response_code(&self) -> i32 {
        match self.kind {
            CdpCommandParseErrorKind::ParseError => -32700,
            CdpCommandParseErrorKind::InvalidRequest => -32600,
        }
    }

    pub const fn response_message(&self) -> &'static str {
        match self.kind {
            CdpCommandParseErrorKind::ParseError => "Parse error",
            CdpCommandParseErrorKind::InvalidRequest => "Invalid Request",
        }
    }
}

impl fmt::Display for CdpCommandParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(formatter)
    }
}

impl Error for CdpCommandParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Validated top-level fields of one inbound CDP command.
///
/// Known CDP fields remain strongly typed after validation. `extension_fields`
/// retains unknown top-level fields so frontend routing can rewrite `id` and
/// `sessionId` without discarding extensions from a newer CDP version.
#[derive(Debug, Deserialize, Serialize)]
pub struct CdpRequest {
    id: u64,
    method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<Map<String, Value>>,
    #[serde(rename = "sessionId", default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(flatten)]
    extension_fields: Map<String, Value>,
}

impl CdpRequest {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn params(&self) -> Option<&Map<String, Value>> {
        self.params.as_ref()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn rewrite_frontend_route(
        &mut self,
        internal_command_id: u64,
        dispatch_session_id: Option<&str>,
    ) {
        self.id = internal_command_id;
        self.session_id = dispatch_session_id.map(str::to_owned);
    }

    fn rewrite_target_session_reference(&mut self, session_id: &str) {
        self.params
            .get_or_insert_default()
            .insert("sessionId".to_owned(), Value::String(session_id.to_owned()));
    }
}

/// Renderer access required by one parsed CDP command.
///
/// Chromium physically separates renderer main-thread and IO
/// DevTools routes. Moli keeps one protocol actor, so the parsed command
/// carries the equivalent scheduling fact explicitly instead of asking later
/// layers to reinterpret the wire method string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CdpRendererCommandAccess {
    /// The command is handled outside the renderer attachment cutover.
    OwnerIndependent,
    /// The command must wait for a cross-document navigation to install its
    /// replacement renderer attachment.
    MainThread,
    /// The command may address the suspended renderer in order to inspect,
    /// interrupt, or release it while navigation is in flight.
    Io,
}

/// Fate of an in-flight renderer command when navigation replaces its
/// renderer attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CdpRendererCommandReplacement {
    /// Dispatch the command again against the replacement attachment.
    Replay,
    /// Complete the command as terminated because it was bound to the old
    /// JavaScript context.
    Terminate,
}

/// How a replayed command must enter the replacement Runtime agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CdpRendererCommandReplayDispatch {
    /// Send the command directly to the replacement agent.
    Direct,
    /// Resolve the replacement Document's default Runtime context before
    /// sending the command.
    ResolveRuntimeContext,
}

/// Immutable renderer scheduling policy derived once from a validated CDP
/// method at command ingress.
///
/// Downstream dispatch and renderer-call registration copy this value instead
/// of rebuilding policy from serialized Inspector JSON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CdpRendererCommandPolicy {
    renderer_access: CdpRendererCommandAccess,
    renderer_replacement: CdpRendererCommandReplacement,
    renderer_replay_dispatch: CdpRendererCommandReplayDispatch,
    executes_page_javascript: bool,
}

impl CdpRendererCommandPolicy {
    pub const fn access(self) -> CdpRendererCommandAccess {
        self.renderer_access
    }

    pub const fn replacement(self) -> CdpRendererCommandReplacement {
        self.renderer_replacement
    }

    pub const fn replay_dispatch(self) -> CdpRendererCommandReplayDispatch {
        self.renderer_replay_dispatch
    }

    pub const fn executes_page_javascript(self) -> bool {
        self.executes_page_javascript
    }
}

/// Parsed inbound CDP command plus its downstream JSON representation.
///
/// Domain handlers still need serialized JSON for Runtime inspector
/// passthrough, while the scheduler and dispatch layer need cheap access to
/// method/session metadata. `parse_str` preserves the supplied JSON text;
/// `parse_value` and `from_serializable` produce a normalized representation
/// after an in-process command has been rewritten or constructed.
pub struct ParsedCdpCommand {
    json: String,
    request: CdpRequest,
    renderer_policy: CdpRendererCommandPolicy,
}

impl ParsedCdpCommand {
    /// Parse a command received as JSON text.
    ///
    /// A successful value is always a structurally valid CDP request. Parse
    /// failures remain at the wire boundary and cannot travel through the
    /// scheduler disguised as a parsed command.
    pub fn parse_str(json: impl Into<String>) -> Result<Self, CdpCommandParseError> {
        let json = json.into();
        let request = serde_json::from_str(&json)
            .map_err(|source| CdpCommandParseError::from_str(&json, source))?;
        Ok(Self::from_request(json, request))
    }

    /// Parse a command that is already represented as a JSON value.
    ///
    /// In-process callers use this for values that have not crossed the text
    /// wire. Frontend routing instead consumes an existing parsed command and
    /// rewrites its validated envelope without entering this parser again.
    pub fn parse_value(value: Value) -> Result<Self, CdpCommandParseError> {
        let command_id = value
            .as_object()
            .and_then(|message| message.get("id"))
            .and_then(Value::as_u64);
        let request: CdpRequest = serde_json::from_value(value)
            .map_err(|source| CdpCommandParseError::from_value(command_id, source))?;
        let json = serde_json::to_string(&request)
            .map_err(|source| CdpCommandParseError::from_value(Some(request.id()), source))?;
        Ok(Self::from_request(json, request))
    }

    /// Build a command from an in-process serializable message.
    ///
    /// This is intentionally distinct from `parse_str`: passing a Rust string
    /// here serializes it as a JSON string rather than interpreting its
    /// contents as CDP JSON.
    pub fn from_serializable(message: impl Serialize) -> Result<Self, CdpCommandParseError> {
        let value = serde_json::to_value(message)
            .map_err(|source| CdpCommandParseError::from_value(None, source))?;
        Self::parse_value(value)
    }

    fn from_request(json: String, request: CdpRequest) -> Self {
        let renderer_policy = CdpRendererCommandPolicy::for_method(request.method());
        Self {
            json,
            request,
            renderer_policy,
        }
    }

    /// JSON sent to the downstream command dispatcher or V8 Inspector.
    pub fn json(&self) -> &str {
        &self.json
    }

    pub fn request(&self) -> &CdpRequest {
        &self.request
    }

    /// Consume this validated command and rewrite only the frontend routing
    /// fields. Unknown top-level fields remain in the flattened extension map;
    /// the validated method and params remain in their typed fields.
    pub fn rewrite_frontend_route(
        mut self,
        internal_command_id: u64,
        dispatch_session_id: Option<&str>,
    ) -> serde_json::Result<Self> {
        self.request
            .rewrite_frontend_route(internal_command_id, dispatch_session_id);
        self.json = serde_json::to_string(&self.request)?;
        Ok(self)
    }

    /// Resolve a legacy Target-domain `params.targetId` route to its concrete
    /// session before dispatch.
    ///
    /// Frontend adapters use this to preserve Chromium's per-client
    /// `TargetHandler::FindSession` ownership boundary even though all socket
    /// clients share one downstream protocol connection.
    pub fn rewrite_target_session_reference(
        mut self,
        session_id: &str,
    ) -> serde_json::Result<Self> {
        self.request.rewrite_target_session_reference(session_id);
        self.json = serde_json::to_string(&self.request)?;
        Ok(self)
    }

    pub fn method(&self) -> &str {
        self.request.method()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.request.session_id()
    }

    /// Returns the exact session whose already-produced output may be
    /// captured at this command's completion boundary.
    pub fn command_output_session_id(&self) -> Option<&str> {
        if self.request.method().contains('.') {
            self.request.session_id()
        } else {
            None
        }
    }

    pub fn runtime_command_executes_page_javascript(&self) -> bool {
        self.renderer_policy.executes_page_javascript()
    }

    pub const fn renderer_policy(&self) -> CdpRendererCommandPolicy {
        self.renderer_policy
    }

    pub fn renderer_access(&self) -> CdpRendererCommandAccess {
        self.renderer_policy.access()
    }

    pub fn renderer_replacement(&self) -> CdpRendererCommandReplacement {
        self.renderer_policy.replacement()
    }

    pub fn renderer_replay_dispatch(&self) -> CdpRendererCommandReplayDispatch {
        self.renderer_policy.replay_dispatch()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CdpMethodDomain {
    Accessibility,
    Console,
    Css,
    Debugger,
    Dom,
    DomSnapshot,
    Emulation,
    HeapProfiler,
    Page,
    Performance,
    Profiler,
    Runtime,
    Other,
}

impl CdpMethodDomain {
    fn parse(domain: &str) -> Self {
        match domain {
            "Accessibility" => Self::Accessibility,
            "Console" => Self::Console,
            "CSS" => Self::Css,
            "Debugger" => Self::Debugger,
            "DOM" => Self::Dom,
            "DOMSnapshot" => Self::DomSnapshot,
            "Emulation" => Self::Emulation,
            "HeapProfiler" => Self::HeapProfiler,
            "Page" => Self::Page,
            "Performance" => Self::Performance,
            "Profiler" => Self::Profiler,
            "Runtime" => Self::Runtime,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeWireAction {
    AddBinding,
    AwaitPromise,
    CallFunctionOn,
    Evaluate,
    RunScript,
    TerminateExecution,
    Other,
}

impl RuntimeWireAction {
    fn parse(action: &str) -> Self {
        match action {
            "addBinding" => Self::AddBinding,
            "awaitPromise" => Self::AwaitPromise,
            "callFunctionOn" => Self::CallFunctionOn,
            "evaluate" => Self::Evaluate,
            "runScript" => Self::RunScript,
            "terminateExecution" => Self::TerminateExecution,
            _ => Self::Other,
        }
    }

    fn executes_page_javascript(self) -> bool {
        matches!(
            self,
            Self::AwaitPromise | Self::CallFunctionOn | Self::Evaluate | Self::RunScript
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageWireAction {
    MainThread,
    Crash,
    Other,
}

impl PageWireAction {
    fn parse(action: &str) -> Self {
        match action {
            "captureScreenshot"
            | "captureSnapshot"
            | "createIsolatedWorld"
            | "getFrameTree"
            | "getResourceTree"
            | "searchInResource"
            | "getLayoutMetrics"
            | "printToPDF" => Self::MainThread,
            "crash" => Self::Crash,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DebuggerWireAction {
    Interruptible,
    IoExecutionControl,
    MainThreadExecutionControl,
    Other,
}

impl DebuggerWireAction {
    fn is_interruptible(self) -> bool {
        matches!(self, Self::Interruptible | Self::IoExecutionControl)
    }

    fn executes_page_javascript(self) -> bool {
        matches!(
            self,
            Self::IoExecutionControl | Self::MainThreadExecutionControl
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PerformanceWireAction {
    GetMetrics,
    Other,
}

impl PerformanceWireAction {
    fn parse(action: &str) -> Self {
        match action {
            "getMetrics" => Self::GetMetrics,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmulationWireAction {
    SetScriptExecutionDisabled,
    Other,
}

impl EmulationWireAction {
    fn parse(action: &str) -> Self {
        match action {
            "setScriptExecutionDisabled" => Self::SetScriptExecutionDisabled,
            _ => Self::Other,
        }
    }
}

impl DebuggerWireAction {
    fn parse(action: &str) -> Self {
        match action {
            "getPossibleBreakpoints"
            | "getScriptSource"
            | "getStackTrace"
            | "pause"
            | "removeBreakpoint"
            | "setBreakpoint"
            | "setBreakpointByUrl"
            | "setBreakpointsActive" => Self::Interruptible,
            "resume" => Self::IoExecutionControl,
            "continueToLocation" | "restartFrame" | "stepInto" | "stepOut" | "stepOver" => {
                Self::MainThreadExecutionControl
            }
            _ => Self::Other,
        }
    }
}

impl CdpRendererCommandPolicy {
    fn for_method(method: &str) -> Self {
        let Some((domain, action)) = method.split_once('.') else {
            return Self {
                renderer_access: CdpRendererCommandAccess::OwnerIndependent,
                renderer_replacement: CdpRendererCommandReplacement::Replay,
                renderer_replay_dispatch: CdpRendererCommandReplayDispatch::Direct,
                executes_page_javascript: false,
            };
        };
        let domain = CdpMethodDomain::parse(domain);
        let runtime_action =
            (domain == CdpMethodDomain::Runtime).then(|| RuntimeWireAction::parse(action));
        let debugger_action =
            (domain == CdpMethodDomain::Debugger).then(|| DebuggerWireAction::parse(action));
        let renderer_access = match domain {
            CdpMethodDomain::Runtime => {
                if runtime_action == Some(RuntimeWireAction::TerminateExecution) {
                    CdpRendererCommandAccess::Io
                } else {
                    CdpRendererCommandAccess::MainThread
                }
            }
            CdpMethodDomain::Debugger => {
                if debugger_action.is_some_and(DebuggerWireAction::is_interruptible) {
                    CdpRendererCommandAccess::Io
                } else {
                    CdpRendererCommandAccess::MainThread
                }
            }
            CdpMethodDomain::Performance => {
                if PerformanceWireAction::parse(action) == PerformanceWireAction::GetMetrics {
                    CdpRendererCommandAccess::Io
                } else {
                    CdpRendererCommandAccess::MainThread
                }
            }
            CdpMethodDomain::Emulation => {
                if EmulationWireAction::parse(action)
                    == EmulationWireAction::SetScriptExecutionDisabled
                {
                    CdpRendererCommandAccess::Io
                } else {
                    CdpRendererCommandAccess::OwnerIndependent
                }
            }
            CdpMethodDomain::Page => match PageWireAction::parse(action) {
                PageWireAction::MainThread => CdpRendererCommandAccess::MainThread,
                PageWireAction::Crash => CdpRendererCommandAccess::Io,
                PageWireAction::Other => CdpRendererCommandAccess::OwnerIndependent,
            },
            CdpMethodDomain::Accessibility
            | CdpMethodDomain::Console
            | CdpMethodDomain::Css
            | CdpMethodDomain::Dom
            | CdpMethodDomain::DomSnapshot
            | CdpMethodDomain::HeapProfiler
            | CdpMethodDomain::Profiler => CdpRendererCommandAccess::MainThread,
            CdpMethodDomain::Other => CdpRendererCommandAccess::OwnerIndependent,
        };
        Self {
            renderer_access,
            renderer_replacement: if runtime_action.is_some_and(|action| {
                matches!(
                    action,
                    RuntimeWireAction::AwaitPromise
                        | RuntimeWireAction::CallFunctionOn
                        | RuntimeWireAction::Evaluate
                        | RuntimeWireAction::RunScript
                        | RuntimeWireAction::TerminateExecution
                )
            }) {
                CdpRendererCommandReplacement::Terminate
            } else {
                CdpRendererCommandReplacement::Replay
            },
            renderer_replay_dispatch: if runtime_action == Some(RuntimeWireAction::AddBinding) {
                CdpRendererCommandReplayDispatch::ResolveRuntimeContext
            } else {
                CdpRendererCommandReplayDispatch::Direct
            },
            executes_page_javascript: runtime_action
                .is_some_and(RuntimeWireAction::executes_page_javascript)
                || debugger_action.is_some_and(DebuggerWireAction::executes_page_javascript),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: impl Into<String>) -> ParsedCdpCommand {
        ParsedCdpCommand::parse_str(raw).expect("test command must be valid CDP JSON")
    }

    #[test]
    fn parsed_cdp_command_keeps_json_and_metadata() {
        let command =
            parse(r#"{"id":7,"method":"Runtime.evaluate","params":{"x":1},"sessionId":"s1"}"#);

        assert!(command.json().contains("Runtime.evaluate"));
        assert_eq!(command.method(), "Runtime.evaluate");
        assert_eq!(command.session_id(), Some("s1"));
        assert_eq!(command.command_output_session_id(), Some("s1"));
        assert!(command.runtime_command_executes_page_javascript());
        assert_eq!(
            command.renderer_access(),
            CdpRendererCommandAccess::MainThread
        );
    }

    #[test]
    fn parsed_cdp_command_keeps_navigation_control_commands_unblocked() {
        let command = parse(
            r#"{"id":9,"method":"Fetch.continueResponse","params":{"requestId":"r"},"sessionId":"s1"}"#,
        );

        assert_eq!(command.method(), "Fetch.continueResponse");
        assert_eq!(command.session_id(), Some("s1"));
        assert_eq!(
            command.renderer_access(),
            CdpRendererCommandAccess::OwnerIndependent
        );
    }

    #[test]
    fn renderer_inspector_control_commands_wait_for_navigation_resume() {
        for method in [
            "Runtime.getIsolateId",
            "Console.clearMessages",
            "Profiler.enable",
            "HeapProfiler.enable",
        ] {
            let command = parse(format!(r#"{{"id":11,"method":"{method}"}}"#));
            assert!(
                command.renderer_access() == CdpRendererCommandAccess::MainThread,
                "{method} must not bind to the suspended attachment"
            );
        }
    }

    #[test]
    fn renderer_navigation_suspension_matches_chromium_io_route_exceptions() {
        for method in [
            "Debugger.getPossibleBreakpoints",
            "Debugger.getScriptSource",
            "Debugger.getStackTrace",
            "Debugger.pause",
            "Debugger.removeBreakpoint",
            "Debugger.resume",
            "Debugger.setBreakpoint",
            "Debugger.setBreakpointByUrl",
            "Debugger.setBreakpointsActive",
            "Emulation.setScriptExecutionDisabled",
            "Page.crash",
            "Performance.getMetrics",
            "Runtime.terminateExecution",
        ] {
            let command = parse(format!(r#"{{"id":12,"method":"{method}"}}"#));
            assert!(
                command.renderer_access() == CdpRendererCommandAccess::Io,
                "{method} must remain dispatchable while the renderer attachment is suspended"
            );
        }

        for method in [
            "Debugger.continueToLocation",
            "Debugger.enable",
            "Debugger.restartFrame",
            "Debugger.stepInto",
            "Debugger.stepOut",
            "Debugger.stepOver",
            "Performance.enable",
            "Runtime.getIsolateId",
        ] {
            let command = parse(format!(r#"{{"id":13,"method":"{method}"}}"#));
            assert!(
                command.renderer_access() == CdpRendererCommandAccess::MainThread,
                "{method} must bind to the replacement renderer attachment"
            );
        }
    }

    #[test]
    fn debugger_execution_controls_admit_exact_command_output_barriers() {
        for method in [
            "Debugger.continueToLocation",
            "Debugger.restartFrame",
            "Debugger.resume",
            "Debugger.stepInto",
            "Debugger.stepOut",
            "Debugger.stepOver",
        ] {
            let command = parse(format!(r#"{{"id":17,"method":"{method}"}}"#));
            assert!(
                command.runtime_command_executes_page_javascript(),
                "{method} must hold its causal resumed/paused output until its response"
            );
        }
        assert!(
            !parse(r#"{"id":18,"method":"Debugger.pause"}"#)
                .runtime_command_executes_page_javascript()
        );
    }

    #[test]
    fn renderer_replacement_traits_are_computed_at_the_wire_boundary() {
        for method in [
            "Runtime.awaitPromise",
            "Runtime.callFunctionOn",
            "Runtime.evaluate",
            "Runtime.runScript",
            "Runtime.terminateExecution",
        ] {
            let command = parse(format!(r#"{{"id":14,"method":"{method}"}}"#));
            assert_eq!(
                command.renderer_replacement(),
                CdpRendererCommandReplacement::Terminate,
                "{method} must remain bound to the replaced JavaScript context"
            );
        }

        for method in [
            "Runtime.enable",
            "Runtime.getProperties",
            "Debugger.enable",
            "Console.clearMessages",
            "FutureDomain.futureControlCommand",
        ] {
            let command = parse(format!(r#"{{"id":15,"method":"{method}"}}"#));
            assert_eq!(
                command.renderer_replacement(),
                CdpRendererCommandReplacement::Replay,
                "{method} must follow the replacement renderer attachment"
            );
        }

        let add_binding = parse(r#"{"id":16,"method":"Runtime.addBinding"}"#);
        assert_eq!(
            add_binding.renderer_replay_dispatch(),
            CdpRendererCommandReplayDispatch::ResolveRuntimeContext
        );
    }

    #[test]
    fn page_resource_search_waits_for_document_navigation() {
        let command = parse(
            r#"{"id":10,"method":"Page.searchInResource","params":{"frameId":"F","url":"https://example.test/","query":"needle"},"sessionId":"s1"}"#,
        );

        assert_eq!(
            command.renderer_access(),
            CdpRendererCommandAccess::MainThread
        );
    }

    #[test]
    fn parsed_command_rejects_invalid_json_instead_of_storing_parse_failure() {
        let error = ParsedCdpCommand::parse_str("{")
            .err()
            .expect("malformed JSON must not produce a parsed command");
        assert_eq!(error.kind(), CdpCommandParseErrorKind::ParseError);
        assert_eq!(error.command_id(), None);
        assert!(ParsedCdpCommand::parse_value(Value::Null).is_err());
    }

    #[test]
    fn request_known_fields_are_typed_at_deserialization() {
        for raw in [
            r#"{"method":"Runtime.enable"}"#,
            r#"{"id":"7","method":"Runtime.enable"}"#,
            r#"{"id":null,"method":"Runtime.enable"}"#,
            r#"{"id":7}"#,
            r#"{"id":7,"method":7}"#,
            r#"{"id":7,"method":"Runtime.enable","sessionId":7}"#,
            r#"{"id":7,"method":"Runtime.enable","params":7}"#,
        ] {
            assert!(
                ParsedCdpCommand::parse_str(raw).is_err(),
                "invalid known CDP field must be rejected: {raw}"
            );
        }

        let error =
            ParsedCdpCommand::parse_str(r#"{"id":7,"method":"Runtime.enable","params":[]}"#)
                .err()
                .expect("non-object params must fail typed command construction");
        assert_eq!(error.kind(), CdpCommandParseErrorKind::InvalidRequest);
        assert_eq!(error.command_id(), Some(7));
        assert_eq!(error.response_code(), -32600);
    }

    #[test]
    fn null_session_id_uses_absent_optional_route_semantics() {
        let command = parse(r#"{"id":14,"method":"Runtime.enable","sessionId":null}"#)
            .rewrite_frontend_route(91, None)
            .expect("null optional session must normalize during route rewrite");

        assert_eq!(command.session_id(), None);
        assert_eq!(
            serde_json::from_str::<Value>(command.json()).expect("rewritten dispatch JSON"),
            serde_json::json!({
                "id": 91,
                "method": "Runtime.enable",
            })
        );
    }

    #[test]
    fn parsed_command_from_value_recomputes_typed_command_traits() {
        let command = ParsedCdpCommand::parse_value(serde_json::json!({
            "id": 14,
            "method": "Runtime.terminateExecution",
            "sessionId": "s2",
        }))
        .expect("rewritten frontend command must remain valid");

        assert_eq!(command.request().id(), 14);
        assert_eq!(command.session_id(), Some("s2"));
        assert_eq!(command.renderer_access(), CdpRendererCommandAccess::Io);
        assert!(!command.runtime_command_executes_page_javascript());
    }

    #[test]
    fn frontend_rewrite_preserves_params_and_unknown_top_level_fields() {
        let command = parse(
            r#"{"id":14,"method":"Runtime.getIsolateId","params":{"probe":true},"sessionId":"client-session","futureExtension":{"enabled":true}}"#,
        )
        .rewrite_frontend_route(91, Some("dispatch-session"))
        .expect("frontend route rewrite must serialize");

        assert_eq!(command.request().id(), 91);
        assert_eq!(command.session_id(), Some("dispatch-session"));
        assert_eq!(
            serde_json::from_str::<Value>(command.json()).expect("rewritten dispatch JSON"),
            serde_json::json!({
                "id": 91,
                "method": "Runtime.getIsolateId",
                "params": {"probe": true},
                "sessionId": "dispatch-session",
                "futureExtension": {"enabled": true},
            })
        );
    }

    #[test]
    fn target_session_reference_rewrite_preserves_other_command_fields() {
        let command = parse(
            r#"{"id":14,"method":"Target.detachFromTarget","params":{"targetId":"TID-1"},"futureExtension":true}"#,
        )
        .rewrite_target_session_reference("SID-owned")
        .expect("target session reference rewrite must serialize")
        .rewrite_frontend_route(91, Some("SID-browser"))
        .expect("frontend route rewrite must serialize");

        assert_eq!(
            serde_json::from_str::<Value>(command.json()).expect("rewritten dispatch JSON"),
            serde_json::json!({
                "id": 91,
                "method": "Target.detachFromTarget",
                "params": {
                    "targetId": "TID-1",
                    "sessionId": "SID-owned",
                },
                "sessionId": "SID-browser",
                "futureExtension": true,
            })
        );
    }

    #[test]
    fn absent_and_null_params_share_chromium_empty_params_semantics() {
        for raw in [
            r#"{"id":14,"method":"Runtime.getIsolateId"}"#,
            r#"{"id":14,"method":"Runtime.getIsolateId","params":null}"#,
        ] {
            let command = parse(raw)
                .rewrite_frontend_route(91, None)
                .expect("frontend route rewrite must serialize");

            assert!(command.request().params().is_none());
            assert_eq!(
                serde_json::from_str::<Value>(command.json()).expect("rewritten dispatch JSON"),
                serde_json::json!({
                    "id": 91,
                    "method": "Runtime.getIsolateId",
                })
            );
        }
    }

    #[test]
    fn parsed_command_from_serializable_builds_valid_dispatch_json() {
        let command = ParsedCdpCommand::from_serializable(serde_json::json!({
            "id": 15,
            "method": "Runtime.addBinding",
            "params": { "name": "exposed" },
        }))
        .expect("serializable test command must produce valid CDP JSON");

        assert_eq!(command.request().id(), 15);
        assert_eq!(command.method(), "Runtime.addBinding");
        assert_eq!(
            serde_json::from_str::<Value>(command.json()).expect("dispatch JSON"),
            serde_json::json!({
                "id": 15,
                "method": "Runtime.addBinding",
                "params": { "name": "exposed" },
            })
        );
    }
}
