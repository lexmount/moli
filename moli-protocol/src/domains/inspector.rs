use crate::conn::{CdpConnection, Cmd, SessionOwnerInspectorEnableResult};
use crate::domains::actions::InspectorAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<InspectorAction>() {
        Some(InspectorAction::Enable) => {
            if let Some(plan) =
                service_worker_inspector_command_output_plan(conn, cmd.session_id, true)
            {
                return plan;
            }
            match conn.set_inspector_enabled_for_session_owner(cmd.session_id, true) {
                SessionOwnerInspectorEnableResult::TargetCrashed { event_session_id } => {
                    let mut plan = CommandOutputPlan::success();
                    plan.push_inspector_target_crashed(event_session_id.as_deref());
                    plan
                }
                SessionOwnerInspectorEnableResult::Handled => CommandOutputPlan::success(),
                SessionOwnerInspectorEnableResult::UnknownSession => {
                    CommandOutputPlan::error(-32001, "Unknown sessionId")
                }
            }
        }
        Some(InspectorAction::Disable) => {
            if let Some(plan) =
                service_worker_inspector_command_output_plan(conn, cmd.session_id, false)
            {
                return plan;
            }
            match conn.set_inspector_enabled_for_session_owner(cmd.session_id, false) {
                SessionOwnerInspectorEnableResult::Handled
                | SessionOwnerInspectorEnableResult::TargetCrashed { .. } => {
                    CommandOutputPlan::success()
                }
                SessionOwnerInspectorEnableResult::UnknownSession => {
                    CommandOutputPlan::error(-32001, "Unknown sessionId")
                }
            }
        }
        None => CommandOutputPlan::error(-32601, "UnknownMethod"),
    }
}

fn service_worker_inspector_command_output_plan(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    enabled: bool,
) -> Option<CommandOutputPlan> {
    let session_id = session_id?;
    let target = conn.service_worker_target_for_session_mut(Some(session_id))?;
    if !target.set_inspector_enabled(session_id, enabled) {
        return Some(CommandOutputPlan::error(-32001, "Unknown sessionId"));
    }

    let mut plan = CommandOutputPlan::success();
    if enabled
        && !target.worker_running()
        && target.record_inspector_target_crashed_for_session(session_id)
    {
        plan.push_inspector_target_crashed(Some(session_id));
    }
    Some(plan)
}

#[cfg(test)]
mod tests {
    use moli_core::page::RendererServiceWorkerVersionStatus;

    use crate::{
        conn::{BrowserContext, PageTargetHost, ServiceWorkerTargetState},
        testing::TestContext,
    };
    use serde_json::json;

