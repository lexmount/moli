use super::*;

pub(super) fn is_attribute_node_remote_metadata(
    remote_type: Option<&str>,
    remote_subtype: Option<&str>,
    description: Option<&str>,
    class_name: Option<&str>,
) -> bool {
    remote_type == Some("object")
        && matches!(remote_subtype, None | Some("node"))
        && (matches!(class_name, Some("Attr"))
            || matches!(description, Some("Attr") | Some("[object Attr]")))
}

pub(super) fn deep_serialized_internal_id(value: &Value) -> Option<String> {
    value
        .get("internalId")
        .or_else(|| value.get("weakLocalObjectReference"))
        .and_then(|reference| {
            reference
                .as_str()
                .map(str::to_owned)
                .or_else(|| reference.as_u64().map(|reference| reference.to_string()))
                .or_else(|| reference.as_i64().map(|reference| reference.to_string()))
        })
}

pub(super) fn deep_serialized_property<'a>(
    properties: &'a [Value],
    name: &str,
) -> Option<&'a Value> {
    properties.iter().find_map(|property| {
        let pair = property.as_array()?;
        let [key, value] = pair.as_slice() else {
            return None;
        };
        (key.as_str() == Some(name)).then_some(value)
    })
}

pub(super) fn collect_deep_serialized_node_candidate_paths(
    value: &Value,
) -> Vec<DeepSerializedNodeCandidatePath> {
    let mut paths = Vec::new();
    let mut stack = vec![DeepSerializedNodeCandidateFrame {
        value,
        json_pointer: String::new(),
        js_path: Vec::new(),
        remaining_tree_depth: MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH,
    }];
    while let Some(frame) = stack.pop() {
        collect_deep_serialized_node_candidate_path_frame(frame, &mut paths, &mut stack);
    }
    paths.sort_by_key(|path| path.js_path.len());
    paths
}

pub(super) fn collect_deep_serialized_node_candidate_path_frame<'a>(
    frame: DeepSerializedNodeCandidateFrame<'a>,
    out: &mut Vec<DeepSerializedNodeCandidatePath>,
    stack: &mut Vec<DeepSerializedNodeCandidateFrame<'a>>,
) {
    let DeepSerializedNodeCandidateFrame {
        value,
        json_pointer,
        js_path,
        remaining_tree_depth,
    } = frame;
    let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
        return;
    };
    if !js_path.is_empty() && is_deep_serialized_node_candidate(value) {
        out.push(DeepSerializedNodeCandidatePath {
            json_pointer: json_pointer.clone(),
            js_path: js_path.clone(),
        });
    }

    let Some(value_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    let Some(children) = value.get("value").and_then(Value::as_array) else {
        return;
    };

    let mut pending = Vec::new();
    match value_type {
        "object" => {
            for (index, property) in children.iter().enumerate() {
                let Some(pair) = property.as_array() else {
                    continue;
                };
                let [key, child] = pair.as_slice() else {
                    continue;
                };
                let Some(key) = key.as_str() else {
                    continue;
                };
                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "property",
                    "key": key,
                }));
                let value_pointer = json_pointer_child(&json_pointer, "value");
                let property_pointer = json_pointer_child(&value_pointer, index);
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&property_pointer, "1"),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        "array" | "htmlcollection" | "nodelist" => {
            for (index, child) in children.iter().enumerate() {
                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "index",
                    "index": index,
                }));
                let value_pointer = json_pointer_child(&json_pointer, "value");
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&value_pointer, index),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        "set" => {
            for (index, child) in children.iter().enumerate() {
                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "iterable",
                    "index": index,
                }));
                let value_pointer = json_pointer_child(&json_pointer, "value");
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&value_pointer, index),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        "map" => {
            for (index, entry) in children.iter().enumerate() {
                let Some(pair) = entry.as_array() else {
                    continue;
                };
                let [key, child] = pair.as_slice() else {
                    continue;
                };
                let value_pointer = json_pointer_child(&json_pointer, "value");
                let entry_pointer = json_pointer_child(&value_pointer, index);
                let mut key_js_path = js_path.clone();
                key_js_path.push(json!({
                    "kind": "mapEntry",
                    "index": index,
                    "part": 0,
                }));
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: key,
                    json_pointer: json_pointer_child(&entry_pointer, "0"),
                    js_path: key_js_path,
                    remaining_tree_depth: next_tree_depth,
                });

                let mut child_js_path = js_path.clone();
                child_js_path.push(json!({
                    "kind": "mapEntry",
                    "index": index,
                    "part": 1,
                }));
                pending.push(DeepSerializedNodeCandidateFrame {
                    value: child,
                    json_pointer: json_pointer_child(&entry_pointer, "1"),
                    js_path: child_js_path,
                    remaining_tree_depth: next_tree_depth,
                });
            }
        }
        _ => {}
    }
    for frame in pending.into_iter().rev() {
        stack.push(frame);
    }
}

