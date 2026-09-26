use super::*;

pub(super) async fn execute_devtools_locate_nodes_command_async(
    conn: &mut CdpConnection,
    mut command: DevToolsLocateNodesCommand,
) -> DevToolsCommandExecutionOutput {
    if matches!(&command.locator, DevToolsLocateNodesLocator::Context(_)) {
        return execute_devtools_locate_nodes_context_command_async(conn, command).await;
    }
    let target =
        match devtools_runtime_target_async(conn, &DevToolsCommand::LocateNodes(command.clone()))
            .await
        {
            Ok(target) => target,
            Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
        };
    let context = command.context.clone();
    let locator = command.locator.clone();
    let start_nodes = std::mem::take(&mut command.start_nodes);
    let start_node_references = std::mem::take(&mut command.start_node_references);
    let start_node_inputs = match locate_nodes_start_node_inputs_async(
        conn,
        &target,
        start_nodes,
        start_node_references,
    )
    .await
    {
        Ok(inputs) => inputs,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    let (start_node_arguments, start_node_handles) =
        match resolve_locate_nodes_start_node_inputs(conn, &context, start_node_inputs).await {
            Ok(arguments) => arguments,
            Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
        };
    let call_function = locate_nodes_call_function_command(command, start_node_arguments);
    let output = Box::pin(execute_devtools_runtime_command_async_with_protocol_events(
        conn,
        DevToolsCommand::CallFunction(call_function),
    ))
    .await;
    let (result, protocol_events, renderer_output_predecessor) = output.into_parts();
    release_locate_nodes_start_node_handles(conn, context, start_node_handles).await;
    let result = match result {
        Ok(result) => {
            locate_nodes_result_from_script_result_async(conn, &target, result, &locator).await
        }
        Err(error) => Err(error),
    };
    DevToolsCommandExecutionOutput::from_parts(result, protocol_events, renderer_output_predecessor)
}

pub(super) async fn execute_devtools_locate_nodes_context_command_async(
    conn: &mut CdpConnection,
    command: DevToolsLocateNodesCommand,
) -> DevToolsCommandExecutionOutput {
    let DevToolsLocateNodesCommand {
        context,
        locator,
        start_nodes,
        start_node_references,
        serialization_options,
        ..
    } = command;
    let DevToolsLocateNodesLocator::Context(ref frame_id) = locator else {
        return DevToolsCommandExecutionOutput::new(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedLocateNodesContextLocator",
        )));
    };
    if !start_nodes.is_empty() || !start_node_references.is_empty() {
        return DevToolsCommandExecutionOutput::new(Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "Start nodes are not supported",
        )));
    }

    let owner_result = match crate::domains::dom::execute_devtools_dom_command_async(
        conn,
        DevToolsCommand::GetFrameOwner(DevToolsGetFrameOwnerCommand {
            context: context.clone(),
            frame_id: frame_id.clone(),
        }),
    )
    .await
    .map_err(locate_nodes_context_owner_error)
    {
        Ok(result) => result,
        Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
    };
    let DevToolsCommandResult::GetFrameOwner(owner_result) = owner_result else {
        return DevToolsCommandExecutionOutput::new(Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedLocateNodesFrameOwnerResult",
        )));
    };
    let owner_node_id = owner_result.node_id;

    let max_dom_depth = serialization_options
        .as_ref()
        .and_then(|options| options.max_dom_depth);
    let (start_node_arguments, start_node_handles) =
        match resolve_locate_nodes_start_node_reference_arguments(
            conn,
            &context,
            vec![DevToolsDomNodeReference::BackendNodeId(
                owner_result.backend_node_id,
            )],
        )
        .await
        .map_err(locate_nodes_context_owner_error)
        {
            Ok(arguments) => arguments,
            Err(error) => return DevToolsCommandExecutionOutput::new(Err(error)),
        };
    let call_function_context = context.clone();
    let call_function = DevToolsCallFunctionCommand {
        context: call_function_context,
        realm_id: None,
        world_name: None,
        object_id: None,
        this_parameter: None,
        function_declaration: r#"function(node) {
            if (!node) {
                throw new Error('Context does not exist');
            }
            return [node];
        }"#
        .to_owned(),
        arguments: start_node_arguments,
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: Some(DevToolsSerializationOptions {
            max_object_depth: Some(1),
            max_dom_depth,
            include_shadow_tree: serialization_options
                .as_ref()
                .and_then(|options| options.include_shadow_tree.clone()),
        }),
    };
    let output = Box::pin(execute_devtools_runtime_command_async_with_protocol_events(
        conn,
        DevToolsCommand::CallFunction(call_function),
    ))
    .await;
    let (result, protocol_events, renderer_output_predecessor) = output.into_parts();
    release_locate_nodes_start_node_handles(conn, context, start_node_handles).await;
    let result = result.and_then(|result| {
        let mut result = locate_nodes_result_from_script_result(result, &locator)?;
        if let DevToolsCommandResult::LocateNodes(result) = &mut result
            && result.node_ids.is_empty()
        {
            result.node_ids.push(owner_node_id);
            if let Some(node) = result.nodes.first_mut() {
                node.node_id = Some(owner_node_id);
            }
        }
        Ok(result)
    });
    DevToolsCommandExecutionOutput::from_parts(result, protocol_events, renderer_output_predecessor)
}

