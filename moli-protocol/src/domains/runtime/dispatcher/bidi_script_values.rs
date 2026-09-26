use super::*;

pub(super) async fn start_protocol_neutral_runtime_command(
    conn: &mut CdpConnection,
    target: DevToolsRuntimeTarget,
    command: DevToolsCommand,
    internal_command_id: u64,
) -> RuntimeCommandTaskStep {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    match command {
        DevToolsCommand::EvaluateScript(command) => {
            let params = devtools_evaluate_script_params(&command, target.execution_context_id);
            let json =
                runtime_inspector_command_json(internal_command_id, "Runtime.evaluate", &params);
            let parsed = match parse_synthesized_runtime_command(json) {
                Ok(command) => command,
                Err(message) => {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        Some(internal_command_id),
                        message,
                    ));
                }
            };
            let cmd = Cmd::from_parsed(&parsed)
                .expect("synthesized Runtime command must contain a domain separator");
            let command = DevToolsCommand::EvaluateScript(command);
            match prepare_pending_devtools_runtime_inspector_json_for_owner(
                conn, &cmd, &owner, &command,
            ) {
                Ok(inspector_json) => start_devtools_runtime_command_for_owner(
                    conn,
                    &cmd,
                    command,
                    inspector_json,
                    runtime_command_awaits_promise(&cmd, RuntimeAction::Evaluate),
                    RendererInspectorResponseDelivery::AdapterReply,
                    owner.clone(),
                ),
                Err(message) => {
                    RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(cmd.id, message))
                }
            }
        }
        DevToolsCommand::CallFunction(mut command) => {
            if matches!(command.context.protocol, DevToolsProtocol::WebDriverBidi)
                && let Err(message) = Box::pin(materialize_bidi_channel_argument_proxies_async(
                    conn,
                    &target,
                    &mut command,
                ))
                .await
            {
                return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    Some(internal_command_id),
                    message,
                ));
            }
            match devtools_call_function_params_async(
                conn,
                &owner,
                &command,
                target.execution_context_id,
            )
            .await
            {
                Ok(params) => {
                    let json = runtime_inspector_command_json(
                        internal_command_id,
                        "Runtime.callFunctionOn",
                        &params,
                    );
                    let parsed = match parse_synthesized_runtime_command(json) {
                        Ok(command) => command,
                        Err(message) => {
                            return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                                Some(internal_command_id),
                                message,
                            ));
                        }
                    };
                    let cmd = Cmd::from_parsed(&parsed)
                        .expect("synthesized Runtime command must contain a domain separator");
                    let command = DevToolsCommand::CallFunction(command);
                    match prepare_pending_devtools_runtime_inspector_json_for_owner(
                        conn, &cmd, &owner, &command,
                    ) {
                        Ok(inspector_json) => start_devtools_runtime_command_for_owner(
                            conn,
                            &cmd,
                            command,
                            inspector_json,
                            runtime_command_awaits_promise(&cmd, RuntimeAction::CallFunctionOn),
                            RendererInspectorResponseDelivery::AdapterReply,
                            owner.clone(),
                        ),
                        Err(message) => RuntimeCommandTaskStep::Complete(
                            runtime_inspector_error_plan(cmd.id, message),
                        ),
                    }
                }
                Err(message) => RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                    Some(internal_command_id),
                    message,
                )),
            }
        }
        DevToolsCommand::TerminateExecution(_) => {
            let json = runtime_inspector_command_json(
                internal_command_id,
                "Runtime.terminateExecution",
                &json!({}),
            );
            let parsed = match parse_synthesized_runtime_command(json) {
                Ok(command) => command,
                Err(message) => {
                    return RuntimeCommandTaskStep::Complete(runtime_inspector_error_plan(
                        Some(internal_command_id),
                        message,
                    ));
                }
            };
            let cmd = Cmd::from_parsed(&parsed)
                .expect("synthesized Runtime command must contain a domain separator")
                .with_terminal_response_delivery_override(Some(
                    RendererInspectorResponseDelivery::AdapterReply,
                ));
            let MainRuntimeCommand::Inspector(command) =
                MainRuntimeCommand::classify(RuntimeAction::TerminateExecution)
            else {
                unreachable!("Runtime.terminateExecution must use an Inspector command route")
            };
            start_main_runtime_inspector_command_for_owner(conn, &cmd, command, owner.clone())
        }
        _ => RuntimeCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(
            DevToolsError::new(DevToolsErrorKind::Unsupported, "UnsupportedDevToolsCommand"),
        )),
    }
}