pub(super) fn is_deep_serialized_node_candidate(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("object")
}

pub(super) fn json_pointer_child(parent: &str, segment: impl ToString) -> String {
    let segment = segment.to_string();
    format!("{parent}/{}", escape_json_pointer_segment(&segment))
}

pub(super) fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

pub(super) fn json_pointer_matches_or_descends_from(pointer: &str, ancestor: &str) -> bool {
    pointer == ancestor
        || pointer
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(super) async fn materialize_devtools_script_node_remote_value_async(
    conn: &mut CdpConnection,
    result: &mut AutomationResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
) {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    let Some(value) = devtools_script_result_remote_value_mut(result) else {
        return;
    };
    let is_attribute_node = is_attribute_node_remote_metadata(
        value.remote_type.as_deref(),
        value.remote_subtype.as_deref(),
        value.description.as_deref(),
        value.class_name.as_deref(),
    );
    if value.remote_subtype.as_deref() != Some("node") && !is_attribute_node {
        return;
    }
    if materialize_devtools_script_node_remote_value_from_deep_serialized(
        conn, &owner, value, realm_id,
    ) {
        return;
    }
    let Some(shared_id) = value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };
    let node_options = bidi_node_serialization_options(serialization_options);
    let object_snapshot = match conn
        .document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
            &owner,
            &shared_id,
            node_options.snapshot_depth,
            true,
        )
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::debug!(%shared_id, %error, "failed to materialize BiDi node remote value");
            None
        }
    };
    let Some(object_snapshot) = object_snapshot else {
        if is_attribute_node {
            match devtools_attribute_node_value_async(conn, target, shared_id.clone()).await {
                Ok(Some(attribute_value)) => {
                    value.remote_subtype = Some("node".to_owned());
                    value.node_value = Some(attribute_value);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(
                        %shared_id,
                        ?error,
                        "failed to materialize BiDi attribute node remote value"
                    );
                }
            }
        }
        if value.node_value.is_some() {
            return;
        }
        match devtools_detached_node_value_async(conn, target, shared_id.clone(), &node_options)
            .await
        {
            Ok(Some(node_value)) => {
                value.remote_subtype = Some("node".to_owned());
                value.node_value = Some(node_value);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(
                    %shared_id,
                    ?error,
                    "failed to materialize detached BiDi node remote value"
                );
            }
        }
        return;
    };
    let Some(canonical_shared_id) = bidi_node_shared_id_for_snapshot(&object_snapshot.snapshot)
    else {
        return;
    };
    register_bidi_node_bindings_for_snapshot_tree(conn, &owner, &object_snapshot.snapshot).await;
    register_bidi_node_shared_id_alias(conn, &owner, &canonical_shared_id, &shared_id, realm_id);
    value.node_id = object_snapshot.snapshot.frontend_node_id;
    value.backend_node_id = object_snapshot.snapshot.backend_node_id;
    value.shared_id = Some(canonical_shared_id);
    value.node_value = Some(bidi_node_value_from_snapshot(
        &object_snapshot.snapshot,
        &node_options,
    ));
}

