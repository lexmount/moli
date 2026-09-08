use std::time::{SystemTime, UNIX_EPOCH};

use moli_protocol::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsRealmId, DevToolsStackTrace, LogEntryEvent,
    PageFileChooserOpenedEvent, PageJavaScriptDialogOpeningEvent, RuntimeConsoleEvent,
    RuntimeExecutionContextEvent, RuntimeExecutionContextsClearedEvent, ScriptExceptionEvent,
    ScriptMessageEvent, TargetLifecycleEvent, UserPromptClosedEvent,
};
use serde_json::{Value, json};

use crate::browsing_context::{
    bidi_browsing_context_info_from_cdp_target_info,
    bidi_browsing_context_info_from_target_lifecycle,
};
use crate::script_values::{
    bidi_remote_value, bidi_remote_value_from_devtools, bidi_remote_value_from_devtools_metadata,
};

pub fn bidi_event_from_automation_event(event: &AutomationEvent) -> Option<Value> {
    match event {
        AutomationEvent::TargetCreated(event) => browsing_context_target_lifecycle_event(
            "browsingContext.contextCreated",
            event,
            Value::Null,
        ),
        AutomationEvent::TargetDestroyed(event) => browsing_context_target_lifecycle_event(
            "browsingContext.contextDestroyed",
            event,
            json!([]),
        ),
        AutomationEvent::RuntimeExecutionContextCreated(event) => script_realm_created_event(event),
        AutomationEvent::RuntimeExecutionContextDestroyed(event) => {
            script_realm_destroyed_event(event)
        }
        AutomationEvent::RuntimeExecutionContextsCleared(event) => {
            script_realm_events_cleared(event)
        }
        AutomationEvent::NavigationStarted(event) => {
            browsing_context_navigation_lifecycle_event("browsingContext.navigationStarted", event)
        }
        AutomationEvent::DomContentLoaded(event) => {
            browsing_context_navigation_lifecycle_event("browsingContext.domContentLoaded", event)
        }
        AutomationEvent::Load(event) => {
            browsing_context_navigation_lifecycle_event("browsingContext.load", event)
        }
        AutomationEvent::LogEntryAdded(event) => log_entry_added_generic_event(event),
        AutomationEvent::RuntimeConsoleApiCalled(event) => runtime_console_log_entry_event(event),
        AutomationEvent::ScriptMessage(event) => script_message_event(event),
        AutomationEvent::ScriptException(event) => script_exception_log_entry_event(event),
        AutomationEvent::UserPromptClosed(event) => user_prompt_closed_event(event),
        AutomationEvent::PageJavaScriptDialogOpening(event) => {
            user_prompt_opened_event(event, "dismiss")
        }
        AutomationEvent::PageFileChooserOpened(event) => {
            input_file_dialog_opened_event(event, None)
        }
        _ => None,
    }
}

pub(crate) fn input_file_dialog_opened_event(
    event: &PageFileChooserOpenedEvent,
    _top_level_context: Option<&str>,
) -> Option<Value> {
    let context = event.frame_id.as_str();
    let mut bidi_event = json!({
        "type": "event",
        "method": "input.fileDialogOpened",
        "params": {
            "context": context,
            "multiple": event.mode == "selectMultiple",
        },
    });
    let shared_id = event.element_shared_id.clone();
    if let (Some(object), Some(shared_id)) = (bidi_event["params"].as_object_mut(), shared_id) {
        object.insert(
            "element".to_owned(),
            json!({
                "sharedId": shared_id.into_string(),
            }),
        );
    }
    Some(bidi_event)
}

pub fn bidi_event_from_protocol_message(message: &Value) -> Option<Value> {
    bidi_event_from_protocol_message_with_prompt_handler(message, "dismiss", None, None)
}

