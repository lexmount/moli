use moli_core::page::InspectorIssueSnapshot;

use crate::conn::{CdpConnection, Cmd};
use crate::devtools_runtime::{DevToolsError, DevToolsErrorKind};
use crate::domains::actions::AuditsAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) struct TargetAuditsReplaySnapshot {
    pub(crate) issues: Vec<InspectorIssueSnapshot>,
}

pub(crate) enum SessionOwnerAuditsEnableResult {
    Handled {
        replay: Option<TargetAuditsReplaySnapshot>,
    },
    UnknownSession,
}

pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<AuditsAction>() {
        Some(AuditsAction::Enable) => enable_command(conn, cmd),
        Some(AuditsAction::Disable) => disable_command(conn, cmd),
        None => CommandOutputPlan::error(-32601, "UnknownMethod"),
    }
}

fn enable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match conn.enable_audits_for_session_owner(cmd.session_id) {
        SessionOwnerAuditsEnableResult::Handled { replay } => {
            let mut plan = CommandOutputPlan::default();
            if let Some(replay) = replay {
                append_replay(conn, &mut plan, cmd.session_id, replay);
            }
            // Blink's InspectorAuditsAgent registers with the Page issue
            // storage and synchronously replays it before enable completes.
            plan.push_success();
            plan
        }
        SessionOwnerAuditsEnableResult::UnknownSession => unknown_session_output_plan(),
    }
}

fn disable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if conn.disable_audits_for_session_owner(cmd.session_id) {
        CommandOutputPlan::success()
    } else {
        unknown_session_output_plan()
    }
}

fn append_replay(
    conn: &CdpConnection,
    plan: &mut CommandOutputPlan,
    session_id: Option<&str>,
    replay: TargetAuditsReplaySnapshot,
) {
    let Some((_, Some(frame_id))) = conn.target_owner_identity_for_session(session_id) else {
        return;
    };
    let loader_id = conn
        .current_document_loader_id_for_session_owner(session_id)
        .unwrap_or_default();
    for issue in replay.issues {
        plan.push_background_event(crate::conn::BackgroundProtocolEvent::audits_issue_added(
            session_id, &issue, &frame_id, &loader_id,
        ));
    }
}

