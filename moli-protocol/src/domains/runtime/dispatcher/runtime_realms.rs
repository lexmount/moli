use super::*;

pub(super) async fn execute_devtools_release_objects_command_async(
    conn: &mut CdpConnection,
    command: DevToolsReleaseObjectsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let target =
        devtools_runtime_target_async(conn, &DevToolsCommand::ReleaseObjects(command.clone()))
            .await?;
    let target_realm = devtools_realm_id_for_runtime_target_async(conn, &target).await;
    let owner = CommandOwnerScope::for_route(target.route);
    release_devtools_objects_for_owner_async(conn, &owner, &command.handles, target_realm.as_ref())
        .await?;
    Ok(DevToolsCommandResult::Empty)
}

pub(super) async fn release_devtools_objects_for_owner_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    handles: &[DevToolsRemoteHandleId],
    target_realm: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    for handle in handles {
        let object_id = handle.as_str().to_owned();
        if !conn.runtime_remote_object_id_known_for_owner(owner, &object_id) {
            continue;
        }
        if let Some(target_realm) = target_realm
            && let Some(owner_realm) = conn.runtime_remote_object_realm_for_owner(owner, &object_id)
            && owner_realm != target_realm.as_str()
        {
            continue;
        }
        let params = json!({ "objectId": object_id });
        let command_id = conn.next_internal_runtime_command_id();
        let raw_json = runtime_inspector_command_json(command_id, "Runtime.releaseObject", &params);
        let response = dispatch_runtime_inspector_command_response_for_owner_async(
            conn, owner, raw_json, command_id,
        )
        .await?;
        if let BackgroundCommandResponsePayload::Error { code, message, .. } = response {
            let error = devtools_error_from_cdp_error_parts(Some(i64::from(code)), &message);
            if matches!(error.kind, DevToolsErrorKind::NoSuchHandle) {
                conn.unregister_runtime_remote_object_ids_for_owner(owner, &[object_id]);
                continue;
            }
            return Err(error);
        }
        conn.unregister_runtime_remote_object_ids_for_owner(owner, &[object_id]);
    }
    Ok(())
}

pub(super) async fn dispatch_runtime_inspector_command_response_for_owner_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    raw_json: String,
    command_id: u64,
) -> Result<BackgroundCommandResponsePayload, DevToolsError> {
    let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json)
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    let pending = conn
        .start_runtime_protocol_message_for_owner_with_deferred_response(
            owner, descriptor, command_id,
        )
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    let mut completed = pending
        .wait()
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    let response_rx = completed.take_deferred_response_receiver().ok_or_else(|| {
        DevToolsError::new(
            DevToolsErrorKind::Internal,
            "MissingRuntimeInspectorResponseReceiver",
        )
    })?;
    let output = conn
        .complete_runtime_protocol_message_async(completed)
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?;
    if let Some(message) = output
        .as_ref()
        .and_then(|output| renderer_command_turn_frontend_protocol_response(output, command_id))
    {
        return Ok(BackgroundCommandResponsePayload::from_runtime_inspector_message(message));
    }
    let response = RuntimeInspectorResponseReady::for_owner(
        command_id,
        owner,
        response_rx
            .await
            .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
    );
    let Some(mut response) = conn.resolve_runtime_inspector_response_ready(response) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "RuntimeInspectorResponseMissingCorrelation",
        ));
    };
    if response
        .renderer_agent_attachment_id()
        .is_some_and(|attachment_id| {
            conn.current_renderer_agent_attachment_id_for_owner(owner) != Some(attachment_id)
        })
    {
        response.replace_with_error("Execution context was destroyed by navigation");
    }
    Ok(response.into_command_response_payload())
}

pub(super) async fn execute_devtools_get_realms_command_async(
    conn: &mut CdpConnection,
    command: DevToolsGetRealmsCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let requested_target_id = command
        .context
        .target_id
        .as_ref()
        .map(|target_id| target_id.as_str().to_owned());
    let routes = devtools_get_realms_routes(conn, command.context.target_id.as_ref())?;
    let requested_target_is_worker_target = routes.iter().any(|route| {
        matches!(
            route,
            CdpSessionRoute::SharedWorkerTarget { .. }
                | CdpSessionRoute::DedicatedWorkerTarget { .. }
                | CdpSessionRoute::ServiceWorkerTarget { .. }
        )
    });
    let mut realms = Vec::new();
    for route in routes {
        realms.extend(devtools_realms_for_route_async(conn, route).await?);
    }
    if let Some(requested_target_id) = requested_target_id.as_deref()
        && !requested_target_is_worker_target
    {
        realms.retain(|realm| {
            realm
                .frame_id
                .as_ref()
                .is_some_and(|frame_id| frame_id.as_str() == requested_target_id)
        });
        if realms.is_empty() {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "NoSuchTarget",
            ));
        }
    }
    if let Some(realm_type) = command.realm_type.as_deref() {
        realms.retain(|realm| devtools_realm_type(realm.context_type.as_deref()) == realm_type);
    }
    dedup_devtools_realms(&mut realms);
    Ok(DevToolsCommandResult::Realms(DevToolsGetRealmsResult {
        realms,
    }))
}

