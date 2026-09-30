use super::*;

pub(super) fn bidi_call_function_value_at_path_mut<'a>(
    command: &'a mut DevToolsCallFunctionCommand,
    root: BidiCallFunctionValueRoot,
    path: &[BidiValuePathSegment],
) -> Option<&'a mut Value> {
    let mut value = match root {
        BidiCallFunctionValueRoot::This => command.this_parameter.as_mut()?,
        BidiCallFunctionValueRoot::Argument(index) => command.arguments.get_mut(index)?,
    };
    for segment in path {
        match segment {
            BidiValuePathSegment::Key(key) => {
                value = value.as_object_mut()?.get_mut(key)?;
            }
            BidiValuePathSegment::Index(index) => {
                value = value.as_array_mut()?.get_mut(*index)?;
            }
        }
    }
    Some(value)
}

pub(super) async fn materialize_bidi_channel_argument_proxies_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    command: &mut DevToolsCallFunctionCommand,
) -> Result<(), String> {
    let realm_id = devtools_realm_id_for_runtime_target_async(conn, target).await;
    if let Some(this_parameter) = command.this_parameter.as_mut() {
        materialize_bidi_channel_value_async(conn, target, realm_id.as_ref(), this_parameter)
            .await?;
    }
    for argument in &mut command.arguments {
        materialize_bidi_channel_value_async(conn, target, realm_id.as_ref(), argument).await?;
    }
    Ok(())
}