    #[tokio::test]
    async fn inspector_enable_and_disable_toggle_browser_context_state() {
        let mut ctx = TestContext::new();
        ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));

        ctx.process_async(json!({"id": 1, "method": "Inspector.enable"}))
            .await;
        ctx.expect_result(1, json!({}), None);
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );

        ctx.process_async(json!({"id": 2, "method": "Inspector.disable"}))
            .await;
        ctx.expect_result(2, json!({}), None);
        assert!(
            !ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
    }

    #[tokio::test]
    async fn inspector_enable_replays_target_crashed_for_crashed_target() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.attach_active_session("SID-1");
        bc.active_page_target_mut()
            .owner_state
            .target_crash_state
            .mark_crashed();
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({"id": 3, "method": "Inspector.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(3, json!({}), Some("SID-1"));
        let event = ctx.take_one();
        assert_eq!(event["method"], "Inspector.targetCrashed");
        assert_eq!(event["sessionId"], "SID-1");
        assert!(
            ctx.conn
                .target_runtime_session_state_for_session(Some("SID-1"))
                .expect("runtime session state")
                .inspector_target_crashed_delivered()
        );
    }

    #[tokio::test]
    async fn inspector_enable_replays_crash_to_exact_attached_session() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-primary");
        assert!(bc.assign_attached_session_to_target("TID-1", "SID-attached".into()));
        bc.active_page_target_mut()
            .owner_state
            .target_crash_state
            .mark_crashed();
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({
            "id": 30,
            "method": "Inspector.enable",
            "sessionId": "SID-attached"
        }))
        .await;
        ctx.expect_result(30, json!({}), Some("SID-attached"));
        let event = ctx.take_one();
        assert_eq!(event["method"], "Inspector.targetCrashed");
        assert_eq!(event["sessionId"], "SID-attached");
        assert!(
            !ctx.conn
                .target_runtime_session_state_for_session(Some("SID-primary"))
                .expect("primary runtime session state")
                .inspector_target_crashed_delivered()
        );
        assert!(
            ctx.conn
                .target_runtime_session_state_for_session(Some("SID-attached"))
                .expect("attached runtime session state")
                .inspector_target_crashed_delivered()
        );
    }

    #[tokio::test]
    async fn inspector_enable_replays_background_target_crash_without_activation() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active");
        bc.attach_active_session("SID-active");
        bc.insert_page_target_host(PageTargetHost::with_url(
            "TID-background".into(),
            Some("SID-background".into()),
            "about:blank#background".into(),
        ));
        bc.background_target_mut("TID-background")
            .expect("background target must exist")
            .owner_state
            .target_crash_state
            .mark_crashed();
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({
            "id": 301,
            "method": "Inspector.enable",
            "sessionId": "SID-background"
        }))
        .await;
        ctx.expect_result(301, json!({}), Some("SID-background"));
        let event = ctx.take_one();
        assert_eq!(event["method"], "Inspector.targetCrashed");
        assert_eq!(event["sessionId"], "SID-background");
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-active"));
        assert!(
            bc.background_target("TID-background")
                .filter(|target| target.has_non_default_session_state())
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .inspector_target_crashed_delivered())
        );
    }

    #[tokio::test]
    async fn inspector_enable_replays_target_crashed_for_stopped_service_worker() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        let mut target = ServiceWorkerTargetState::new(
            41,
            7,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-service-worker".to_owned());
        bc.insert_service_worker_target(target);
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({
            "id": 31,
            "method": "Inspector.enable",
            "sessionId": "SID-service-worker"
        }))
        .await;
        ctx.expect_result(31, json!({}), Some("SID-service-worker"));
        let event = ctx.take_one();
        assert_eq!(event["method"], "Inspector.targetCrashed");
        assert_eq!(event["sessionId"], "SID-service-worker");
        assert!(
            ctx.conn
                .service_worker_target_for_session(Some("SID-service-worker"))
                .expect("service worker target")
                .inspector_enabled("SID-service-worker")
        );

        ctx.process_async(json!({
            "id": 32,
            "method": "Inspector.disable",
            "sessionId": "SID-service-worker"
        }))
        .await;
        ctx.expect_result(32, json!({}), Some("SID-service-worker"));
        assert!(
            !ctx.conn
                .service_worker_target_for_session(Some("SID-service-worker"))
                .expect("service worker target")
                .inspector_enabled("SID-service-worker")
        );
    }

    #[tokio::test]
    async fn inspector_enable_and_disable_stage_background_target_session_state() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-A");
        bc.attach_active_session("SID-active");
        bc.insert_page_target_host(crate::conn::PageTargetHost::new(
            "TID-B".into(),
            Some("SID-B".into()),
            crate::conn::TargetIdentityState::new(
                "about:blank#background".into(),
                "null".into(),
                "InsecureScheme".into(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
        bc.active_page_target_mut()
            .owner_state
            .target_crash_state
            .mark_crashed();
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({"id": 4, "method": "Inspector.enable", "sessionId": "SID-B"}))
            .await;
        ctx.expect_result(4, json!({}), Some("SID-B"));
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Inspector.targetCrashed")),
            "background staging should not replay active target crash state"
        );
        {
            let bc = ctx.conn.browser_context.as_ref().expect("browser context");
            assert!(
                !bc.active_page_target().devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .inspector_enabled
            );
            assert!(
                bc.background_target("TID-B")
                    .filter(|target| target.has_non_default_session_state())
                    .is_some_and(|state| {
                        state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                            .runtime_session_state
                            .inspector_enabled
                    })
            );
        }

        ctx.process_async(json!({"id": 5, "method": "Inspector.disable", "sessionId": "SID-B"}))
            .await;
        ctx.expect_result(5, json!({}), Some("SID-B"));
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert!(
            !bc.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert!(
            bc.background_target("TID-B")
                .filter(|target| target.has_non_default_session_state())
                .is_none(),
            "disable should collapse staged background state back to default"
        );
    }
}