pub(super) fn locate_nodes_context_owner_error(error: DevToolsError) -> DevToolsError {
    match error.kind {
        DevToolsErrorKind::NoSuchSession | DevToolsErrorKind::NoSuchTarget => error,
        _ => DevToolsError::new(DevToolsErrorKind::InvalidArgument, "Context does not exist"),
    }
}

pub(super) fn locate_nodes_call_function_command(
    command: DevToolsLocateNodesCommand,
    resolved_start_node_arguments: Vec<Value>,
) -> DevToolsCallFunctionCommand {
    let DevToolsLocateNodesCommand {
        context,
        locator,
        max_node_count,
        start_nodes,
        start_node_references: _,
        serialization_options,
    } = command;
    let max_node_count = max_node_count.unwrap_or(0);
    let mut arguments = locate_nodes_locator_arguments(&locator, max_node_count);
    arguments.extend(resolved_start_node_arguments);
    arguments.extend(start_nodes);
    let max_dom_depth = serialization_options
        .as_ref()
        .and_then(|options| options.max_dom_depth);

    DevToolsCallFunctionCommand {
        context,
        realm_id: None,
        world_name: None,
        object_id: None,
        this_parameter: None,
        function_declaration: locate_nodes_function_declaration(&locator).to_owned(),
        arguments,
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: Some(DevToolsSerializationOptions {
            max_object_depth: Some(2),
            max_dom_depth,
            include_shadow_tree: serialization_options
                .as_ref()
                .and_then(|options| options.include_shadow_tree.clone()),
        }),
    }
}

pub(super) async fn locate_nodes_start_node_inputs_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    start_nodes: Vec<Value>,
    start_node_references: Vec<DevToolsDomNodeReference>,
) -> Result<Vec<LocateNodesStartNodeInput>, DevToolsError> {
    let mut inputs = start_node_references
        .into_iter()
        .map(LocateNodesStartNodeInput::Reference)
        .collect::<Vec<_>>();
    for node in start_nodes {
        let Some(object) = node.as_object() else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "locateNodes startNodes entries must be node remote values",
            ));
        };
        if let Some(shared_id) = object.get("sharedId").and_then(Value::as_str) {
            if let Some(reference) = renderer_locate_nodes_start_node_reference_for_shared_id_async(
                conn, target, shared_id,
            )
            .await?
            {
                inputs.push(LocateNodesStartNodeInput::Reference(reference));
            } else {
                inputs.push(LocateNodesStartNodeInput::Raw(node));
            }
            continue;
        }
        if object.get("handle").and_then(Value::as_str).is_some() {
            inputs.push(LocateNodesStartNodeInput::Raw(node));
            continue;
        }
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "locateNodes startNodes entries must be node remote values",
        ));
    }
    Ok(inputs)
}

