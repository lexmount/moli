use crate::automation::{
    AutomationCommand, AutomationEvent, DevToolsDomGeometryOperation, DevToolsDomNodeReference,
    DevToolsDomObjectReferenceOperation, DevToolsErrorKind, FrontendProtocol,
};
use moli_page_types::DocumentSnapshotNodeId;
use serde_json::{Value, json};

use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};

fn start_devtools_dom_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: AutomationCommand,
) -> Result<Option<super::PendingDomCommandDispatch>, super::PendingDomCommandStartError> {
    let owner = CommandOwnerScope::capture(conn, command_session_id);
    super::start_devtools_dom_command_for_owner(conn, command_id, &owner, command)
}

fn unique_test_file_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-protocol-{name}-{}-{nanos}.txt",
        std::process::id()
    ))
}

#[test]
fn set_child_nodes_output_preserves_typed_automation_sidecar() {
    let mut out = super::DomCommandOutput::default();
    let nodes = vec![json!({
        "nodeId": 8,
        "backendNodeId": 8,
        "nodeType": 1,
        "nodeName": "DIV",
        "localName": "div",
        "nodeValue": "",
    })];

    super::push_set_child_nodes_event(
        &mut out,
        Some("SID-dom"),
        DocumentSnapshotNodeId::new(7).encoded(),
        nodes.clone(),
    );

    let mut events = out.into_plan().into_background_events(Some(42), None);
    assert_eq!(events.len(), 1);
    assert!(
        events[0].protocol_message().is_none(),
        "DOM.setChildNodes should stay typed until wire projection"
    );
    assert_eq!(events[0].protocol_method(), Some("DOM.setChildNodes"));
    assert!(events[0].has_protocol_wire_message());
    let (message, automation_event) = events.remove(0).into_parts();
    assert_eq!(message["method"], json!("DOM.setChildNodes"));
    assert_eq!(message["sessionId"], json!("SID-dom"));
    assert_eq!(message["params"]["parentId"], json!(8));
    assert_eq!(message["params"]["nodes"], json!(nodes));

    let Some(AutomationEvent::DomSetChildNodes(event)) = automation_event else {
        panic!("expected typed DOM.setChildNodes automation sidecar");
    };
    assert_eq!(event.parent_node_id, 8);
    assert_eq!(event.nodes, nodes);
}

#[test]
fn cdp_get_document_builds_protocol_neutral_document_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "depth": 2,
        "pierce": true
    });
    let cmd = Cmd::for_test(
        Some(61),
        "DOM.getDocument",
        &params,
        Some("SID-dom"),
        r#"{"id":61,"method":"DOM.getDocument"}"#,
    );

    let command = super::build_cdp_get_document_command(
        &conn,
        &cmd,
        super::PendingDomDocumentSnapshotOperation::GetDocument,
    );
    let Ok(command) = command else {
        panic!("valid getDocument command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(command.context.target_id, None);
    assert_eq!(command.context.browser_context_id, None);
    assert_eq!(command.depth, Some(2));
    assert!(command.pierce);
    assert!(!command.flattened);
}

#[test]
fn cdp_get_flattened_document_builds_protocol_neutral_document_command() {
    let conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(62),
        "DOM.getFlattenedDocument",
        &params,
        Some("SID-dom"),
        r#"{"id":62,"method":"DOM.getFlattenedDocument"}"#,
    );

    let command = super::build_cdp_get_document_command(
        &conn,
        &cmd,
        super::PendingDomDocumentSnapshotOperation::GetFlattenedDocument,
    );
    let Ok(command) = command else {
        panic!("valid getFlattenedDocument command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(command.depth, Some(-1));
    assert!(!command.pierce);
    assert!(command.flattened);
}

#[test]
fn devtools_dom_entry_routes_document_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(63),
        "DOM.getDocument",
        &params,
        Some("SID-dom"),
        r#"{"id":63,"method":"DOM.getDocument"}"#,
    );
    let command = super::build_cdp_get_document_command(
        &conn,
        &cmd,
        super::PendingDomDocumentSnapshotOperation::GetDocument,
    );
    let Ok(command) = command else {
        panic!("valid getDocument command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::GetDocument(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn cdp_get_frame_owner_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "frameId": "TID-child"
    });
    let cmd = Cmd::for_test(
        Some(98),
        "DOM.getFrameOwner",
        &params,
        Some("SID-dom"),
        r#"{"id":98,"method":"DOM.getFrameOwner"}"#,
    );

    let command = super::build_cdp_get_frame_owner_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getFrameOwner command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(command.frame_id.as_str(), "TID-child");
}