pub(super) async fn materialize_devtools_script_deep_serialized_node_remote_values_async(
    conn: &mut CdpConnection,
    result: &mut AutomationResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(value) = devtools_script_result_remote_value_mut(result) else {
        return;
    };
    let Some(root_object_id) = value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };
    let Some(deep_serialized_value) = value.deep_serialized_value.as_mut() else {
        return;
    };
    let candidate_paths = collect_deep_serialized_node_candidate_paths(deep_serialized_value);
    if candidate_paths.is_empty() {
        return;
    }

    let node_options = bidi_node_serialization_options(serialization_options);
    let mut materialized_paths: Vec<String> = Vec::new();
    for candidate_path in candidate_paths {
        if materialized_paths
            .iter()
            .any(|path| json_pointer_matches_or_descends_from(&candidate_path.json_pointer, path))
        {
            continue;
        }

        let internal_id = deep_serialized_value
            .pointer(&candidate_path.json_pointer)
            .and_then(deep_serialized_internal_id);
        let Some(remote_value) =
            materialize_devtools_script_deep_serialized_node_remote_value_async(
                conn,
                target,
                &root_object_id,
                &candidate_path.js_path,
                &node_options,
                serialization_options,
                realm_id,
            )
            .await
        else {
            continue;
        };
        let mut remote_value = remote_value;
        if let (Some(internal_id), Some(map)) = (internal_id, remote_value.as_object_mut()) {
            map.insert("internalId".to_owned(), json!(internal_id));
        }
        if let Some(slot) = deep_serialized_value.pointer_mut(&candidate_path.json_pointer) {
            *slot = remote_value;
            materialized_paths.push(candidate_path.json_pointer);
        }
    }
}

pub(super) async fn materialize_devtools_script_dom_collection_remote_value_async(
    conn: &mut CdpConnection,
    result: &mut AutomationResult,
    serialization_options: Option<&DevToolsSerializationOptions>,
    target: &DevToolsRuntimeTarget,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(root_object_id) = devtools_script_result_remote_value(result)
        .and_then(|value| value.shared_id.as_ref())
        .map(|shared_id| shared_id.as_str().to_owned())
    else {
        return;
    };
    let probe = match devtools_dom_collection_probe_async(conn, target, &root_object_id).await {
        Ok(probe) => probe,
        Err(error) => {
            tracing::debug!(
                %root_object_id,
                ?error,
                "failed to probe BiDi DOM collection remote value"
            );
            None
        }
    };
    let Some(probe) = probe else {
        return;
    };

    let node_options = bidi_node_serialization_options(serialization_options);
    let entries = materialize_bidi_dom_collection_entries_async(
        conn,
        target,
        &root_object_id,
        probe.length,
        &node_options.with_decremented_value_depth(),
        realm_id,
    )
    .await;
    if let Some(remote_value) = devtools_script_result_remote_value_mut(result) {
        remote_value.remote_type = Some("object".to_owned());
        remote_value.remote_subtype = Some(probe.kind.as_bidi_type().to_owned());
        remote_value.node_value = None;
        remote_value.deep_serialized_value = Some(json!({
            "type": probe.kind.as_bidi_type(),
            "value": entries,
        }));
    }
}

