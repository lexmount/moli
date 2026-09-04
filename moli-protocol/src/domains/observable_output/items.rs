use std::collections::HashMap;

use crate::devtools_runtime::{
    DevToolsScriptException, DevToolsStackCallFrame, DevToolsStackTrace, DevToolsTargetId,
    LogEntryEvent, RuntimeConsoleEvent, ScriptExceptionEvent,
};
use serde_json::{Value, json};

use crate::conn::{BackgroundProtocolEvent, DocumentId};
use crate::domains::log_output_state::TargetNetworkLogEntry;
use moli_core::page::{InspectorIssueSnapshot, SubresourceNetworkRequestHandle};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ObservableOutputItem {
    AuditsIssueAdded {
        issue: InspectorIssueSnapshot,
        frame_id: String,
        loader_id: String,
    },
    ConsoleMessageAdded {
        source: String,
        level: String,
        text: String,
        url: String,
    },
    LogEntryAdded {
        source: String,
        level: String,
        text: String,
        url: String,
        timestamp_micros: Option<u64>,
        network_request_handle: Option<SubresourceNetworkRequestHandle>,
        network_request_id: Option<String>,
    },
    RuntimeConsoleApiCalled {
        console_type: String,
        text: String,
        args: Vec<Value>,
        stack: Option<String>,
        execution_context_id: i64,
    },
    RuntimeExceptionThrown {
        text: String,
        url: String,
        execution_context_id: i64,
        exception_index: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ObservableRuntimePreparedItem {
    Output(ObservableOutputItem),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableRuntimePreparedItems {
    source_identity: Option<ObservableRuntimePreparedSourceIdentity>,
    items: Vec<ObservableRuntimePreparedItem>,
    cursor: ObservableRuntimeEmissionCursor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableRuntimeEmissionCursor {
    context_console_counts: HashMap<i64, usize>,
    exception_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::domains::observable_output) struct ObservableRuntimePreparedSourceIdentity {
    url: String,
    document_id: DocumentId,
}

impl ObservableRuntimePreparedItems {
    pub(in crate::domains::observable_output) fn from_runtime_source_items(
        url: String,
        document_id: DocumentId,
        items: Vec<ObservableRuntimePreparedItem>,
        context_console_counts: HashMap<i64, usize>,
        exception_end: usize,
    ) -> Self {
        Self {
            source_identity: Some(ObservableRuntimePreparedSourceIdentity { url, document_id }),
            items,
            cursor: ObservableRuntimeEmissionCursor::new(context_console_counts, exception_end),
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(
        items: Vec<ObservableOutputItem>,
        context_console_counts: HashMap<i64, usize>,
        exception_end: usize,
    ) -> Self {
        Self {
            source_identity: None,
            items: items
                .into_iter()
                .map(ObservableRuntimePreparedItem::Output)
                .collect(),
            cursor: ObservableRuntimeEmissionCursor::new(context_console_counts, exception_end),
        }
    }

    pub(in crate::domains::observable_output) fn matches_source_identity(
        &self,
        url: &str,
        document_id: DocumentId,
    ) -> bool {
        self.source_identity
            .as_ref()
            .is_none_or(|identity| identity.url == url && identity.document_id == document_id)
    }

    pub(super) fn into_emission_parts(
        self,
    ) -> (
        Vec<ObservableRuntimePreparedItem>,
        ObservableRuntimeEmissionCursor,
    ) {
        (self.items, self.cursor)
    }

    #[cfg(test)]
    pub(super) fn into_output_emission_parts_for_test(
        self,
    ) -> (Vec<ObservableOutputItem>, ObservableRuntimeEmissionCursor) {
        let items = self
            .items
            .into_iter()
            .map(|item| match item {
                ObservableRuntimePreparedItem::Output(item) => item,
            })
            .collect();
        (items, self.cursor)
    }
}

impl ObservableRuntimePreparedItem {
    pub(super) fn output(item: ObservableOutputItem) -> Self {
        Self::Output(item)
    }
}

impl ObservableRuntimeEmissionCursor {
    fn new(context_console_counts: HashMap<i64, usize>, exception_end: usize) -> Self {
        Self {
            context_console_counts,
            exception_end,
        }
    }

    pub(super) fn into_parts(self) -> (HashMap<i64, usize>, usize) {
        (self.context_console_counts, self.exception_end)
    }

    #[cfg(test)]
    pub(super) fn context_console_counts(&self) -> &HashMap<i64, usize> {
        &self.context_console_counts
    }

    #[cfg(test)]
    pub(super) fn exception_end(&self) -> usize {
        self.exception_end
    }
}

pub(super) fn runtime_console_api_called_item(
    message: &moli_core::page::RuntimeConsoleMessageSnapshot,
) -> ObservableOutputItem {
    let (console_type, text) = runtime_console_message_type_and_text(&message.message);
    ObservableOutputItem::RuntimeConsoleApiCalled {
        console_type: console_type.to_owned(),
        text: text.to_owned(),
        args: message.args.clone(),
        stack: message.stack.clone(),
        execution_context_id: message.execution_context_id,
    }
}

pub(super) fn runtime_exception_thrown_item(
    text: String,
    url: &str,
    execution_context_id: i64,
    exception_index: usize,
) -> ObservableOutputItem {
    ObservableOutputItem::RuntimeExceptionThrown {
        text,
        url: url.to_owned(),
        execution_context_id,
        exception_index,
    }
}

impl ObservableOutputItem {
    pub(super) fn materialize_network_request_id(
        &mut self,
        conn: &mut crate::conn::CdpConnection,
        session_id: Option<&str>,
    ) {
        let Self::LogEntryAdded {
            network_request_handle: Some(handle),
            network_request_id,
            ..
        } = self
        else {
            return;
        };
        if network_request_id.is_none() {
            *network_request_id = conn
                .network_request_id_for_subresource_handle_for_session_owner(session_id, *handle);
        }
    }

    pub(super) fn duplicates_existing_background_event(
        &self,
        out: &[BackgroundProtocolEvent],
    ) -> bool {
        if let Self::ConsoleMessageAdded {
            source,
            level,
            text,
            ..
        } = self
        {
            return out
                .iter()
                .any(|event| event.matches_console_message_added(source, level, text));
        }
        self.duplicates_existing_protocol_message(
            out.iter()
                .filter_map(BackgroundProtocolEvent::protocol_message),
        )
    }

    fn duplicates_existing_protocol_message<'a>(
        &self,
        messages: impl IntoIterator<Item = &'a Value>,
    ) -> bool {
        match self {
            Self::AuditsIssueAdded { .. } => false,
            Self::ConsoleMessageAdded {
                source,
                level,
                text,
                ..
            } => messages.into_iter().any(|message| {
                message["method"] == json!("Console.messageAdded")
                    && message["params"]["message"]["source"] == json!(source)
                    && message["params"]["message"]["level"] == json!(level)
                    && message["params"]["message"]["text"] == json!(text)
            }),
            Self::RuntimeConsoleApiCalled {
                console_type,
                text,
                execution_context_id,
                ..
            } => messages.into_iter().any(|message| {
                message["method"] == json!("Runtime.consoleAPICalled")
                    && message["params"]["type"] == json!(console_type)
                    && message["params"]["executionContextId"] == json!(execution_context_id)
                    && message["params"]["args"]
                        .as_array()
                        .is_some_and(|args| args.iter().any(|arg| arg["value"] == json!(text)))
            }),
            Self::LogEntryAdded { .. } | Self::RuntimeExceptionThrown { .. } => false,
        }
    }

    pub(super) fn emit_background_event(
        &self,
        session_id: Option<&str>,
        target_id: Option<&DevToolsTargetId>,
        timestamp: f64,
    ) -> BackgroundProtocolEvent {
        match self {
            Self::AuditsIssueAdded {
                issue,
                frame_id,
                loader_id,
            } => {
                BackgroundProtocolEvent::audits_issue_added(session_id, issue, frame_id, loader_id)
            }
            Self::ConsoleMessageAdded {
                source,
                level,
                text,
                url,
            } => console_message_added_background_event(session_id, source, level, text, url),
            Self::LogEntryAdded {
                source,
                level,
                text,
                url,
                timestamp_micros,
                network_request_id,
                ..
            } => BackgroundProtocolEvent::log_entry_added(
                session_id,
                source,
                level,
                text,
                url,
                timestamp_micros
                    .map(|timestamp| timestamp as f64 / 1_000.0)
                    .unwrap_or(timestamp),
                network_request_id.as_deref(),
            ),
            Self::RuntimeConsoleApiCalled {
                console_type,
                text,
                args,
                stack,
                execution_context_id,
            } => runtime_console_api_called_background_event(
                session_id,
                target_id,
                console_type,
                text,
                args,
                stack.as_deref(),
                *execution_context_id,
                timestamp,
            ),
            Self::RuntimeExceptionThrown {
                text,
                url,
                execution_context_id,
                exception_index,
            } => runtime_exception_thrown_background_event(
                session_id,
                target_id,
                text,
                url,
                *execution_context_id,
                *exception_index,
                timestamp,
                None,
                None,
            ),
        }
    }
}

pub(in crate::domains) fn console_message_level_and_text(message: &str) -> (&'static str, &str) {
    let Some((level, text)) = message.split_once(": ") else {
        return ("log", message);
    };
    match level {
        "debug" => ("debug", text),
        "warn" => ("warning", text),
        "error" => ("error", text),
        "info" => ("info", text),
        "log" => ("log", text),
        _ => ("log", message),
    }
}

pub(in crate::domains) fn log_lifecycle_error_level_and_text(error: &str) -> (&'static str, &str) {
    ("error", error)
}

pub(super) fn console_domain_items<'a>(
    url: &str,
    console_messages: impl IntoIterator<Item = &'a str>,
    lifecycle_errors: impl IntoIterator<Item = &'a str>,
) -> Vec<ObservableOutputItem> {
    let mut items = Vec::new();
    for message in console_messages {
        let (level, text) = console_message_level_and_text(message);
        items.push(ObservableOutputItem::ConsoleMessageAdded {
            source: "console-api".to_owned(),
            level: level.to_owned(),
            text: text.to_owned(),
            url: url.to_owned(),
        });
    }
    for error in lifecycle_errors {
        items.push(ObservableOutputItem::ConsoleMessageAdded {
            source: "javascript".to_owned(),
            level: "error".to_owned(),
            text: error.to_owned(),
            url: url.to_owned(),
        });
    }
    items
}

pub(super) fn log_domain_items<'a>(
    url: &str,
    _console_messages: impl IntoIterator<Item = &'a str>,
    lifecycle_errors: impl IntoIterator<Item = &'a str>,
    network_entries: impl IntoIterator<Item = &'a TargetNetworkLogEntry>,
) -> Vec<ObservableOutputItem> {
    let mut items = Vec::new();
    for error in lifecycle_errors {
        let (level, text) = log_lifecycle_error_level_and_text(error);
        items.push(ObservableOutputItem::LogEntryAdded {
            source: "javascript".to_owned(),
            level: level.to_owned(),
            text: text.to_owned(),
            url: url.to_owned(),
            timestamp_micros: None,
            network_request_handle: None,
            network_request_id: None,
        });
    }
    for entry in network_entries {
        items.push(ObservableOutputItem::LogEntryAdded {
            source: "network".to_owned(),
            level: "error".to_owned(),
            text: entry.text().to_owned(),
            url: entry.url().to_owned(),
            timestamp_micros: Some(entry.timestamp_micros()),
            network_request_handle: entry.request_handle(),
            network_request_id: None,
        });
    }
    items
}

pub(in crate::domains) fn runtime_console_message_type_and_text(
    message: &str,
) -> (&'static str, &str) {
    let Some((level, text)) = message.split_once(": ") else {
        return ("log", message);
    };
    match level {
        "debug" => ("debug", text),
        "warn" => ("warning", text),
        "error" => ("error", text),
        "info" => ("info", text),
        "log" => ("log", text),
        _ => ("log", message),
    }
}

pub(in crate::domains) fn console_message_added_background_event(
    session_id: Option<&str>,
    source: &str,
    level: &str,
    text: &str,
    url: &str,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::console_message_added(session_id, source, level, text, url)
}

pub(in crate::domains) fn log_entry_event(
    source: &str,
    level: &str,
    text: &str,
    url: &str,
    timestamp: f64,
    network_request_id: Option<&str>,
) -> LogEntryEvent {
    LogEntryEvent {
        target_id: None,
        source: source.to_owned(),
        level: level.to_owned(),
        text: text.to_owned(),
        url: Some(url.to_owned()),
        timestamp: Some(timestamp),
        network_request_id: network_request_id.map(str::to_owned),
        args: Vec::new(),
    }
}

fn script_exception_event(
    target_id: Option<&DevToolsTargetId>,
    text: &str,
    url: &str,
    execution_context_id: i64,
    exception_index: usize,
    timestamp: f64,
    line_number: Option<u64>,
    column_number: Option<u64>,
) -> ScriptExceptionEvent {
    ScriptExceptionEvent {
        target_id: target_id.cloned(),
        url: Some(url.to_owned()),
        execution_context_id: Some(execution_context_id),
        exception_index: Some(exception_index),
        timestamp: Some(timestamp),
        exception: Box::new(DevToolsScriptException {
            exception_id: Some((exception_index + 1) as u64),
            script_id: None,
            text: text.to_owned(),
            value: None,
            realm: None,
            line_number,
            column_number,
            stack_trace: None,
        }),
    }
}

pub(in crate::domains) fn runtime_console_api_called_background_event(
    session_id: Option<&str>,
    target_id: Option<&DevToolsTargetId>,
    console_type: &str,
    text: &str,
    args: &[Value],
    stack: Option<&str>,
    execution_context_id: i64,
    timestamp: f64,
) -> BackgroundProtocolEvent {
    let args = runtime_console_args(text, args);
    let stack_trace = console_stack_trace(stack);
    let event = RuntimeConsoleEvent {
        target_id: target_id.cloned(),
        console_type: console_type.to_owned(),
        text: text.to_owned(),
        args,
        stack: stack.map(str::to_owned),
        stack_trace,
        execution_context_id: Some(execution_context_id),
        timestamp: Some(timestamp),
    };
    BackgroundProtocolEvent::runtime_console_api_called(session_id, event)
}

pub(in crate::domains) fn runtime_exception_thrown_background_event(
    session_id: Option<&str>,
    target_id: Option<&DevToolsTargetId>,
    text: &str,
    url: &str,
    execution_context_id: i64,
    exception_index: usize,
    timestamp: f64,
    line_number: Option<u64>,
    column_number: Option<u64>,
) -> BackgroundProtocolEvent {
    let event = script_exception_event(
        target_id,
        text,
        url,
        execution_context_id,
        exception_index,
        timestamp,
        line_number,
        column_number,
    );
    BackgroundProtocolEvent::runtime_exception_thrown(session_id, event)
}

fn runtime_console_args(text: &str, args: &[Value]) -> Vec<Value> {
    if args.is_empty() {
        vec![json!({
            "type": "string",
            "value": text,
        })]
    } else {
        args.to_vec()
    }
}

fn console_stack_trace(stack: Option<&str>) -> Option<DevToolsStackTrace> {
    let call_frames = stack?
        .lines()
        .filter_map(console_stack_call_frame)
        .collect::<Vec<_>>();
    if call_frames.is_empty() {
        None
    } else {
        Some(DevToolsStackTrace { call_frames })
    }
}

fn console_stack_call_frame(line: &str) -> Option<DevToolsStackCallFrame> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("Error") {
        return None;
    }
    let line = line.strip_prefix("at ").unwrap_or(line).trim();

    let (function_name, location) = if let Some(without_suffix) = line.strip_suffix(')') {
        if let Some((function_name, location)) = without_suffix.rsplit_once(" (") {
            (function_name.trim(), location.trim())
        } else {
            ("", without_suffix.trim())
        }
    } else {
        ("", line)
    };
    let (url, line_number, column_number) = parse_stack_location(location)?;
    Some(DevToolsStackCallFrame {
        function_name: function_name.to_owned(),
        script_id: None,
        url: url.to_owned(),
        line_number,
        column_number,
    })
}

fn parse_stack_location(location: &str) -> Option<(&str, u64, u64)> {
    let (location, column_number) = location.rsplit_once(':')?;
    let (url, line_number) = location.rsplit_once(':')?;
    let line_number = line_number.parse::<u64>().ok()?.saturating_sub(1);
    let column_number = column_number.parse::<u64>().ok()?.saturating_sub(1);
    Some((url, line_number, column_number))
}

#[cfg(test)]
mod tests {
    use crate::conn::BackgroundProtocolEvent;
    use crate::devtools_runtime::AutomationEvent;
    use serde_json::json;

    use super::ObservableOutputItem;

    #[test]
    fn console_domain_items_project_console_and_lifecycle_backlog() {
        let items =
            super::console_domain_items("http://example.test/app", ["warn: heads up"], ["boom"]);

        assert!(matches!(
            items.as_slice(),
            [
                ObservableOutputItem::ConsoleMessageAdded {
                    source,
                    level,
                    text,
                    url,
                },
                ObservableOutputItem::ConsoleMessageAdded {
                    source: error_source,
                    level: error_level,
                    text: error_text,
                    url: error_url,
                },
            ]
            if source == "console-api"
                && level == "warning"
                && text == "heads up"
                && url == "http://example.test/app"
                && error_source == "javascript"
                && error_level == "error"
                && error_text == "boom"
                && error_url == "http://example.test/app"
        ));
    }

    #[test]
    fn log_domain_items_project_lifecycle_backlog_without_console_api_messages() {
        let items = super::log_domain_items(
            "http://example.test/app",
            ["debug: trace"],
            ["boom"],
            std::iter::empty(),
        );

        assert!(matches!(
            items.as_slice(),
            [ObservableOutputItem::LogEntryAdded {
                source: error_source,
                level: error_level,
                text: error_text,
                url: error_url,
                ..
            }]
            if error_source == "javascript"
                && error_level == "error"
                && error_text == "boom"
                && error_url == "http://example.test/app"
        ));
    }

    #[test]
    fn console_message_added_background_event_keeps_sidecar_cdp_params_free() {
        let event = super::console_message_added_background_event(
            Some("SID-1"),
            "console-api",
            "warning",
            "typed console domain",
            "http://example.test/app",
        );
        assert!(
            event.protocol_message().is_none(),
            "Console.messageAdded should stay typed until wire projection"
        );
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["method"], json!("Console.messageAdded"));
        assert_eq!(message["params"]["message"]["source"], json!("console-api"));
        assert_eq!(message["params"]["message"]["level"], json!("warning"));
        assert_eq!(
            message["params"]["message"]["text"],
            json!("typed console domain")
        );
        assert_eq!(
            message["params"]["message"]["url"],
            json!("http://example.test/app")
        );
        let Some(AutomationEvent::RuntimeConsoleApiCalled(event)) = automation_event else {
            panic!("expected RuntimeConsoleApiCalled sidecar");
        };
        assert_eq!(event.console_type, "warning");
        assert_eq!(event.text, "typed console domain");
        assert_eq!(
            event.args,
            vec![json!({"type": "string", "value": "typed console domain"})]
        );
    }

    #[test]
    fn log_entry_added_background_event_keeps_sidecar_cdp_params_free() {
        let event = BackgroundProtocolEvent::log_entry_added(
            Some("SID-1"),
            "javascript",
            "error",
            "typed log",
            "http://example.test/app",
            12.5,
            None,
        );
        assert!(
            event.protocol_message().is_none(),
            "Log.entryAdded should stay typed until wire projection"
        );
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["method"], json!("Log.entryAdded"));
        assert_eq!(message["params"]["entry"]["source"], json!("javascript"));
        assert_eq!(message["params"]["entry"]["level"], json!("error"));
        assert_eq!(message["params"]["entry"]["text"], json!("typed log"));
        assert_eq!(
            message["params"]["entry"]["url"],
            json!("http://example.test/app")
        );
        let Some(AutomationEvent::LogEntryAdded(event)) = automation_event else {
            panic!("expected LogEntryAdded sidecar");
        };
        assert_eq!(event.source, "javascript");
        assert_eq!(event.level, "error");
        assert_eq!(event.text, "typed log");
    }

    #[test]
    fn runtime_console_stack_trace_parses_function_and_direct_frames() {
        let stack = "\
Error: Console
    at ErrorFactory (http://127.0.0.1:8123/app.js:8:9)
    at inner (http://127.0.0.1:8123/app.js:10:15)
    at http://127.0.0.1:8123/app.js:12:3";

        let trace = super::console_stack_trace(Some(stack)).expect("stack trace should parse");
        let call_frames = trace.call_frames;
        assert_eq!(call_frames.len(), 3);
        assert_eq!(call_frames[0].function_name, "ErrorFactory");
        assert_eq!(call_frames[0].url, "http://127.0.0.1:8123/app.js");
        assert_eq!(call_frames[0].line_number, 7);
        assert_eq!(call_frames[0].column_number, 8);
        assert_eq!(call_frames[1].function_name, "inner");
        assert_eq!(call_frames[1].url, "http://127.0.0.1:8123/app.js");
        assert_eq!(call_frames[1].line_number, 9);
        assert_eq!(call_frames[1].column_number, 14);
        assert_eq!(call_frames[2].function_name, "");
        assert_eq!(call_frames[2].url, "http://127.0.0.1:8123/app.js");
        assert_eq!(call_frames[2].line_number, 11);
        assert_eq!(call_frames[2].column_number, 2);
    }

    #[test]
    fn runtime_console_background_event_preserves_sidecar_and_message() {
        let target_id = crate::devtools_runtime::DevToolsTargetId::from("TID-1");
        let event = super::runtime_console_api_called_background_event(
            Some("SID-1"),
            Some(&target_id),
            "warning",
            "typed console",
            &[json!({"type": "string", "value": "typed console"})],
            Some("    at run (http://example.test/app.js:2:3)"),
            7,
            12.5,
        );
        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Runtime.consoleAPICalled"));
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["method"], json!("Runtime.consoleAPICalled"));
        assert_eq!(message["params"]["type"], json!("warning"));
        assert_eq!(
            message["params"]["args"][0]["value"],
            json!("typed console")
        );
        assert_eq!(message["params"]["executionContextId"], json!(7));
        assert_eq!(message["params"]["timestamp"], json!(12.5));
        assert_eq!(
            message["params"]["stackTrace"]["callFrames"][0]["functionName"],
            json!("run")
        );
        let Some(AutomationEvent::RuntimeConsoleApiCalled(event)) = automation_event else {
            panic!("expected RuntimeConsoleApiCalled sidecar");
        };
        assert_eq!(event.console_type, "warning");
        assert_eq!(event.text, "typed console");
        assert_eq!(event.target_id.as_ref(), Some(&target_id));
        assert_eq!(event.execution_context_id, Some(7));
        let stack_trace = event.stack_trace.expect("expected typed stack trace");
        assert_eq!(stack_trace.call_frames[0].function_name, "run");
    }

    #[test]
    fn runtime_exception_background_event_preserves_sidecar_and_message() {
        let target_id = crate::devtools_runtime::DevToolsTargetId::from("TID-1");
        let event = super::runtime_exception_thrown_background_event(
            Some("SID-1"),
            Some(&target_id),
            "typed exception",
            "http://example.test/app.js",
            7,
            2,
            13.5,
            Some(4),
            Some(8),
        );
        assert!(event.protocol_message().is_none());
        assert_eq!(event.protocol_method(), Some("Runtime.exceptionThrown"));
        let (message, automation_event) = event.into_parts();

        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["method"], json!("Runtime.exceptionThrown"));
        assert_eq!(message["params"]["timestamp"], json!(13.5));
        assert_eq!(
            message["params"]["exceptionDetails"]["text"],
            json!("typed exception")
        );
        assert_eq!(
            message["params"]["exceptionDetails"]["exceptionId"],
            json!(3)
        );
        assert_eq!(
            message["params"]["exceptionDetails"]["url"],
            json!("http://example.test/app.js")
        );
        assert_eq!(
            message["params"]["exceptionDetails"]["executionContextId"],
            json!(7)
        );
        assert_eq!(
            message["params"]["exceptionDetails"]["lineNumber"],
            json!(4)
        );
        assert_eq!(
            message["params"]["exceptionDetails"]["columnNumber"],
            json!(8)
        );
        let Some(AutomationEvent::ScriptException(event)) = automation_event else {
            panic!("expected ScriptException sidecar");
        };
        assert_eq!(event.target_id.as_ref(), Some(&target_id));
        assert_eq!(event.exception.text, "typed exception");
        assert_eq!(event.exception.line_number, Some(4));
        assert_eq!(event.exception.column_number, Some(8));
    }
}
