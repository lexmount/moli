use std::borrow::Cow;

use tracing::{debug, error, info, warn};

use super::util::{v8_string, v8str};

const LOG_SUMMARY_MAX_BYTES: usize = 4 * 1024;
const LOG_SOURCE_MAX_BYTES: usize = 2 * 1024;
const LOG_SOURCE_LINE_MAX_BYTES: usize = 1024;
const LOG_STACK_MAX_BYTES: usize = 16 * 1024;
const LOG_CALLBACK_CONTEXT_MAX_BYTES: usize = 4 * 1024;
const VALUE_DEBUG_SUMMARY_MAX_BYTES: usize = 160 * 4;
const VALUE_DEBUG_OBJECT_SUMMARY_MAX_BYTES: usize = 240 * 4;

pub(super) struct V8ExceptionReport {
    /// Script-origin taint supplied by V8, retained until web-facing reporting.
    pub(super) muted_errors: bool,
    pub(super) summary: String,
    pub(super) source: Option<String>,
    pub(super) line: Option<usize>,
    pub(super) column: Option<usize>,
    pub(super) source_line: Option<String>,
    pub(super) stack: Option<String>,
    pub(super) callback_context: Option<String>,
    pub(super) exception: Option<v8::Global<v8::Value>>,
}

#[derive(Clone, Copy)]
pub(super) enum CallbackExceptionLogLevel {
    Debug,
    Error,
}

impl V8ExceptionReport {
    pub(super) fn formatted_error(&self, callback_kind: &str, callback_name: &str) -> String {
        let location = match (&self.source, self.line, self.column) {
            (Some(source), Some(line), Some(column)) => format!("{source}:{line}:{column}"),
            (Some(source), Some(line), None) => format!("{source}:{line}"),
            (Some(source), None, _) => source.clone(),
            _ => "<unknown>".to_owned(),
        };
        format!(
            "{callback_kind} `{callback_name}` threw: {location}: {}",
            self.summary
        )
    }
}

fn fill_location_from_stack_trace(
    scope: &mut v8::PinScope<'_, '_>,
    report: &mut V8ExceptionReport,
    stack_trace: v8::Local<'_, v8::StackTrace>,
) {
    // Do not query stack-frame location metadata here.
    //
    // We previously tried to backfill `report.source` from the top frame via
    // `StackFrame::GetScriptNameOrSourceURL()`, but some promise-rejection
    // paths can surface a stack frame object that crashes inside V8 when that
    // metadata accessor runs. The Feishu article crash reproduced this as a
    // native SIGSEGV during unhandled promise rejection reporting.
    //
    // Stable location data should come from `v8::Message` when available. When
    // it is not, we still keep the textual stack trace, but we intentionally
    // skip best-effort frame metadata probing rather than crash the process
    // while trying to print an error.
    let _ = (scope, report, stack_trace);
}

fn local_value_to_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn exception_property_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
    key: &str,
) -> Option<String> {
    let object = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let key = v8_string(&scope, key)?;
    let value = object.get(&scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(&scope)
        .map(|value| value.to_rust_string_lossy(&scope))
}

fn exception_summary_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let message =
        exception_property_string(scope, exception, "message").filter(|value| !value.is_empty())?;
    if let Some(name) =
        exception_property_string(scope, exception, "name").filter(|value| !value.is_empty())
    {
        Some(format!("Uncaught {name}: {message}"))
    } else {
        Some(format!("Uncaught {message}"))
    }
}

fn exception_stack_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let object = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let stack_key = v8str(scope, "stack");
    let stack = object.get(scope, stack_key.into())?;
    local_value_to_string(scope, stack).filter(|stack| !stack.is_empty())
}