pub(super) fn runtime_inspector_command_json(
    command_id: u64,
    method: &str,
    params: &Value,
) -> String {
    json!({
        "id": command_id,
        "method": method,
        "params": params,
    })
    .to_string()
}

pub(super) fn devtools_evaluate_script_params(
    command: &DevToolsEvaluateScriptCommand,
    execution_context_id: Option<i64>,
) -> Value {
    let expression = if command.materialize_bidi_script_result {
        bidi_window_remote_result_expression_source(&command.expression, command.await_promise)
    } else {
        command.expression.clone()
    };
    let mut params = json!({
        "expression": expression,
        "awaitPromise": command.await_promise,
        "returnByValue": devtools_result_ownership_returns_by_value(
            command.result_ownership,
            command.preserve_remote_metadata,
        ),
    });
    if command.user_gesture
        && let Some(map) = params.as_object_mut()
    {
        map.insert("userGesture".to_owned(), Value::Bool(true));
    }
    if let Some(handler) = command.webdriver_bidi_file_prompt_handler.as_deref()
        && let Some(map) = params.as_object_mut()
    {
        map.insert(
            WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM.to_owned(),
            Value::String(handler.to_owned()),
        );
    }
    apply_devtools_serialization_options(&mut params, command.serialization_options.as_ref());
    if let Some(execution_context_id) = execution_context_id
        && let Some(map) = params.as_object_mut()
    {
        map.insert("contextId".to_owned(), json!(execution_context_id));
    }
    if matches!(command.result_ownership, DevToolsResultOwnership::Root)
        && let Some(map) = params.as_object_mut()
    {
        map.insert("objectGroup".to_owned(), json!("webdriver-bidi"));
    }
    params
}

pub(super) async fn devtools_call_function_params_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: &DevToolsCallFunctionCommand,
    execution_context_id: Option<i64>,
) -> Result<Value, String> {
    let deserialize_bidi_local_values =
        devtools_call_function_deserializes_bidi_local_values(command);
    let primary_argument_count = devtools_call_function_arguments(command).len();
    let function_declaration = devtools_call_function_declaration(
        command,
        deserialize_bidi_local_values,
        primary_argument_count,
    );
    let arguments = devtools_call_function_cdp_arguments(command, deserialize_bidi_local_values);
    let mut params = json!({
        "functionDeclaration": function_declaration,
        "arguments": arguments,
        "awaitPromise": command.await_promise,
        "returnByValue": devtools_result_ownership_returns_by_value(
            command.result_ownership,
            command.preserve_remote_metadata,
        ),
    });
    if command.user_gesture
        && let Some(map) = params.as_object_mut()
    {
        map.insert("userGesture".to_owned(), Value::Bool(true));
    }
    if let Some(handler) = command.webdriver_bidi_file_prompt_handler.as_deref()
        && let Some(map) = params.as_object_mut()
    {
        map.insert(
            WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM.to_owned(),
            Value::String(handler.to_owned()),
        );
    }
    apply_devtools_serialization_options(&mut params, command.serialization_options.as_ref());
    if let Some(map) = params.as_object_mut() {
        if let Some(object_id) = command.object_id.as_ref() {
            map.insert("objectId".to_owned(), json!(object_id.as_str()));
        } else if let Some(execution_context_id) = execution_context_id {
            map.insert("executionContextId".to_owned(), json!(execution_context_id));
        } else if command.realm_id.is_none() {
            let Some(execution_context_id) = conn
                .runtime_default_or_initial_execution_context_id_for_owner_async(owner)
                .await?
            else {
                return Err("NoDefaultExecutionContext".to_owned());
            };
            map.insert("executionContextId".to_owned(), json!(execution_context_id));
        }
    }
    let object_group = command.object_group.as_deref().or_else(|| {
        matches!(command.result_ownership, DevToolsResultOwnership::Root)
            .then_some("webdriver-bidi")
    });
    if let Some(object_group) = object_group
        && let Some(map) = params.as_object_mut()
    {
        map.insert("objectGroup".to_owned(), json!(object_group));
    }
    Ok(params)
}