pub(super) fn dedup_devtools_realms(realms: &mut Vec<RuntimeExecutionContextEvent>) {
    let mut seen = HashSet::new();
    realms.retain(|realm| {
        let key = if let Some(realm_id) = realm.realm_id.as_ref() {
            format!(
                "realm:{}:{}",
                realm
                    .frame_id
                    .as_ref()
                    .map(|frame_id| frame_id.as_str())
                    .unwrap_or_default(),
                realm_id.as_str()
            )
        } else {
            format!(
                "context:{}:{}:{}",
                realm
                    .frame_id
                    .as_ref()
                    .map(|frame_id| frame_id.as_str())
                    .unwrap_or_default(),
                realm
                    .context_id
                    .map(|context_id| context_id.to_string())
                    .unwrap_or_default(),
                realm.context_type.as_deref().unwrap_or_default()
            )
        };
        seen.insert(key)
    });
}

pub(super) fn devtools_get_realms_routes(
    conn: &CdpConnection,
    target_id: Option<&crate::devtools_runtime::DevToolsTargetId>,
) -> Result<Vec<CdpSessionRoute>, DevToolsError> {
    if let Some(target_id) = target_id {
        if let Some(route) = conn
            .target_session_route_for_target_id(target_id.as_str())
            .or_else(|| conn.target_session_route_for_child_frame_id(target_id.as_str()))
        {
            return Ok(vec![route]);
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    }

    let mut routes = Vec::new();
    for target_info in conn
        .browser_contexts()
        .flat_map(crate::conn::BrowserContext::devtools_target_infos)
    {
        if !matches!(
            target_info.kind,
            crate::devtools_runtime::DevToolsTargetKind::Page
                | crate::devtools_runtime::DevToolsTargetKind::Frame
                | crate::devtools_runtime::DevToolsTargetKind::SharedWorker
                | crate::devtools_runtime::DevToolsTargetKind::Worker
                | crate::devtools_runtime::DevToolsTargetKind::ServiceWorker
        ) {
            continue;
        }
        let Some(target_id) = target_info.target_id else {
            continue;
        };
        if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str()) {
            routes.push(route);
        }
    }
    if routes.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "NoSuchTarget",
        ));
    }
    Ok(routes)
}

pub(super) async fn devtools_realms_for_route_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
) -> Result<Vec<RuntimeExecutionContextEvent>, DevToolsError> {
    if let CdpSessionRoute::SharedWorkerTarget {
        browser_context_id,
        target_id,
    } = &route
    {
        let target = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.shared_worker_target(target_id))
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok(shared_worker_target_runtime_realm(target)
            .into_iter()
            .collect());
    }
    if let CdpSessionRoute::DedicatedWorkerTarget {
        browser_context_id,
        target_id,
    } = &route
    {
        let target = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.dedicated_worker_target(target_id))
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok(dedicated_worker_target_runtime_realm(target)
            .into_iter()
            .collect());
    }
    if let CdpSessionRoute::ServiceWorkerTarget {
        browser_context_id,
        target_id,
    } = &route
    {
        let target = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.service_worker_target(target_id))
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok(service_worker_target_runtime_realm(target)
            .into_iter()
            .collect());
    }
    let owner = CommandOwnerScope::for_route(route);
    conn.runtime_realm_inventory_for_owner_async(&owner)
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))
}

pub(super) fn shared_worker_target_runtime_realm(
    target: &crate::conn::SharedWorkerTargetState,
) -> Option<RuntimeExecutionContextEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let origin = url::Url::parse(&target.url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
        context_id: Some(context_id),
        realm_id: Some(DevToolsRealmId::from(format!(
            "shared-worker-{}",
            target.target_id
        ))),
        frame_id: None,
        origin: Some(origin),
        name: Some(target.name.clone()),
        is_default: Some(true),
        context_type: Some("shared-worker".to_owned()),
        grant_universal_access: None,
    })
}

pub(super) fn dedicated_worker_target_runtime_realm(
    target: &crate::conn::DedicatedWorkerTargetState,
) -> Option<RuntimeExecutionContextEvent> {
    let mut realm = shared_worker_target_runtime_realm(&target.inner)?;
    realm.realm_id = Some(DevToolsRealmId::from(format!(
        "dedicated-worker-{}",
        target.target_id
    )));
    realm.context_type = Some("worker".to_owned());
    Some(realm)
}