#[test]
fn devtools_dom_entry_routes_get_frame_owner_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "frameId": "TID-child"
    });
    let cmd = Cmd::for_test(
        Some(99),
        "DOM.getFrameOwner",
        &params,
        Some("SID-dom"),
        r#"{"id":99,"method":"DOM.getFrameOwner"}"#,
    );
    let command = super::build_cdp_get_frame_owner_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getFrameOwner command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::GetFrameOwner(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM start entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(
        error.message,
        "Frame with the given id does not belong to the target."
    );
}

#[test]
fn cdp_request_child_nodes_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 24,
        "depth": 3,
        "pierce": true
    });
    let cmd = Cmd::for_test(
        Some(89),
        "DOM.requestChildNodes",
        &params,
        Some("SID-dom"),
        r#"{"id":89,"method":"DOM.requestChildNodes"}"#,
    );

    let command = super::build_cdp_request_child_nodes_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid requestChildNodes command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(24)
    );
    assert_eq!(command.depth, 3);
    assert!(command.pierce);
}

#[test]
fn devtools_dom_entry_routes_request_child_nodes_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 25
    });
    let cmd = Cmd::for_test(
        Some(90),
        "DOM.requestChildNodes",
        &params,
        Some("SID-dom"),
        r#"{"id":90,"method":"DOM.requestChildNodes"}"#,
    );
    let command = super::build_cdp_request_child_nodes_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid requestChildNodes command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::RequestChildNodes(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM start entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn devtools_dom_complete_routes_request_child_nodes_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let mut out = super::DomCommandOutput::default();
    let params = json!({
        "nodeId": 26
    });
    let cmd = Cmd::for_test(
        Some(91),
        "DOM.requestChildNodes",
        &params,
        Some("SID-dom"),
        r#"{"id":91,"method":"DOM.requestChildNodes"}"#,
    );
    let command = super::build_cdp_request_child_nodes_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid requestChildNodes command");
    };

    let result = super::complete_devtools_request_child_nodes_command(&mut conn, command, &mut out);

    let Err(error) = result else {
        panic!("requestChildNodes complete helper should require pending renderer capture");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(
        error.message,
        "RequestChildNodesRequiresPendingRendererCapture"
    );
    assert!(out.is_empty());
}

#[test]
fn cdp_get_node_for_location_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "x": 12,
        "y": 34,
        "includeUserAgentShadowDOM": true,
        "ignorePointerEventsNone": true
    });
    let cmd = Cmd::for_test(
        Some(95),
        "DOM.getNodeForLocation",
        &params,
        Some("SID-dom"),
        r#"{"id":95,"method":"DOM.getNodeForLocation"}"#,
    );

    let command = super::build_cdp_get_node_for_location_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getNodeForLocation command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(command.x, 12.0);
    assert_eq!(command.y, 34.0);
    assert!(command.include_user_agent_shadow_dom);
    assert!(command.ignore_pointer_events_none);
}