fn unknown_session_output_plan() -> CommandOutputPlan {
    CommandOutputPlan::from_devtools_error(DevToolsError::new(
        DevToolsErrorKind::NoSuchSession,
        "Unknown sessionId",
    ))
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        ContentSecurityPolicyIssueSnapshot, ContentSecurityPolicyViolationType,
        InspectorIssueSnapshot,
    };
    use serde_json::json;

    use crate::conn::CommandOwnerScope;
    use crate::domains::observable_output::{
        ObservableOutputProjectionStep, ObservablePreparedOutputSlot,
        inspector_issue_prepared_outputs,
    };
    use crate::testing::TestContext;

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut browser_context = ctx
            .conn
            .new_browser_context_fixture_for_test("BID-audits".to_owned());
        browser_context.set_active_target_id("TID-audits".to_owned());
        browser_context.set_target_url("data:text/html,audits-test".to_owned());
        browser_context.attach_active_session("SID-audits".to_owned());
        ctx.conn
            .install_browser_context_fixture_for_test(browser_context);
        ctx.install_quiet_navigation_fixture_for_session_owner(
            &format!("data:text/html,{html}"),
            Some("SID-audits"),
        )
        .await;
    }

    fn csp_issue() -> InspectorIssueSnapshot {
        InspectorIssueSnapshot::ContentSecurityPolicy(ContentSecurityPolicyIssueSnapshot::new(
            false,
            "script-src-elem".to_owned(),
            ContentSecurityPolicyViolationType::Inline,
        ))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reenable_replays_concrete_issue_before_lagging_report_snapshot() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body>standards</body>").await;
        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .unwrap()
                .assign_attached_session_to_target("TID-audits", "SID-audits-peer".to_owned())
        );
        for (id, method, session) in [
            (1, "Audits.enable", "SID-audits"),
            (2, "Audits.enable", "SID-audits-peer"),
            (3, "Audits.disable", "SID-audits-peer"),
        ] {
            ctx.process_async(json!({"id": id, "method": method, "sessionId": session}))
                .await;
            ctx.expect_result(id, json!({}), Some(session));
        }

        let owner = CommandOwnerScope::capture(&ctx.conn, Some("SID-audits"));
        let source_document = ctx
            .conn
            .committed_renderer_document_binding_for_owner(&owner)
            .unwrap()
            .renderer_document_identity();
        // Admit the concrete FIFO fact while the cumulative Page report still
        // describes the issue-free fixture. This is the production race's exact
        // ordering, without depending on how quickly a renderer snapshot arrives.
        let prepared =
            inspector_issue_prepared_outputs(&mut ctx.conn, source_document, csp_issue(), &owner);
        let mut slot = ObservablePreparedOutputSlot::from_outputs(prepared);
        let mut live = Vec::new();
        slot.emit_activity_background_events_async(
            ObservableOutputProjectionStep::Audits,
            &mut ctx.conn,
            &mut live,
            owner.session_id(),
        )
        .await;
        assert_eq!(live.len(), 1);
        let live = live.pop().unwrap().into_protocol_message();
        assert_eq!(live["sessionId"], json!("SID-audits"));
        assert_eq!(live["method"], json!("Audits.issueAdded"));

        for (id, method) in [
            (4, "Audits.enable"),
            (5, "Audits.enable"),
            (6, "Audits.disable"),
            (7, "Audits.enable"),
        ] {
            ctx.process_async(json!({
                "id": id, "method": method, "sessionId": "SID-audits-peer",
            }))
            .await;
            if id == 4 || id == 7 {
                let replay = ctx.take_one();
                assert_eq!(replay["method"], json!("Audits.issueAdded"));
                assert_eq!(replay["sessionId"], json!("SID-audits-peer"));
                assert_eq!(replay["params"], live["params"]);
            }
            ctx.expect_result(id, json!({}), Some("SID-audits-peer"));
            assert!(
                ctx.sent.is_empty(),
                "unexpected replay or peer fanout: {:?}",
                ctx.sent
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replacement_document_retires_stored_issues_and_rejects_late_facts() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body>predecessor</body>").await;
        let owner = CommandOwnerScope::capture(&ctx.conn, Some("SID-audits"));
        let source_document = ctx
            .conn
            .committed_renderer_document_binding_for_owner(&owner)
            .unwrap()
            .renderer_document_identity();
        assert!(
            inspector_issue_prepared_outputs(&mut ctx.conn, source_document, csp_issue(), &owner)
                .is_empty()
        );
        assert_eq!(
            ctx.conn
                .target_owner_state_for_owner(&owner)
                .unwrap()
                .audits_storage_state
                .issue_end(),
            1
        );

        ctx.install_quiet_navigation_fixture_for_session_owner(
            "data:text/html,<!doctype html><body>replacement</body>",
            Some("SID-audits"),
        )
        .await;
        let owner = CommandOwnerScope::capture(&ctx.conn, Some("SID-audits"));
        assert_ne!(
            ctx.conn
                .committed_renderer_document_binding_for_owner(&owner)
                .unwrap()
                .renderer_document_identity(),
            source_document
        );
        assert!(
            inspector_issue_prepared_outputs(&mut ctx.conn, source_document, csp_issue(), &owner)
                .is_empty()
        );
        assert_eq!(
            ctx.conn
                .target_owner_state_for_owner(&owner)
                .unwrap()
                .audits_storage_state
                .issue_end(),
            0
        );
        ctx.process_async(json!({
            "id": 1, "method": "Audits.enable", "sessionId": "SID-audits",
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-audits"));
        assert!(
            ctx.sent.is_empty(),
            "retired issues must not replay: {:?}",
            ctx.sent
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enable_replays_quirks_issue_before_response() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<html><body>quirks</body></html>").await;

        ctx.process_async(json!({
            "id": 1,
            "method": "Audits.enable",
            "sessionId": "SID-audits",
        }))
        .await;

        let issue = ctx.take_one();
        assert_eq!(issue["method"], json!("Audits.issueAdded"));
        assert_eq!(issue["sessionId"], json!("SID-audits"));
        assert_eq!(issue["params"]["issue"]["code"], json!("QuirksModeIssue"));
        let details = &issue["params"]["issue"]["details"]["quirksModeIssueDetails"];
        assert_eq!(details["isLimitedQuirksMode"], json!(false));
        assert_eq!(details["frameId"], json!("TID-audits"));
        assert!(details["documentNodeId"].as_u64().is_some_and(|id| id > 0));
        ctx.expect_result(1, json!({}), Some("SID-audits"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repeated_enable_is_idempotent_and_reenable_replays_storage() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<html><body>quirks</body></html>").await;

        for id in [1, 2] {
            ctx.process_async(json!({
                "id": id,
                "method": "Audits.enable",
                "sessionId": "SID-audits",
            }))
            .await;
            if id == 1 {
                assert_eq!(ctx.take_one()["method"], json!("Audits.issueAdded"));
            }
            ctx.expect_result(id, json!({}), Some("SID-audits"));
        }

        ctx.process_async(json!({
            "id": 3,
            "method": "Audits.disable",
            "sessionId": "SID-audits",
        }))
        .await;
        ctx.expect_result(3, json!({}), Some("SID-audits"));
        ctx.process_async(json!({
            "id": 4,
            "method": "Audits.enable",
            "sessionId": "SID-audits",
        }))
        .await;
        ctx.take_first_matching("replayed Audits issue", |message| {
            message["method"] == json!("Audits.issueAdded")
        });
        ctx.expect_result(4, json!({}), Some("SID-audits"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enabled_session_receives_live_csp_issue() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'none'\"><body></body>",
        )
        .await;
        ctx.process_async(json!({
            "id": 1,
            "method": "Audits.enable",
            "sessionId": "SID-audits",
        }))
        .await;
        ctx.expect_result(1, json!({}), Some("SID-audits"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "sessionId": "SID-audits",
            "params": {
                "expression": "var s=document.createElement('script');s.text='window.blocked=1';document.body.appendChild(s);",
            },
        }))
        .await;

        let issue_index = ctx
            .sent
            .iter()
            .position(|message| message["method"] == json!("Audits.issueAdded"))
            .expect("live CSP issue should be published");
        let response_index = ctx
            .sent
            .iter()
            .position(|message| message["id"] == json!(2))
            .expect("Runtime.evaluate response should be published");
        assert!(
            issue_index < response_index,
            "the live CSP issue must precede the command response: {:?}",
            ctx.sent
        );

        let issue = ctx.take_first_matching("live CSP issue", |message| {
            message["method"] == json!("Audits.issueAdded")
        });
        let issue = &issue["params"]["issue"];
        assert_eq!(issue["code"], json!("ContentSecurityPolicyIssue"));
        let details = &issue["details"]["contentSecurityPolicyIssueDetails"];
        assert_eq!(details["isReportOnly"], json!(false));
        assert_eq!(details["violatedDirective"], json!("script-src-elem"));
        assert_eq!(
            details["contentSecurityPolicyViolationType"],
            json!("kInlineViolation")
        );
        assert!(details["violatingNodeId"].as_u64().is_some_and(|id| id > 0));
        let response = ctx.take_response_by_id(2);
        assert_eq!(response["id"], json!(2));
        assert!(response.get("result").is_some(), "{response:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sessions_enable_disable_and_replay_audits_independently() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"script-src 'none'\"><body></body>",
        )
        .await;
        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .expect("Audits fixture should retain its browser context")
                .assign_attached_session_to_target("TID-audits", "SID-audits-peer".to_owned())
        );

        for (id, session_id) in [(1, "SID-audits"), (2, "SID-audits-peer")] {
            ctx.process_async(json!({
                "id": id,
                "method": "Audits.enable",
                "sessionId": session_id,
            }))
            .await;
            ctx.expect_result(id, json!({}), Some(session_id));
        }
        ctx.process_async(json!({
            "id": 3,
            "method": "Audits.disable",
            "sessionId": "SID-audits",
        }))
        .await;
        ctx.expect_result(3, json!({}), Some("SID-audits"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 4,
            "method": "Runtime.evaluate",
            "sessionId": "SID-audits-peer",
            "params": {
                "expression": "var s=document.createElement('script');s.text='window.blocked=1';document.body.appendChild(s);",
            },
        }))
        .await;

        let live_issues = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("Audits.issueAdded"))
            .collect::<Vec<_>>();
        assert_eq!(
            live_issues.len(),
            1,
            "unexpected Audits fanout: {:?}",
            ctx.sent
        );
        assert_eq!(live_issues[0]["sessionId"], json!("SID-audits-peer"));
        ctx.take_response_by_id(4);
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 5,
            "method": "Audits.enable",
            "sessionId": "SID-audits",
        }))
        .await;
        let replay = ctx.take_one();
        assert_eq!(replay["method"], json!("Audits.issueAdded"));
        assert_eq!(replay["sessionId"], json!("SID-audits"));
        assert_eq!(
            replay["params"]["issue"]["code"],
            json!("ContentSecurityPolicyIssue")
        );
        ctx.expect_result(5, json!({}), Some("SID-audits"));
        assert!(
            ctx.sent.iter().all(|message| {
                message["method"] != json!("Audits.issueAdded")
                    || message["sessionId"] != json!("SID-audits-peer")
            }),
            "re-enabling one session must not replay to its peer: {:?}",
            ctx.sent
        );
    }
}