pub(super) fn build_event_handler_exception_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: Option<v8::Local<'s, v8::Value>>,
    message: Option<v8::Local<'s, v8::Message>>,
    stack_value: Option<v8::Local<'s, v8::Value>>,
) -> V8ExceptionReport {
    // Prefer v8::Message when it exists because it carries richer source text, but do not
    // assume it is present or complete for every exception path.
    let summary = message
        .map(|message| message.get(scope).to_rust_string_lossy(scope))
        .or_else(|| exception.and_then(|exception| local_value_to_string(scope, exception)))
        .unwrap_or_else(|| "Uncaught exception".to_owned());
    let summary = if summary.trim() == "Uncaught" {
        exception
            .and_then(|exception| exception_summary_override(scope, exception))
            .unwrap_or(summary)
    } else {
        summary
    };
    let source = message
        .and_then(|message| message.get_script_resource_name(scope))
        .and_then(|value| local_value_to_string(scope, value))
        .filter(|value| !value.is_empty());
    let line = message.and_then(|message| message.get_line_number(scope));
    let column = message.and_then(|message| {
        // V8 may report an "unknown" start column via a sentinel that would overflow when turned
        // into the user-facing 1-based column number. Treat that as "no precise column".
        message.get_start_column().checked_add(1)
    });
    let source_line = message
        .and_then(|message| message.get_source_line(scope))
        .map(|line| line.to_rust_string_lossy(scope))
        .filter(|line| !line.is_empty());
    let stack = stack_value
        .and_then(|value| local_value_to_string(scope, value))
        .filter(|stack| !stack.is_empty())
        .or_else(|| exception.and_then(|exception| exception_stack_property(scope, exception)));

    let mut report = V8ExceptionReport {
        muted_errors: message.is_some_and(|message| message.is_opaque()),
        summary,
        source,
        line,
        column,
        source_line,
        stack,
        callback_context: None,
        exception: exception.map(|exception| v8::Global::new(scope, exception)),
    };
    if let Some(stack_trace) = v8::StackTrace::current_stack_trace(scope, 32) {
        // Use the current stack only to fill holes; when v8::Message already gave us a
        // location, keep that as the primary source of truth.
        fill_location_from_stack_trace(scope, &mut report, stack_trace);
    }
    report
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiagnosticLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

struct DiagnosticLogFields<'a> {
    message: Cow<'a, str>,
    source: Cow<'a, str>,
    source_line: Cow<'a, str>,
    callback_context: Cow<'a, str>,
}

impl<'a> DiagnosticLogFields<'a> {
    fn from_report(report: &'a V8ExceptionReport) -> Self {
        Self {
            message: bounded_log_field(&report.summary, LOG_SUMMARY_MAX_BYTES),
            source: bounded_log_field(
                report.source.as_deref().unwrap_or("<unknown>"),
                LOG_SOURCE_MAX_BYTES,
            ),
            source_line: bounded_log_field(
                report.source_line.as_deref().unwrap_or(""),
                LOG_SOURCE_LINE_MAX_BYTES,
            ),
            callback_context: bounded_log_field(
                report.callback_context.as_deref().unwrap_or(""),
                LOG_CALLBACK_CONTEXT_MAX_BYTES,
            ),
        }
    }
}

fn v8_message_log_level(level_bits: i32) -> DiagnosticLogLevel {
    if level_bits == v8::MessageErrorLevel::WARNING.bits() {
        DiagnosticLogLevel::Warn
    } else if level_bits == v8::MessageErrorLevel::INFO.bits() {
        DiagnosticLogLevel::Info
    } else if level_bits == v8::MessageErrorLevel::DEBUG.bits()
        || level_bits == v8::MessageErrorLevel::LOG.bits()
    {
        DiagnosticLogLevel::Debug
    } else {
        DiagnosticLogLevel::Error
    }
}

fn log_stack(level: DiagnosticLogLevel, report: &V8ExceptionReport) {
    let Some(stack) = stack_log_field(report.stack.as_deref()) else {
        return;
    };
    match level {
        DiagnosticLogLevel::Debug => {
            debug!(backtrace = &*stack, "v8 exception stack backtrace:")
        }
        DiagnosticLogLevel::Info => info!(backtrace = &*stack, "v8 exception stack backtrace:"),
        DiagnosticLogLevel::Warn => warn!(backtrace = &*stack, "v8 exception stack backtrace:"),
        DiagnosticLogLevel::Error => error!(backtrace = &*stack, "v8 exception stack backtrace:"),
    }
}