pub(crate) fn bidi_event_from_protocol_message_with_prompt_handler(
    message: &Value,
    prompt_handler: &str,
    top_level_context: Option<&str>,
    destroyed_realm: Option<&str>,
) -> Option<Value> {
    match message.get("method").and_then(Value::as_str) {
        Some("Target.targetCreated") => {
            browsing_context_context_created_event_from_cdp_target_info(
                &message["params"]["targetInfo"],
            )
        }
        Some("Runtime.executionContextCreated") => {
            let context = &message["params"]["context"];
            let aux_data = &context["auxData"];
            let event = RuntimeExecutionContextEvent {
                target_id: None,
                context_id: context["id"].as_i64(),
                realm_id: runtime_realm_id_from_protocol_context(
                    context,
                    aux_data,
                    top_level_context,
                )
                .map(DevToolsRealmId::from),
                frame_id: aux_data["frameId"].as_str().map(DevToolsFrameId::from),
                origin: context["origin"].as_str().map(str::to_owned),
                name: context["name"].as_str().map(str::to_owned),
                is_default: aux_data["isDefault"].as_bool(),
                context_type: runtime_context_type_from_protocol_context(
                    context,
                    aux_data,
                    top_level_context,
                ),
                grant_universal_access: aux_data["grantUniversalAccess"].as_bool(),
            };
            script_realm_created_event(&event)
        }
        Some("Runtime.executionContextDestroyed") => {
            let event = RuntimeExecutionContextEvent {
                target_id: None,
                context_id: message["params"]["executionContextId"].as_i64(),
                realm_id: destroyed_realm
                    .or_else(|| message["params"]["executionContextUniqueId"].as_str())
                    .map(DevToolsRealmId::from),
                frame_id: None,
                origin: None,
                name: None,
                is_default: None,
                context_type: None,
                grant_universal_access: None,
            };
            script_realm_destroyed_event(&event)
        }
        Some("Page.javascriptDialogOpening") => {
            user_prompt_opened_event_from_protocol_params(&message["params"], prompt_handler)
        }
        Some("Page.fileChooserOpened") => input_file_dialog_opened_event_from_protocol_params(
            message.get("params").unwrap_or(&Value::Null),
            top_level_context,
        ),
        _ => None,
    }
}

pub(crate) fn owner_scoped_shared_worker_realm_id_from_protocol_context(
    context: &Value,
    aux_data: &Value,
    owner_context: Option<&str>,
) -> Option<String> {
    let owner_context = owner_context?;
    if aux_data["type"].as_str()? != "worker" || aux_data["frameId"].as_str().is_some() {
        return None;
    }
    if context["uniqueId"]
        .as_str()
        .is_some_and(|unique_id| unique_id.starts_with("shared-worker-"))
    {
        return None;
    }
    Some(format!("shared-worker-{owner_context}"))
}

pub(crate) fn runtime_realm_id_from_protocol_context(
    context: &Value,
    aux_data: &Value,
    owner_context: Option<&str>,
) -> Option<String> {
    owner_scoped_shared_worker_realm_id_from_protocol_context(context, aux_data, owner_context)
        .or_else(|| context["uniqueId"].as_str().map(str::to_owned))
}

fn runtime_context_type_from_protocol_context(
    context: &Value,
    aux_data: &Value,
    owner_context: Option<&str>,
) -> Option<String> {
    let context_type = aux_data["type"].as_str()?;
    if context_type == "worker"
        && (context["uniqueId"]
            .as_str()
            .is_some_and(|unique_id| unique_id.starts_with("shared-worker-"))
            || owner_scoped_shared_worker_realm_id_from_protocol_context(
                context,
                aux_data,
                owner_context,
            )
            .is_some())
    {
        return Some("shared-worker".to_owned());
    }
    Some(context_type.to_owned())
}

fn browsing_context_context_created_event_from_cdp_target_info(
    target_info: &Value,
) -> Option<Value> {
    Some(browsing_context_context_event(
        "browsingContext.contextCreated",
        bidi_browsing_context_info_from_cdp_target_info(target_info, Value::Null)?,
    ))
}