pub(super) async fn devtools_dom_collection_probe_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
) -> Result<Option<BidiDomCollectionProbe>, DevToolsError> {
    let command = AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(root_object_id.to_owned())),
        this_parameter: None,
        function_declaration: r#"function() {
            const length = Number(this && this.length);
            if (!Number.isFinite(length) || length < 0) {
                return null;
            }
            function hasPrototypeNamed(value, name) {
                let proto = Object.getPrototypeOf(value);
                const expected = globalThis[name] && globalThis[name].prototype;
                while (proto !== null) {
                    if (expected && proto === expected) {
                        return true;
                    }
                    const ctorName = proto.constructor && proto.constructor.name;
                    if (ctorName === name) {
                        return true;
                    }
                    proto = Object.getPrototypeOf(proto);
                }
                return false;
            }
            let kind = null;
            try {
                if (
                    (typeof HTMLCollection === "function" && this instanceof HTMLCollection) ||
                    hasPrototypeNamed(this, "HTMLCollection") ||
                    (typeof this.item === "function" && typeof this.namedItem === "function")
                ) {
                    kind = "htmlcollection";
                }
            } catch (_) {}
            try {
                if (
                    kind === null &&
                    (
                        (typeof NodeList === "function" && this instanceof NodeList) ||
                        hasPrototypeNamed(this, "NodeList") ||
                        (
                            typeof this.item === "function" &&
                            typeof this.namedItem !== "function" &&
                            (typeof this.forEach === "function" ||
                                typeof this[Symbol.iterator] === "function")
                        )
                    )
                ) {
                    kind = "nodelist";
                }
            } catch (_) {}
            if (kind === null) {
                return null;
            }
            return {
                kind,
                length: Math.min(Math.trunc(length), 4096),
            };
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    });
    let Some(value) = devtools_probe_value_async(conn, target.clone(), command).await? else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(kind) = object
        .get("kind")
        .and_then(Value::as_str)
        .and_then(BidiDomCollectionKind::from_bidi_type)
    else {
        return Ok(None);
    };
    let length = object
        .get("length")
        .and_then(Value::as_u64)
        .map(|length| length.min(4096) as usize)
        .unwrap_or(0);
    Ok(Some(BidiDomCollectionProbe { kind, length }))
}

pub(super) async fn materialize_bidi_dom_collection_entries_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
    length: usize,
    node_options: &BidiNodeSerializationOptions,
    realm_id: Option<&DevToolsRealmId>,
) -> Vec<Value> {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    let mut entries = Vec::new();
    for index in 0..length {
        let js_path = [json!({
            "kind": "index",
            "index": index,
        })];
        let remote_value = match devtools_deep_serialized_path_remote_value_async(
            conn,
            target,
            root_object_id,
            &js_path,
            Some(&devtools_serialization_options_for_node_probe(node_options)),
        )
        .await
        {
            Ok(remote_value) => remote_value,
            Err(error) => {
                tracing::debug!(
                    %root_object_id,
                    index,
                    ?error,
                    "failed to resolve BiDi DOM collection entry"
                );
                None
            }
        };
        let Some(remote_value) = remote_value else {
            continue;
        };
        let Some(object_id) = remote_value
            .shared_id
            .as_ref()
            .map(|shared_id| shared_id.as_str().to_owned())
        else {
            continue;
        };
        if let Some(remote) =
            bidi_node_remote_value_from_deep_serialized_remote_value(&remote_value)
        {
            register_devtools_script_remote_object_realm(conn, &owner, &object_id, realm_id);
            register_bidi_node_remote_value_shared_id_alias(
                conn, &owner, &remote, &object_id, realm_id,
            );
            entries.push(remote);
            continue;
        }
        let object_snapshot = match conn
            .document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
                &owner,
                &object_id,
                node_options.snapshot_depth,
                true,
            )
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::debug!(
                    %object_id,
                    index,
                    %error,
                    "failed to materialize BiDi DOM collection entry node"
                );
                None
            }
        };
        let Some(object_snapshot) = object_snapshot else {
            continue;
        };
        register_devtools_script_remote_object_realm(conn, &owner, &object_id, realm_id);
        let Some(shared_id) = bidi_node_shared_id_for_snapshot(&object_snapshot.snapshot) else {
            continue;
        };
        register_bidi_node_bindings_for_snapshot_tree(conn, &owner, &object_snapshot.snapshot)
            .await;
        register_bidi_node_shared_id_alias(conn, &owner, &shared_id, &object_id, realm_id);
        entries.push(bidi_node_remote_value_from_snapshot(
            &object_snapshot.snapshot,
            shared_id,
            node_options,
        ));
    }
    entries
}