pub(super) fn log_callback_exception(
    level: CallbackExceptionLogLevel,
    log_label: &str,
    callback_name: &str,
    report: &V8ExceptionReport,
) {
    let fields = DiagnosticLogFields::from_report(report);
    match level {
        CallbackExceptionLogLevel::Debug => {
            debug!(
                callback = callback_name,
                message = &*fields.message,
                source = &*fields.source,
                line = report.line.unwrap_or(0),
                column = report.column.unwrap_or(0),
                source_line = &*fields.source_line,
                callback_context = &*fields.callback_context,
                "{log_label}"
            );
            log_stack(DiagnosticLogLevel::Debug, report);
        }
        CallbackExceptionLogLevel::Error => {
            error!(
                callback = callback_name,
                message = &*fields.message,
                source = &*fields.source,
                line = report.line.unwrap_or(0),
                column = report.column.unwrap_or(0),
                source_line = &*fields.source_line,
                callback_context = &*fields.callback_context,
                "{log_label}"
            );
            log_stack(DiagnosticLogLevel::Error, report);
        }
    }
}

fn log_v8_message_report(level_bits: i32, report: &V8ExceptionReport) {
    let fields = DiagnosticLogFields::from_report(report);
    let line = report.line.unwrap_or(0);
    let column = report.column.unwrap_or(0);
    let level = v8_message_log_level(level_bits);

    macro_rules! log_message {
        ($macro:ident) => {
            $macro!(
                message = &*fields.message,
                source = &*fields.source,
                line = line,
                column = column,
                source_line = &*fields.source_line,
                "v8 message listener"
            )
        };
    }

    match level {
        DiagnosticLogLevel::Debug => log_message!(debug),
        DiagnosticLogLevel::Info => log_message!(info),
        DiagnosticLogLevel::Warn => log_message!(warn),
        DiagnosticLogLevel::Error => log_message!(error),
    }
    log_stack(level, report);
}

pub(super) fn log_uncaught_script_exception(report: &V8ExceptionReport) {
    let fields = DiagnosticLogFields::from_report(report);
    error!(
        message = &*fields.message,
        source = &*fields.source,
        line = report.line.unwrap_or(0),
        column = report.column.unwrap_or(0),
        source_line = &*fields.source_line,
        "uncaught script error"
    );
    log_stack(DiagnosticLogLevel::Error, report);
}

pub(super) fn uncaught_script_error(
    report: V8ExceptionReport,
    phase: &'static str,
) -> anyhow::Error {
    log_uncaught_script_exception(&report);
    let stack = report.stack.unwrap_or_else(|| "<no stack>".to_owned());
    anyhow::anyhow!("v8 failed to {phase} script: {}\n{stack}", report.summary)
}

pub(super) unsafe extern "C" fn v8_message_listener<'s>(
    message: v8::Local<'s, v8::Message>,
    exception: v8::Local<'s, v8::Value>,
) {
    let scope = std::pin::pin!(unsafe { v8::CallbackScope::new(message) });
    let scope = &mut scope.init();
    v8::scope!(let scope, scope);

    let report = build_event_handler_exception_report(scope, Some(exception), Some(message), None);
    log_v8_message_report(message.error_level(), &report);
}

pub(super) fn log_unhandled_promise_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: Option<v8::Local<'s, v8::Value>>,
) {
    let report = build_event_handler_exception_report(scope, reason, None, None);
    let fields = DiagnosticLogFields::from_report(&report);
    error!(
        message = &*fields.message,
        source = &*fields.source,
        line = report.line.unwrap_or(0),
        column = report.column.unwrap_or(0),
        source_line = &*fields.source_line,
        "unhandled promise rejection"
    );
    log_stack(DiagnosticLogLevel::Error, &report);
}

fn bounded_log_field(value: &str, max_bytes: usize) -> Cow<'_, str> {
    if value.len() <= max_bytes {
        return Cow::Borrowed(value);
    }

    let suffix = format!("...[truncated; original_bytes={}]", value.len());
    if suffix.len() >= max_bytes {
        let mut end = max_bytes;
        while !suffix.is_char_boundary(end) {
            end -= 1;
        }
        return Cow::Owned(suffix[..end].to_owned());
    }

    let (prefix, _) = safe_split_at(value, max_bytes - suffix.len());
    Cow::Owned(format!("{prefix}{suffix}"))
}

