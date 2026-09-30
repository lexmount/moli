use super::*;

pub(super) async fn devtools_realm_id_for_runtime_target_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
) -> Option<DevToolsRealmId> {
    devtools_runtime_realm_for_target_async(conn, target)
        .await
        .and_then(|realm| realm.realm_id)
}

pub(super) async fn devtools_runtime_realm_for_target_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
) -> Option<RuntimeExecutionContextEvent> {
    let mut realms = devtools_realms_for_route_async(conn, target.route.clone())
        .await
        .ok()?;
    if let Some(execution_context_id) = target.execution_context_id {
        return realms
            .into_iter()
            .find(|realm| realm.context_id == Some(execution_context_id));
    }
    if let Some(index) = realms
        .iter()
        .position(|realm| realm.is_default != Some(false))
    {
        return Some(realms.remove(index));
    }
    realms.into_iter().next()
}

pub(super) fn devtools_runtime_result_ownership(
    command: &AutomationCommand,
) -> DevToolsResultOwnership {
    match command {
        AutomationCommand::EvaluateScript(command) => command.result_ownership,
        AutomationCommand::CallFunction(command) => command.result_ownership,
        _ => DevToolsResultOwnership::None,
    }
}

pub(super) fn devtools_runtime_command_result_kind(
    command: &AutomationCommand,
) -> DevToolsRuntimeCommandResultKind {
    match command {
        AutomationCommand::TerminateExecution(_) => DevToolsRuntimeCommandResultKind::Empty,
        _ => DevToolsRuntimeCommandResultKind::Script,
    }
}

pub(super) fn devtools_runtime_serialization_options(
    command: &AutomationCommand,
) -> Option<DevToolsSerializationOptions> {
    match command {
        AutomationCommand::EvaluateScript(command) => command.serialization_options.clone(),
        AutomationCommand::CallFunction(command) => command.serialization_options.clone(),
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn automation_command_has_bidi_script_channel_arguments(
    command: &AutomationCommand,
) -> bool {
    let AutomationCommand::CallFunction(command) = command else {
        return false;
    };
    matches!(command.context.protocol, FrontendProtocol::WebDriverBidi)
        && command
            .this_parameter
            .iter()
            .chain(command.arguments.iter())
            .any(bidi_local_value_contains_channel)
}

#[cfg(test)]
pub(super) fn bidi_local_value_contains_channel(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    match map.get("type").and_then(Value::as_str) {
        Some("channel") => true,
        Some("array" | "set") => map
            .get("value")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(bidi_local_value_contains_channel)),
        Some("object" | "map") => {
            map.get("value")
                .and_then(Value::as_array)
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        let Some(pair) = entry.as_array() else {
                            return false;
                        };
                        pair.iter().any(bidi_local_value_contains_channel)
                    })
                })
        }
        _ => false,
    }
}

pub(super) async fn devtools_runtime_target_async(
    conn: &mut CdpConnection,
    command: &AutomationCommand,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    if let AutomationCommand::TerminateExecution(command) = command {
        let target_id =
            command.context.target_id.as_ref().ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget")
            })?;
        return devtools_runtime_control_target(conn, target_id);
    }
    let (target_id, realm_id, world_name) = match command {
        AutomationCommand::EvaluateScript(command) => (
            command.context.target_id.as_ref(),
            command.realm_id.as_ref(),
            command.world_name.as_deref(),
        ),
        AutomationCommand::CallFunction(command) => (
            command.context.target_id.as_ref(),
            command.realm_id.as_ref(),
            command.world_name.as_deref(),
        ),
        AutomationCommand::LocateNodes(command) => (command.context.target_id.as_ref(), None, None),
        AutomationCommand::ReleaseObjects(command) => (
            command.context.target_id.as_ref(),
            command.realm_id.as_ref(),
            command.world_name.as_deref(),
        ),
        _ => (None, None, None),
    };
    if let Some(target_id) = target_id {
        return devtools_runtime_context_target_async(conn, target_id, world_name).await;
    }
    if let Some(realm_id) = realm_id {
        return devtools_runtime_realm_target_async(conn, realm_id).await;
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchTarget",
    ))
}

pub(super) fn devtools_runtime_control_target(
    conn: &CdpConnection,
    target_id: &DevToolsTargetId,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    // Do not fall back to realm discovery here: it is a MainThread operation
    // and would put the escape hatch behind the work it needs to interrupt.
    let route = conn
        .target_session_route_for_target_id(target_id.as_str())
        .or_else(|| conn.target_session_route_for_child_frame_id(target_id.as_str()))
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    Ok(DevToolsRuntimeTarget {
        route,
        execution_context_id: None,
        window_context_id: Some(target_id.clone()),
    })
}

