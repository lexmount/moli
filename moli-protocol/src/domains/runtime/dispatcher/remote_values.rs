use super::*;

pub(super) fn cdp_value_from_devtools_argument(argument: &Value) -> Option<Value> {
    let Some(map) = argument.as_object() else {
        return Some(argument.clone());
    };
    if bidi_remote_reference_object_id(map).is_some() {
        return None;
    }
    match map.get("type").and_then(Value::as_str) {
        Some("null") => Some(Value::Null),
        Some("array") => map.get("value")?.as_array().and_then(|values| {
            values
                .iter()
                .map(cdp_value_from_devtools_argument)
                .collect::<Option<Vec<_>>>()
                .map(Value::Array)
        }),
        Some("object") => map.get("value")?.as_array().and_then(|entries| {
            let mut object = serde_json::Map::new();
            for entry in entries {
                let pair = entry.as_array()?;
                let [key, value] = pair.as_slice() else {
                    return None;
                };
                object.insert(
                    key.as_str()?.to_owned(),
                    cdp_value_from_devtools_argument(value)?,
                );
            }
            Some(Value::Object(object))
        }),
        Some("string" | "number" | "boolean" | "bigint" | "date") => map.get("value").cloned(),
        None => map.get("value").cloned(),
        _ => None,
    }
}

pub(super) fn devtools_call_function_remote_object_ids(
    command: &DevToolsCallFunctionCommand,
) -> Vec<String> {
    let mut object_ids = Vec::new();
    if let Some(object_id) = command.object_id.as_ref() {
        object_ids.push(object_id.as_str().to_owned());
    }
    if let Some(this_parameter) = &command.this_parameter {
        collect_devtools_remote_object_ids(this_parameter, &mut object_ids);
    }
    for argument in &command.arguments {
        collect_devtools_remote_object_ids(argument, &mut object_ids);
    }
    object_ids.sort();
    object_ids.dedup();
    object_ids
}

pub(super) fn devtools_call_function_remote_references(
    command: &DevToolsCallFunctionCommand,
) -> Vec<RuntimeRemoteReference> {
    let mut references = Vec::new();
    if let Some(object_id) = command.object_id.as_ref() {
        references.push(RuntimeRemoteReference {
            object_id: object_id.as_str().to_owned(),
            kind: RuntimeRemoteReferenceKind::Object,
        });
    }
    if let Some(this_parameter) = &command.this_parameter {
        collect_devtools_remote_references(this_parameter, &mut references);
    }
    for argument in &command.arguments {
        collect_devtools_remote_references(argument, &mut references);
    }
    references.sort();
    references.dedup();
    references
}