pub(super) async fn materialize_devtools_script_deep_serialized_node_remote_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
    js_path: &[Value],
    node_options: &BidiNodeSerializationOptions,
    serialization_options: Option<&DevToolsSerializationOptions>,
    realm_id: Option<&DevToolsRealmId>,
) -> Option<Value> {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    let remote_value = match devtools_deep_serialized_path_remote_value_async(
        conn,
        target,
        root_object_id,
        js_path,
        serialization_options,
    )
    .await
    {
        Ok(remote_value) => remote_value?,
        Err(error) => {
            tracing::debug!(
                %root_object_id,
                ?js_path,
                ?error,
                "failed to resolve deep serialized BiDi node candidate"
            );
            return None;
        }
    };
    let object_id = remote_value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned())?;
    if let Some(remote) = bidi_node_remote_value_from_deep_serialized_remote_value(&remote_value) {
        register_devtools_script_remote_object_realm(conn, &owner, &object_id, realm_id);
        register_bidi_node_remote_value_shared_id_alias(
            conn, &owner, &remote, &object_id, realm_id,
        );
        return Some(remote);
    }

    let object_snapshot = match conn
        .document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
            &owner,
            &object_id,
            node_options.snapshot_depth,
            true,
        )
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::debug!(
                %object_id,
                ?js_path,
                %error,
                "failed to materialize deep serialized BiDi node candidate"
            );
            None
        }
    };

    if let Some(object_snapshot) = object_snapshot {
        register_devtools_script_remote_object_realm(conn, &owner, &object_id, realm_id);
        let shared_id = bidi_node_shared_id_for_snapshot(&object_snapshot.snapshot)?;
        register_bidi_node_bindings_for_snapshot_tree(conn, &owner, &object_snapshot.snapshot)
            .await;
        register_bidi_node_shared_id_alias(conn, &owner, &shared_id, &object_id, realm_id);
        return Some(bidi_node_remote_value_from_snapshot(
            &object_snapshot.snapshot,
            shared_id,
            node_options,
        ));
    }

    let is_attribute_node = is_attribute_node_remote_metadata(
        remote_value.remote_type.as_deref(),
        remote_value.remote_subtype.as_deref(),
        remote_value.description.as_deref(),
        remote_value.class_name.as_deref(),
    );
    if !is_attribute_node {
        let detached_value =
            match devtools_detached_node_value_async(conn, target, object_id.clone(), node_options)
                .await
            {
                Ok(Some(value)) => value,
                Ok(None) => return None,
                Err(error) => {
                    tracing::debug!(
                        %object_id,
                        ?js_path,
                        ?error,
                        "failed to materialize detached deep serialized BiDi node candidate"
                    );
                    return None;
                }
            };
        register_devtools_script_remote_object_realm(conn, &owner, &object_id, realm_id);
        return Some(json!({
            "type": "node",
            "sharedId": object_id,
            "value": detached_value,
        }));
    }

    let attribute_value =
        match devtools_attribute_node_value_async(conn, target, object_id.clone()).await {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(
                    %object_id,
                    ?js_path,
                    ?error,
                    "failed to materialize deep serialized BiDi attribute node candidate"
                );
                return None;
            }
        };
    register_devtools_script_remote_object_realm(conn, &owner, &object_id, realm_id);
    Some(json!({
        "type": "node",
        "sharedId": object_id,
        "value": attribute_value,
    }))
}