pub(super) fn service_worker_target_runtime_realm(
    target: &crate::conn::ServiceWorkerTargetState,
) -> Option<RuntimeExecutionContextEvent> {
    let context_id = target.real_runtime_execution_context_id()?;
    let origin = url::Url::parse(&target.script_url)
        .ok()
        .map(|url| moli_url::origin_ascii_serialization(&url))
        .unwrap_or_else(|| "null".to_owned());
    Some(RuntimeExecutionContextEvent {
        target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
        context_id: Some(context_id),
        realm_id: Some(DevToolsRealmId::from(format!(
            "service-worker-{}",
            target.target_id
        ))),
        frame_id: None,
        origin: Some(origin),
        name: Some(String::new()),
        is_default: Some(true),
        context_type: Some("service-worker".to_owned()),
        grant_universal_access: None,
    })
}

pub(super) fn devtools_realm_type(context_type: Option<&str>) -> &'static str {
    match context_type {
        Some("dedicated-worker") => "dedicated-worker",
        Some("shared-worker") => "shared-worker",
        Some("service-worker") => "service-worker",
        Some("paint-worklet") => "paint-worklet",
        Some("audio-worklet") => "audio-worklet",
        Some("worklet") => "worklet",
        Some("worker") => "worker",
        _ => "window",
    }
}

pub(super) fn devtools_remote_value_from_cdp(
    remote: &Value,
    retain_handle: bool,
    realm: Option<DevToolsRealmId>,
) -> DevToolsRemoteValue {
    let object_id = remote.get("objectId").and_then(Value::as_str);
    DevToolsRemoteValue {
        value: cdp_remote_object_value(remote),
        handle: retain_handle
            .then(|| object_id.map(DevToolsRemoteHandleId::from))
            .flatten(),
        shared_id: object_id.map(DevToolsRemoteHandleId::from),
        node_id: None,
        backend_node_id: None,
        window_context: None,
        realm,
        remote_type: remote
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        remote_subtype: remote
            .get("subtype")
            .and_then(Value::as_str)
            .map(str::to_owned),
        unserializable_value: remote
            .get("unserializableValue")
            .and_then(Value::as_str)
            .map(str::to_owned),
        description: remote
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        class_name: remote
            .get("className")
            .and_then(Value::as_str)
            .map(str::to_owned),
        deep_serialized_value: remote.get("deepSerializedValue").cloned(),
        node_value: None,
    }
}

pub(super) fn devtools_script_exception_from_cdp(
    exception_details: &Value,
    retain_handle: bool,
) -> DevToolsScriptException {
    let exception = exception_details.get("exception");
    let text = exception_details
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| {
            exception.and_then(|exception| exception.get("description").and_then(Value::as_str))
        })
        .unwrap_or("JavaScript exception")
        .to_owned();
    DevToolsScriptException {
        exception_id: exception_details.get("exceptionId").and_then(Value::as_u64),
        script_id: exception_details
            .get("scriptId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        text,
        value: exception.map(|remote| devtools_remote_value_from_cdp(remote, retain_handle, None)),
        realm: exception_details
            .get("executionContextUniqueId")
            .and_then(Value::as_str)
            .map(DevToolsRealmId::from)
            .or_else(|| {
                exception_details
                    .get("executionContextId")
                    .and_then(Value::as_i64)
                    .map(|id| DevToolsRealmId::from(id.to_string()))
            }),
        line_number: exception_details.get("lineNumber").and_then(Value::as_u64),
        column_number: exception_details
            .get("columnNumber")
            .and_then(Value::as_u64),
        stack_trace: exception_details
            .get("stackTrace")
            .and_then(crate::devtools_runtime::DevToolsStackTrace::from_cdp_value),
    }
}

pub(super) fn cdp_remote_object_value(remote: &Value) -> Value {
    if let Some(value) = remote.get("value") {
        return value.clone();
    }
    if remote
        .get("subtype")
        .and_then(Value::as_str)
        .is_some_and(|subtype| subtype == "null")
    {
        return Value::Null;
    }
    if let Some(unserializable) = remote.get("unserializableValue").and_then(Value::as_str) {
        return Value::String(unserializable.to_owned());
    }
    match remote.get("type").and_then(Value::as_str) {
        Some("undefined") => Value::Null,
        Some("boolean") => Value::Bool(false),
        Some("number") => json!(0),
        Some("string") => Value::String(String::new()),
        Some("object") | Some("function") => json!({}),
        _ => Value::Null,
    }
}
