use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    network::ResourceInterceptionPause,
    runtime::RendererOwnerAction,
    types::{
        PendingSubresourceContinueEvent, PendingSubresourceFetchInfo, ScriptNetworkOutputItem,
        SubresourceNetworkRecord,
    },
};

use super::ResourceInterceptionContext;

/// Native loader facts. No V8 object, Worker-local request id or Page command
/// is needed to deliver an interceptor's decision to its waiting request.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceInterceptionEvent {
    Request {
        info: Box<PendingSubresourceFetchInfo>,
        pause: ResourceInterceptionPause,
    },
    Continue {
        event: Box<PendingSubresourceContinueEvent>,
        pause: Option<ResourceInterceptionPause>,
    },
    Network(Box<ScriptNetworkOutputItem>),
}

pub(crate) struct WorkerInterceptionOutput {
    control: Option<ResourceInterceptionContext>,
    publish: Arc<dyn Fn(ResourceInterceptionEvent) -> bool + Send + Sync>,
    closed: AtomicBool,
    closed_notify: tokio::sync::Notify,
}

impl std::fmt::Debug for WorkerInterceptionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerInterceptionOutput")
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl ResourceInterceptionContext {
    pub(crate) fn is_worker(&self) -> bool {
        matches!(self, Self::Worker(_))
    }
    pub(crate) fn worker(
        control: Option<Self>,
        publish: impl Fn(ResourceInterceptionEvent) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self::Worker(Arc::new(WorkerInterceptionOutput {
            control,
            publish: Arc::new(publish),
            closed: AtomicBool::new(false),
            closed_notify: tokio::sync::Notify::new(),
        }))
    }

    pub(crate) fn fork_worker_observer(&self) -> Self {
        match self {
            Self::Document { .. } => self.clone(),
            Self::Worker(output) => Self::Worker(Arc::new(WorkerInterceptionOutput {
                control: output.control.clone(),
                publish: output.publish.clone(),
                closed: AtomicBool::new(false),
                closed_notify: tokio::sync::Notify::new(),
            })),
        }
    }
    pub(crate) fn worker_control_closed(&self) -> bool {
        let mut control = self;
        loop {
            match control {
                Self::Document { output, .. } => return output.is_closed(),
                Self::Worker(output) => match &output.control {
                    Some(parent) => control = parent,
                    None => return output.closed.load(Ordering::Acquire),
                },
            }
        }
    }
    pub(crate) async fn worker_control_shutdown(&self) {
        let mut control = self;
        loop {
            match control {
                Self::Document { output, .. } => return output.shutdown().await,
                Self::Worker(output) => {
                    if let Some(parent) = &output.control {
                        control = parent;
                        continue;
                    }
                    loop {
                        let notified = output.closed_notify.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        if output.closed.load(Ordering::Acquire) {
                            return;
                        }
                        notified.await;
                    }
                }
            }
        }
    }

    pub(crate) fn close_worker_observer(&self) {
        if let Self::Worker(output) = self {
            output.closed.store(true, Ordering::Release);
            output.closed_notify.notify_waiters();
        }
    }

    pub(crate) fn request_action(
        &self,
        info: PendingSubresourceFetchInfo,
        pause: ResourceInterceptionPause,
    ) -> RendererOwnerAction {
        let Self::Document {
            source_document, ..
        } = self
        else {
            panic!("only a Document can prepare a Page-turn request admission");
        };
        RendererOwnerAction::SubresourceFetchPause {
            source_document: *source_document,
            info: Box::new(info),
            network_pause: Some(pause),
            child_document: None,
            redirect: None,
        }
    }

    pub(crate) fn publish_request(
        &self,
        info: PendingSubresourceFetchInfo,
        pause: ResourceInterceptionPause,
    ) -> bool {
        match self {
            Self::Document { output, .. } => output.publish(self.request_action(info, pause)),
            Self::Worker(output) => {
                if let Some(control) = &output.control {
                    return control.publish_request(info, pause);
                }
                if output.closed.load(Ordering::Acquire) {
                    return false;
                }
                (output.publish)(ResourceInterceptionEvent::Request {
                    info: Box::new(info),
                    pause,
                })
            }
        }
    }

    pub(crate) fn publish_continue(
        &self,
        event: PendingSubresourceContinueEvent,
        pause: Option<ResourceInterceptionPause>,
    ) -> bool {
        match self {
            Self::Document {
                source_document,
                output,
            } => output.publish(RendererOwnerAction::SubresourceContinue {
                source_document: *source_document,
                response_body: None,
                event: Box::new(event),
                network_pause: pause,
            }),
            Self::Worker(output) => {
                if let Some(control) = &output.control {
                    return control.publish_continue(event, pause);
                }
                if output.closed.load(Ordering::Acquire) {
                    return false;
                }
                (output.publish)(ResourceInterceptionEvent::Continue {
                    event: Box::new(event),
                    pause,
                })
            }
        }
    }

    pub(crate) fn publish_network(&self, item: ScriptNetworkOutputItem) -> bool {
        match self {
            Self::Document {
                source_document,
                output,
            } => output.publish_network(*source_document, item),
            Self::Worker(output) => {
                !output.closed.load(Ordering::Acquire)
                    && (output.publish)(ResourceInterceptionEvent::Network(Box::new(item)))
            }
        }
    }

    pub(crate) fn record_subresource_network(&self, record: SubresourceNetworkRecord) -> bool {
        self.publish_network(ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
            record,
        )))
    }
}

/// The interception configuration of a Worker target, independent of clients.
#[derive(Clone, Debug)]
pub struct ResourceInterceptionPolicy(pub(crate) crate::network::PageNetworkPolicy);
impl ResourceInterceptionPolicy {
    pub fn set_interception(&self, classes: crate::types::SubresourceInterceptionClasses) {
        self.0.set_subresource_interception(classes);
    }
}
impl PartialEq for ResourceInterceptionPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.0.shares_state_with(&other.0)
    }
}
impl Eq for ResourceInterceptionPolicy {}