pub(super) async fn devtools_runtime_context_target_async(
    conn: &mut CdpConnection,
    target_id: &DevToolsTargetId,
    world_name: Option<&str>,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    if let Some(route) = conn.target_session_route_for_target_id(target_id.as_str()) {
        let execution_context_id = if let Some(world_name) = world_name {
            Some(
                devtools_ensure_runtime_world_async(conn, route.clone(), target_id, world_name)
                    .await?,
            )
        } else {
            None
        };
        return Ok(DevToolsRuntimeTarget {
            route,
            execution_context_id,
            window_context_id: Some(target_id.clone()),
        });
    }

    if let Some(route) = conn.target_session_route_for_child_frame_id(target_id.as_str()) {
        let execution_context_id = if let Some(world_name) = world_name {
            Some(
                devtools_ensure_runtime_world_async(conn, route.clone(), target_id, world_name)
                    .await?,
            )
        } else {
            let owner = CommandOwnerScope::for_route(route.clone());
            let result = conn
                .child_default_execution_context_id_for_frame_id_for_owner_async(
                    &owner,
                    target_id.as_str(),
                )
                .await;
            Some(
                result
                    .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
                    .ok_or_else(|| {
                        DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            "NoExecutionContextForFrame",
                        )
                    })?,
            )
        };
        return Ok(DevToolsRuntimeTarget {
            route,
            execution_context_id,
            window_context_id: Some(target_id.clone()),
        });
    }

    let routes = devtools_get_realms_routes(conn, None)?;
    for route in routes {
        let realms = devtools_realms_for_search_route_async(conn, route.clone()).await?;
        let Some(default_realm) = realms.iter().find(|realm| {
            realm
                .frame_id
                .as_ref()
                .is_some_and(|frame_id| frame_id.as_str() == target_id.as_str())
                && realm.is_default != Some(false)
        }) else {
            continue;
        };
        let execution_context_id = if let Some(world_name) = world_name {
            Some(
                devtools_ensure_runtime_world_async(conn, route.clone(), target_id, world_name)
                    .await?,
            )
        } else {
            Some(default_realm.context_id.ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, "NoExecutionContextForFrame")
            })?)
        };
        return Ok(DevToolsRuntimeTarget {
            route,
            execution_context_id,
            window_context_id: default_realm
                .frame_id
                .clone()
                .map(|frame_id| DevToolsTargetId::from(frame_id.into_string())),
        });
    }

    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchTarget",
    ))
}

pub(super) async fn devtools_ensure_runtime_world_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
    frame_id: &DevToolsTargetId,
    world_name: &str,
) -> Result<i64, DevToolsError> {
    let owner = CommandOwnerScope::for_route(route);
    let result = conn
        .runtime_ensure_isolated_world_for_owner_async(&owner, Some(frame_id.as_str()), world_name)
        .await;
    result.map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))
}

pub(super) async fn devtools_runtime_realm_target_async(
    conn: &mut CdpConnection,
    realm_id: &DevToolsRealmId,
) -> Result<DevToolsRuntimeTarget, DevToolsError> {
    let routes = devtools_get_realms_routes(conn, None)?;
    for route in routes {
        let realms = devtools_realms_for_search_route_async(conn, route.clone()).await?;
        if let Some(realm) = realms
            .into_iter()
            .find(|realm| realm.realm_id.as_ref() == Some(realm_id))
        {
            let execution_context_id = realm.context_id.ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, "NoExecutionContextForRealm")
            })?;
            return Ok(DevToolsRuntimeTarget {
                route,
                execution_context_id: Some(execution_context_id),
                window_context_id: realm
                    .frame_id
                    .clone()
                    .map(|frame_id| DevToolsTargetId::from(frame_id.into_string())),
            });
        }
    }
    Err(DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "NoSuchRealm",
    ))
}