#[test]
fn devtools_dom_entry_routes_get_node_for_location_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "x": 10,
        "y": 20
    });
    let cmd = Cmd::for_test(
        Some(96),
        "DOM.getNodeForLocation",
        &params,
        Some("SID-dom"),
        r#"{"id":96,"method":"DOM.getNodeForLocation"}"#,
    );
    let command = super::build_cdp_get_node_for_location_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getNodeForLocation command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::GetNodeForLocation(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM start entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn devtools_dom_complete_routes_get_node_for_location_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "x": 10,
        "y": 20
    });
    let cmd = Cmd::for_test(
        Some(97),
        "DOM.getNodeForLocation",
        &params,
        Some("SID-dom"),
        r#"{"id":97,"method":"DOM.getNodeForLocation"}"#,
    );
    let command = super::build_cdp_get_node_for_location_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getNodeForLocation command");
    };

    let result = super::complete_devtools_get_node_for_location_command(&mut conn, command);

    let Err(error) = result else {
        panic!("getNodeForLocation requires the pending renderer hit-test path");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(
        error.message,
        "GetNodeForLocationRequiresPendingRendererHitTest"
    );
}

#[test]
fn cdp_query_selector_builds_protocol_neutral_dom_query_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 7,
        "selector": "section > p.target"
    });
    let cmd = Cmd::for_test(
        Some(64),
        "DOM.querySelector",
        &params,
        Some("SID-dom"),
        r#"{"id":64,"method":"DOM.querySelector"}"#,
    );

    let command = super::build_cdp_query_selector_command(&conn, &cmd, false);
    let Ok(command) = command else {
        panic!("valid querySelector command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(
        command.root,
        Some(DevToolsDomNodeReference::FrontendNodeId(7))
    );
    assert_eq!(command.selector, "section > p.target");
    assert!(!command.multiple);
}

#[test]
fn cdp_query_selector_all_builds_protocol_neutral_dom_query_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 8,
        "selector": ".item"
    });
    let cmd = Cmd::for_test(
        Some(65),
        "DOM.querySelectorAll",
        &params,
        Some("SID-dom"),
        r#"{"id":65,"method":"DOM.querySelectorAll"}"#,
    );

    let command = super::build_cdp_query_selector_command(&conn, &cmd, true);
    let Ok(command) = command else {
        panic!("valid querySelectorAll command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.root,
        Some(DevToolsDomNodeReference::FrontendNodeId(8))
    );
    assert_eq!(command.selector, ".item");
    assert!(command.multiple);
}

#[test]
fn devtools_dom_entry_routes_query_selector_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 9,
        "selector": "main"
    });
    let cmd = Cmd::for_test(
        Some(66),
        "DOM.querySelector",
        &params,
        Some("SID-dom"),
        r#"{"id":66,"method":"DOM.querySelector"}"#,
    );
    let command = super::build_cdp_query_selector_command(&conn, &cmd, false);
    let Ok(command) = command else {
        panic!("valid querySelector command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::QuerySelector(command),
    );

    let Err(error) = result else {
        panic!("missing page should surface through the unified DOM entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "Could not find node with given id");
}

#[test]
fn cdp_resolve_node_builds_protocol_neutral_dom_resolve_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 10,
        "executionContextId": 42,
        "objectGroup": "webdriver"
    });
    let cmd = Cmd::for_test(
        Some(67),
        "DOM.resolveNode",
        &params,
        Some("SID-dom"),
        r#"{"id":67,"method":"DOM.resolveNode"}"#,
    );

    let command = super::build_cdp_resolve_node_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid resolveNode command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(10)
    );
    assert_eq!(command.execution_context_id, Some(42));
    assert_eq!(command.object_group.as_deref(), Some("webdriver"));
}

#[test]
fn cdp_resolve_node_preserves_backend_node_reference_source() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeId": 31,
        "objectGroup": "backend"
    });
    let cmd = Cmd::for_test(
        Some(107),
        "DOM.resolveNode",
        &params,
        Some("SID-dom"),
        r#"{"id":107,"method":"DOM.resolveNode"}"#,
    );

    let command = super::build_cdp_resolve_node_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid resolveNode backendNodeId command");
    };

    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::BackendNodeId(31)
    );
    assert_eq!(command.object_group.as_deref(), Some("backend"));
}