fn stack_log_field(stack: Option<&str>) -> Option<Cow<'_, str>> {
    stack
        .filter(|stack| !stack.is_empty())
        .map(|stack| bounded_log_field(stack, LOG_STACK_MAX_BYTES))
}

fn safe_split_at(value: &str, mut index: usize) -> (&str, &str) {
    if index >= value.len() {
        return (value, "");
    }
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    value.split_at(index)
}

fn truncated_to_byte_limit(value: &str, max_bytes: usize) -> String {
    let (prefix, remainder) = safe_split_at(value, max_bytes);
    if remainder.is_empty() {
        return value.to_owned();
    }

    let mut truncated = String::with_capacity(prefix.len() + 3);
    truncated.push_str(prefix);
    truncated.push_str("...");
    truncated
}

fn get_object_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8_string(scope, key)?;
    object.get(scope, key.into())
}

fn get_object_property_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<String> {
    get_object_property(scope, object, key)
        .and_then(|value| local_value_to_string(scope, value))
        .filter(|value| !value.is_empty())
}

fn value_debug_summary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> String {
    if value.is_undefined() {
        return "undefined".to_owned();
    }
    if value.is_null() {
        return "null".to_owned();
    }
    if value.is_boolean() || value.is_number() || value.is_string() {
        return local_value_to_string(scope, value).unwrap_or_else(|| "<scalar>".to_owned());
    }
    if value.is_function() {
        let detail = value
            .to_detail_string(scope)
            .map(|text| text.to_rust_string_lossy(scope))
            .filter(|text| !text.is_empty());
        return truncated_to_byte_limit(
            detail.as_deref().unwrap_or("[function]"),
            VALUE_DEBUG_SUMMARY_MAX_BYTES,
        );
    }

    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return local_value_to_string(scope, value)
            .map(|text| truncated_to_byte_limit(&text, VALUE_DEBUG_SUMMARY_MAX_BYTES))
            .unwrap_or_else(|| "<value>".to_owned());
    };

    let mut details = Vec::new();
    for key in ["type", "nodeName", "id", "className", "src", "href"] {
        if let Some(property_value) = get_object_property(scope, object, key)
            && !property_value.is_null_or_undefined()
        {
            let property_text = value_debug_summary(scope, property_value);
            if property_text != "undefined" && property_text != "null" {
                details.push(format!("{key}={property_text}"));
            }
        }
    }

    let constructor = object.get_constructor_name().to_rust_string_lossy(scope);
    if details.is_empty() {
        let detail = value
            .to_detail_string(scope)
            .map(|text| text.to_rust_string_lossy(scope))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| format!("[object {constructor}]"));
        return truncated_to_byte_limit(&detail, VALUE_DEBUG_SUMMARY_MAX_BYTES);
    }

    truncated_to_byte_limit(
        &format!("{constructor}{{{}}}", details.join(", ")),
        VALUE_DEBUG_OBJECT_SUMMARY_MAX_BYTES,
    )
}

fn global_path_summary(scope: &mut v8::PinScope<'_, '_>, path: &[&str]) -> String {
    let global = scope.get_current_context().global(scope);
    let mut current: v8::Local<'_, v8::Value> = global.into();
    let mut prefix = String::from("window");

    for segment in path {
        prefix.push('.');
        prefix.push_str(segment);

        let Ok(object) = v8::Local::<v8::Object>::try_from(current) else {
            return format!("{prefix}=<non-object parent>");
        };
        let Some(next) = get_object_property(scope, object, segment) else {
            return format!("{prefix}=<unreadable>");
        };
        current = next;
    }

    format!("{prefix}={}", value_debug_summary(scope, current))
}