pub(super) fn devtools_call_function_declaration(
    command: &DevToolsCallFunctionCommand,
    deserialize_bidi_local_values: bool,
    primary_argument_count: usize,
) -> String {
    let declaration = if command.this_parameter.is_none() && !deserialize_bidi_local_values {
        command.function_declaration.clone()
    } else if deserialize_bidi_local_values {
        let this_expression = if command.this_parameter.is_some() {
            "__deserialize(__primaryArgs.shift())"
        } else {
            "undefined"
        };
        let call_expression = if command.this_parameter.is_some() {
            "f.apply(deserializedThis, deserializedArgs)"
        } else {
            "f(...deserializedArgs)"
        };
        let deserializer_source = bidi_local_value_deserializer_source();
        format!(
            "(...args) => {{\n\
             function callFunction(f, args) {{\n\
             const __primaryArgs = args.slice(0, {primary_argument_count});\n\
             const __remoteReferences = args.slice({primary_argument_count});\n\
             {deserializer_source}\n\
             const deserializedThis = {this_expression};\n\
             const deserializedArgs = __primaryArgs.map(__deserialize);\n\
             return {call_expression};\n\
             }}\n\
             return callFunction((\n\
             {}\n\
             ), args);\n\
             }}",
            command.function_declaration
        )
    } else {
        format!(
            "(...args) => {{\n\
         function callFunction(f, args) {{\n\
         const deserializedThis = args.shift();\n\
         const deserializedArgs = args;\n\
         return f.apply(deserializedThis, deserializedArgs);\n\
         }}\n\
         return callFunction((\n\
         {}\n\
         ), args);\n\
         }}",
            command.function_declaration
        )
    };
    if command.materialize_bidi_script_result {
        bidi_window_remote_result_function_declaration_source(&declaration, command.await_promise)
    } else {
        declaration
    }
}

pub(super) fn bidi_local_value_deserializer_source() -> &'static str {
    "const __deserialize = (value) => {\n\
     if (value && value.__moliBidiRemoteReference === true) { return __remoteReferences[value.index]; }\n\
     if (value && value.__moliBidiChannelProxy === true) {\n\
     const proxy = __remoteReferences[value.index];\n\
     return proxy && typeof proxy.sendMessage === 'function'\n\
     ? proxy.sendMessage.bind(proxy)\n\
     : undefined;\n\
     }\n\
     if (!value || value.__moliBidiLocalValue !== true) { return value; }\n\
     switch (value.type) {\n\
     case 'undefined': return undefined;\n\
     case 'null': return null;\n\
     case 'string':\n\
     case 'boolean': return value.value;\n\
     case 'number':\n\
     if (value.value === 'NaN') { return NaN; }\n\
     if (value.value === '-0') { return -0; }\n\
     if (value.value === 'Infinity') { return Infinity; }\n\
     if (value.value === '-Infinity') { return -Infinity; }\n\
     return value.value;\n\
     case 'bigint': return BigInt(value.value);\n\
     case 'date': return new Date(value.value);\n\
     case 'regexp': return new RegExp(value.value.pattern, value.value.flags || '');\n\
     case 'array': return value.value.map(__deserialize);\n\
     case 'object': return Object.fromEntries(value.value.map(([key, item]) => [key, __deserialize(item)]));\n\
     case 'map': return new Map(value.value.map(([key, item]) => [__deserialize(key), __deserialize(item)]));\n\
     case 'set': return new Set(value.value.map(__deserialize));\n\
     case 'channel': return typeof __moliCreateBidiChannelDelegate === 'function'\n\
     ? __moliCreateBidiChannelDelegate(value.value)\n\
     : undefined;\n\
     default: return value.value;\n\
     }\n\
     };"
}

