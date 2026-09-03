use serde_json::json;

use crate::conn::Cmd;
use crate::domains::actions::SystemInfoAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) fn command_output_plan(cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<SystemInfoAction>() {
        Some(SystemInfoAction::GetInfo) => get_info(cmd),
        Some(SystemInfoAction::GetProcessInfo) => get_process_info(cmd),
        None => CommandOutputPlan::error(-32601, "UnknownMethod"),
    }
}

fn get_info(cmd: &Cmd<'_>) -> CommandOutputPlan {
    if cmd.session_id.is_some() {
        return CommandOutputPlan::error(
            -32000,
            "SystemInfo.getInfo is only supported on the browser target",
        );
    }
    CommandOutputPlan::result(json!({
        "gpu": {
            "devices": [],
            "auxAttributes": {},
            "featureStatus": {},
            "driverBugWorkarounds": [],
            "videoDecoding": [],
            "videoEncoding": [],
            "imageDecoding": [],
        },
        "modelName": "",
        "modelVersion": "",
        "commandLine": "",
    }))
}

fn get_process_info(cmd: &Cmd<'_>) -> CommandOutputPlan {
    if cmd.session_id.is_some() {
        return CommandOutputPlan::error(
            -32000,
            "SystemInfo.getProcessInfo is only supported on the browser target",
        );
    }
    CommandOutputPlan::result(json!({
        "processInfo": [{
            "type": "browser",
            "id": std::process::id(),
            "cpuTime": 0.0,
        }],
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::conn::BrowserContext;
    use crate::testing::TestContext;

    #[tokio::test(flavor = "multi_thread")]
    async fn system_info_get_info_returns_expected_shape() {
        // Chromium source:
        // content/browser/devtools/protocol/system_info_handler.cc
        let mut ctx = TestContext::new();
        ctx.process_async(json!({
            "id": 1,
            "method": "SystemInfo.getInfo"
        }))
        .await;
        let response = ctx.take_response_by_id(1);
        let result = &response["result"];
        assert!(result["gpu"]["devices"].is_array());
        assert!(result["gpu"]["auxAttributes"].is_object());
        assert!(result["gpu"]["featureStatus"].is_object());
        assert!(result["gpu"]["driverBugWorkarounds"].is_array());
        assert!(result["gpu"]["videoDecoding"].is_array());
        assert!(result["gpu"]["videoEncoding"].is_array());
        assert!(result["gpu"]["imageDecoding"].is_array());
        assert!(result["modelName"].is_string());
        assert!(result["modelVersion"].is_string());
        assert!(result["commandLine"].is_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn system_info_get_info_rejects_frame_target_session() {
        // Chromium source:
        // content/browser/devtools/protocol/system_info_handler.cc
        let mut ctx = TestContext::new();
        let mut browser_context = BrowserContext::new("BID-system-info".to_owned());
        browser_context.set_active_target_id("TID-system-info".to_owned());
        browser_context.attach_active_session("SID-frame".to_owned());
        ctx.conn
            .install_browser_context_fixture_for_test(browser_context);

        ctx.process_async(json!({
            "id": 2,
            "method": "SystemInfo.getInfo",
            "sessionId": "SID-frame"
        }))
        .await;
        assert_eq!(
            ctx.take_response_by_id(2),
            json!({
                "id": 2,
                "error": {
                    "code": -32000,
                    "message": "SystemInfo.getInfo is only supported on the browser target",
                },
                "sessionId": "SID-frame",
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn system_info_get_process_info_returns_browser_process() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({
            "id": 1,
            "method": "SystemInfo.getProcessInfo"
        }))
        .await;
        let response = ctx.take_response_by_id(1);
        assert_eq!(response["result"]["processInfo"][0]["type"], "browser");
        assert!(response["result"]["processInfo"][0]["id"].is_number());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn system_info_get_process_info_rejects_frame_target_session() {
        let mut ctx = TestContext::new();
        let mut browser_context = BrowserContext::new("BID-system-info".to_owned());
        browser_context.set_active_target_id("TID-system-info".to_owned());
        browser_context.attach_active_session("SID-frame".to_owned());
        ctx.conn
            .install_browser_context_fixture_for_test(browser_context);

        ctx.process_async(json!({
            "id": 2,
            "method": "SystemInfo.getProcessInfo",
            "sessionId": "SID-frame"
        }))
        .await;
        assert_eq!(
            ctx.take_response_by_id(2),
            json!({
                "id": 2,
                "error": {
                    "code": -32000,
                    "message": "SystemInfo.getProcessInfo is only supported on the browser target",
                },
                "sessionId": "SID-frame",
            })
        );
    }
}