fn object_property_state(
    scope: &mut v8::PinScope<'_, '_>,
    label: &str,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> Vec<String> {
    let property_key = v8str(scope, key);
    let own = object
        .has_own_property(scope, property_key.into())
        .unwrap_or(false);
    let real = object.get_real_named_property(scope, property_key.into());
    let resolved = object.get(scope, property_key.into());
    let mut parts = vec![format!("{label}.{key}.own={own}")];
    parts.push(format!(
        "{label}.{key}.real={}",
        real.map(|value| value_debug_summary(scope, value))
            .unwrap_or_else(|| "<none>".to_owned())
    ));
    parts.push(format!(
        "{label}.{key}={}",
        resolved
            .map(|value| value_debug_summary(scope, value))
            .unwrap_or_else(|| "<unreadable>".to_owned())
    ));
    parts
}

fn eval_debug_probe(scope: &mut v8::PinScope<'_, '_>, source: &str) -> String {
    let Some(source) = v8_string(scope, source) else {
        return "<alloc failed>".to_owned();
    };
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let Some(script) = v8::Script::compile(&scope, source, None) else {
        return "<compile failed>".to_owned();
    };
    let Some(result) = script.run(&scope) else {
        return scope
            .exception()
            .and_then(|value| value.to_string(&scope))
            .map(|value| value.to_rust_string_lossy(&scope))
            .unwrap_or_else(|| "<probe threw>".to_owned());
    };
    result
        .to_string(&scope)
        .map(|value| value.to_rust_string_lossy(&scope))
        .unwrap_or_else(|| "<probe non-string>".to_owned())
}

pub(super) fn build_callback_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) -> String {
    let mut parts = vec![format!("receiver={}", value_debug_summary(scope, receiver))];
    let global = scope.get_current_context().global(scope);
    parts.push(format!(
        "globalThis={}",
        value_debug_summary(scope, global.into())
    ));

    if let Some(first_arg) = args.first().copied() {
        parts.push(format!("arg0={}", value_debug_summary(scope, first_arg)));
        if let Ok(event) = v8::Local::<v8::Object>::try_from(first_arg) {
            if let Some(event_type) = get_object_property_string(scope, event, "type") {
                parts.push(format!("event.type={event_type}"));
            }
            if let Some(target) = get_object_property(scope, event, "target") {
                parts.push(format!(
                    "event.target={}",
                    value_debug_summary(scope, target)
                ));
            }
            if let Some(current_target) = get_object_property(scope, event, "currentTarget") {
                parts.push(format!(
                    "event.currentTarget={}",
                    value_debug_summary(scope, current_target)
                ));
            }
        }
    }

    let window_value = get_object_property(scope, global, "window");
    parts.push(format!(
        "window={}",
        window_value
            .map(|value| value_debug_summary(scope, value))
            .unwrap_or_else(|| "<unreadable>".to_owned())
    ));
    if let Some(window_value) = window_value {
        parts.push(format!(
            "window===globalThis={}",
            window_value.strict_equals(global.into())
        ));
        if let Ok(window_object) = v8::Local::<v8::Object>::try_from(window_value) {
            parts.extend(object_property_state(
                scope,
                "window",
                window_object,
                "addEventListener",
            ));
            parts.extend(object_property_state(
                scope,
                "window",
                window_object,
                "removeEventListener",
            ));
            parts.extend(object_property_state(
                scope,
                "window",
                window_object,
                "dispatchEvent",
            ));
            let nested_window = get_object_property(scope, window_object, "window");
            parts.push(format!(
                "window.window={}",
                nested_window
                    .map(|value| value_debug_summary(scope, value))
                    .unwrap_or_else(|| "<unreadable>".to_owned())
            ));
            if let Some(nested_window) = nested_window {
                parts.push(format!(
                    "window.window===window={}",
                    nested_window.strict_equals(window_object.into())
                ));
            }
        }
    }
    parts.extend(object_property_state(scope, "globalThis", global, "window"));
    parts.extend(object_property_state(scope, "globalThis", global, "self"));
    parts.extend(object_property_state(
        scope,
        "globalThis",
        global,
        "addEventListener",
    ));
    parts.extend(object_property_state(
        scope,
        "globalThis",
        global,
        "removeEventListener",
    ));
    parts.extend(object_property_state(
        scope,
        "globalThis",
        global,
        "dispatchEvent",
    ));
    if moli_trace::v8_exception_probe_enabled() {
        parts.push(format!(
            "js.window.probe={}",
            eval_debug_probe(
                scope,
                r#"(() => {
                    const w = window;
                    return JSON.stringify({
                        eq: w === globalThis,
                        ctor: w && w.constructor && w.constructor.name,
                        add: w ? typeof w.addEventListener : "missing",
                        remove: w ? typeof w.removeEventListener : "missing",
                        dispatch: w ? typeof w.dispatchEvent : "missing",
                        bridgeEq: typeof __moliNativeBridge !== "undefined"
                            && w === __moliNativeBridge.window,
                        globalPropEq: globalThis.window === globalThis,
                        docEq: !!w && w.document === document,
                        bridgeGetElementById: w ? typeof w.getElementById : "missing",
                        nested: !!w && w.window === w,
                        selfEq: self === globalThis,
                        topEq: top === globalThis,
                        parentEq: parent === globalThis,
                        framesEq: frames === globalThis
                    });
                })()"#,
            )
        ));
    }

    for path in [
        ["page"].as_slice(),
        ["page", "comm"].as_slice(),
        ["page", "comm", "invokeApps"].as_slice(),
        ["sSession"].as_slice(),
        ["sSession", "invokeApps"].as_slice(),
    ] {
        parts.push(global_path_summary(scope, path));
    }

    parts.join("; ")
}