pub(super) fn bidi_window_remote_result_expression_source(
    expression: &str,
    await_promise: bool,
) -> String {
    let serializer = bidi_window_remote_result_serializer_source();
    let encoded_expression = serde_json::to_string(expression)
        .expect("serializing a string expression to JSON should not fail");
    if await_promise {
        format!(
            "Promise.resolve((0, eval)({encoded_expression})).then((__moliBidiResult) => {{\n\
             {serializer}\n\
             return __moliSerializeBidiWindowRemoteResult(__moliBidiResult);\n\
             }})"
        )
    } else {
        format!(
            "((__moliBidiResult) => {{\n\
             {serializer}\n\
             return __moliSerializeBidiWindowRemoteResult(__moliBidiResult);\n\
             }})(\n\
             (0, eval)({encoded_expression})\n\
             )"
        )
    }
}

pub(super) fn bidi_window_remote_result_function_declaration_source(
    function_declaration: &str,
    await_promise: bool,
) -> String {
    let serializer = bidi_window_remote_result_serializer_source();
    if await_promise {
        format!(
            "(...args) => {{\n\
             {serializer}\n\
             const __moliBidiFunction = ({function_declaration});\n\
             const __moliBidiResult = Reflect.apply(__moliBidiFunction, this, args);\n\
             return Promise.resolve(__moliBidiResult).then(__moliSerializeBidiWindowRemoteResult);\n\
             }}"
        )
    } else {
        format!(
            "(...args) => {{\n\
             {serializer}\n\
             const __moliBidiFunction = ({function_declaration});\n\
             const __moliBidiResult = Reflect.apply(__moliBidiFunction, this, args);\n\
             return __moliSerializeBidiWindowRemoteResult(__moliBidiResult);\n\
             }}"
        )
    }
}

pub(super) fn bidi_window_remote_result_serializer_source() -> &'static str {
    "const __moliSerializeBidiWindowRemoteResult = (value) => {\n\
     if (typeof __moliHostBidiWindowRemoteValue === 'function') {\n\
     const windowRemoteValue = __moliHostBidiWindowRemoteValue(value);\n\
     if (windowRemoteValue && windowRemoteValue.__moliBidiRemoteValue === true) {\n\
     return windowRemoteValue;\n\
     }\n\
     }\n\
     return value;\n\
     };"
}

pub(super) fn bidi_script_message_ownership_from_value(
    value: Option<&Value>,
) -> Result<DevToolsResultOwnership, String> {
    match value {
        None => Ok(DevToolsResultOwnership::None),
        Some(Value::String(value)) if value == "root" => Ok(DevToolsResultOwnership::Root),
        Some(Value::String(value)) if value == "none" => Ok(DevToolsResultOwnership::None),
        Some(_) => Err("InvalidChannelArgument".to_owned()),
    }
}

pub(super) fn bidi_script_message_ownership_from_str(
    value: Option<&str>,
) -> Result<DevToolsResultOwnership, String> {
    match value {
        None => Ok(DevToolsResultOwnership::None),
        Some("root") => Ok(DevToolsResultOwnership::Root),
        Some("none") => Ok(DevToolsResultOwnership::None),
        Some(_) => Err("InvalidChannelArgument".to_owned()),
    }
}

pub(super) fn devtools_call_function_arguments(
    command: &DevToolsCallFunctionCommand,
) -> Vec<Value> {
    command
        .this_parameter
        .iter()
        .cloned()
        .chain(command.arguments.iter().cloned())
        .collect()
}

pub(super) fn devtools_call_function_cdp_arguments(
    command: &DevToolsCallFunctionCommand,
    deserialize_bidi_local_values: bool,
) -> Vec<Value> {
    let arguments = devtools_call_function_arguments(command);
    if !deserialize_bidi_local_values {
        return arguments
            .into_iter()
            .map(cdp_call_argument_from_devtools_argument)
            .collect();
    }

    let mut remote_object_ids = Vec::new();
    let mut context = BidiLocalValueDescriptorContext {
        remote_object_ids: &mut remote_object_ids,
        preload_channel_handoffs: None,
        preload_channel_error: None,
    };
    let mut cdp_arguments = arguments
        .into_iter()
        .map(|argument| {
            let descriptor =
                bidi_local_value_descriptor_from_devtools_argument(&argument, &mut context)
                    .or_else(|| cdp_value_from_devtools_argument(&argument))
                    .or_else(|| {
                        argument
                            .as_object()
                            .and_then(|map| map.get("value"))
                            .cloned()
                    })
                    .unwrap_or(argument);
            json!({ "value": descriptor })
        })
        .collect::<Vec<_>>();
    drop(context);
    cdp_arguments.extend(
        remote_object_ids
            .into_iter()
            .map(|object_id| json!({ "objectId": object_id })),
    );
    cdp_arguments
}