#[test]
fn cdp_resolve_node_without_node_reference_keeps_cdp_invalid_param_shape() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectGroup": "webdriver"
    });
    let cmd = Cmd::for_test(
        Some(68),
        "DOM.resolveNode",
        &params,
        Some("SID-dom"),
        r#"{"id":68,"method":"DOM.resolveNode"}"#,
    );

    let result = super::build_cdp_resolve_node_command(&conn, &cmd);

    let Err(error) = result else {
        panic!("resolveNode without nodeId/backendNodeId should be rejected");
    };
    assert_eq!(error.code, -32602);
    assert_eq!(error.message, "InvalidParam");
}

#[test]
fn pending_dom_start_error_preserves_invalid_param_as_invalid_argument() {
    let singular = super::PendingDomCommandStartError {
        code: -32602,
        message: "InvalidParam".to_owned(),
    };
    let plural = super::PendingDomCommandStartError {
        code: -32602,
        message: "InvalidParams".to_owned(),
    };
    let selector = super::PendingDomCommandStartError {
        code: -32602,
        message: "The selector is not a valid selector".to_owned(),
    };

    let singular: crate::automation::DevToolsError = singular.into();
    let plural: crate::automation::DevToolsError = plural.into();
    let selector: crate::automation::DevToolsError = selector.into();

    assert_eq!(singular.kind, DevToolsErrorKind::InvalidArgument);
    assert_eq!(plural.kind, DevToolsErrorKind::InvalidArgument);
    assert_eq!(selector.kind, DevToolsErrorKind::InvalidSelector);
}

#[test]
fn devtools_dom_entry_routes_resolve_node_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 11
    });
    let cmd = Cmd::for_test(
        Some(69),
        "DOM.resolveNode",
        &params,
        Some("SID-dom"),
        r#"{"id":69,"method":"DOM.resolveNode"}"#,
    );
    let command = super::build_cdp_resolve_node_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid resolveNode command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::ResolveNode(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn cdp_get_attributes_builds_protocol_neutral_dom_attributes_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 12
    });
    let cmd = Cmd::for_test(
        Some(70),
        "DOM.getAttributes",
        &params,
        Some("SID-dom"),
        r#"{"id":70,"method":"DOM.getAttributes"}"#,
    );

    let command = super::build_cdp_get_attributes_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getAttributes command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::FrontendNodeId(12)
    );
}

#[test]
fn devtools_dom_complete_entry_requires_pending_get_attributes_command() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 13
    });
    let cmd = Cmd::for_test(
        Some(71),
        "DOM.getAttributes",
        &params,
        Some("SID-dom"),
        r#"{"id":71,"method":"DOM.getAttributes"}"#,
    );
    let command = super::build_cdp_get_attributes_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid getAttributes command");
    };

    let result =
        super::complete_devtools_dom_command(&mut conn, AutomationCommand::GetAttributes(command));

    let Err(error) = result else {
        panic!("getAttributes sync completion should require a pending DOM command");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "MissingDomCommand");
}

#[test]
fn cdp_push_nodes_by_backend_ids_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeIds": [14, 15, 16]
    });
    let cmd = Cmd::for_test(
        Some(102),
        "DOM.pushNodesByBackendIdsToFrontend",
        &params,
        Some("SID-dom"),
        r#"{"id":102,"method":"DOM.pushNodesByBackendIdsToFrontend"}"#,
    );

    let command = super::build_cdp_push_nodes_by_backend_ids_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid pushNodesByBackendIds command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(command.backend_node_ids, vec![14, 15, 16]);
}

