use anyhow::Context as _;

use super::*;

#[derive(Debug, thiserror::Error)]
#[error("InvalidChannelArgument")]
pub(super) struct InvalidBidiChannelArgument;

#[derive(Debug, thiserror::Error)]
#[error("BiDi preload channel Inspector error {code}: {message}")]
struct BidiPreloadChannelInspectorError {
    code: i32,
    message: String,
}

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

pub(super) async fn materialize_bidi_channel_argument_proxies_using(
    workflow: &mut RuntimeWorkflow,
    target: &DevToolsRuntimeTarget,
    command: &mut DevToolsCallFunctionCommand,
) -> Result<
    Vec<(
        crate::conn::BidiChannelObjectGroupLease,
        PendingBidiChannelListener,
    )>,
    DevToolsError,
> {
    let mut listeners = Vec::new();
    let realm_id = devtools_realm_id_for_runtime_target_using(workflow, target).await;
    if let Some(this_parameter) = command.this_parameter.as_mut() {
        materialize_bidi_channel_value_using(
            workflow,
            target,
            realm_id.as_ref(),
            this_parameter,
            &mut listeners,
        )
        .await?;
    }
    for argument in &mut command.arguments {
        materialize_bidi_channel_value_using(
            workflow,
            target,
            realm_id.as_ref(),
            argument,
            &mut listeners,
        )
        .await?;
    }
    Ok(listeners)
}

#[cfg(test)]
mod workflow_tests {
    use super::*;
    use crate::conn::{BrowserContext, ProtocolWorkflowDriver};
    use crate::testing::TestContext;

    #[tokio::test]
    async fn unpublished_channel_arguments_release_proxy_groups_without_starting_listeners() {
        for invalid_second in [false, true] {
            let mut fixture = TestContext::new();
            let mut browser = BrowserContext::new("BID-staged-runtime".into());
            browser.set_active_target_id("TID-staged-runtime");
            browser.attach_active_session("SID-staged-runtime");
            fixture.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
            fixture
                .install_navigation_fixture_for_session_owner(
                    "about:blank",
                    Some("SID-staged-runtime"),
                )
                .await;
            let access = fixture.handle();
            let mut command = DevToolsCallFunctionCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    target_id: Some("TID-staged-runtime".into()),
                    session_id: None,
                    browser_context_id: Some("BID-staged-runtime".into()),
                },
                realm_id: None,
                world_name: None,
                object_id: None,
                this_parameter: None,
                function_declaration: "function() {}".into(),
                arguments: Vec::new(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            };
            command.arguments = vec![
                json!({"type":"channel","value":{"channel":"first"}}),
                if invalid_second {
                    json!({"type":"channel","value":null})
                } else {
                    json!({"type":"channel","value":{"channel":"second"}})
                },
            ];
            let (context, _) = crate::conn::CommandWorkflowContext::take_from(
                &mut crate::conn::CommandDispatchContext::default(),
                None,
                None,
            );
            let mut workflow =
                RuntimeWorkflow::new(ProtocolWorkflowDriver::new(access.clone()), context, 99);
            let target = fixture
                .drive_owned_wait(devtools_runtime_context_target_using(
                    &mut workflow,
                    &DevToolsTargetId::from("TID-staged-runtime"),
                    None,
                ))
                .await
                .unwrap();
            let staged = fixture
                .drive_owned_wait(materialize_bidi_channel_argument_proxies_using(
                    &mut workflow,
                    &target,
                    &mut command,
                ))
                .await;
            assert_eq!(staged.is_err(), invalid_second);
            let handles = command
                .arguments
                .iter()
                .filter_map(|argument| argument["handle"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            assert_eq!(handles.len(), if invalid_second { 1 } else { 2 });
            drop(staged);
            let work = fixture.with_fixture(|conn| {
                assert!(
                    !conn
                        .has_pending_inspector_awaits_for_session_owner(Some("SID-staged-runtime"))
                );
                conn.take_scheduler_events()
                    .into_iter()
                    .map(|event| match event {
                        crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } => {
                            assert_eq!(
                                work.bidi_channel_owner_action_kind(),
                                Some(crate::conn::BidiChannelOwnerActionKind::ReleaseObjectGroup)
                            );
                            work
                        }
                        other => panic!("unexpected work before author admission: {other:?}"),
                    })
                    .collect::<Vec<_>>()
            });
            assert_eq!(work.len(), handles.len());
            for work in work {
                let pending = workflow
                    .driver
                    .with_state(move |state| state.try_start_owned_protocol_scheduler_work(work))
                    .unwrap()
                    .unwrap()
                    .unwrap();
                let completed = fixture.drive_owned_wait(pending.wait()).await;
                workflow
                    .driver
                    .with_state(move |state| {
                        state.complete_owned_protocol_scheduler_work(completed)
                    })
                    .unwrap();
            }
            fixture.with_fixture(|conn| {
                for handle in &handles {
                    assert!(
                        conn.runtime_remote_object_group_for_session_owner(
                            Some("SID-staged-runtime"),
                            handle
                        )
                        .is_none()
                    );
                }
                assert!(
                    !conn
                        .has_pending_inspector_awaits_for_session_owner(Some("SID-staged-runtime"))
                );
            });
            for handle in handles {
                let response = fixture
                    .drive_owned_wait(workflow.helper(
                        CommandOwnerScope::for_session("SID-staged-runtime"),
                        "Runtime.getProperties",
                        json!({"objectId":handle}),
                    ))
                    .await
                    .unwrap();
                assert!(
                    matches!(
                        response,
                        BackgroundCommandResponsePayload::Error { code: -32000, .. }
                    ),
                    "the original Native object must be released, not only its protocol registry entry: {response:?}"
                );
            }
        }
    }
}

