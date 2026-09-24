use std::collections::HashSet;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_protocol::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsCommandResult, DevToolsError, DevToolsErrorKind,
    DevToolsNetworkDataBytesType, DevToolsScriptResult, RuntimeConsoleEvent,
};
use serde_json::{Value, json};

use crate::BidiErrorCode;
use crate::browsing_context::{
    bidi_browsing_context_info, bidi_browsing_context_infos_from_frame_tree_result,
};
use crate::events::{bidi_stack_trace_from_devtools, script_realm_info};
use crate::script_values::bidi_remote_value_from_devtools;
use crate::storage::bidi_cookie_from_cdp_cookie;
use crate::user_context::{DEFAULT_BIDI_USER_CONTEXT, bidi_user_context_from_browser_context_id};

pub fn success_response(id: u64, result: Value) -> Value {
    json!({
        "type": "success",
        "id": id,
        "result": result,
    })
}

pub fn error_response(id: Option<u64>, code: BidiErrorCode, message: &str) -> Value {
    let mut response = json!({
        "type": "error",
        "error": code.as_str(),
        "message": message,
        "stacktrace": "",
    });
    if let Some(id) = id
        && let Some(response) = response.as_object_mut()
    {
        response.insert("id".to_owned(), json!(id));
    }
    response
}

pub fn bidi_response_from_devtools_result(id: u64, result: DevToolsCommandResult) -> Value {
    if matches!(
        result,
        DevToolsCommandResult::ElementClickPreparation(_)
            | DevToolsCommandResult::ElementClickDispatch(_)
    ) {
        return error_response(
            Some(id),
            BidiErrorCode::UnsupportedOperation,
            "element click results are internal to WebDriver Classic",
        );
    }
    if matches!(result, DevToolsCommandResult::GetNodeForLocation(_)) {
        return error_response(
            Some(id),
            BidiErrorCode::UnsupportedOperation,
            "DOM.getNodeForLocation results are not part of the WebDriver BiDi surface",
        );
    }
    if let DevToolsCommandResult::SetCookies(result) = &result
        && !result.success
    {
        return error_response(
            Some(id),
            BidiErrorCode::UnableToSetCookie,
            "cookie could not be set",
        );
    }
    success_response(id, bidi_result_payload_from_devtools_result(result))
}

pub fn bidi_response_from_devtools_error(id: u64, error: DevToolsError) -> Value {
    error_response(
        Some(id),
        bidi_error_code_from_devtools_error(&error),
        &error.message,
    )
}

