use crate::conn::NavigationDispatchState;
use crate::devtools_runtime::DevToolsProtocol;
use crate::domains::command_output::CommandOutputBuffer;
use crate::domains::network::{
    FailedNavigationResponseMode, MainDocumentProgressBackgroundEventBarrier,
    MainDocumentProgressGate,
};
use serde_json::json;

pub(crate) struct MainDocumentFailedNavigationActivity {
    state: NavigationDispatchState,
    progress_gate: MainDocumentProgressGate,
    response_mode: FailedNavigationResponseMode,
}

impl MainDocumentFailedNavigationActivity {
    pub(crate) fn new(
        state: NavigationDispatchState,
        progress_gate: MainDocumentProgressGate,
        response_mode: FailedNavigationResponseMode,
    ) -> Self {
        Self {
            state,
            progress_gate,
            response_mode,
        }
    }

    pub(crate) fn emit_navigation_error_into_buffer(
        mut self,
        out: &mut CommandOutputBuffer,
        message: &str,
    ) {
        let navigate_id = self.state.navigate_id;
        {
            let mut background_events = Vec::new();
            let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
                &mut background_events,
                &mut self.progress_gate,
            );
            output.drain_progress();
            out.extend_background_events_after_messages(background_events);
        }
        if navigate_id.is_some() {
            if self.response_mode == FailedNavigationResponseMode::CdpErrorTextResult
                && self.state.result_projection.protocol() == DevToolsProtocol::Cdp
            {
                let mut result_payload = self.state.result_projection.into_payload();
                if let Some(payload) = result_payload.as_object_mut() {
                    payload.insert("errorText".to_owned(), json!(message));
                    payload.insert("isDownload".to_owned(), json!(false));
                }
                out.push_result_after_messages(result_payload);
            } else {
                out.push_error_after_messages(-32000, message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{CommandOwnerScope, NavigationResultProjection};
    use crate::domains::network::{
        empty_main_document_progress_gate_for_test, failed_navigation_progress_gate,
    };
    use serde_json::Value;
    use url::Url;
    fn navigation_state() -> NavigationDispatchState {
        NavigationDispatchState {
            redirect_chain: Vec::new(),
            redirect_headers: None,
            navigate_id: Some(77),
            owner: CommandOwnerScope::for_session("SID-nav"),
            web_contents: NavigationDispatchState::detached_web_contents_for_test(),
            result_projection: NavigationResultProjection::Cdp(
                json!({ "frameId": "FRAME-1", "loaderId": "LID-1" }),
            ),
            frame_id: "FRAME-1".to_owned(),
            session_id: Some("SID-page".to_owned()),
            request_id: Some("REQ-1".to_owned()),
            loader_id: "LID-1".to_owned(),
            request_announced: false,
            requested_url: Url::parse("https://example.test/start").unwrap(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: vec![("Accept".to_owned(), "text/html".to_owned())].into(),
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 12.5,
        }
    }

    #[test]
    fn navigation_activity_error_drains_progress_before_error_response() {
        let mut conn = crate::test_support::connection();
        let mut browser_context = conn.new_page_target_fixture_for_test("BID-1", "TID-page");
        browser_context.attach_active_session("SID-page");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .enable_primary_network_events();
        conn.install_browser_context_fixture_for_test(browser_context);

        let state = NavigationDispatchState {
            owner: CommandOwnerScope::capture(&conn, Some("SID-page")),
            ..navigation_state()
        };
        let progress_gate = failed_navigation_progress_gate(&conn, &state, "net::ERR_ABORTED");
        let mut output = CommandOutputBuffer::default();
        MainDocumentFailedNavigationActivity::new(
            state,
            progress_gate,
            FailedNavigationResponseMode::ProtocolError,
        )
        .emit_navigation_error_into_buffer(&mut output, "download activation failed");
        let mut out = Vec::new();
        output
            .into_plan()
            .emit_into(&mut out, Some(77), Some("SID-nav"));

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["method"], json!("Network.loadingFailed"));
        assert_eq!(out[0]["params"]["errorText"], json!("net::ERR_ABORTED"));
        assert_eq!(out[0]["params"]["requestId"], json!("REQ-1"));
        assert_eq!(out[1]["id"], json!(77));
        assert_eq!(
            out[1]["error"]["message"],
            json!("download activation failed")
        );
    }

    fn failed_navigation_messages(
        error_text: &str,
        response_mode: FailedNavigationResponseMode,
    ) -> Vec<Value> {
        let mut conn = crate::test_support::connection();
        let mut browser_context = conn.new_page_target_fixture_for_test("BID-1", "TID-page");
        browser_context.attach_active_session("SID-page");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .enable_primary_network_events();
        conn.install_browser_context_fixture_for_test(browser_context);

        let state = NavigationDispatchState {
            owner: CommandOwnerScope::capture(&conn, Some("SID-page")),
            ..navigation_state()
        };
        let progress_gate = failed_navigation_progress_gate(&conn, &state, error_text);
        let mut output = CommandOutputBuffer::default();
        MainDocumentFailedNavigationActivity::new(state, progress_gate, response_mode)
            .emit_navigation_error_into_buffer(&mut output, error_text);
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));
        messages
    }

    fn assert_failed_navigation_terminal(messages: &[Value], error_text: &str) {
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["method"], json!("Network.loadingFailed"));
        assert_eq!(messages[0]["sessionId"], json!("SID-page"));
        assert_eq!(messages[0]["params"]["requestId"], json!("REQ-1"));
        assert_eq!(messages[0]["params"]["errorText"], json!(error_text));
        assert_eq!(messages[1]["id"], json!(77));
        assert_eq!(messages[1]["sessionId"], json!("SID-nav"));
    }

