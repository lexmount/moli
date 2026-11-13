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

pub(super) fn devtools_script_result_from_response(
    response: Value,
    result_ownership: DevToolsResultOwnership,
    target_realm: Option<DevToolsRealmId>,
) -> Result<AutomationResult, DevToolsError> {
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
        return Ok(AutomationResult::Script(Box::new(
            DevToolsScriptResult::Exception(exception),
        )));
    }
    let remote = result.get("result").unwrap_or(&Value::Null);
    Ok(AutomationResult::Script(Box::new(
        DevToolsScriptResult::Value(devtools_remote_value_from_cdp(
            remote,
            matches!(result_ownership, DevToolsResultOwnership::Root),
            target_realm,
        )),
    )))
}

pub(super) fn devtools_empty_result_from_response(
    response: Value,
) -> Result<AutomationResult, DevToolsError> {
    if let Some(error) = response.get("error") {
        return Err(devtools_error_from_cdp_error_value(error));
    }
    Ok(AutomationResult::Empty)
}

pub(crate) fn validate_protocol_neutral_node_references(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    command: &AutomationCommand,
) -> Result<(), DevToolsError> {
    let AutomationCommand::CallFunction(command) = command else {
        return Ok(());
    };
    if command.context.protocol != FrontendProtocol::WebDriverBidi {
        return Ok(());
    }
    let mut stack = command
        .this_parameter
        .iter()
        .chain(command.arguments.iter())
        .collect::<Vec<_>>();
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                if let Some(shared_id) = map.get("sharedId").and_then(Value::as_str) {
                    if !is_webdriver_bidi_node_shared_id(shared_id)
                        && conn.bidi_node_alias(owner, shared_id).is_none()
                    {
                        return Err(DevToolsError::new(
                            DevToolsErrorKind::NoSuchNode,
                            "Could not find node with given id",
                        ));
                    }
                } else if !map.contains_key("handle") {
                    stack.extend(map.values());
                }
            }
            Value::Array(values) => stack.extend(values.iter()),
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn resolve_protocol_neutral_runtime_handles(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    command: &mut AutomationCommand,
    target_realm: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    let AutomationCommand::CallFunction(command) = command else {
        return Ok(());
    };
    if command.context.protocol != FrontendProtocol::WebDriverBidi {
        return Ok(());
    }
    let resolve = |handle: &str| {
        conn.resolve_bidi_handle(owner, handle, target_realm.map(DevToolsRealmId::as_str))
            .ok_or_else(|| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchHandle,
                    "Cannot find object with given id",
                )
            })
    };
    if let Some(handle) = &mut command.object_id {
        *handle = resolve(handle.as_str())?.into();
    }
    let mut stack = command
        .this_parameter
        .iter_mut()
        .chain(command.arguments.iter_mut())
        .collect::<Vec<_>>();
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                // sharedId resolution is a separate Document/node operation;
                // it must retain stale-node and cross-realm remapping semantics.
                if let Some(shared) = map.get_mut("sharedId") {
                    if let Some(native) = shared
                        .as_str()
                        .and_then(|id| conn.bidi_node_alias(owner, id))
                    {
                        *shared = Value::String(native);
                    }
                    continue;
                }
                if let Some(handle) = map.get_mut("handle") {
                    let Some(id) = handle.as_str() else {
                        continue;
                    };
                    *handle = Value::String(resolve(id)?);
                } else {
                    stack.extend(map.values_mut());
                }
            }
            Value::Array(values) => stack.extend(values.iter_mut()),
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn export_devtools_script_result_handles(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    result: &mut AutomationResult,
    realm: Option<&DevToolsRealmId>,
    group: Option<&str>,
) {
    let Some(value) = devtools_script_result_remote_value_mut(result) else {
        return;
    };
    let Some(realm) = realm.or(value.realm.as_ref()) else {
        return;
    };
    if value.remote_subtype.as_deref() == Some("node")
        && let Some(shared) = &mut value.shared_id
        && !is_webdriver_bidi_node_shared_id(shared.as_str())
        && conn.bidi_node_alias(owner, shared.as_str()).is_none()
    {
        *shared = conn
            .export_bidi_node_reference(owner, shared.as_str().to_owned(), realm.as_str(), group)
            .into();
    }
    if let Some(native) = value.handle.take() {
        value.handle = conn
            .export_bidi_handle(owner, native.as_str().to_owned(), realm.as_str(), group)
            .map(Into::into);
    }
}