pub(super) async fn materialize_bidi_channel_value_using(
    workflow: &mut RuntimeWorkflow,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    value: &mut Value,
    listeners: &mut Vec<(
        crate::conn::BidiChannelObjectGroupLease,
        PendingBidiChannelListener,
    )>,
) -> Result<(), DevToolsError> {
    let Some(map) = value.as_object_mut() else {
        return Ok(());
    };
    if bidi_remote_reference_object_id(map).is_some() {
        return Ok(());
    }
    match map.get("type").and_then(Value::as_str) {
        Some("channel") => {
            let properties =
                bidi_channel_properties_from_local_value(map).map_err(workflow::workflow_error)?;
            let (proxy_handle, lease, listener) = prepare_bidi_channel_proxy_and_listener_using(
                workflow, target, realm_id, properties,
            )
            .await?;
            *value = json!({
                "__moliBidiChannelProxy": true,
                "handle": proxy_handle.as_str(),
            });
            listeners.push((lease, listener));
        }
        Some("array" | "set") => {
            if let Some(items) = map.get_mut("value").and_then(Value::as_array_mut) {
                for item in items {
                    Box::pin(materialize_bidi_channel_value_using(
                        workflow, target, realm_id, item, listeners,
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
                        Box::pin(materialize_bidi_channel_value_using(
                            workflow, target, realm_id, item, listeners,
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
) -> Result<DevToolsBidiChannelProperties, InvalidBidiChannelArgument> {
    let channel = map
        .get("value")
        .and_then(Value::as_object)
        .ok_or(InvalidBidiChannelArgument)?;
    bidi_channel_properties_from_channel_map(channel)
}

pub(super) async fn bidi_preload_channel_proxy_handle_using(
    driver: &mut crate::conn::ProtocolWorkflowDriver,
    owner: BidiChannelPageOwner,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    handoff_id: &str,
    token: &str,
    channel_object_group: &str,
    predecessor: &mut Option<moli_core::RendererOutputFence>,
) -> Result<Option<DevToolsRemoteHandleId>> {
    let mut params = json!({
        "functionDeclaration": bidi_preload_channel_proxy_handle_source(),
        "arguments": [{"value":handoff_id},{"value":token}],
        "awaitPromise":false,
        "returnByValue":false,
        "userGesture":false,
        "objectGroup":channel_object_group,
    });
    if let Some(context_id) = target.execution_context_id {
        params["executionContextId"] = json!(context_id);
    } else if let Some(realm_id) = realm_id {
        params["uniqueContextId"] = json!(realm_id.as_str());
    }
    let Some(prepared) =
        driver.with_state(move |state| state.prepare_bidi_channel_proxy(owner, params))??
    else {
        return Ok(None);
    };
    let output = prepared.run(driver).await;
    if let Some(fence) = output.predecessor {
        fence.merge_into_same_stream_tail(predecessor);
    }
    let response = output.response?;
    match response {
        BackgroundCommandResponsePayload::Error { code, message, .. } => {
            Err(BidiPreloadChannelInspectorError { code, message }.into())
        }
        BackgroundCommandResponsePayload::Success { result } => {
            if result.get("exceptionDetails").is_some() {
                return Ok(None);
            }
            Ok(result
                .get("result")
                .and_then(|result| result.get("objectId"))
                .and_then(Value::as_str)
                .map(DevToolsRemoteHandleId::from))
        }
    }
}

pub(super) fn bidi_preload_channel_properties_from_handoff(
    handoff: &BidiPreloadChannelHandoff,
) -> Result<DevToolsBidiChannelProperties, InvalidBidiChannelArgument> {
    Ok(DevToolsBidiChannelProperties {
        channel: handoff.channel.clone(),
        ownership: bidi_script_message_ownership_from_str(handoff.ownership.as_deref())?,
        serialization_options: match handoff.serialization_options.as_ref() {
            Some(value) => Some(bidi_script_message_serialization_options_from_value(value)?),
            None => None,
        },
    })
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

#[derive(Debug, thiserror::Error)]
enum BidiChannelPreparationError {
    #[error("NoSuchBidiChannelTarget")]
    NoTarget,
    #[error("NoSuchBidiChannelRealm")]
    NoRealm,
    #[error("CannotCreateBidiChannelProxy")]
    NoProxy,
}

pub(super) async fn prepare_bidi_channel_proxy_and_listener_using(
    workflow: &mut RuntimeWorkflow,
    target: &DevToolsRuntimeTarget,
    realm: Option<&DevToolsRealmId>,
    properties: DevToolsBidiChannelProperties,
) -> Result<
    (
        DevToolsRemoteHandleId,
        crate::conn::BidiChannelObjectGroupLease,
        PendingBidiChannelListener,
    ),
    DevToolsError,
> {
    use BidiChannelPreparationError::{NoProxy, NoRealm, NoTarget};
    let target_id = target
        .window_context_id
        .clone()
        .ok_or_else(|| workflow::workflow_error(NoTarget))?;
    let realm_id = realm
        .cloned()
        .ok_or_else(|| workflow::workflow_error(NoRealm))?;
    let route = target.route.clone();
    let owner = workflow
        .state(move |state| state.runtime_bidi_channel_owner(route))?
        .ok_or_else(|| workflow::workflow_error(NoTarget))?;
    let lease = workflow
        .state(move |state| state.allocate_bidi_channel_object_group(owner))?
        .ok_or_else(|| workflow::workflow_error(NoTarget))?;
    let proxy = Box::pin(create_bidi_channel_proxy_using(
        workflow,
        target,
        realm,
        lease.name(),
    ))
    .await;
    let handle = match proxy {
        Ok(Some(handle)) => handle,
        failed => {
            workflow.state(move |state| state.release_runtime_object_group(lease))?;
            return Err(match failed {
                Err(error) => error,
                _ => workflow::workflow_error(NoProxy),
            });
        }
    };
    let listener = PendingBidiChannelListener::new(
        Some(target_id),
        Some(realm_id),
        handle.clone(),
        lease.name().to_owned(),
        properties,
    )
    .ok_or_else(|| workflow::workflow_error(NoRealm))?;
    Ok((handle, lease, listener))
}

pub(super) async fn create_bidi_channel_proxy_using(
    workflow: &mut RuntimeWorkflow,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
    channel_object_group: &str,
) -> Result<Option<DevToolsRemoteHandleId>, DevToolsError> {
    let call_target = if let Some(realm_id) = realm_id
        && target.execution_context_id.is_none()
    {
        devtools_runtime_realm_target_using(workflow, realm_id).await?
    } else {
        target.clone()
    };
    let command_target_id = if realm_id.is_some() {
        None
    } else {
        call_target.window_context_id.clone()
    };
    let value = Box::pin(devtools_probe_remote_value_using(
        workflow,
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
    .await?;
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
) -> Result<BidiPreloadChannelHandoff> {
    let properties = bidi_channel_properties_from_channel_map(channel)?;
    Ok(BidiPreloadChannelHandoff {
        handoff_id: format!(
            "__lmBidiPreloadChannel_{}",
            random_bidi_preload_channel_handoff_id()?
        ),
        token: random_bidi_preload_channel_handoff_id()?,
        channel: properties.channel,
        ownership: match properties.ownership {
            DevToolsResultOwnership::Root => Some("root".to_owned()),
            DevToolsResultOwnership::None | DevToolsResultOwnership::ByValue => None,
        },
        serialization_options: channel.get("serializationOptions").cloned(),
    })
}

pub(super) fn bidi_channel_properties_from_channel_map(
    channel: &serde_json::Map<String, Value>,
) -> Result<DevToolsBidiChannelProperties, InvalidBidiChannelArgument> {
    let ownership = match channel.get("ownership") {
        None => bidi_script_message_ownership_from_str(None)?,
        Some(Value::String(value)) => bidi_script_message_ownership_from_str(Some(value))?,
        Some(_) => return Err(InvalidBidiChannelArgument),
    };
    Ok(DevToolsBidiChannelProperties {
        channel: channel
            .get("channel")
            .and_then(Value::as_str)
            .ok_or(InvalidBidiChannelArgument)?
            .to_owned(),
        ownership,
        serialization_options: channel
            .get("serializationOptions")
            .map(bidi_script_message_serialization_options_from_value)
            .transpose()?,
    })
}

pub(super) fn random_bidi_preload_channel_handoff_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    moli_crypto::fill_secure_random(&mut bytes)
        .context("failed to generate BiDi preload channel id")?;
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
