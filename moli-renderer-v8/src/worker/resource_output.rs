use super::WorkerToParentMessage;
use crate::{
    network::{ResourceInterceptionEvent, interception_output::ResourceInterceptionContext},
    types::{
        ScriptExecutionReport, ScriptNetworkOutput, ScriptNetworkOutputItem,
        SubresourceNetworkRecord, SubresourceResourceType,
    },
};

pub(crate) fn record_output(
    report: &mut ScriptExecutionReport,
    item: ScriptNetworkOutputItem,
) -> Vec<SubresourceNetworkRecord> {
    // This adapter only forwards request records. There is no consumer for
    // WebSocket frames or lifecycle events in its diagnostic report.
    if matches!(
        item,
        ScriptNetworkOutputItem::WebSocketNetworkEvent(_)
            | ScriptNetworkOutputItem::WebSocketLifecycleEvent(_)
    ) {
        return Vec::new();
    }
    let mut output = ScriptNetworkOutput::default();
    output.push_item(item);
    report.extend_network_output(output);
    report.take_subresource_network_records()
}

pub(crate) fn unobserved(
    parent: tokio::sync::mpsc::UnboundedSender<WorkerToParentMessage>,
) -> ResourceInterceptionContext {
    let report = parking_lot::Mutex::new(ScriptExecutionReport::default());
    ResourceInterceptionContext::worker(None, move |event| {
        use ResourceInterceptionEvent as E;
        match event {
            E::Network(item) => {
                for record in record_output(&mut report.lock(), *item) {
                    if record.resource_type() != SubresourceResourceType::WebSocket {
                        let _ = parent.send(WorkerToParentMessage::SubresourceNetwork(record));
                    }
                }
            }
            E::Request { info, pause } => {
                let _ = pause.dispatch(
                    crate::RendererPageCommand::ContinuePendingSubresourceFetch {
                        internal_id: info.internal_id,
                        url: None,
                        method: None,
                        body: None,
                        headers: None,
                        intercept_response: false,
                        handle_auth_requests: false,
                    },
                );
            }
            E::Continue {
                event,
                pause: Some(pause),
            } => {
                let command = match *event {
                    crate::types::PendingSubresourceContinueEvent::ResponsePaused(info) => {
                        crate::RendererPageCommand::ContinuePendingSubresourceResponse {
                            internal_id: info.internal_id,
                            response_code: None,
                            response_headers: None,
                        }
                    }
                    crate::types::PendingSubresourceContinueEvent::AuthRequired(info) => {
                        crate::RendererPageCommand::CancelPendingSubresourceAuth {
                            internal_id: info.internal_id,
                        }
                    }
                    crate::types::PendingSubresourceContinueEvent::Completed { .. } => return true,
                };
                let _ = pause.dispatch(command);
            }
            E::Continue { .. } => {}
        }
        true
    })
}