pub(super) fn materialize_devtools_script_window_remote_value(
    result: &mut AutomationResult,
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
    result: &AutomationResult,
) -> Option<DevToolsWindowRemoteCandidate> {
    let value = devtools_script_result_remote_value(result)?;
    Some(DevToolsWindowRemoteCandidate {
        deep_serialized_value: value.deep_serialized_value.clone(),
    })
}

pub(super) async fn devtools_probe_remote_value_using(
    workflow: &mut RuntimeWorkflow,
    target: DevToolsRuntimeTarget,
    command: AutomationCommand,
) -> Result<Option<DevToolsRemoteValue>, DevToolsError> {
    let result_ownership = devtools_runtime_result_ownership(&command);
    let response = workflow.native(target, command).await?;
    let result = devtools_script_result_from_response(response, result_ownership, None)?;
    let AutomationResult::Script(result) = result else {
        return Ok(None);
    };
    let DevToolsScriptResult::Value(value) = *result else {
        return Ok(None);
    };
    Ok(Some(value))
}

pub(super) async fn materialize_devtools_script_deep_serialized_root_value_using(
    workflow: &mut RuntimeWorkflow,
    result: &mut AutomationResult,
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

    let command = AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
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
        match devtools_probe_remote_value_using(workflow, target.clone(), command).await {
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
    result: &AutomationResult,
) -> Option<&DevToolsRemoteValue> {
    let AutomationResult::Script(result) = result else {
        return None;
    };
    match result.as_ref() {
        DevToolsScriptResult::Value(value) => Some(value),
        DevToolsScriptResult::Exception(exception) => exception.value.as_ref(),
    }
}

pub(super) fn devtools_script_result_remote_value_mut(
    result: &mut AutomationResult,
) -> Option<&mut DevToolsRemoteValue> {
    let AutomationResult::Script(result) = result else {
        return None;
    };
    match result.as_mut() {
        DevToolsScriptResult::Value(value) => Some(value),
        DevToolsScriptResult::Exception(exception) => exception.value.as_mut(),
    }
}

pub(super) fn bidi_script_message_serialization_options_from_value(
    value: &Value,
) -> Result<DevToolsSerializationOptions, InvalidBidiChannelArgument> {
    let object = value.as_object().ok_or(InvalidBidiChannelArgument)?;
    let max_object_depth = match object.get("maxObjectDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(value.as_u64().ok_or(InvalidBidiChannelArgument)?),
    };
    let max_dom_depth = match object.get("maxDomDepth") {
        Some(Value::Null) | None => None,
        Some(value) => Some(value.as_u64().ok_or(InvalidBidiChannelArgument)?),
    };
    let include_shadow_tree = match object.get("includeShadowTree") {
        None => None,
        Some(Value::String(value)) if matches!(value.as_str(), "none" | "open" | "all") => {
            Some(value.clone())
        }
        Some(_) => return Err(InvalidBidiChannelArgument),
    };
    Ok(DevToolsSerializationOptions {
        max_object_depth,
        max_dom_depth,
        include_shadow_tree,
    })
}

pub(super) async fn devtools_probe_value_using(
    workflow: &mut RuntimeWorkflow,
    target: DevToolsRuntimeTarget,
    command: AutomationCommand,
) -> Result<Option<Value>, DevToolsError> {
    let Some(value) = devtools_probe_remote_value_using(workflow, target, command).await? else {
        return Ok(None);
    };
    Ok(Some(value.value))
}