    #[test]
    fn failed_navigation_protocol_error_preserves_context_chain() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        ))
        .context("failed to read page body from stream")
        .context("failed to load document")
        .context("failed to continue intercepted navigation");
        let messages = failed_navigation_messages(
            &format!("{error:#}"),
            FailedNavigationResponseMode::ProtocolError,
        );
        let expected = "failed to continue intercepted navigation: failed to load document: failed to read page body from stream: connection reset by peer";

        assert_eq!(messages[1]["error"]["code"], json!(-32000));
        assert_eq!(messages[1]["error"]["message"], json!(expected));
        assert!(messages[1].get("result").is_none());
        assert_failed_navigation_terminal(&messages, expected);
    }

    #[test]
    fn failed_navigation_protocol_error_does_not_classify_network_display_text() {
        for error_text in [
            "net::ERR_INTERNET_DISCONNECTED",
            "net::ERR_BLOCKED_BY_CLIENT",
        ] {
            let error = anyhow::anyhow!(error_text).context("failed to load document");
            let messages = failed_navigation_messages(
                &format!("{error:#}"),
                FailedNavigationResponseMode::ProtocolError,
            );
            let expected = format!("failed to load document: {error_text}");

            assert_eq!(messages[1]["error"]["code"], json!(-32000));
            assert_eq!(messages[1]["error"]["message"], json!(expected));
            assert!(messages[1].get("result").is_none());
            assert_failed_navigation_terminal(&messages, &expected);
        }
    }

    #[test]
    fn failed_cdp_navigation_returns_error_text_result_after_network_terminal() {
        let messages = failed_navigation_messages(
            "net::ERR_CONNECTION_RESET",
            FailedNavigationResponseMode::CdpErrorTextResult,
        );

        assert_failed_navigation_terminal(&messages, "net::ERR_CONNECTION_RESET");
        assert_eq!(messages[1]["result"]["frameId"], json!("FRAME-1"));
        assert_eq!(messages[1]["result"]["loaderId"], json!("LID-1"));
        assert_eq!(
            messages[1]["result"]["errorText"],
            json!("net::ERR_CONNECTION_RESET")
        );
        assert_eq!(messages[1]["result"]["isDownload"], json!(false));
        assert!(messages[1].get("error").is_none());
    }

    #[test]
    fn failed_bidi_navigation_remains_a_protocol_error() {
        let mut state = navigation_state();
        state.result_projection = NavigationResultProjection::WebDriverBidi(json!({
            "frameId": "must-not-select-cdp",
            "navigation": "LID-1",
            "url": "https://example.test/start"
        }));
        let mut output = CommandOutputBuffer::default();
        MainDocumentFailedNavigationActivity::new(
            state,
            empty_main_document_progress_gate_for_test(),
            FailedNavigationResponseMode::CdpErrorTextResult,
        )
        .emit_navigation_error_into_buffer(&mut output, "net::ERR_CONNECTION_RESET");
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["error"]["code"], json!(-32000));
        assert_eq!(
            messages[0]["error"]["message"],
            json!("net::ERR_CONNECTION_RESET")
        );
    }
}