pub(super) fn invoke_callback_with_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_kind: &str,
    log_label: &str,
    log_level: CallbackExceptionLogLevel,
    callback_name: &str,
    handler: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) -> std::result::Result<v8::Global<v8::Value>, Box<V8ExceptionReport>> {
    invoke_callback_with_report_inner(
        scope,
        callback_kind,
        Some((log_label, log_level)),
        callback_name,
        handler,
        receiver,
        args,
    )
}

fn invoke_callback_with_report_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_kind: &str,
    log: Option<(&str, CallbackExceptionLogLevel)>,
    callback_name: &str,
    handler: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) -> std::result::Result<v8::Global<v8::Value>, Box<V8ExceptionReport>> {
    let mut returned_value: Option<v8::Global<v8::Value>> = None;
    let mut captured_report: Option<V8ExceptionReport> = None;

    {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        if let Some(returned) = handler.call(&scope, receiver, args) {
            returned_value = Some(v8::Global::new(&scope, returned));
        } else {
            if scope.is_execution_terminating() {
                let _ = scope.rethrow();
                captured_report = Some(V8ExceptionReport {
                    muted_errors: false,
                    summary: format!("{callback_kind} `{callback_name}` was terminated"),
                    source: None,
                    line: None,
                    column: None,
                    source_line: None,
                    stack: None,
                    callback_context: Some(build_callback_context(&mut scope, receiver, args)),
                    exception: None,
                });
                return returned_value.ok_or_else(|| {
                    Box::new(captured_report.expect("termination report should be present"))
                });
            }
            let exception = scope.exception();
            let message = scope.message();
            let stack_trace = scope.stack_trace();
            let mut exception_report =
                build_event_handler_exception_report(&mut scope, exception, message, stack_trace);
            exception_report.callback_context =
                Some(build_callback_context(&mut scope, receiver, args));
            if let Some((log_label, log_level)) = log {
                log_callback_exception(log_level, log_label, callback_name, &exception_report);
            }
            captured_report = Some(exception_report);
        }
    }

    let _ = callback_kind;
    returned_value.ok_or_else(|| {
        Box::new(captured_report.unwrap_or_else(|| V8ExceptionReport {
            muted_errors: false,
            summary: format!("{callback_kind} `{callback_name}` threw"),
            source: None,
            line: None,
            column: None,
            source_line: None,
            stack: None,
            callback_context: None,
            exception: None,
        }))
    })
}

fn invoke_callback_with_reporting<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_kind: &str,
    log_label: &str,
    log_level: CallbackExceptionLogLevel,
    callback_name: &str,
    handler: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) -> std::result::Result<v8::Global<v8::Value>, String> {
    invoke_callback_with_report(
        scope,
        callback_kind,
        log_label,
        log_level,
        callback_name,
        handler,
        receiver,
        args,
    )
    .map_err(|report| report.formatted_error(callback_kind, callback_name))
}