pub(super) async fn renderer_locate_nodes_start_node_reference_for_shared_id_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    shared_id: &str,
) -> Result<Option<DevToolsDomNodeReference>, DevToolsError> {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    let result = conn
        .document_bidi_node_binding_for_owner_async(&owner, shared_id)
        .await;
    match result {
        Ok(RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id)) => Ok(Some(
            DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        )),
        Ok(RendererDomBidiNodeBindingResolution::NotFound) => Ok(None),
        Err(error) => {
            tracing::debug!(
                %error,
                %shared_id,
                "failed to resolve locateNodes start node through renderer BiDi binding"
            );
            Ok(None)
        }
    }
}

pub(super) async fn resolve_locate_nodes_start_node_inputs(
    conn: &mut CdpConnection,
    context: &DevToolsCommandContext,
    inputs: Vec<LocateNodesStartNodeInput>,
) -> Result<(Vec<Value>, Vec<DevToolsRemoteHandleId>), DevToolsError> {
    let mut arguments = Vec::with_capacity(inputs.len());
    let mut handles = Vec::new();
    for input in inputs {
        match input {
            LocateNodesStartNodeInput::Raw(value) => arguments.push(value),
            LocateNodesStartNodeInput::Reference(reference) => {
                let resolved = resolve_locate_nodes_start_node_reference_arguments(
                    conn,
                    context,
                    vec![reference],
                )
                .await;
                let (mut resolved_arguments, mut resolved_handles) = match resolved {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        release_locate_nodes_start_node_handles(conn, context.clone(), handles)
                            .await;
                        return Err(error);
                    }
                };
                arguments.append(&mut resolved_arguments);
                handles.append(&mut resolved_handles);
            }
        }
    }
    Ok((arguments, handles))
}

pub(super) async fn resolve_locate_nodes_start_node_reference_arguments(
    conn: &mut CdpConnection,
    context: &DevToolsCommandContext,
    references: Vec<DevToolsDomNodeReference>,
) -> Result<(Vec<Value>, Vec<DevToolsRemoteHandleId>), DevToolsError> {
    let mut arguments = Vec::with_capacity(references.len());
    let mut handles = Vec::with_capacity(references.len());
    for reference in references {
        let result = crate::domains::dom::execute_devtools_dom_command_async(
            conn,
            DevToolsCommand::ResolveNode(DevToolsResolveNodeCommand {
                context: context.clone(),
                reference,
                execution_context_id: None,
                object_group: Some(LOCATE_NODES_START_NODE_OBJECT_GROUP.to_owned()),
            }),
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                release_locate_nodes_start_node_handles(conn, context.clone(), handles).await;
                return Err(error);
            }
        };
        let DevToolsCommandResult::ResolveNode(result) = result else {
            release_locate_nodes_start_node_handles(conn, context.clone(), handles).await;
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "LocateNodesStartNodeResolveReturnedUnexpectedResult",
            ));
        };
        let Some(object_id) = result
            .object
            .get("objectId")
            .and_then(Value::as_str)
            .map(DevToolsRemoteHandleId::from)
        else {
            release_locate_nodes_start_node_handles(conn, context.clone(), handles).await;
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchNode,
                "start node is no longer attached to the DOM",
            ));
        };
        arguments.push(json!({ "handle": object_id.as_str() }));
        handles.push(object_id);
    }
    Ok((arguments, handles))
}

pub(super) async fn release_locate_nodes_start_node_handles(
    conn: &mut CdpConnection,
    context: DevToolsCommandContext,
    handles: Vec<DevToolsRemoteHandleId>,
) {
    if handles.is_empty() {
        return;
    }
    if let Err(error) = execute_devtools_release_objects_command_async(
        conn,
        DevToolsReleaseObjectsCommand {
            context,
            realm_id: None,
            world_name: None,
            handles,
        },
    )
    .await
    {
        tracing::debug!(?error, "failed to release locateNodes start node handles");
    }
}