pub(super) fn devtools_call_function_deserializes_bidi_local_values(
    command: &DevToolsCallFunctionCommand,
) -> bool {
    matches!(command.context.protocol, DevToolsProtocol::WebDriverBidi)
        && command
            .this_parameter
            .iter()
            .chain(command.arguments.iter())
            .any(bidi_local_value_needs_js_deserialization)
}

pub(super) fn bidi_local_value_needs_js_deserialization(argument: &Value) -> bool {
    bidi_local_value_needs_js_deserialization_nested(argument, false)
}

pub(super) fn bidi_local_value_needs_js_deserialization_nested(
    argument: &Value,
    nested: bool,
) -> bool {
    let Some(map) = argument.as_object() else {
        return false;
    };
    if map.get("__moliBidiChannelProxy").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if bidi_remote_reference_object_id(map).is_some() {
        return nested;
    }
    match map.get("type").and_then(Value::as_str) {
        Some("date" | "map" | "regexp" | "set") => true,
        Some("channel") => true,
        Some("bigint") => nested,
        Some("number") => {
            nested
                && map
                    .get("value")
                    .and_then(Value::as_str)
                    .is_some_and(is_bidi_unserializable_number)
        }
        Some("array") => map
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| bidi_local_value_needs_js_deserialization_nested(value, true))
            }),
        Some("object") => map
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .as_array()
                        .and_then(|pair| pair.get(1))
                        .is_some_and(|value| {
                            bidi_local_value_needs_js_deserialization_nested(value, true)
                        })
                })
            }),
        _ => false,
    }
}

pub(super) fn apply_devtools_serialization_options(
    params: &mut Value,
    serialization_options: Option<&DevToolsSerializationOptions>,
) {
    let Some(serialization_options) = serialization_options else {
        return;
    };
    let Some(map) = params.as_object_mut() else {
        return;
    };
    map.insert(
        "serializationOptions".to_owned(),
        devtools_deep_serialization_options_json(serialization_options),
    );
}

pub(super) fn devtools_result_ownership_returns_by_value(
    ownership: DevToolsResultOwnership,
    preserve_remote_metadata: bool,
) -> bool {
    !preserve_remote_metadata && !matches!(ownership, DevToolsResultOwnership::Root)
}

pub(super) fn cdp_call_argument_from_devtools_argument(argument: Value) -> Value {
    let Some(map) = argument.as_object() else {
        return json!({ "value": argument });
    };
    if let Some(object_id) = bidi_remote_reference_object_id(map) {
        return json!({ "objectId": object_id });
    }
    if map.get("type").and_then(Value::as_str) == Some("number")
        && let Some(unserializable_value) = map.get("value").and_then(Value::as_str)
        && is_bidi_unserializable_number(unserializable_value)
    {
        return json!({ "unserializableValue": unserializable_value });
    }
    if map.get("type").and_then(Value::as_str) == Some("bigint")
        && let Some(value) = map.get("value").and_then(Value::as_str)
    {
        let unserializable_value = if value.ends_with('n') {
            value.to_owned()
        } else {
            format!("{value}n")
        };
        return json!({ "unserializableValue": unserializable_value });
    }
    if let Some(value) = cdp_value_from_devtools_argument(&argument) {
        return json!({ "value": value });
    }
    if let Some(value) = map.get("value") {
        return json!({ "value": value.clone() });
    }
    if map.get("type").and_then(Value::as_str) == Some("null") {
        return json!({ "value": null });
    }
    argument
}

pub(super) fn is_bidi_unserializable_number(value: &str) -> bool {
    matches!(value, "NaN" | "-0" | "Infinity" | "-Infinity")
}