pub(super) fn invoke_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handler_name: &str,
    handler: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) -> std::result::Result<v8::Global<v8::Value>, String> {
    invoke_callback_with_reporting(
        scope,
        "event handler",
        "host event handler threw",
        CallbackExceptionLogLevel::Debug,
        handler_name,
        handler,
        receiver,
        args,
    )
}

pub(super) fn invoke_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_name: &str,
    handler: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
) -> std::result::Result<v8::Global<v8::Value>, String> {
    invoke_callback_with_reporting(
        scope,
        "callback",
        "host callback threw",
        CallbackExceptionLogLevel::Debug,
        callback_name,
        handler,
        receiver,
        args,
    )
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{
        DiagnosticLogLevel, LOG_STACK_MAX_BYTES, VALUE_DEBUG_SUMMARY_MAX_BYTES, bounded_log_field,
        safe_split_at, stack_log_field, truncated_to_byte_limit, v8_message_log_level,
    };

    #[test]
    fn bounded_log_field_borrows_values_within_the_limit() {
        assert!(matches!(
            bounded_log_field("short diagnostic", 64),
            Cow::Borrowed("short diagnostic")
        ));
    }

    #[test]
    fn bounded_log_field_reports_original_size_within_the_limit() {
        let value = "x".repeat(10_000);
        let bounded = bounded_log_field(&value, 128);

        assert_eq!(bounded.len(), 128);
        assert!(bounded.starts_with('x'));
        assert!(bounded.ends_with("...[truncated; original_bytes=10000]"));
    }

    #[test]
    fn bounded_log_field_preserves_utf8_boundaries() {
        let value = "诊断".repeat(100);
        let bounded = bounded_log_field(&value, 97);

        assert!(bounded.len() <= 97);
        assert!(bounded.contains("[truncated; original_bytes=600]"));
    }

    #[test]
    fn safe_split_at_moves_back_to_a_utf8_boundary() {
        let (prefix, remainder) = safe_split_at("a诊b", 3);

        assert_eq!(prefix, "a");
        assert_eq!(remainder, "诊b");
    }

    #[test]
    fn debug_summary_byte_budget_preserves_the_previous_character_capacity() {
        let within_old_limit = "𐍈".repeat(160);
        assert_eq!(within_old_limit.len(), VALUE_DEBUG_SUMMARY_MAX_BYTES);
        assert_eq!(
            truncated_to_byte_limit(&within_old_limit, VALUE_DEBUG_SUMMARY_MAX_BYTES),
            within_old_limit
        );

        let over_limit = "𐍈".repeat(161);
        let bounded = truncated_to_byte_limit(&over_limit, VALUE_DEBUG_SUMMARY_MAX_BYTES);

        assert_eq!(bounded, format!("{}...", "𐍈".repeat(160)));
        assert_eq!(bounded.chars().count(), 163);
    }

    #[test]
    fn stack_log_field_omits_missing_stacks_and_bounds_present_stacks() {
        assert!(stack_log_field(None).is_none());
        assert!(stack_log_field(Some("")).is_none());

        let stack = "frame\n".repeat(10_000);
        let bounded = stack_log_field(Some(&stack)).expect("non-empty stack should be logged");
        assert_eq!(bounded.len(), LOG_STACK_MAX_BYTES);
        assert!(bounded.ends_with("...[truncated; original_bytes=60000]"));
    }

    #[test]
    fn v8_message_levels_keep_warning_and_error_diagnostics_separate() {
        assert_eq!(
            v8_message_log_level(v8::MessageErrorLevel::WARNING.bits()),
            DiagnosticLogLevel::Warn
        );
        assert_eq!(
            v8_message_log_level(v8::MessageErrorLevel::INFO.bits()),
            DiagnosticLogLevel::Info
        );
        assert_eq!(
            v8_message_log_level(v8::MessageErrorLevel::DEBUG.bits()),
            DiagnosticLogLevel::Debug
        );
        assert_eq!(
            v8_message_log_level(v8::MessageErrorLevel::LOG.bits()),
            DiagnosticLogLevel::Debug
        );
        assert_eq!(
            v8_message_log_level(v8::MessageErrorLevel::ERROR.bits()),
            DiagnosticLogLevel::Error
        );
    }
}
