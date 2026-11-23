//! CDP instance commands run after normal frontend routing and before page work.

use std::time::Duration;

use moli_protocol::ParsedCdpCommand;
use serde_json::json;

use crate::{BrowserInstanceControl, InstanceCommand, cdp_frontend_router::CdpFrontendRouter};

const BROWSER_CLOSE_FLUSH_BUDGET: Duration = Duration::from_secs(1);

/// The actor has already validated the request's session through the Router.
/// Its internal command ID routes the response back to the original frontend,
/// client command ID, and public session using the ordinary response path.
pub(super) async fn handle_instance_command(
    command: &ParsedCdpCommand,
    router: &CdpFrontendRouter,
    control: &BrowserInstanceControl,
) -> bool {
    if command.method() == "Browser.getBrowserCommandLine" {
        let id = command.request().id();
        let response = if control.expose_command_line {
            let arguments: Vec<_> = std::env::args_os()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            json!({"id": id, "result": {"arguments": arguments}})
        } else {
            json!({"id": id, "error": {
                "code": -32000,
                "message": "Command line not returned because --enable-automation not set.",
            }})
        };
        router.enqueue_response(response);
        return true;
    }
    let instance_command = match command.method() {
        "Browser.close" => InstanceCommand::Shutdown,
        _ => return false,
    };
    if let Some(sink) = router.enqueue_response(json!({"id": command.request().id(), "result": {}}))
        && !matches!(
            tokio::time::timeout(BROWSER_CLOSE_FLUSH_BUDGET, sink.flush()).await,
            Ok(true)
        )
    {
        sink.close();
    }
    // Routing accepted the instance command. Finish it after the bounded reply
    // attempt even if the frontend has left or its writer cannot make progress.
    control.request(instance_command);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cdp_frontend_router::CdpPreparedFrontendCommand, cdp_writer::CdpSocketSink};
    use futures_util::FutureExt;

    fn prepare_request(
        router: &CdpFrontendRouter,
        sink: CdpSocketSink,
        method: &str,
    ) -> ParsedCdpCommand {
        router
            .register_browser_frontend(1, "hidden-browser".into(), sink)
            .unwrap();
        let parsed = ParsedCdpCommand::parse_value(json!({
            "id": 42, "method": method,
        }));
        let Some(CdpPreparedFrontendCommand::Command(command)) = router.prepare_command(1, parsed)
        else {
            panic!("registered frontend must route normally");
        };
        command
    }

    #[tokio::test]
    async fn browser_command_line_requires_enable_automation() {
        for enabled in [false, true] {
            let mut control = BrowserInstanceControl::new();
            control.expose_command_line = enabled;
            let router = CdpFrontendRouter::new();
            let (sink, mut writer) = CdpSocketSink::with_stalled_writer_for_test(2);
            let command = prepare_request(&router, sink, "Browser.getBrowserCommandLine");
            assert!(handle_instance_command(&command, &router, &control).await);
            let response = writer.take_message();
            if enabled {
                assert_eq!(
                    response["result"]["arguments"].as_array().unwrap().len(),
                    std::env::args_os().count()
                );
            } else {
                assert_eq!(response["error"]["code"], -32000);
            }
        }
    }

    #[tokio::test]
    async fn browser_close_survives_failed_response_enqueue() {
        let control = BrowserInstanceControl::new();
        let router = CdpFrontendRouter::new();
        let command = prepare_request(&router, CdpSocketSink::for_test(), "Browser.close");
        assert!(handle_instance_command(&command, &router, &control).await);
        assert!(control.shutdown_requested().now_or_never().is_some());
    }

    #[tokio::test]
    async fn browser_close_survives_unconfirmed_writer_flush() {
        let control = BrowserInstanceControl::new();
        let router = CdpFrontendRouter::new();
        let (sink, mut writer) = CdpSocketSink::with_stalled_writer_for_test(2);
        let command = prepare_request(&router, sink, "Browser.close");
        assert!(handle_instance_command(&command, &router, &control).await);
        assert_eq!(writer.take_message(), json!({"id":42,"result":{}}));
        assert!(!writer.is_open(), "failed flush closes its frontend");
        assert!(control.shutdown_requested().now_or_never().is_some());
    }

    #[tokio::test]
    async fn browser_close_survives_frontend_departure_after_acceptance() {
        let control = BrowserInstanceControl::new();
        let router = CdpFrontendRouter::new();
        let command = prepare_request(&router, CdpSocketSink::for_test(), "Browser.close");
        assert!(router.unregister_browser_frontend(1).is_some());
        assert!(handle_instance_command(&command, &router, &control).await);
        assert!(control.shutdown_requested().now_or_never().is_some());
    }
}