pub(super) fn bidi_remote_reference_object_id(
    map: &serde_json::Map<String, Value>,
) -> Option<&str> {
    map.get("sharedId")
        .or_else(|| map.get("handle"))
        .and_then(Value::as_str)
}

pub(super) fn bidi_local_value_descriptor_from_devtools_argument(
    argument: &Value,
    context: &mut BidiLocalValueDescriptorContext<'_>,
) -> Option<Value> {
    let Some(map) = argument.as_object() else {
        return Some(argument.clone());
    };
    if map.get("__moliBidiChannelProxy").and_then(Value::as_bool) == Some(true) {
        let object_id = map.get("handle")?.as_str()?;
        let index = context.remote_object_ids.len();
        context.remote_object_ids.push(object_id.to_owned());
        return Some(json!({
            "__moliBidiChannelProxy": true,
            "index": index,
        }));
    }
    if let Some(object_id) = bidi_remote_reference_object_id(map) {
        let index = context.remote_object_ids.len();
        context.remote_object_ids.push(object_id.to_owned());
        return Some(json!({
            "__moliBidiRemoteReference": true,
            "index": index,
        }));
    }
    let type_name = map.get("type").and_then(Value::as_str)?;
    let descriptor_value = match type_name {
        "undefined" | "null" => Value::Null,
        "array" | "set" => Value::Array(
            map.get("value")?
                .as_array()?
                .iter()
                .map(|value| bidi_local_value_descriptor_from_devtools_argument(value, context))
                .collect::<Option<Vec<_>>>()?,
        ),
        "object" => {
            let mut entries = Vec::new();
            for entry in map.get("value")?.as_array()? {
                let pair = entry.as_array()?;
                let [key, value] = pair.as_slice() else {
                    return None;
                };
                entries.push(json!([
                    key.as_str()?,
                    bidi_local_value_descriptor_from_devtools_argument(value, context)?
                ]));
            }
            Value::Array(entries)
        }
        "map" => {
            let mut entries = Vec::new();
            for entry in map.get("value")?.as_array()? {
                let pair = entry.as_array()?;
                let [key, value] = pair.as_slice() else {
                    return None;
                };
                let key = key
                    .as_str()
                    .map(Value::from)
                    .or_else(|| bidi_local_value_descriptor_from_devtools_argument(key, context))?;
                entries.push(json!([
                    key,
                    bidi_local_value_descriptor_from_devtools_argument(value, context)?
                ]));
            }
            Value::Array(entries)
        }
        "regexp" => {
            let regexp = map.get("value")?.as_object()?;
            json!({
                "pattern": regexp.get("pattern")?.as_str()?,
                "flags": regexp.get("flags").and_then(Value::as_str).unwrap_or_default(),
            })
        }
        "channel" => {
            let channel = map.get("value")?.as_object()?;
            let mut descriptor = serde_json::Map::new();
            descriptor.insert(
                "channel".to_owned(),
                json!(channel.get("channel")?.as_str()?),
            );
            if let Some(ownership) = channel.get("ownership").and_then(Value::as_str) {
                descriptor.insert("ownership".to_owned(), json!(ownership));
            }
            if let Some(serialization_options) = channel.get("serializationOptions") {
                descriptor.insert(
                    "serializationOptions".to_owned(),
                    serialization_options.clone(),
                );
            }
            if let Some(handoffs) = context.preload_channel_handoffs.as_deref_mut() {
                let handoff = match new_bidi_preload_channel_handoff(channel) {
                    Ok(handoff) => handoff,
                    Err(message) => {
                        context.preload_channel_error = Some(message);
                        return None;
                    }
                };
                descriptor.insert("handoffId".to_owned(), json!(handoff.handoff_id));
                descriptor.insert("handoffToken".to_owned(), json!(handoff.token));
                handoffs.push(handoff);
            }
            Value::Object(descriptor)
        }
        "string" | "number" | "boolean" | "bigint" | "date" => map.get("value")?.clone(),
        _ => return None,
    };
    Some(json!({
        "__moliBidiLocalValue": true,
        "type": type_name,
        "value": descriptor_value,
    }))
}
