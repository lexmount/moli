use crate::devtools_runtime::DevToolsProtocol;
use serde_json::Value;

use crate::conn::Cmd;

use super::activation::{ActivateTargetParams, build_cdp_activate_target_command};
use super::closing::{CloseTargetParams, build_cdp_close_target_command};
use super::creation::{CreateTargetParams, build_cdp_create_target_command};

#[test]
fn cdp_create_target_builds_protocol_neutral_target_command() {
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(31),
        "Target.createTarget",
        &params,
        Some("SID-create"),
        r#"{"id":31,"method":"Target.createTarget"}"#,
    );

    let command = build_cdp_create_target_command(
        &cmd,
        CreateTargetParams {
            url: "https://example.com/new".to_owned(),
            browser_context_id: Some("BID-create".to_owned()),
            for_tab: None,
            background: None,
            focus: None,
        },
    )
    .expect("valid target disposition");

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-create")
    );
    assert_eq!(command.context.target_id, None);
    assert_eq!(
        command
            .context
            .browser_context_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("BID-create")
    );
    assert_eq!(command.url, "https://example.com/new");
    assert_eq!(
        command.browser_context_id.as_ref().map(|id| id.as_str()),
        Some("BID-create")
    );
    assert!(command.activate);
}

#[test]
fn cdp_close_target_builds_protocol_neutral_target_command() {
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(32),
        "Target.closeTarget",
        &params,
        Some("SID-close"),
        r#"{"id":32,"method":"Target.closeTarget"}"#,
    );

    let command = build_cdp_close_target_command(
        &cmd,
        CloseTargetParams {
            target_id: "TID-close".to_owned(),
        },
    );

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-close")
    );
    assert_eq!(
        command.context.target_id.as_ref().map(|id| id.as_str()),
        Some("TID-close")
    );
    assert_eq!(command.context.browser_context_id, None);
    assert_eq!(command.target_id.as_str(), "TID-close");
}

#[test]
fn cdp_activate_target_builds_protocol_neutral_target_command() {
    let params = Value::Null;
    let cmd = Cmd::for_test(
        Some(33),
        "Target.activateTarget",
        &params,
        Some("SID-activate"),
        r#"{"id":33,"method":"Target.activateTarget"}"#,
    );

    let command = build_cdp_activate_target_command(
        &cmd,
        ActivateTargetParams {
            target_id: "TID-activate".to_owned(),
        },
    );

    assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
    assert_eq!(
        command.context.session_id.as_ref().map(|id| id.as_str()),
        Some("SID-activate")
    );
    assert_eq!(
        command.context.target_id.as_ref().map(|id| id.as_str()),
        Some("TID-activate")
    );
    assert_eq!(command.context.browser_context_id, None);
    assert_eq!(command.target_id.as_str(), "TID-activate");
}