pub(super) fn locate_nodes_locator_arguments(
    locator: &DevToolsLocateNodesLocator,
    max_node_count: u64,
) -> Vec<Value> {
    match locator {
        DevToolsLocateNodesLocator::Css(selector)
        | DevToolsLocateNodesLocator::XPath(selector)
        | DevToolsLocateNodesLocator::TagName(selector) => {
            vec![json!(selector), json!(max_node_count)]
        }
        DevToolsLocateNodesLocator::LinkText { value, match_type } => {
            vec![
                json!(value),
                json!(matches!(match_type, DevToolsLocateNodesTextMatch::Full)),
                json!(max_node_count),
            ]
        }
        DevToolsLocateNodesLocator::Context(_) => {
            unreachable!("context locator is handled before selector delegate")
        }
        DevToolsLocateNodesLocator::InnerText {
            value,
            ignore_case,
            match_type,
            max_depth,
        } => vec![
            json!(value),
            json!(matches!(match_type, DevToolsLocateNodesTextMatch::Full)),
            json!(*ignore_case),
            json!(max_node_count),
            json!(max_depth),
        ],
        DevToolsLocateNodesLocator::Accessibility { role, name } => vec![
            json!(name.as_deref().unwrap_or_default()),
            json!(role.as_deref().unwrap_or_default()),
            json!(max_node_count),
        ],
    }
}

