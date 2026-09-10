use tokio::sync::mpsc;

use super::{WorkerGlobalKind, WorkerToParentMessage};
use crate::runtime::{
    RendererDedicatedWorkerHost, RendererDedicatedWorkerObservation, RendererProtocolObservation,
    RendererSharedWorkerConsoleMessage,
};

/// JS messages keep their parent channel. Dedicated diagnostics and Inspector
/// replies belong to the execution's own ordered stream, including nested
/// Workers whose parent may already have gone away.
#[derive(Clone, Debug)]
pub(crate) enum WorkerParentSender {
    Dedicated {
        host: RendererDedicatedWorkerHost,
        sender: mpsc::UnboundedSender<WorkerToParentMessage>,
    },
    Channel(mpsc::UnboundedSender<WorkerToParentMessage>),
}

impl WorkerParentSender {
    pub(super) fn new(
        sender: mpsc::UnboundedSender<WorkerToParentMessage>,
        kind: &WorkerGlobalKind,
    ) -> Self {
        match kind {
            WorkerGlobalKind::Dedicated(host) => Self::Dedicated {
                host: host.clone(),
                sender,
            },
            _ => Self::Channel(sender),
        }
    }

    pub(crate) fn send(
        &self,
        message: WorkerToParentMessage,
    ) -> Result<(), Box<mpsc::error::SendError<WorkerToParentMessage>>> {
        let Self::Dedicated { host, sender } = self else {
            let Self::Channel(sender) = self else {
                unreachable!()
            };
            return sender.send(message).map_err(Box::new);
        };
        match message {
            WorkerToParentMessage::FetchInterception(pause) => {
                host.publish(RendererProtocolObservation::Network(pause.report(None)))
            }
            WorkerToParentMessage::Network(observation) => {
                host.publish(RendererProtocolObservation::Network(observation))
            }
            WorkerToParentMessage::Console(console) => {
                host.publish(RendererProtocolObservation::DedicatedWorker(
                    RendererDedicatedWorkerObservation::Console {
                        instance_id: host.instance_id(),
                        message: RendererSharedWorkerConsoleMessage {
                            message: console.message,
                            args: console.args,
                            stack: console.stack,
                        },
                    },
                ))
            }
            WorkerToParentMessage::RuntimeInspectorMessages(batches) => {
                for batch in batches {
                    host.publish(RendererProtocolObservation::DedicatedWorker(
                        RendererDedicatedWorkerObservation::RuntimeInspectorMessages {
                            instance_id: host.instance_id(),
                            inspector_session_id: batch.inspector_session_id,
                            messages: batch.messages,
                        },
                    ));
                }
            }
            WorkerToParentMessage::RuntimeInspectorResponse(response) => {
                host.publish_response(response)
            }
            message => return sender.send(message).map_err(Box::new),
        }
        Ok(())
    }
}

#[cfg(test)]
impl From<mpsc::UnboundedSender<WorkerToParentMessage>> for WorkerParentSender {
    fn from(sender: mpsc::UnboundedSender<WorkerToParentMessage>) -> Self {
        Self::Channel(sender)
    }
}
