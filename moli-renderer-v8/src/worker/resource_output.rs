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
            E::Request { pause, .. } => {
                pause.resume_unobserved();
            }
            E::Continue {
                event,
                pause: Some(pause),
            } => match *event {
                crate::types::PendingSubresourceContinueEvent::ResponsePaused(_)
                | crate::types::PendingSubresourceContinueEvent::AuthRequired(_) => {
                    pause.resume_unobserved();
                }
                crate::types::PendingSubresourceContinueEvent::Completed { .. } => {}
            },
            E::Continue { .. } => {}
        }
        true
    })
}