pub(super) async fn materialize_bidi_channel_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    value: &mut Value,
) -> Result<(), String> {
    let Some(map) = value.as_object_mut() else {
        return Ok(());
    };
    if bidi_remote_reference_object_id(map).is_some() {
        return Ok(());
    }
    match map.get("type").and_then(Value::as_str) {
        Some("channel") => {
            let properties = bidi_channel_properties_from_local_value(map)?;
            let proxy_handle = create_bidi_channel_proxy_and_start_listener_async(
                conn, target, realm_id, properties,
            )
            .await?;
            *value = json!({
                "__moliBidiChannelProxy": true,
                "handle": proxy_handle.as_str(),
            });
        }
        Some("array" | "set") => {
            if let Some(items) = map.get_mut("value").and_then(Value::as_array_mut) {
                for item in items {
                    Box::pin(materialize_bidi_channel_value_async(
                        conn, target, realm_id, item,
                    ))
                    .await?;
                }
            }
        }
        Some("object" | "map") => {
            if let Some(entries) = map.get_mut("value").and_then(Value::as_array_mut) {
                for entry in entries {
                    let Some(pair) = entry.as_array_mut() else {
                        continue;
                    };
                    for item in pair {
                        Box::pin(materialize_bidi_channel_value_async(
                            conn, target, realm_id, item,
                        ))
                        .await?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn bidi_channel_properties_from_local_value(
    map: &serde_json::Map<String, Value>,
) -> Result<DevToolsBidiChannelProperties, String> {
    let channel = map
        .get("value")
        .and_then(Value::as_object)
        .ok_or_else(|| "InvalidChannelArgument".to_owned())?;
    Ok(DevToolsBidiChannelProperties {
        channel: channel
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| "InvalidChannelArgument".to_owned())?
            .to_owned(),
        ownership: bidi_script_message_ownership_from_value(channel.get("ownership"))?,
        serialization_options: match channel.get("serializationOptions") {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
}

pub(super) async fn bidi_preload_channel_proxy_handle_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    handoff_id: &str,
    token: &str,
    channel_object_group: &str,
) -> Result<Option<DevToolsRemoteHandleId>, DevToolsError> {
    let value = devtools_probe_remote_value_async(
        conn,
        target.clone(),
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            realm_id: realm_id.cloned(),
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: bidi_preload_channel_proxy_handle_source().to_owned(),
            arguments: vec![json!(handoff_id), json!(token)],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::Root,
            object_group: Some(channel_object_group.to_owned()),
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await?;
    Ok(value.and_then(|value| value.handle.or(value.shared_id)))
}

pub(super) fn bidi_preload_channel_properties_from_handoff(
    handoff: &BidiPreloadChannelHandoff,
) -> Result<DevToolsBidiChannelProperties, String> {
    Ok(DevToolsBidiChannelProperties {
        channel: handoff.channel.clone(),
        ownership: bidi_script_message_ownership_from_str(handoff.ownership.as_deref())?,
        serialization_options: match handoff.serialization_options.as_ref() {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
}

pub(super) async fn release_bidi_channel_object_group_for_target_best_effort_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    session_id: Option<&str>,
    object_group: &str,
) {
    let owner = session_id
        .map(CommandOwnerScope::for_session)
        .unwrap_or_else(|| CommandOwnerScope::for_route(target.route.clone()));
    conn.release_bidi_channel_object_group_for_owner_best_effort_async(&owner, object_group)
        .await;
}

pub(super) fn bidi_channel_page_owner_for_runtime_target(
    conn: &CdpConnection,
    target: &DevToolsRuntimeTarget,
    owner: &CommandOwnerScope,
) -> Option<BidiChannelPageOwner> {
    let exact_owner = owner
        .session_id()
        .map(CommandOwnerScope::for_session)
        .unwrap_or_else(|| CommandOwnerScope::for_route(target.route.clone()));
    BidiChannelPageOwner::capture_for_owner(conn, exact_owner)
}

pub(super) async fn create_bidi_channel_proxy_and_start_listener_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    properties: DevToolsBidiChannelProperties,
) -> Result<DevToolsRemoteHandleId, String> {
    let listener_target_id = target
        .window_context_id
        .clone()
        .ok_or_else(|| "NoSuchBidiChannelTarget".to_owned())?;
    let listener_realm_id = realm_id
        .cloned()
        .ok_or_else(|| "NoSuchBidiChannelRealm".to_owned())?;
    let listener_owner = bidi_channel_page_owner_for_runtime_target(
        conn,
        target,
        &CommandOwnerScope::for_route(target.route.clone()),
    )
    .ok_or_else(|| "NoSuchBidiChannelTarget".to_owned())?;
    let channel_object_group = conn.next_bidi_channel_object_group();
    let proxy_handle = match create_bidi_channel_proxy_async(
        conn,
        target,
        realm_id,
        &channel_object_group,
    )
    .await
    {
        Ok(Some(handle)) => handle,
        Ok(None) => {
            release_bidi_channel_object_group_for_target_best_effort_async(
                conn,
                target,
                None,
                &channel_object_group,
            )
            .await;
            return Err("CannotCreateBidiChannelProxy".to_owned());
        }
        Err(error) => {
            release_bidi_channel_object_group_for_target_best_effort_async(
                conn,
                target,
                None,
                &channel_object_group,
            )
            .await;
            return Err(error);
        }
    };
    let listener = PendingBidiChannelListener::new(
        Some(listener_target_id),
        Some(listener_realm_id),
        proxy_handle.clone(),
        channel_object_group,
        properties,
    )
    .ok_or_else(|| "NoSuchBidiChannelRealm".to_owned())?;
    conn.publish_bidi_channel_listener_start(BidiChannelListenerResidence::new(
        listener_owner,
        listener,
    ));
    Ok(proxy_handle)
}

pub(super) async fn create_bidi_channel_proxy_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    channel_object_group: &str,
) -> Result<Option<DevToolsRemoteHandleId>, String> {
    let call_target = if let Some(realm_id) = realm_id
        && target.execution_context_id.is_none()
    {
        devtools_runtime_realm_target_async(conn, realm_id)
            .await
            .map_err(|error| error.message)?
    } else {
        target.clone()
    };
    let command_target_id = if realm_id.is_some() {
        None
    } else {
        call_target.window_context_id.clone()
    };
    let value = Box::pin(devtools_probe_remote_value_async(
        conn,
        call_target,
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: None,
                target_id: command_target_id,
                browser_context_id: None,
            },
            realm_id: realm_id.cloned(),
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: format!(
                "function() {{ return {}; }}",
                bidi_channel_proxy_expression_source()
            ),
            arguments: Vec::new(),
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::Root,
            object_group: Some(channel_object_group.to_owned()),
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    ))
    .await
    .map_err(|error| error.message)?;
    Ok(value.and_then(|value| value.handle.or(value.shared_id)))
}

pub(super) fn bidi_channel_proxy_expression_source() -> String {
    let mut source = String::new();
    source.push_str("(() => {\n");
    source.push_str(
        "const queue = [];\n\
     let queueNonEmptyResolver = null;\n\
     return {\n\
     async getMessage() {\n\
     const onMessage = queue.length > 0 ? Promise.resolve() : new Promise((resolve) => { queueNonEmptyResolver = resolve; });\n\
     await onMessage;\n\
     return queue.shift();\n\
     },\n\
     sendMessage(message) {\n\
     queue.push(message);\n\
     if (queueNonEmptyResolver !== null) {\n\
     queueNonEmptyResolver();\n\
     queueNonEmptyResolver = null;\n\
     }\n\
     },\n\
     };\n\
     })()",
    );
    source
}

pub(super) fn bidi_preload_channel_delegate_source(has_channel_handoffs: bool) -> String {
    if !has_channel_handoffs {
        return String::new();
    }
    let mut source = String::new();
    source.push_str(
        "const __moliPutBidiPreloadChannelProxy = (handoffId, token, proxy) => {\n\
     if (typeof handoffId !== 'string' || handoffId.length === 0 || typeof token !== 'string') { return; }\n\
     const take = (providedToken) => {\n\
     if (providedToken !== token) { return undefined; }\n\
     delete globalThis[handoffId];\n\
     return proxy;\n\
     };\n\
     Object.defineProperty(globalThis, handoffId, {\n\
     value: take,\n\
     configurable: true,\n\
     enumerable: false,\n\
     writable: false,\n\
     });\n\
     };\n",
    );
    source.push_str(
        "const __moliCreateBidiChannelDelegate = (properties) => {\n\
     const queue = [];\n\
     let queueNonEmptyResolver = null;\n\
     const proxy = {\n\
     async getMessage() {\n\
     const onMessage = queue.length > 0 ? Promise.resolve() : new Promise((resolve) => { queueNonEmptyResolver = resolve; });\n\
     await onMessage;\n\
     return queue.shift();\n\
     },\n\
     sendMessage(message) {\n\
     queue.push(message);\n\
     if (queueNonEmptyResolver !== null) {\n\
     queueNonEmptyResolver();\n\
     queueNonEmptyResolver = null;\n\
     }\n\
     },\n\
     };\n\
     __moliPutBidiPreloadChannelProxy(properties && properties.handoffId, properties && properties.handoffToken, proxy);\n\
     return proxy.sendMessage.bind(proxy);\n\
     };",
    );
    source
}

pub(super) fn bidi_preload_channel_proxy_handle_source() -> &'static str {
    "(function(handoffId, token) {\n\
     const take = globalThis[handoffId];\n\
     if (typeof take !== 'function') { return undefined; }\n\
     return take(token);\n\
     })"
}

pub(super) fn new_bidi_preload_channel_handoff(
    channel: &serde_json::Map<String, Value>,
) -> Result<BidiPreloadChannelHandoff, String> {
    let properties = bidi_channel_properties_from_channel_map(channel)?;
    let channel_name = channel
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| "InvalidChannelArgument".to_owned())?
        .to_owned();
    Ok(BidiPreloadChannelHandoff {
        handoff_id: format!(
            "__lmBidiPreloadChannel_{}",
            random_bidi_preload_channel_handoff_id()?
        ),
        token: random_bidi_preload_channel_handoff_id()?,
        channel: channel_name,
        ownership: match properties.ownership {
            DevToolsResultOwnership::Root => Some("root".to_owned()),
            DevToolsResultOwnership::None | DevToolsResultOwnership::ByValue => None,
        },
        serialization_options: channel.get("serializationOptions").cloned(),
    })
}

pub(super) fn bidi_channel_properties_from_channel_map(
    channel: &serde_json::Map<String, Value>,
) -> Result<DevToolsBidiChannelProperties, String> {
    Ok(DevToolsBidiChannelProperties {
        channel: channel
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| "InvalidChannelArgument".to_owned())?
            .to_owned(),
        ownership: bidi_script_message_ownership_from_value(channel.get("ownership"))?,
        serialization_options: match channel.get("serializationOptions") {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
}

pub(super) fn random_bidi_preload_channel_handoff_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    moli_crypto::fill_secure_random(&mut bytes)
        .map_err(|error| format!("failed to generate BiDi preload channel id: {error}"))?;
    Ok(hex_bytes(&bytes))
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