pub(super) async fn devtools_deep_serialized_path_remote_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    root_object_id: &str,
    js_path: &[Value],
    serialization_options: Option<&DevToolsSerializationOptions>,
) -> Result<Option<DevToolsRemoteValue>, DevToolsError> {
    let command = AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(root_object_id.to_owned())),
        this_parameter: None,
        function_declaration: r#"function(path) {
            let value = this;
            for (const step of path) {
                if (value == null || step == null || typeof step !== "object") {
                    return null;
                }
                if (step.kind === "property") {
                    value = value[step.key];
                } else if (step.kind === "index") {
                    value = value[Number(step.index)];
                } else if (step.kind === "iterable") {
                    if (typeof value[Symbol.iterator] !== "function") {
                        return null;
                    }
                    value = Array.from(value)[Number(step.index)];
                } else if (step.kind === "mapEntry") {
                    if (typeof value[Symbol.iterator] !== "function") {
                        return null;
                    }
                    const entry = Array.from(value)[Number(step.index)];
                    if (!entry) {
                        return null;
                    }
                    value = entry[Number(step.part)];
                } else {
                    return null;
                }
            }
            return value == null ? null : value;
        }"#
        .to_owned(),
        arguments: vec![Value::Array(js_path.to_vec())],
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::Root,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: serialization_options.cloned(),
    });
    devtools_probe_remote_value_async(conn, target.clone(), command).await
}

pub(super) fn materialize_devtools_script_node_remote_value_from_deep_serialized(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    value: &mut DevToolsRemoteValue,
    realm_id: Option<&DevToolsRealmId>,
) -> bool {
    let Some(remote) = bidi_node_remote_value_from_deep_serialized_remote_value(value) else {
        return false;
    };
    let original_object_id = value
        .shared_id
        .as_ref()
        .map(|shared_id| shared_id.as_str().to_owned());
    let Some(shared_id) = bidi_node_remote_value_shared_id(&remote).map(str::to_owned) else {
        return false;
    };
    let Some(node_value) = remote.get("value").cloned() else {
        return false;
    };
    if let Some(original_object_id) = original_object_id.as_deref() {
        register_bidi_node_remote_value_shared_id_alias(
            conn,
            owner,
            &remote,
            original_object_id,
            realm_id,
        );
    }
    value.remote_type = Some("object".to_owned());
    value.remote_subtype = Some("node".to_owned());
    value.shared_id = Some(DevToolsRemoteHandleId::from(shared_id));
    value.node_value = Some(node_value);
    true
}

pub(super) fn register_bidi_node_remote_value_shared_id_alias(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    remote: &Value,
    remote_object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(shared_id) = bidi_node_remote_value_shared_id(remote) else {
        return;
    };
    register_bidi_node_shared_id_alias(
        conn,
        owner,
        &DevToolsRemoteHandleId::from(shared_id.to_owned()),
        remote_object_id,
        realm_id,
    );
}

pub(super) async fn register_bidi_node_bindings_for_snapshot_tree(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    snapshot: &DocumentNodeSnapshot,
) {
    let mut entries = Vec::new();
    let mut stack = vec![snapshot];
    while let Some(snapshot) = stack.pop() {
        if let Some(backend_node_id) = snapshot.backend_node_id {
            entries.push((
                webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id),
                backend_node_id,
            ));
        }
        stack.extend(snapshot.shadow_roots.iter().rev());
        stack.extend(snapshot.children.iter().rev());
    }

    for (shared_id, backend_node_id) in entries {
        if let Err(error) = conn
            .register_document_bidi_node_binding_for_owner_async(
                owner,
                shared_id.as_str(),
                backend_node_id,
            )
            .await
        {
            tracing::debug!(
                %error,
                shared_id = shared_id.as_str(),
                backend_node_id,
                "failed to register renderer BiDi node binding"
            );
        }
    }
}