pub(super) async fn devtools_realms_for_search_route_async(
    conn: &mut CdpConnection,
    route: CdpSessionRoute,
) -> Result<Vec<RuntimeExecutionContextEvent>, DevToolsError> {
    match devtools_realms_for_route_async(conn, route).await {
        Ok(realms) => Ok(realms),
        Err(error)
            if matches!(error.kind, DevToolsErrorKind::Internal)
                && error.message == "NoDocumentLoaded" =>
        {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn remap_bidi_node_shared_references_for_target_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    command: &mut DevToolsCallFunctionCommand,
    realm_id: Option<&DevToolsRealmId>,
) -> Result<(), DevToolsError> {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    let mut references = Vec::new();
    if let Some(this_parameter) = command.this_parameter.as_ref() {
        collect_bidi_node_shared_reference_paths(
            this_parameter,
            BidiCallFunctionValueRoot::This,
            &mut references,
        );
    }
    for (index, argument) in command.arguments.iter().enumerate() {
        collect_bidi_node_shared_reference_paths(
            argument,
            BidiCallFunctionValueRoot::Argument(index),
            &mut references,
        );
    }

    references.sort_by(|left, right| {
        (left.root, &left.path, &left.shared_id).cmp(&(right.root, &right.path, &right.shared_id))
    });
    references.dedup();
    if references.is_empty() {
        return Ok(());
    }

    let execution_context_id = if let Some(execution_context_id) = target.execution_context_id {
        Some(execution_context_id)
    } else {
        let result = conn
            .runtime_default_or_initial_execution_context_id_for_owner_async(&owner)
            .await;
        Some(
            result
                .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
                .ok_or_else(|| {
                    DevToolsError::new(DevToolsErrorKind::Internal, "NoDefaultExecutionContext")
                })?,
        )
    };

    for reference in references {
        let Some(target_shared_id) = remap_bidi_node_shared_id_for_target_async(
            conn,
            &owner,
            execution_context_id,
            &reference.shared_id,
            realm_id,
        )
        .await?
        else {
            continue;
        };
        let Some(value) =
            bidi_call_function_value_at_path_mut(command, reference.root, &reference.path)
        else {
            continue;
        };
        if let Some(map) = value.as_object_mut() {
            map.insert("sharedId".to_owned(), json!(target_shared_id.into_string()));
        }
    }

    Ok(())
}

pub(super) fn collect_bidi_node_shared_reference_paths(
    value: &Value,
    root: BidiCallFunctionValueRoot,
    out: &mut Vec<BidiNodeSharedReferencePath>,
) {
    let mut stack = vec![(
        value,
        Vec::<BidiValuePathSegment>::new(),
        MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH,
    )];
    while let Some((value, path, remaining_tree_depth)) = stack.pop() {
        let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
            continue;
        };
        match value {
            Value::Object(map) => {
                if let Some(shared_id) = map.get("sharedId").and_then(Value::as_str) {
                    out.push(BidiNodeSharedReferencePath {
                        root,
                        path,
                        shared_id: shared_id.to_owned(),
                    });
                    continue;
                }
                if map.get("handle").and_then(Value::as_str).is_some() {
                    continue;
                }
                let children = map.iter().collect::<Vec<_>>();
                for (key, child) in children.into_iter().rev() {
                    let mut child_path = path.clone();
                    child_path.push(BidiValuePathSegment::Key(key.clone()));
                    stack.push((child, child_path, next_tree_depth));
                }
            }
            Value::Array(values) => {
                for index in (0..values.len()).rev() {
                    let mut child_path = path.clone();
                    child_path.push(BidiValuePathSegment::Index(index));
                    stack.push((&values[index], child_path, next_tree_depth));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

pub(super) async fn remap_bidi_node_shared_id_for_target_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    execution_context_id: Option<i64>,
    shared_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) -> Result<Option<DevToolsRemoteHandleId>, DevToolsError> {
    let is_internal_bidi_node_id = is_webdriver_bidi_node_shared_id(shared_id);
    let snapshot = bidi_node_snapshot_for_shared_id_async(conn, owner, shared_id, 0, false).await?;
    let Some(snapshot) = snapshot else {
        return if is_internal_bidi_node_id {
            Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchNode,
                "Could not find node with given id",
            ))
        } else {
            Ok(None)
        };
    };

    let Some(backend_node_id) = snapshot.snapshot.backend_node_id else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        ));
    };
    let Some(remote_object) = conn
        .runtime_remote_object_for_backend_node_id_for_owner_async(
            owner,
            backend_node_id,
            execution_context_id,
            None,
        )
        .await
        .map_err(|message| DevToolsError::new(DevToolsErrorKind::Internal, message))?
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        ));
    };
    let Some(object_id) = remote_object
        .get("objectId")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchNode,
            "Could not find node with given id",
        ));
    };

    register_remapped_bidi_node_remote_object(conn, owner, &remote_object, &object_id, realm_id);
    Ok(Some(DevToolsRemoteHandleId::from(object_id)))
}

pub(super) fn register_remapped_bidi_node_remote_object(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    remote_object: &Value,
    object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    if let Some(realm_id) = realm_id {
        conn.register_runtime_remote_object_ids_for_owner_with_realm(
            owner,
            vec![object_id.to_owned()],
            realm_id.as_str(),
        );
    } else {
        conn.register_runtime_remote_object_ids_from_value_for_owner(owner, remote_object);
    }
}