#[test]
fn devtools_dom_complete_entry_requires_pending_push_nodes_by_backend_ids_command() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "backendNodeIds": [17]
    });
    let cmd = Cmd::for_test(
        Some(103),
        "DOM.pushNodesByBackendIdsToFrontend",
        &params,
        Some("SID-dom"),
        r#"{"id":103,"method":"DOM.pushNodesByBackendIdsToFrontend"}"#,
    );
    let command = super::build_cdp_push_nodes_by_backend_ids_command(&conn, &cmd);
    let Ok(command) = command else {
        panic!("valid pushNodesByBackendIds command");
    };

    let result = super::complete_devtools_dom_command(
        &mut conn,
        AutomationCommand::PushNodesByBackendIds(command),
    );

    let Err(error) = result else {
        panic!("pushNodesByBackendIds sync completion should require a pending DOM command");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "MissingDomCommand");
}

#[test]
fn cdp_request_node_builds_protocol_neutral_object_reference_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-1"
    });
    let cmd = Cmd::for_test(
        Some(72),
        "DOM.requestNode",
        &params,
        Some("SID-dom"),
        r#"{"id":72,"method":"DOM.requestNode"}"#,
    );

    let command = super::build_cdp_dom_object_reference_command(
        &conn,
        &cmd,
        DevToolsDomObjectReferenceOperation::RequestNode,
    );
    let Ok(Some(command)) = command else {
        panic!("valid requestNode object reference command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(command.object_id.as_str(), "remote-object-1");
    assert_eq!(
        command.operation,
        DevToolsDomObjectReferenceOperation::RequestNode
    );
}

#[test]
fn cdp_get_outer_html_without_object_id_keeps_node_reference_path() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 14
    });
    let cmd = Cmd::for_test(
        Some(73),
        "DOM.getOuterHTML",
        &params,
        Some("SID-dom"),
        r#"{"id":73,"method":"DOM.getOuterHTML"}"#,
    );

    let command = super::build_cdp_dom_object_reference_command(
        &conn,
        &cmd,
        DevToolsDomObjectReferenceOperation::GetOuterHtml {
            include_shadow_dom: false,
        },
    );

    let Ok(None) = command else {
        panic!("getOuterHTML without objectId should stay on typed node reference path");
    };
}

#[test]
fn cdp_get_outer_html_builds_protocol_neutral_node_reference_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "nodeId": 15,
        "includeShadowDOM": true
    });
    let cmd = Cmd::for_test(
        Some(76),
        "DOM.getOuterHTML",
        &params,
        Some("SID-dom"),
        r#"{"id":76,"method":"DOM.getOuterHTML"}"#,
    );

    let command = super::build_cdp_get_outer_html_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid getOuterHTML typed node reference command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.reference,
        Some(DevToolsDomNodeReference::FrontendNodeId(15))
    );
    assert!(command.include_shadow_dom);
}

#[test]
fn cdp_get_outer_html_object_id_keeps_object_reference_path() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-outer",
        "includeShadowDOM": true
    });
    let cmd = Cmd::for_test(
        Some(77),
        "DOM.getOuterHTML",
        &params,
        Some("SID-dom"),
        r#"{"id":77,"method":"DOM.getOuterHTML"}"#,
    );

    let object_command = super::build_cdp_dom_object_reference_command(
        &conn,
        &cmd,
        DevToolsDomObjectReferenceOperation::GetOuterHtml {
            include_shadow_dom: true,
        },
    );
    let Ok(Some(object_command)) = object_command else {
        panic!("valid getOuterHTML object reference command");
    };
    assert_eq!(object_command.object_id.as_str(), "remote-object-outer");
    assert_eq!(
        object_command.operation,
        DevToolsDomObjectReferenceOperation::GetOuterHtml {
            include_shadow_dom: true,
        }
    );

    let command = super::build_cdp_get_outer_html_command(&conn, &cmd);

    let Ok(None) = command else {
        panic!("getOuterHTML with objectId should stay on object reference path");
    };
}

