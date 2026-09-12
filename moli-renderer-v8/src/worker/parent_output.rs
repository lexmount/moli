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

/// A request can observe the original FIFO without keeping its Worker alive.
#[derive(Clone, Debug)]
pub(crate) enum WorkerNetworkObserver {
    Dedicated(crate::runtime::RendererDedicatedWorkerNetworkObserver),
    Channel(mpsc::WeakUnboundedSender<WorkerToParentMessage>),
}

impl WorkerNetworkObserver {
    pub(crate) fn publish(&self, observation: crate::runtime::RendererNetworkObservation) {
        match self {
            Self::Dedicated(observer) => observer.publish(observation),
            Self::Channel(sender) => {
                if let Some(sender) = sender.upgrade() {
                    let _ = sender.send(WorkerToParentMessage::Network(observation));
                }
            }
        }
    }
}

impl WorkerParentSender {
    pub(crate) fn network_observer(&self) -> WorkerNetworkObserver {
        match self {
            Self::Dedicated { host, .. } => {
                WorkerNetworkObserver::Dedicated(host.network_observer())
            }
            Self::Channel(sender) => WorkerNetworkObserver::Channel(sender.downgrade()),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_tail_does_not_keep_worker_parent_message_pump_open() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let parent = WorkerParentSender::Channel(sender);
        let observer = parent.network_observer();
        let source = crate::runtime::RendererWorkerNetworkReporter::unobserved_for_test();
        let request = source.start_request().unwrap();
        drop(parent);
        assert!(
            receiver.is_closed(),
            "only VM-owned senders may keep the parent pump alive"
        );
        observer.publish(request.report(
            moli_page_types::ScriptNetworkOutputItem::SubresourceBodyFinished(std::sync::Arc::new(
                moli_page_types::SubresourceBodyFinished::failed(
                    request.handle(),
                    "detached transfer".into(),
                ),
            )),
        ));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }
}
