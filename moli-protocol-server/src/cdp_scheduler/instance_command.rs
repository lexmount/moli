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
    let instance_command = match command.method() {
        "Browser.close" => InstanceCommand::Shutdown,
        _ => return false,
    };
    let Some(sink) = router.enqueue_response(json!({"id": command.request().id(), "result": {}}))
    else {
        return true;
    };
    if matches!(
        tokio::time::timeout(BROWSER_CLOSE_FLUSH_BUDGET, sink.flush()).await,
        Ok(true)
    ) {
        control.request(instance_command);
    } else {
        // A failed acknowledgment retires this frontend, not the instance or
        // the actor serving other connections.
        sink.close();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cdp_frontend_router::CdpPreparedFrontendCommand, cdp_writer::CdpSocketSink};

    #[tokio::test]
    async fn failed_or_unconfirmed_flush_does_not_stop_the_instance() {
        let control = BrowserInstanceControl::new();
        let router = CdpFrontendRouter::new();
        let (stalled_sink, stalled_writer) = CdpSocketSink::with_stalled_writer_for_test(2);
        router
            .register_browser_frontend(1, "hidden-dead".into(), CdpSocketSink::for_test())
            .unwrap();
        router
            .register_browser_frontend(2, "hidden-stalled".into(), stalled_sink)
            .unwrap();
        for frontend_id in [1, 2] {
            let parsed = ParsedCdpCommand::parse_value(json!({
                "id": 42, "method": "Browser.close",
            }));
            let Some(CdpPreparedFrontendCommand::Command(command)) =
                router.prepare_command(frontend_id, parsed)
            else {
                panic!("registered frontend must route normally");
            };
            assert!(handle_instance_command(&command, &router, &control).await);
        }
        assert!(
            !stalled_writer.is_open(),
            "failed flush closes its frontend"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), control.shutdown_requested())
                .await
                .is_err()
        );
    }
}