pub(super) fn locate_nodes_function_declaration(
    locator: &DevToolsLocateNodesLocator,
) -> &'static str {
    match locator {
        DevToolsLocateNodesLocator::Context(_) => {
            unreachable!("context locator is handled before selector delegate")
        }
        DevToolsLocateNodesLocator::Css(_) => {
            r#"function(cssSelector, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const locateNodesUsingCss = (node) => {
                    if (!(node instanceof HTMLElement ||
                        node instanceof Document ||
                        node instanceof DocumentFragment ||
                        node instanceof SVGElement)) {
                        throw new Error('startNodes in css selector should be HTMLElement, SVGElement or Document or DocumentFragment');
                    }
                    return Array.from(node.querySelectorAll(cssSelector));
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingCss(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::XPath(_) => {
            r#"function(xPathSelector, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const locateNodesUsingXpath = (node) => {
                    const documentForNode = node.nodeType === Node.DOCUMENT_NODE ? node : node.ownerDocument;
                    const xPathResult = documentForNode.evaluate(
                        xPathSelector,
                        node,
                        null,
                        XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
                        null
                    );
                    const returnedNodes = [];
                    for (let index = 0; index < xPathResult.snapshotLength; index += 1) {
                        returnedNodes.push(xPathResult.snapshotItem(index));
                    }
                    return returnedNodes;
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingXpath(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::TagName(_) => {
            r#"function(tagName, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                if (tagName === '') {
                    throw new Error('Unable to locate an element with the tagName ""');
                }
                const locateNodesUsingTagName = (node) => {
                    if (!(node instanceof HTMLElement ||
                        node instanceof Document ||
                        node instanceof DocumentFragment ||
                        node instanceof SVGElement)) {
                        throw new Error('startNodes in tag name should be HTMLElement, SVGElement or Document or DocumentFragment');
                    }
                    return Array.from(node.getElementsByTagName(tagName));
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingTagName(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::LinkText { .. } => {
            r#"function(linkText, fullMatch, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const visibleLinkText = (element) => (element.innerText || element.textContent || '').trim();
                const linkMatches = (element) => {
                    const text = visibleLinkText(element);
                    return fullMatch ? text === linkText : text.includes(linkText);
                };
                const locateNodesUsingLinkText = (node) => {
                    if (!(node instanceof HTMLElement ||
                        node instanceof Document ||
                        node instanceof DocumentFragment ||
                        node instanceof SVGElement)) {
                        throw new Error('startNodes in link text should be HTMLElement, SVGElement or Document or DocumentFragment');
                    }
                    return Array.from(node.getElementsByTagName('a')).filter(linkMatches);
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingLinkText(startNode));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::InnerText { .. } => {
            r#"function(innerTextSelector, fullMatch, ignoreCase, maxNodeCount, maxDepth, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const searchText = ignoreCase ? innerTextSelector.toUpperCase() : innerTextSelector;
                const locateNodesUsingInnerText = (node, currentMaxDepth) => {
                    const returnedNodes = [];
                    if (node instanceof DocumentFragment || node instanceof Document) {
                        for (const child of node.children) {
                            returnedNodes.push(...locateNodesUsingInnerText(child, currentMaxDepth));
                        }
                        return returnedNodes;
                    }
                    if (!(node instanceof HTMLElement)) {
                        return [];
                    }
                    const nodeInnerText = ignoreCase ? node.innerText?.toUpperCase() : node.innerText;
                    if (!nodeInnerText || !nodeInnerText.includes(searchText)) {
                        return [];
                    }
                    const childNodes = Array.from(node.children).filter((child) => child instanceof HTMLElement);
                    if (childNodes.length === 0) {
                        if (!fullMatch || nodeInnerText === searchText) {
                            returnedNodes.push(node);
                        }
                        return returnedNodes;
                    }
                    const childNodeMatches = currentMaxDepth <= 0
                        ? []
                        : childNodes.flatMap((child) => locateNodesUsingInnerText(child, currentMaxDepth - 1));
                    if (childNodeMatches.length === 0) {
                        if (!fullMatch || nodeInnerText === searchText) {
                            returnedNodes.push(node);
                        }
                    } else {
                        returnedNodes.push(...childNodeMatches);
                    }
                    return returnedNodes;
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : [document];
                const returnedNodes = startNodes.flatMap((startNode) => locateNodesUsingInnerText(startNode, maxDepth));
                return locatedNodeRecords(returnedNodes);
            }"#
        }
        DevToolsLocateNodesLocator::Accessibility { .. } => {
            r#"function(name, role, maxNodeCount, ...startNodes) {
                const locatedNodeRecords = (nodes) => {
                    const returned = maxNodeCount === 0 ? nodes : nodes.slice(0, maxNodeCount);
                    return returned.map((node) => ({
                        backendNodeId: __moliHostResolveBackendNodeIdForObject(node),
                        node,
                    }));
                };
                const implicitRole = (element) => {
                    const localName = element.localName;
                    if (/^h[1-6]$/.test(localName)) { return 'heading'; }
                    if (localName === 'article') { return 'article'; }
                    if (localName === 'button') { return 'button'; }
                    if (localName === 'input' && String(element.type || 'text') === 'text') { return 'textbox'; }
                    if (localName === 'input' && String(element.type || '') === 'search') { return 'searchbox'; }
                    if (localName === 'a' && element.hasAttribute('href')) { return 'link'; }
                    return '';
                };
                const accessibleRole = (element) => element.getAttribute('role') || implicitRole(element);
                const accessibleName = (element) => {
                    if (element.hasAttribute('aria-label')) { return element.getAttribute('aria-label') || ''; }
                    return (element.innerText || element.textContent || '').trim();
                };
                const returnedNodes = [];
                let aborted = false;
                const collect = (contextNodes) => {
                    if (aborted) { return; }
                    for (const contextNode of contextNodes) {
                        if (!(contextNode instanceof HTMLElement || contextNode instanceof SVGElement)) {
                            continue;
                        }
                        let matches = true;
                        if (role && accessibleRole(contextNode) !== role) { matches = false; }
                        if (name && accessibleName(contextNode) !== name) { matches = false; }
                        if (matches) {
                            if (maxNodeCount !== 0 && returnedNodes.length === maxNodeCount) {
                                aborted = true;
                                break;
                            }
                            returnedNodes.push(contextNode);
                        }
                        collect(Array.from(contextNode.children));
                    }
                };
                startNodes = startNodes.length > 0
                    ? startNodes.filter(Boolean)
                    : Array.from(document.documentElement.children).filter((child) => child instanceof HTMLElement || child instanceof SVGElement);
                collect(startNodes);
                return locatedNodeRecords(returnedNodes);
            }"#
        }
    }
}

pub(super) async fn locate_nodes_result_from_script_result_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    result: DevToolsCommandResult,
    locator: &DevToolsLocateNodesLocator,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let mut result = locate_nodes_result_from_script_result(result, locator)?;
    if let DevToolsCommandResult::LocateNodes(result) = &mut result {
        materialize_locate_nodes_result_node_ids_async(conn, target, result).await;
    }
    Ok(result)
}

pub(super) async fn materialize_locate_nodes_result_node_ids_async(
    conn: &mut CdpConnection,
    target: &DevToolsRuntimeTarget,
    result: &mut DevToolsLocateNodesResult,
) {
    let owner = CommandOwnerScope::for_route(target.route.clone());
    for index in 0..result.nodes.len() {
        let shared_id = result.nodes[index]
            .shared_id
            .as_ref()
            .map(|shared_id| shared_id.as_str().to_owned());
        let mut materialized = false;
        if let Some(shared_id) = shared_id {
            let snapshot = conn
                .document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
                    &owner, &shared_id, 0, false,
                )
                .await;
            if let Ok(Some(snapshot)) = snapshot {
                result.nodes[index].node_id =
                    frontend_node_id_for_locate_node_snapshot(&snapshot.snapshot);
                result.nodes[index].backend_node_id = snapshot
                    .snapshot
                    .backend_node_id
                    .or(result.nodes[index].backend_node_id);
                materialized = true;
            }
        }

        if !materialized
            && let Some(backend_node_id) = result.nodes[index].backend_node_id
            && let Ok(Some(snapshot)) = conn
                .document_node_snapshot_for_backend_node_id_for_owner_async(
                    &owner,
                    backend_node_id,
                    0,
                    false,
                )
                .await
        {
            result.nodes[index].node_id =
                frontend_node_id_for_locate_node_snapshot(&snapshot.snapshot);
            result.nodes[index].backend_node_id =
                snapshot.snapshot.backend_node_id.or(Some(backend_node_id));
        }
    }
    result.node_ids = result
        .nodes
        .iter()
        .filter_map(|node| node.node_id)
        .collect();
}

pub(super) fn frontend_node_id_for_locate_node_snapshot(
    snapshot: &DocumentNodeSnapshot,
) -> Option<u32> {
    snapshot.frontend_node_id
}

pub(super) fn locate_nodes_result_from_script_result(
    result: DevToolsCommandResult,
    locator: &DevToolsLocateNodesLocator,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let DevToolsCommandResult::Script(result) = result else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "UnexpectedLocateNodesResult",
        ));
    };
    match *result {
        DevToolsScriptResult::Value(value) => {
            let Some(deep_serialized_value) = value.deep_serialized_value else {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "LocateNodesMissingSerializedArray",
                ));
            };
            let Some(nodes) =
                locate_nodes_remote_values_from_deep_serialized_array(&deep_serialized_value)
            else {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "LocateNodesUnexpectedSerializedArray",
                ));
            };
            Ok(DevToolsCommandResult::LocateNodes(
                DevToolsLocateNodesResult {
                    node_ids: nodes.iter().filter_map(|node| node.node_id).collect(),
                    nodes,
                },
            ))
        }
        DevToolsScriptResult::Exception(exception) => {
            Err(locate_nodes_error_from_exception(exception, locator))
        }
    }
}