#[test]
fn devtools_dom_complete_entry_requires_pending_get_outer_html_command() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 16
    });
    let cmd = Cmd::for_test(
        Some(78),
        "DOM.getOuterHTML",
        &params,
        Some("SID-dom"),
        r#"{"id":78,"method":"DOM.getOuterHTML"}"#,
    );
    let command = super::build_cdp_get_outer_html_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid getOuterHTML typed node reference command");
    };

    let result =
        super::complete_devtools_dom_command(&mut conn, AutomationCommand::GetOuterHtml(command));

    let Err(error) = result else {
        panic!("getOuterHTML sync completion should require a pending DOM command");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "MissingDomCommand");
}

#[test]
fn cdp_scroll_into_view_builds_protocol_neutral_node_reference_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeId": 17,
        "rect": { "x": 1, "y": 2, "width": 3, "height": 4 }
    });
    let cmd = Cmd::for_test(
        Some(79),
        "DOM.scrollIntoViewIfNeeded",
        &params,
        Some("SID-dom"),
        r#"{"id":79,"method":"DOM.scrollIntoViewIfNeeded"}"#,
    );

    let command = super::build_cdp_scroll_into_view_if_needed_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid scrollIntoViewIfNeeded typed node reference command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.reference,
        Some(DevToolsDomNodeReference::BackendNodeId(17))
    );
    assert_eq!(
        command.rect,
        moli_core::page::DomScrollIntoViewRect::try_new(1.0, 2.0, 3.0, 4.0)
    );
}

#[test]
fn cdp_scroll_into_view_rejects_every_non_finite_rect_component() {
    for rect in [
        super::ScrollIntoViewRectParams {
            x: f64::NAN,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        },
        super::ScrollIntoViewRectParams {
            x: 0.0,
            y: f64::INFINITY,
            width: 0.0,
            height: 0.0,
        },
        super::ScrollIntoViewRectParams {
            x: 0.0,
            y: 0.0,
            width: f64::NEG_INFINITY,
            height: 0.0,
        },
        super::ScrollIntoViewRectParams {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: f64::NAN,
        },
    ] {
        let Err(error) = super::validated_scroll_into_view_rect(Some(rect)) else {
            panic!("non-finite scroll rect should be rejected");
        };
        assert_eq!(error.code, -32602);
        assert_eq!(error.message, "InvalidParams");
    }
}

#[test]
fn cdp_scroll_into_view_object_id_keeps_object_reference_path() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-scroll"
    });
    let cmd = Cmd::for_test(
        Some(80),
        "DOM.scrollIntoViewIfNeeded",
        &params,
        Some("SID-dom"),
        r#"{"id":80,"method":"DOM.scrollIntoViewIfNeeded"}"#,
    );

    let command = super::build_cdp_scroll_into_view_if_needed_command(&conn, &cmd);

    let Ok(None) = command else {
        panic!("scrollIntoViewIfNeeded with objectId should stay on object reference path");
    };
}

#[test]
fn devtools_dom_start_entry_routes_scroll_command_to_renderer_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 18
    });
    let cmd = Cmd::for_test(
        Some(81),
        "DOM.scrollIntoViewIfNeeded",
        &params,
        Some("SID-dom"),
        r#"{"id":81,"method":"DOM.scrollIntoViewIfNeeded"}"#,
    );
    let command = super::build_cdp_scroll_into_view_if_needed_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid scrollIntoViewIfNeeded typed node reference command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::ScrollIntoViewIfNeeded(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM start entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn cdp_get_box_model_builds_protocol_neutral_geometry_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeId": 19
    });
    let cmd = Cmd::for_test(
        Some(82),
        "DOM.getBoxModel",
        &params,
        Some("SID-dom"),
        r#"{"id":82,"method":"DOM.getBoxModel"}"#,
    );

    let command = super::build_cdp_dom_geometry_command(
        &conn,
        &cmd,
        DevToolsDomGeometryOperation::GetBoxModel,
    );
    let Ok(Some(command)) = command else {
        panic!("valid getBoxModel typed node reference command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::BackendNodeId(19)
    );
    assert_eq!(command.operation, DevToolsDomGeometryOperation::GetBoxModel);
}