fn bidi_result_payload_from_devtools_result(result: DevToolsCommandResult) -> Value {
    match result {
        DevToolsCommandResult::ElementClickPreparation(_)
        | DevToolsCommandResult::ElementClickDispatch(_) => {
            unreachable!("element click results have no WebDriver BiDi success projection")
        }
        DevToolsCommandResult::Empty | DevToolsCommandResult::TraverseHistory(_) => json!({}),
        DevToolsCommandResult::Navigate(result) => json!({
            "navigation": result
                .navigation_id
                .map(|navigation_id| Value::String(navigation_id.into_string()))
                .unwrap_or(Value::Null),
            "url": result.url,
        }),
        DevToolsCommandResult::GetNavigationHistory(result) => json!({
            "currentIndex": result.current_index,
            "entries": result
                .entries
                .into_iter()
                .map(|entry| {
                    json!({
                        "id": entry.id,
                        "url": entry.url,
                        "title": entry.title,
                        "transitionType": entry.transition_type,
                    })
                })
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::CreateTarget(result) => json!({
            "context": result.target_id.into_string(),
        }),
        DevToolsCommandResult::CloseTarget(_) => json!({}),
        DevToolsCommandResult::GetTargets(result) => json!({
            "contexts": result
                .targets
                .into_iter()
                .filter_map(bidi_browsing_context_info)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::ServiceWorkerLogs(result) => json!({
            "entries": result
                .entries
                .into_iter()
                .map(bidi_service_worker_log_entry)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::ClientWindows(result) => json!({
            "clientWindows": result
                .client_windows
                .into_iter()
                .map(bidi_client_window_info)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::ClientWindow(result) => {
            bidi_client_window_info(result.client_window)
        }
        DevToolsCommandResult::CreateBrowserContext(result) => json!({
            "userContext": result.browser_context_id.into_string(),
        }),
        DevToolsCommandResult::GetBrowserContexts(result) => json!({
            "userContexts": bidi_user_contexts_from_browser_contexts(result.browser_context_ids),
        }),
        DevToolsCommandResult::GetFrameTree(result) => json!({
            "contexts": bidi_browsing_context_infos_from_frame_tree_result(&result),
        }),
        DevToolsCommandResult::GetFrameTrees(result) => json!({
            "contexts": result
                .frame_trees
                .iter()
                .flat_map(bidi_browsing_context_infos_from_frame_tree_result)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::GetTargetInfo(result) => {
            bidi_browsing_context_info(result.target_info).unwrap_or_else(|| json!({}))
        }
        DevToolsCommandResult::GetCookies(result) => json!({
            "cookies": result
                .cookies
                .into_iter()
                .map(bidi_cookie_from_cdp_cookie)
                .collect::<Vec<_>>(),
            "partitionKey": {},
        }),
        DevToolsCommandResult::DeleteCookies(result) => json!({
            "partitionKey": result.partition_key,
        }),
        DevToolsCommandResult::SetCookies(result) => json!({
            "partitionKey": result.partition_key,
        }),
        DevToolsCommandResult::AddPreloadScript(result) => json!({
            "script": result.script_id.into_string(),
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
            "realms": bidi_script_realm_infos(result.realms),
        }),
        DevToolsCommandResult::Script(result) => match *result {
            DevToolsScriptResult::Value(value) => {
                let realm = value.realm.clone();
                let mut payload = json!({
                    "type": "success",
                    "result": bidi_remote_value_from_devtools(value),
                });
                if let Some(realm) = realm
                    && let Some(payload) = payload.as_object_mut()
                {
                    payload.insert("realm".to_owned(), json!(realm.into_string()));
                }
                payload
            }
            DevToolsScriptResult::Exception(exception) => {
                let details = json!({
                    "text": exception.text,
                    "lineNumber": exception.line_number.unwrap_or(0),
                    "columnNumber": exception.column_number.unwrap_or(0),
                    "stackTrace": exception
                        .stack_trace
                        .as_ref()
                        .map(bidi_stack_trace_from_devtools)
                        .unwrap_or_else(|| json!({"callFrames": []})),
                    "exception": exception
                        .value
                        .map(bidi_remote_value_from_devtools)
                        .unwrap_or_else(|| json!({"type": "undefined"})),
                });
                let mut payload = json!({
                    "type": "exception",
                    "exceptionDetails": details,
                });
                if let Some(realm) = exception.realm
                    && let Some(payload) = payload.as_object_mut()
                {
                    payload.insert("realm".to_owned(), json!(realm.into_string()));
                }
                payload
            }
        },
        DevToolsCommandResult::LocateNodes(result) => json!({
            "nodes": result
                .nodes
                .into_iter()
                .map(bidi_remote_value_from_devtools)
                .collect::<Vec<_>>(),
        }),
        DevToolsCommandResult::DescribeNode(result) => json!({
            "node": result.node,
        }),
        DevToolsCommandResult::GetFrameOwner(result) => json!({
            "nodeId": result.node_id,
            "backendNodeId": result.backend_node_id,
        }),
        DevToolsCommandResult::QuerySelector(result) => json!({
            "nodeIds": result.node_ids,
            "multiple": result.multiple,
        }),
        DevToolsCommandResult::ResolveNode(result) => json!({
            "object": result.object,
        }),
        DevToolsCommandResult::GetAttributes(result) => json!({
            "attributes": result
                .attributes
                .into_iter()
                .map(|attribute| json!({
                    "name": attribute.name,
                    "value": attribute.value,
                }))
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
        DevToolsCommandResult::GetOuterHtml(result) => json!({
            "outerHTML": result.outer_html,
        }),
        // The public projection rejects this CDP-only variant before reaching
        // the BiDi success mapper. Keep the arm explicit so adding a BiDi
        // command cannot silently expose a CDP response shape.
        DevToolsCommandResult::GetNodeForLocation(_) => unreachable!(
            "get-node-for-location results must be rejected before BiDi success projection"
        ),
        DevToolsCommandResult::DomGeometry(result) => json!({
            "quads": result
                .quads
                .into_iter()
                .map(|quad| quad.points)
                .collect::<Vec<_>>(),
            "width": result.width,
            "height": result.height,
        }),
        DevToolsCommandResult::LayoutMetrics(result) => json!({
            "layoutViewport": {
                "clientWidth": result.layout_viewport_width,
                "clientHeight": result.layout_viewport_height,
            },
            "contentSize": {
                "width": result.content_width,
                "height": result.content_height,
            },
        }),
        DevToolsCommandResult::JavaScriptDialog(result) => json!({
            "type": result.dialog_type,
            "message": result.message,
            "defaultValue": result.default_prompt,
        }),
        DevToolsCommandResult::CaptureScreenshot(result) => json!({
            "data": BASE64_STANDARD.encode(result.bytes.as_ref()),
        }),
    }
}

fn bidi_service_worker_log_entry(entry: RuntimeConsoleEvent) -> Value {
    let mut value = json!({
        "type": entry.console_type,
        "text": entry.text,
        "args": entry.args,
    });
    if let Some(object) = value.as_object_mut() {
        if let Some(target_id) = entry.target_id {
            object.insert("targetId".to_owned(), json!(target_id.into_string()));
        }
        if let Some(stack) = entry.stack {
            object.insert("stack".to_owned(), json!(stack));
        }
        if let Some(context_id) = entry
            .execution_context_id
            .filter(|execution_context_id| *execution_context_id > 0)
        {
            object.insert("executionContextId".to_owned(), json!(context_id));
        }
        if let Some(timestamp) = entry.timestamp {
            object.insert("timestamp".to_owned(), json!(timestamp));
        }
    }
    value
}

fn bidi_script_realm_infos(
    realms: Vec<moli_protocol::devtools_runtime::RuntimeExecutionContextEvent>,
) -> Vec<Value> {
    let mut realms = realms
        .into_iter()
        .filter_map(|realm| script_realm_info(&realm))
        .collect::<Vec<_>>();
    realms.sort_by(compare_bidi_script_realm_info);
    realms
}

fn compare_bidi_script_realm_info(left: &Value, right: &Value) -> std::cmp::Ordering {
    bidi_realm_string_field(left, "context")
        .cmp(bidi_realm_string_field(right, "context"))
        .then_with(|| bidi_window_realm_order(left).cmp(&bidi_window_realm_order(right)))
        .then_with(|| {
            bidi_realm_string_field(left, "sandbox").cmp(bidi_realm_string_field(right, "sandbox"))
        })
        .then_with(|| {
            bidi_realm_string_field(left, "type").cmp(bidi_realm_string_field(right, "type"))
        })
        .then_with(|| {
            bidi_realm_string_field(left, "realm").cmp(bidi_realm_string_field(right, "realm"))
        })
}

fn bidi_window_realm_order(realm: &Value) -> u8 {
    if realm.get("type").and_then(Value::as_str) != Some("window") {
        return 2;
    }
    if realm.get("sandbox").is_some() { 1 } else { 0 }
}

fn bidi_realm_string_field<'a>(realm: &'a Value, field: &str) -> &'a str {
    realm.get(field).and_then(Value::as_str).unwrap_or("")
}

fn bidi_client_window_info(
    window: moli_protocol::devtools_runtime::DevToolsClientWindowInfo,
) -> Value {
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

fn bidi_user_contexts_from_browser_contexts(
    browser_context_ids: Vec<DevToolsBrowserContextId>,
) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut user_contexts = Vec::new();
    push_bidi_user_context(&mut user_contexts, &mut seen, DEFAULT_BIDI_USER_CONTEXT);
    for browser_context_id in browser_context_ids {
        push_bidi_user_context(
            &mut user_contexts,
            &mut seen,
            bidi_user_context_from_browser_context_id(Some(browser_context_id.as_str())),
        );
    }
    user_contexts
}

fn push_bidi_user_context(
    user_contexts: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    user_context: &str,
) {
    if seen.insert(user_context.to_owned()) {
        user_contexts.push(json!({ "userContext": user_context }));
    }
}

fn bidi_error_code_from_devtools_error(error: &DevToolsError) -> BidiErrorCode {
    match error.kind {
        DevToolsErrorKind::InvalidArgument => BidiErrorCode::InvalidArgument,
        DevToolsErrorKind::InvalidSelector => BidiErrorCode::InvalidSelector,
        DevToolsErrorKind::NoSuchAlert => BidiErrorCode::NoSuchAlert,
        DevToolsErrorKind::NoSuchHandle => BidiErrorCode::NoSuchHandle,
        DevToolsErrorKind::NoSuchHistoryEntry => BidiErrorCode::NoSuchHistoryEntry,
        DevToolsErrorKind::NoSuchNode => BidiErrorCode::NoSuchNode,
        DevToolsErrorKind::NoSuchNetworkCollector => BidiErrorCode::NoSuchNetworkCollector,
        DevToolsErrorKind::NoSuchNetworkData => BidiErrorCode::NoSuchNetworkData,
        DevToolsErrorKind::NoSuchRequest => BidiErrorCode::NoSuchRequest,
        DevToolsErrorKind::NoSuchScript => BidiErrorCode::NoSuchScript,
        DevToolsErrorKind::NoSuchTarget if error.message == "UnknownBrowserContextId" => {
            BidiErrorCode::NoSuchUserContext
        }
        DevToolsErrorKind::NoSuchTarget => BidiErrorCode::NoSuchFrame,
        DevToolsErrorKind::NoSuchSession => BidiErrorCode::InvalidSessionId,
        DevToolsErrorKind::UnableToCaptureScreen => BidiErrorCode::UnableToCaptureScreen,
        DevToolsErrorKind::UnableToSetFileInput => BidiErrorCode::UnableToSetFileInput,
        DevToolsErrorKind::Unsupported => BidiErrorCode::UnsupportedOperation,
        DevToolsErrorKind::NavigationChangingDocument
        | DevToolsErrorKind::Timeout
        | DevToolsErrorKind::Internal => BidiErrorCode::UnknownError,
    }
}