pub(super) fn locate_nodes_error_from_exception(
    exception: DevToolsScriptException,
    locator: &DevToolsLocateNodesLocator,
) -> DevToolsError {
    let text = locate_nodes_exception_text(&exception);
    if locate_nodes_exception_is_invalid_selector(&text, locator) {
        return DevToolsError::new(DevToolsErrorKind::InvalidSelector, text);
    }
    if text
        == "Error: startNodes in css selector should be HTMLElement, SVGElement or Document or DocumentFragment"
        || text
            == "Error: startNodes in tag name should be HTMLElement, SVGElement or Document or DocumentFragment"
    {
        return DevToolsError::new(DevToolsErrorKind::InvalidArgument, text);
    }
    if text == "Error: Context does not exist" {
        return DevToolsError::new(DevToolsErrorKind::InvalidArgument, text);
    }
    DevToolsError::new(
        DevToolsErrorKind::Internal,
        format!("Unexpected error in selector script: {text}"),
    )
}

pub(super) fn locate_nodes_exception_is_invalid_selector(
    text: &str,
    locator: &DevToolsLocateNodesLocator,
) -> bool {
    (text.starts_with("SyntaxError: Failed to execute 'querySelectorAll' on '")
        && text.ends_with(" is not a valid selector."))
        || text.starts_with("SyntaxError: Failed to execute 'evaluate' on 'Document': ")
        || text.starts_with("DOMException: Failed to execute 'evaluate' on 'Document': ")
        || text.starts_with("DOMException: The string ")
            && text.ends_with(" is not a valid XPath expression.")
        || text == "DOMException" && matches!(locator, DevToolsLocateNodesLocator::XPath(_))
}