#[test]
fn cdp_get_content_quads_object_id_keeps_object_reference_path() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-geometry"
    });
    let cmd = Cmd::for_test(
        Some(83),
        "DOM.getContentQuads",
        &params,
        Some("SID-dom"),
        r#"{"id":83,"method":"DOM.getContentQuads"}"#,
    );

    let command = super::build_cdp_dom_geometry_command(
        &conn,
        &cmd,
        DevToolsDomGeometryOperation::GetContentQuads,
    );

    let Ok(None) = command else {
        panic!("getContentQuads with objectId should stay on object reference path");
    };
}

#[test]
fn devtools_dom_entry_routes_geometry_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 20
    });
    let cmd = Cmd::for_test(
        Some(84),
        "DOM.getContentQuads",
        &params,
        Some("SID-dom"),
        r#"{"id":84,"method":"DOM.getContentQuads"}"#,
    );
    let command = super::build_cdp_dom_geometry_command(
        &conn,
        &cmd,
        DevToolsDomGeometryOperation::GetContentQuads,
    );
    let Ok(Some(command)) = command else {
        panic!("valid getContentQuads typed node reference command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::DomGeometry(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn cdp_remove_node_builds_protocol_neutral_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeId": 21
    });
    let cmd = Cmd::for_test(
        Some(100),
        "DOM.removeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":100,"method":"DOM.removeNode"}"#,
    );

    let command = super::build_cdp_remove_node_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid removeNode command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.reference,
        DevToolsDomNodeReference::BackendNodeId(21)
    );
}

#[test]
fn devtools_dom_entry_routes_remove_node_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 22
    });
    let cmd = Cmd::for_test(
        Some(101),
        "DOM.removeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":101,"method":"DOM.removeNode"}"#,
    );
    let command = super::build_cdp_remove_node_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid removeNode command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::RemoveNode(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM start entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn cdp_describe_node_builds_protocol_neutral_node_reference_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeId": 21,
        "depth": 2,
        "pierce": true
    });
    let cmd = Cmd::for_test(
        Some(85),
        "DOM.describeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":85,"method":"DOM.describeNode"}"#,
    );

    let command = super::build_cdp_describe_node_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid describeNode typed node reference command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.reference,
        Some(DevToolsDomNodeReference::BackendNodeId(21))
    );
    assert_eq!(command.depth, 2);
    assert!(command.pierce);
}

#[test]
fn cdp_describe_node_object_id_keeps_object_reference_path() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-describe"
    });
    let cmd = Cmd::for_test(
        Some(86),
        "DOM.describeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":86,"method":"DOM.describeNode"}"#,
    );

    let command = super::build_cdp_describe_node_command(&conn, &cmd);

    let Ok(None) = command else {
        panic!("describeNode with objectId should stay on object reference path");
    };
}

#[test]
fn devtools_dom_entry_routes_describe_node_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 22
    });
    let cmd = Cmd::for_test(
        Some(87),
        "DOM.describeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":87,"method":"DOM.describeNode"}"#,
    );
    let command = super::build_cdp_describe_node_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid describeNode typed node reference command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::DescribeNode(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM start entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn devtools_dom_complete_entry_requires_pending_describe_node_command() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "nodeId": 23
    });
    let cmd = Cmd::for_test(
        Some(88),
        "DOM.describeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":88,"method":"DOM.describeNode"}"#,
    );
    let command = super::build_cdp_describe_node_command(&conn, &cmd);
    let Ok(Some(command)) = command else {
        panic!("valid describeNode typed node reference command");
    };

    let result =
        super::complete_devtools_dom_command(&mut conn, AutomationCommand::DescribeNode(command));

    let Err(error) = result else {
        panic!("describeNode sync completion should require a pending DOM command");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "MissingDomCommand");
}