pub(super) fn collect_devtools_remote_object_ids(value: &Value, out: &mut Vec<String>) {
    let mut stack = vec![(value, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)];
    while let Some((value, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                for key in ["objectId", "promiseObjectId"] {
                    if let Some(object_id) = map.get(key).and_then(Value::as_str) {
                        out.push(object_id.to_owned());
                    }
                }
                if let Some(object_id) = bidi_remote_reference_object_id(map) {
                    out.push(object_id.to_owned());
                }
                for child in map.values() {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Array(values) => {
                for child in values {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

pub(super) fn collect_devtools_remote_references(
    value: &Value,
    out: &mut Vec<RuntimeRemoteReference>,
) {
    let mut stack = vec![(value, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH)];
    while let Some((value, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                let mut found_cdp_remote_reference = false;
                for key in ["objectId", "promiseObjectId"] {
                    if let Some(object_id) = map.get(key).and_then(Value::as_str) {
                        out.push(RuntimeRemoteReference {
                            object_id: object_id.to_owned(),
                            kind: RuntimeRemoteReferenceKind::Object,
                        });
                        found_cdp_remote_reference = true;
                    }
                }
                if found_cdp_remote_reference {
                    continue;
                }
                if let Some(shared_id) = map.get("sharedId").and_then(Value::as_str) {
                    out.push(RuntimeRemoteReference {
                        object_id: shared_id.to_owned(),
                        kind: if map.get("type").and_then(Value::as_str) == Some("node") {
                            RuntimeRemoteReferenceKind::Node
                        } else {
                            RuntimeRemoteReferenceKind::Object
                        },
                    });
                    continue;
                } else if let Some(handle) = map.get("handle").and_then(Value::as_str) {
                    out.push(RuntimeRemoteReference {
                        object_id: handle.to_owned(),
                        kind: RuntimeRemoteReferenceKind::Object,
                    });
                    continue;
                }
                for child in map.values() {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Array(values) => {
                for child in values {
                    stack.push((child, next_tree_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

pub(super) fn devtools_script_result_from_response(
    response: Value,
    result_ownership: DevToolsResultOwnership,
    target_realm: Option<DevToolsRealmId>,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if let Some(error) = response.get("error") {
        return Err(devtools_error_from_cdp_error_value(error));
    }
    let result = response.get("result").unwrap_or(&Value::Null);
    if let Some(exception_details) = result.get("exceptionDetails") {
        let mut exception = devtools_script_exception_from_cdp(
            exception_details,
            matches!(result_ownership, DevToolsResultOwnership::Root),
        );
        if exception.realm.is_none() {
            exception.realm = target_realm;
        }
        return Ok(DevToolsCommandResult::Script(Box::new(
            DevToolsScriptResult::Exception(exception),
        )));
    }
    let remote = result.get("result").unwrap_or(&Value::Null);
    Ok(DevToolsCommandResult::Script(Box::new(
        DevToolsScriptResult::Value(devtools_remote_value_from_cdp(
            remote,
            matches!(result_ownership, DevToolsResultOwnership::Root),
            target_realm,
        )),
    )))
}

pub(super) fn devtools_empty_result_from_response(
    response: Value,
) -> Result<DevToolsCommandResult, DevToolsError> {
    if let Some(error) = response.get("error") {
        return Err(devtools_error_from_cdp_error_value(error));
    }
    Ok(DevToolsCommandResult::Empty)
}

pub(super) fn validate_protocol_neutral_runtime_handle_realms(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    command: &DevToolsCommand,
    target_realm: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    let DevToolsCommand::CallFunction(command) = command else {
        return Ok(());
    };
    if command.context.protocol != DevToolsProtocol::WebDriverBidi {
        return Ok(());
    }

    let references = devtools_call_function_remote_references(command);

    for reference in references {
        if !conn.runtime_remote_object_id_known_for_owner(owner, &reference.object_id) {
            return Err(match reference.kind {
                RuntimeRemoteReferenceKind::Node => DevToolsError::new(
                    DevToolsErrorKind::NoSuchNode,
                    "Could not find node with given id",
                ),
                RuntimeRemoteReferenceKind::Object => DevToolsError::new(
                    DevToolsErrorKind::NoSuchHandle,
                    "Cannot find object with given id",
                ),
            });
        }
        if reference.kind == RuntimeRemoteReferenceKind::Object
            && let Some(target_realm) = target_realm
            && let Some(owner_realm) =
                conn.runtime_remote_object_realm_for_owner(owner, &reference.object_id)
            && owner_realm != target_realm.as_str()
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchHandle,
                "Cannot find object with given id",
            ));
        }
    }
    Ok(())
}

pub(super) fn register_devtools_script_result_remote_object_realm(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    result: &DevToolsCommandResult,
    realm_id: Option<&DevToolsRealmId>,
) {
    let (DevToolsCommandResult::Script(result), Some(realm_id)) = (result, realm_id) else {
        return;
    };
    let DevToolsScriptResult::Value(value) = result.as_ref() else {
        return;
    };
    let Some(remote_object_id) = value.handle.as_ref().or(value.shared_id.as_ref()) else {
        return;
    };
    if let Some(object_id) =
        conn.runtime_remote_object_alias_for_owner(owner, remote_object_id.as_str())
    {
        conn.register_runtime_remote_object_alias_for_owner_with_realm(
            owner,
            remote_object_id.as_str().to_owned(),
            object_id,
            realm_id.as_str(),
        );
        return;
    }
    conn.register_runtime_remote_object_ids_for_owner_with_realm(
        owner,
        vec![remote_object_id.as_str().to_owned()],
        realm_id.as_str(),
    );
}

pub(super) fn register_devtools_script_result_remote_object(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    result: &DevToolsCommandResult,
) {
    let Some(value) = devtools_script_result_remote_value(result) else {
        return;
    };
    let Some(remote_object_id) = value.handle.as_ref().or(value.shared_id.as_ref()) else {
        return;
    };
    conn.register_runtime_remote_object_ids_from_value_for_owner(
        owner,
        &json!({ "objectId": remote_object_id.as_str() }),
    );
}

pub(super) fn materialize_devtools_script_window_remote_value(
    result: &mut DevToolsCommandResult,
    target: &DevToolsRuntimeTarget,
) {
    let Some(target_window_context_id) = target.window_context_id.clone() else {
        return;
    };
    let Some(candidate) = devtools_window_remote_candidate(result) else {
        return;
    };
    let window_context_id = if let Some(window_remote) =
        deep_serialized_bidi_window_remote_result(candidate.deep_serialized_value.as_ref())
    {
        match window_remote {
            BidiWindowRemoteResult::TargetWindow => Some(target_window_context_id),
            BidiWindowRemoteResult::Context(window_context_id) => Some(window_context_id),
        }
    } else {
        None
    };
    let Some(window_context_id) = window_context_id else {
        return;
    };
    if let Some(value) = devtools_script_result_remote_value_mut(result) {
        value.window_context = Some(Box::new(window_context_id));
    }
}

pub(super) fn deep_serialized_bidi_window_remote_result(
    value: Option<&Value>,
) -> Option<BidiWindowRemoteResult> {
    let value = value?;
    if value.get("type").and_then(Value::as_str) == Some("window") {
        return value
            .get("value")
            .and_then(|value| value.get("context"))
            .and_then(Value::as_str)
            .map(|context| BidiWindowRemoteResult::Context(DevToolsTargetId::from(context)));
    }
    let properties = value.get("value").and_then(Value::as_array)?;
    let marker = deep_serialized_property(properties, "__moliBidiRemoteValue")?;
    if deep_serialized_bool_value(marker) != Some(true) {
        return None;
    }
    let type_value = deep_serialized_property(properties, "type")?;
    if deep_serialized_string_value(type_value).as_deref() != Some("window") {
        return None;
    }
    if let Some(target_window) = deep_serialized_property(properties, "targetWindow")
        && deep_serialized_bool_value(target_window) == Some(true)
    {
        return Some(BidiWindowRemoteResult::TargetWindow);
    }
    let context = deep_serialized_property(properties, "context")?;
    deep_serialized_string_value(context)
        .map(|context| BidiWindowRemoteResult::Context(DevToolsTargetId::from(context)))
}

pub(super) fn deep_serialized_bool_value(value: &Value) -> Option<bool> {
    let object = value.as_object()?;
    (object.get("type").and_then(Value::as_str) == Some("boolean"))
        .then(|| object.get("value")?.as_bool())
        .flatten()
}

pub(super) fn deep_serialized_string_value(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    (object.get("type").and_then(Value::as_str) == Some("string"))
        .then(|| object.get("value")?.as_str().map(str::to_owned))
        .flatten()
}

pub(super) fn devtools_window_remote_candidate(
    result: &DevToolsCommandResult,
) -> Option<DevToolsWindowRemoteCandidate> {
    let value = devtools_script_result_remote_value(result)?;
    Some(DevToolsWindowRemoteCandidate {
        deep_serialized_value: value.deep_serialized_value.clone(),
    })
}

pub(super) async fn devtools_probe_remote_value_async(
    conn: &mut CdpConnection,
    target: DevToolsRuntimeTarget,
    command: DevToolsCommand,
) -> Result<Option<DevToolsRemoteValue>, DevToolsError> {
    let result_ownership = devtools_runtime_result_ownership(&command);
    let internal_command_id = conn.next_internal_runtime_command_id();
    let mut step =
        start_protocol_neutral_runtime_command(conn, target, command, internal_command_id).await;
    loop {
        match step {
            RuntimeCommandTaskStep::Complete(plan) => {
                let (response, _events) = plan
                    .into_runtime_inspector_response_and_background_events(
                        internal_command_id,
                        None,
                    );
                let Some(response) = response else {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "MissingDevToolsCommandResult",
                    ));
                };
                let result =
                    devtools_script_result_from_response(response, result_ownership, None)?;
                let DevToolsCommandResult::Script(result) = result else {
                    return Ok(None);
                };
                let DevToolsScriptResult::Value(value) = *result else {
                    return Ok(None);
                };
                return Ok(Some(value));
            }
            RuntimeCommandTaskStep::Pending(pending) => {
                let completed = pending.wait().await;
                step = complete_pending_runtime_command(conn, completed).await;
            }
        }
    }
}

pub(super) async fn materialize_devtools_script_deep_serialized_root_value_async(
    conn: &mut CdpConnection,
    result: &mut DevToolsCommandResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
) {
    let Some(serialization_options) = serialization_options else {
        return;
    };
    let Some(remote_value) = devtools_script_result_remote_value(result) else {
        return;
    };
    if remote_value.deep_serialized_value.is_some() {
        return;
    }
    if !matches!(remote_value.remote_type.as_deref(), Some("object")) {
        return;
    }
    let Some(root_object_id) = remote_value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };

    let command = DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(root_object_id.clone())),
        this_parameter: None,
        function_declaration: "function() { return this; }".to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: Some(serialization_options.clone()),
    });
    let deep_serialized_value =
        match devtools_probe_remote_value_async(conn, target.clone(), command).await {
            Ok(Some(readback)) => readback.deep_serialized_value,
            Ok(None) => None,
            Err(error) => {
                tracing::debug!(
                    %root_object_id,
                    ?error,
                    "failed to materialize root BiDi deep serialized value"
                );
                None
            }
        };
    let Some(deep_serialized_value) = deep_serialized_value else {
        return;
    };
    if let Some(remote_value) = devtools_script_result_remote_value_mut(result) {
        remote_value.deep_serialized_value = Some(deep_serialized_value);
    }
}

pub(super) fn devtools_script_result_remote_value(
    result: &DevToolsCommandResult,
) -> Option<&DevToolsRemoteValue> {
    let DevToolsCommandResult::Script(result) = result else {
        return None;
    };
    match result.as_ref() {
        DevToolsScriptResult::Value(value) => Some(value),
        DevToolsScriptResult::Exception(exception) => exception.value.as_ref(),
    }
}

pub(super) fn devtools_script_result_remote_value_mut(
    result: &mut DevToolsCommandResult,
) -> Option<&mut DevToolsRemoteValue> {
    let DevToolsCommandResult::Script(result) = result else {
        return None;
    };
    match result.as_mut() {
        DevToolsScriptResult::Value(value) => Some(value),
        DevToolsScriptResult::Exception(exception) => exception.value.as_mut(),
    }
}

pub(super) fn bidi_script_message_serialization_options_from_value(
    value: &Value,
) -> Result<DevToolsSerializationOptions, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "InvalidChannelArgument".to_owned())?;
    let max_object_depth = match object.get("maxObjectDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| "InvalidChannelArgument".to_owned())?,
        ),
    };
    let max_dom_depth = match object.get("maxDomDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| "InvalidChannelArgument".to_owned())?,
        ),
    };
    let include_shadow_tree = match object.get("includeShadowTree") {
        None => None,
        Some(Value::String(value)) if matches!(value.as_str(), "none" | "open" | "all") => {
            Some(value.clone())
        }
        Some(_) => return Err("InvalidChannelArgument".to_owned()),
    };
    Ok(DevToolsSerializationOptions {
        max_object_depth,
        max_dom_depth,
        include_shadow_tree,
    })
}

pub(super) async fn devtools_probe_value_async(
    conn: &mut CdpConnection,
    target: DevToolsRuntimeTarget,
    command: DevToolsCommand,
) -> Result<Option<Value>, DevToolsError> {
    let Some(value) = devtools_probe_remote_value_async(conn, target, command).await? else {
        return Ok(None);
    };
    Ok(Some(value.value))
}