pub(super) fn locate_nodes_exception_text(exception: &DevToolsScriptException) -> String {
    if exception.text != "Uncaught" {
        return exception.text.clone();
    }
    if let Some(description) = exception
        .value
        .as_ref()
        .and_then(|value| value.description.as_ref())
    {
        return description.clone();
    }
    if let Some(value) = exception
        .value
        .as_ref()
        .and_then(|value| value.value.as_str())
    {
        return value.to_owned();
    }
    exception.text.clone()
}

pub(super) fn locate_nodes_remote_values_from_deep_serialized_array(
    value: &Value,
) -> Option<Vec<DevToolsRemoteValue>> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("array") {
        return None;
    }
    object
        .get("value")?
        .as_array()?
        .iter()
        .map(locate_node_remote_value_from_deep_serialized)
        .collect()
}

pub(super) fn locate_node_remote_value_from_deep_serialized(
    value: &Value,
) -> Option<DevToolsRemoteValue> {
    if let Some((backend_node_id, node)) = locate_node_record_from_deep_serialized(value) {
        let mut remote = locate_node_remote_value_from_deep_serialized_node(node)?;
        remote.backend_node_id = backend_node_id;
        return Some(remote);
    }
    locate_node_remote_value_from_deep_serialized_node(value)
}

pub(super) fn locate_node_record_from_deep_serialized(
    value: &Value,
) -> Option<(Option<u32>, &Value)> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return None;
    }
    let properties = object.get("value")?.as_array()?;
    let node = deep_serialized_property(properties, "node")?;
    let backend_node_id = deep_serialized_property(properties, "backendNodeId")
        .and_then(deep_serialized_number_value)
        .and_then(|id| u32::try_from(id).ok());
    Some((backend_node_id, node))
}

pub(super) fn deep_serialized_number_value(value: &Value) -> Option<u64> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("number") {
        return None;
    }
    object.get("value")?.as_u64()
}

pub(super) fn locate_node_remote_value_from_deep_serialized_node(
    value: &Value,
) -> Option<DevToolsRemoteValue> {
    let object = value.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("node") {
        return None;
    }
    let shared_id = object
        .get("sharedId")
        .and_then(Value::as_str)
        .map(|shared_id| DevToolsRemoteHandleId::from(shared_id.to_owned()));
    Some(DevToolsRemoteValue {
        value: Value::Null,
        handle: None,
        shared_id,
        node_id: None,
        backend_node_id: None,
        window_context: None,
        realm: None,
        remote_type: Some("object".to_owned()),
        remote_subtype: Some("node".to_owned()),
        unserializable_value: None,
        description: None,
        class_name: None,
        deep_serialized_value: None,
        node_value: object.get("value").cloned(),
    })
}