#[test]
fn cdp_describe_node_object_id_builds_protocol_neutral_object_reference_command() {
    let conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-3",
        "depth": 2,
        "pierce": true
    });
    let cmd = Cmd::for_test(
        Some(75),
        "DOM.describeNode",
        &params,
        Some("SID-dom"),
        r#"{"id":75,"method":"DOM.describeNode"}"#,
    );

    let command = super::build_cdp_dom_object_reference_command(
        &conn,
        &cmd,
        DevToolsDomObjectReferenceOperation::DescribeNode {
            depth: 2,
            pierce: true,
        },
    );
    let Ok(Some(command)) = command else {
        panic!("valid describeNode object reference command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(command.object_id.as_str(), "remote-object-3");
    assert_eq!(
        command.operation,
        DevToolsDomObjectReferenceOperation::DescribeNode {
            depth: 2,
            pierce: true
        }
    );
}

#[test]
fn devtools_dom_entry_routes_object_reference_command_to_dom_owner() {
    let mut conn = CdpConnection::new();
    let params = json!({
        "objectId": "remote-object-2"
    });
    let cmd = Cmd::for_test(
        Some(74),
        "DOM.requestNode",
        &params,
        Some("SID-dom"),
        r#"{"id":74,"method":"DOM.requestNode"}"#,
    );
    let command = super::build_cdp_dom_object_reference_command(
        &conn,
        &cmd,
        DevToolsDomObjectReferenceOperation::RequestNode,
    );
    let Ok(Some(command)) = command else {
        panic!("valid requestNode object reference command");
    };

    let result = start_devtools_dom_command(
        &mut conn,
        cmd.id,
        cmd.session_id,
        AutomationCommand::DomObjectReference(command),
    );

    let Err(error) = result else {
        panic!("missing document should surface through the unified DOM entry");
    };
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "NoDocumentLoaded");
}

#[test]
fn cdp_set_file_input_files_builds_protocol_neutral_object_command() {
    let conn = CdpConnection::new();
    let file_path = unique_test_file_path("set-file-object");
    std::fs::write(&file_path, b"upload bytes").expect("test upload file should be writable");
    let params = json!({
        "objectId": "remote-file-input",
        "files": [file_path.to_string_lossy()]
    });
    let cmd = Cmd::for_test(
        Some(104),
        "DOM.setFileInputFiles",
        &params,
        Some("SID-dom"),
        r#"{"id":104,"method":"DOM.setFileInputFiles"}"#,
    );

    let command = super::super::set_file_input::build_cdp_set_file_input_files_command(&conn, &cmd);
    let _ = std::fs::remove_file(&file_path);
    let Ok(Some(command)) = command else {
        panic!("valid setFileInputFiles object command");
    };

    assert_eq!(command.context.protocol, FrontendProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-dom")
    );
    assert_eq!(command.object_id.as_str(), "remote-file-input");
    assert_eq!(command.files.len(), 1);
    assert_eq!(command.files[0].bytes, b"upload bytes");
    assert!(
        command.files[0]
            .name
            .starts_with("moli-protocol-set-file-object-"),
        "unexpected selected file name: {}",
        command.files[0].name
    );
    assert!(!command.append);
}

#[test]
fn cdp_set_file_input_files_node_reference_falls_back_to_pending_node_reference_path() {
    let conn = CdpConnection::new();
    let params = json!({
        "backendNodeId": 25,
        "files": ["/tmp/upload.txt"]
    });
    let cmd = Cmd::for_test(
        Some(105),
        "DOM.setFileInputFiles",
        &params,
        Some("SID-dom"),
        r#"{"id":105,"method":"DOM.setFileInputFiles"}"#,
    );

    let command = super::super::set_file_input::build_cdp_set_file_input_files_command(&conn, &cmd);

    let Ok(None) = command else {
        panic!("setFileInputFiles without objectId should use node-reference pending path");
    };
}