pub(super) async fn devtools_detached_node_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    remote_object_id: String,
    node_options: &BidiNodeSerializationOptions,
) -> Result<Option<Value>, DevToolsError> {
    let command = AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(remote_object_id)),
        this_parameter: None,
        function_declaration: r#"function(maxDepth) {
            if (!this || typeof this.nodeType !== "number") {
                return null;
            }
            const nodeType = Number(this.nodeType);
            if (!Number.isFinite(nodeType)) {
                return null;
            }
            const childNodes = this.childNodes;
            const childNodeCount = childNodes == null ? 0 : Number(childNodes.length) || 0;
            const value = {
                nodeType,
                childNodeCount,
            };
            if (nodeType === 1) {
                value.localName = this.localName == null ? "" : String(this.localName);
                value.namespaceURI = this.namespaceURI == null ? null : String(this.namespaceURI);
                const attributes = {};
                if (this.attributes) {
                    for (let index = 0; index < this.attributes.length; index++) {
                        const attr = this.attributes[index];
                        if (!attr) {
                            continue;
                        }
                        const localName = attr.localName == null ? String(attr.name ?? "") : String(attr.localName);
                        const name = attr.prefix ? `${attr.prefix}:${localName}` : (attr.name == null ? localName : String(attr.name));
                        attributes[name] = attr.value == null ? "" : String(attr.value);
                    }
                }
                value.attributes = attributes;
                value.shadowRoot = null;
            } else if (nodeType === 3 || nodeType === 4 || nodeType === 7 || nodeType === 8) {
                value.nodeValue = this.nodeValue == null ? "" : String(this.nodeValue);
            } else if (nodeType === 11 && this.mode != null) {
                value.mode = String(this.mode);
            }
            const depth = Number(maxDepth);
            if (depth !== 0 && childNodeCount === 0) {
                value.children = [];
            }
            return value;
        }"#
        .to_owned(),
        arguments: vec![json!(node_options.value_depth)],
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
        serialization_options: None,
    });
    let Some(value) = devtools_probe_value_async(conn, target.clone(), command).await? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(value))
}

pub(super) fn register_devtools_script_remote_object_realm(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    remote_object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(realm_id) = realm_id else {
        return;
    };
    conn.register_runtime_remote_object_ids_for_owner_with_realm(
        owner,
        vec![remote_object_id.to_owned()],
        realm_id.as_str(),
    );
}

pub(super) fn register_bidi_node_shared_id_alias(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    shared_id: &DevToolsRemoteHandleId,
    remote_object_id: &str,
    realm_id: Option<&DevToolsRealmId>,
) {
    let Some(realm_id) = realm_id else {
        return;
    };
    if shared_id.as_str() == remote_object_id || is_webdriver_bidi_node_shared_id(remote_object_id)
    {
        return;
    }
    conn.register_runtime_remote_object_alias_for_owner_with_realm(
        owner,
        shared_id.as_str().to_owned(),
        remote_object_id.to_owned(),
        realm_id.as_str(),
    );
}

pub(super) async fn devtools_attribute_node_value_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    remote_object_id: String,
) -> Result<Option<Value>, DevToolsError> {
    let command = AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
        context: AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: target.window_context_id.clone(),
            browser_context_id: None,
        },
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(remote_object_id)),
        this_parameter: None,
        function_declaration: r#"function() {
            if (!this || Number(this.nodeType) !== 2) {
                return null;
            }
            const localName = this.localName == null ? String(this.name ?? "") : String(this.localName);
            const namespaceURI = this.namespaceURI == null ? null : String(this.namespaceURI);
            const nodeValue = this.nodeValue == null ? String(this.value ?? "") : String(this.nodeValue);
            return {
                childNodeCount: 0,
                localName,
                namespaceURI,
                nodeType: 2,
                nodeValue,
            };
        }"#
        .to_owned(),
        arguments: Vec::new(),
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::ByValue,
        object_group: None,
        preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
        serialization_options: None,
    });
    let Some(value) = devtools_probe_value_async(conn, target.clone(), command).await? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(value))
}