fn browsing_context_target_lifecycle_event(
    method: &str,
    event: &TargetLifecycleEvent,
    children: Value,
) -> Option<Value> {
    Some(browsing_context_context_event(
        method,
        bidi_browsing_context_info_from_target_lifecycle(event, children)?,
    ))
}

fn browsing_context_context_event(method: &str, params: Value) -> Value {
    json!({
        "type": "event",
        "method": method,
        "params": params,
    })
}

fn browsing_context_navigation_lifecycle_event(
    method: &str,
    event: &moli_protocol::devtools_runtime::NavigationLifecycleEvent,
) -> Option<Value> {
    Some(browsing_context_navigation_event(
        method,
        event.frame_id.as_str(),
        &event.url,
        event
            .navigation_id
            .as_ref()
            .map(|navigation| navigation.as_str()),
    ))
}

fn user_prompt_opened_event_from_protocol_params(
    params: &Value,
    prompt_handler: &str,
) -> Option<Value> {
    let dialog_type = params.get("type").and_then(Value::as_str)?;
    user_prompt_opened_event(
        &PageJavaScriptDialogOpeningEvent {
            frame_id: params
                .get("frameId")
                .and_then(Value::as_str)
                .map(DevToolsFrameId::from),
            url: params
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            message: params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            dialog_type: dialog_type.to_owned(),
            has_browser_handler: params
                .get("hasBrowserHandler")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            default_prompt: params
                .get("defaultPrompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
        prompt_handler,
    )
}

pub(crate) fn user_prompt_opened_event(
    event: &PageJavaScriptDialogOpeningEvent,
    prompt_handler: &str,
) -> Option<Value> {
    let context = event.frame_id.as_ref()?.as_str();
    let prompt_type = event.dialog_type.as_str();
    let mut bidi_event = json!({
        "type": "event",
        "method": "browsingContext.userPromptOpened",
        "params": {
            "context": context,
            "type": prompt_type,
            "message": event.message.as_str(),
            "handler": prompt_handler,
        },
    });
    if prompt_type == "prompt"
        && let Some(object) = bidi_event["params"].as_object_mut()
    {
        object.insert(
            "defaultValue".to_owned(),
            json!(event.default_prompt.as_str()),
        );
    }
    Some(bidi_event)
}

fn user_prompt_closed_event(event: &UserPromptClosedEvent) -> Option<Value> {
    let mut params = json!({
        "context": event.frame_id.as_str(),
        "accepted": event.accepted,
        "type": event.prompt_type,
    });
    if event.prompt_type == "prompt"
        && event.accepted
        && let Some(object) = params.as_object_mut()
    {
        object.insert("userText".to_owned(), json!(event.user_text));
    }
    Some(json!({
        "type": "event",
        "method": "browsingContext.userPromptClosed",
        "params": params,
    }))
}

fn input_file_dialog_opened_event_from_protocol_params(
    params: &Value,
    top_level_context: Option<&str>,
) -> Option<Value> {
    let event = PageFileChooserOpenedEvent {
        frame_id: DevToolsFrameId::from(params.get("frameId").and_then(Value::as_str)?),
        mode: params.get("mode").and_then(Value::as_str)?.to_owned(),
        backend_node_id: params
            .get("backendNodeId")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())?,
        element_shared_id: None,
    };
    input_file_dialog_opened_event(&event, top_level_context)
}

pub(crate) fn browsing_context_navigation_event(
    method: &str,
    context: &str,
    url: &str,
    navigation_id: Option<&str>,
) -> Value {
    json!({
        "type": "event",
        "method": method,
        "params": {
            "context": context,
            "navigation": navigation_id.map(Value::from).unwrap_or(Value::Null),
            "timestamp": bidi_timestamp_millis(),
            "url": url,
        },
    })
}

pub(crate) fn browsing_context_history_updated_event(context: &str, url: &str) -> Value {
    json!({
        "type": "event",
        "method": "browsingContext.historyUpdated",
        "params": {
            "context": context,
            "timestamp": bidi_timestamp_millis(),
            "url": url,
        },
    })
}

pub(crate) fn bidi_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(crate) fn non_empty_json_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn log_entry_added_event(params: Value) -> Value {
    json!({
        "type": "event",
        "method": "log.entryAdded",
        "params": params,
    })
}

fn log_entry_added_generic_event(event: &LogEntryEvent) -> Option<Value> {
    let mut entry = json!({
        "type": event.source,
        "level": bidi_log_level(&event.level),
        "source": {
            "realm": event
                .target_id
                .as_ref()
                .map(|target| target.as_str())
                .unwrap_or("unknown"),
        },
        "text": event.text,
        "timestamp": bidi_timestamp_millis(),
    });
    if let Some(args) = (!event.args.is_empty()).then(|| {
        event
            .args
            .iter()
            .cloned()
            .map(bidi_remote_value_from_devtools)
            .collect::<Vec<_>>()
    }) && let Some(entry) = entry.as_object_mut()
    {
        entry.insert("args".to_owned(), json!(args));
    }
    Some(log_entry_added_event(entry))
}

fn runtime_console_log_entry_event(event: &RuntimeConsoleEvent) -> Option<Value> {
    let args = event
        .args
        .iter()
        .map(bidi_remote_value_from_cdp_remote_object)
        .collect::<Vec<_>>();
    let mut source = json!({});
    if let Some(realm) = positive_execution_context_realm(event.execution_context_id) {
        source["realm"] = json!(realm);
    } else if event.target_id.is_none() {
        source["realm"] = json!("unknown");
    }
    let mut entry = json!({
        "type": "console",
        "method": bidi_log_method(&event.console_type),
        "level": bidi_log_level(&event.console_type),
        "source": source,
        "text": bidi_log_text_from_remote_values(&event.console_type, &args),
        "timestamp": bidi_timestamp_millis(),
        "args": args,
    });
    if let Some(stack_trace) = event
        .stack_trace
        .as_ref()
        .map(bidi_stack_trace_from_devtools)
        .or_else(|| console_stack_trace_for_bidi(event.stack.as_deref()))
        && let Some(entry) = entry.as_object_mut()
    {
        entry.insert("stackTrace".to_owned(), stack_trace);
    }
    Some(log_entry_added_event(entry))
}

fn script_exception_log_entry_event(event: &ScriptExceptionEvent) -> Option<Value> {
    let mut source = json!({});
    if let Some(realm) =
        positive_execution_context_realm(event.execution_context_id).or_else(|| {
            event
                .exception
                .realm
                .as_ref()
                .map(|realm| realm.as_str().to_owned())
        })
    {
        source["realm"] = json!(realm);
    } else if event.target_id.is_none() {
        source["realm"] = json!("unknown");
    }
    if let Some(target_id) = event.target_id.as_ref()
        && let Some(source) = source.as_object_mut()
    {
        source.insert("context".to_owned(), json!(target_id.as_str()));
    }
    let mut entry = json!({
        "type": "javascript",
        "level": "error",
        "source": source,
        "text": event.exception.text,
        "timestamp": bidi_timestamp_millis(),
    });
    if let Some(stack_trace) = event
        .exception
        .stack_trace
        .as_ref()
        .map(bidi_stack_trace_from_devtools)
        && let Some(entry) = entry.as_object_mut()
    {
        entry.insert("stackTrace".to_owned(), stack_trace);
    }
    Some(log_entry_added_event(entry))
}

fn positive_execution_context_realm(execution_context_id: Option<i64>) -> Option<String> {
    execution_context_id
        .filter(|execution_context_id| *execution_context_id > 0)
        .map(|execution_context_id| execution_context_id.to_string())
}

fn script_message_event(event: &ScriptMessageEvent) -> Option<Value> {
    let mut source = json!({
        "realm": event.realm_id.as_ref()?.as_str(),
    });
    if let Some(target_id) = event.target_id.as_ref()
        && let Some(source) = source.as_object_mut()
    {
        source.insert("context".to_owned(), json!(target_id.as_str()));
    }
    Some(json!({
        "type": "event",
        "method": "script.message",
        "params": {
            "channel": event.channel.as_str(),
            "data": bidi_remote_value_from_devtools(event.data.clone()),
            "source": source,
        },
    }))
}

pub(crate) fn log_entry_added_generic_event_from_protocol_message(
    message: &Value,
    owner_context: Option<&str>,
) -> Option<Value> {
    let entry = &message["params"]["entry"];
    let source_name = entry
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("generic");
    let level = entry.get("level").and_then(Value::as_str).unwrap_or("info");
    let mut source = json!({
        "realm": owner_context.unwrap_or("unknown"),
    });
    if let Some(context) = owner_context
        && let Some(source) = source.as_object_mut()
    {
        source.insert("context".to_owned(), json!(context));
    }
    Some(log_entry_added_event(json!({
        "type": source_name,
        "level": bidi_log_level(level),
        "source": source,
        "text": entry.get("text").and_then(Value::as_str).unwrap_or_default(),
        "timestamp": bidi_timestamp_millis(),
    })))
}

pub(crate) fn bidi_remote_value_from_cdp_remote_object(value: &Value) -> Value {
    let remote_type = value.get("type").and_then(Value::as_str);
    let remote_subtype = value.get("subtype").and_then(Value::as_str);
    let unserializable_value = value.get("unserializableValue").and_then(Value::as_str);
    let description = value.get("description").and_then(Value::as_str);
    let class_name = value.get("className").and_then(Value::as_str);
    let value = value.get("value").cloned().unwrap_or(Value::Null);
    match remote_type {
        Some(remote_type) => bidi_remote_value_from_devtools_metadata(
            value,
            remote_type,
            remote_subtype,
            unserializable_value,
            description,
            class_name,
        ),
        None => bidi_remote_value(value, None),
    }
}

pub(crate) fn bidi_log_text_from_remote_values(console_type: &str, args: &[Value]) -> String {
    let mut args = args;
    if console_type == "assert"
        && args
            .first()
            .is_some_and(|arg| arg["type"] == "boolean" && arg["value"] == false)
    {
        args = &args[1..];
    }
    args.iter()
        .map(bidi_log_text_from_remote_value)
        .collect::<Vec<_>>()
        .join(" ")
}

fn bidi_log_text_from_remote_value(value: &Value) -> String {
    match value.get("type").and_then(Value::as_str) {
        Some("undefined") => "undefined".to_owned(),
        Some("null") => "null".to_owned(),
        Some("string") | Some("number") | Some("boolean") | Some("bigint") => value
            .get("value")
            .and_then(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Bool(value) => Some(value.to_string()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .unwrap_or_default(),
        Some("array") => value
            .get("value")
            .and_then(Value::as_array)
            .map(|value| format!("Array({})", value.len()))
            .unwrap_or_else(|| "array".to_owned()),
        Some("object") => value
            .get("value")
            .and_then(Value::as_array)
            .map(|value| format!("Object({})", value.len()))
            .unwrap_or_else(|| "object".to_owned()),
        Some(other) => other.to_owned(),
        None => String::new(),
    }
}

pub(crate) fn bidi_log_level(console_type: &str) -> &'static str {
    match console_type {
        "error" | "assert" => "error",
        "debug" | "trace" => "debug",
        "warn" | "warning" => "warn",
        _ => "info",
    }
}

pub(crate) fn bidi_log_method(console_type: &str) -> &str {
    match console_type {
        "warning" => "warn",
        "startGroup" => "group",
        "startGroupCollapsed" => "groupCollapsed",
        "endGroup" => "groupEnd",
        _ => console_type,
    }
}

pub(crate) fn bidi_stack_trace_from_cdp(stack_trace: Option<&Value>) -> Option<Value> {
    let call_frames = stack_trace?
        .get("callFrames")
        .and_then(Value::as_array)?
        .iter()
        .map(|frame| {
            json!({
                "columnNumber": frame
                    .get("columnNumber")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                "functionName": frame
                    .get("functionName")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                "lineNumber": frame
                    .get("lineNumber")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                "url": frame
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    Some(json!({ "callFrames": call_frames }))
}

pub(crate) fn bidi_stack_trace_from_devtools(stack_trace: &DevToolsStackTrace) -> Value {
    json!({
        "callFrames": stack_trace
            .call_frames
            .iter()
            .map(|frame| {
                json!({
                    "columnNumber": frame.column_number,
                    "functionName": frame.function_name,
                    "lineNumber": frame.line_number,
                    "url": frame.url,
                })
            })
            .collect::<Vec<_>>()
    })
}

fn console_stack_trace_for_bidi(stack: Option<&str>) -> Option<Value> {
    let stack = stack?;
    Some(json!({
        "callFrames": [{
            "columnNumber": 0,
            "functionName": "",
            "lineNumber": 0,
            "url": stack,
        }]
    }))
}

pub fn script_realm_created_event(event: &RuntimeExecutionContextEvent) -> Option<Value> {
    Some(json!({
        "type": "event",
        "method": "script.realmCreated",
        "params": script_realm_info(event)?,
    }))
}

pub fn script_realm_destroyed_event(event: &RuntimeExecutionContextEvent) -> Option<Value> {
    Some(json!({
        "type": "event",
        "method": "script.realmDestroyed",
        "params": {
            "realm": event.realm_id.as_ref()?.as_str(),
        },
    }))
}

fn script_realm_events_cleared(_event: &RuntimeExecutionContextsClearedEvent) -> Option<Value> {
    None
}

pub(crate) fn script_realm_info(event: &RuntimeExecutionContextEvent) -> Option<Value> {
    let mut realm = event.realm_id.as_ref()?.as_str().to_owned();
    let origin = event.origin.as_deref().unwrap_or("null");
    let mut context_type = event.context_type.as_deref();
    if context_type == Some("worker")
        && event.frame_id.is_none()
        && let Some(target_id) = event.target_id.as_ref()
    {
        realm = format!("shared-worker-{}", target_id.as_str());
        context_type = Some("shared-worker");
    }
    if matches!(
        context_type,
        Some("worker")
            | Some("dedicated-worker")
            | Some("shared-worker")
            | Some("service-worker")
            | Some("paint-worklet")
            | Some("audio-worklet")
            | Some("worklet")
    ) {
        let mut info = json!({
            "realm": realm,
            "origin": origin,
            "type": bidi_realm_type(context_type),
        });
        if context_type == Some("dedicated-worker") {
            let owner = event.frame_id.as_ref()?;
            info["owners"] = json!([owner.as_str()]);
        }
        return Some(info);
    }

    let mut info = json!({
        "realm": realm,
        "origin": origin,
        "type": "window",
        "context": event.frame_id.as_ref()?.as_str(),
    });
    if event.is_default == Some(false)
        && let Some(name) = event.name.as_deref().filter(|name| !name.is_empty())
        && let Some(info) = info.as_object_mut()
    {
        info.insert("sandbox".to_owned(), json!(name));
    }
    Some(info)
}

fn bidi_realm_type(context_type: Option<&str>) -> &'static str {
    match context_type {
        Some("dedicated-worker") => "dedicated-worker",
        Some("shared-worker") => "shared-worker",
        Some("service-worker") => "service-worker",
        Some("paint-worklet") => "paint-worklet",
        Some("audio-worklet") => "audio-worklet",
        Some("worklet") => "worklet",
        _ => "worker",
    }
}
