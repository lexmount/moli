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
    target: Option<(
        crate::runtime::RendererWorkerTargetIdentity,
        crate::runtime::RendererTurnOutputJournal,
    )>,
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
            target: None,
            closed: AtomicBool::new(false),
            closed_notify: tokio::sync::Notify::new(),
        }))
    }

    pub(crate) fn with_worker_target(
        mut self,
        identity: crate::runtime::RendererWorkerTargetIdentity,
        journal: crate::runtime::RendererTurnOutputJournal,
    ) -> Self {
        let Self::Worker(output) = &mut self else {
            unreachable!("Worker target requires a Worker observer")
        };
        Arc::get_mut(output)
            .expect("target is bound before observer publication")
            .target = Some((identity, journal));
        self
    }

    pub(crate) fn worker_target(
        &self,
    ) -> Option<(
        crate::runtime::RendererWorkerTargetIdentity,
        crate::runtime::RendererTurnOutputJournal,
    )> {
        match self {
            Self::Worker(output) => output.target.clone(),
            Self::Document { .. } => None,
        }
    }

    pub(crate) fn fork_worker_observer(&self) -> Self {
        match self {
            Self::Document { .. } => self.clone(),
            Self::Worker(output) => Self::Worker(Arc::new(WorkerInterceptionOutput {
                control: output.control.clone(),
                publish: output.publish.clone(),
                target: output.target.clone(),
                closed: AtomicBool::new(false),
                closed_notify: tokio::sync::Notify::new(),
            })),
        }
    }
    fn control_root(&self) -> &Self {
        let mut control = self;
        while let Self::Worker(output) = control {
            match &output.control {
                Some(parent) => control = parent,
                None => break,
            }
        }
        control
    }

    pub(crate) fn worker_control_closed(&self) -> bool {
        match self.control_root() {
            Self::Document { output, .. } => output.is_closed(),
            Self::Worker(output) => output.closed.load(Ordering::Acquire),
        }
    }

    pub(crate) async fn worker_control_shutdown(&self) {
        match self.control_root() {
            Self::Document { output, .. } => output.shutdown().await,
            Self::Worker(output) => loop {
                let notified = output.closed_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if output.closed.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            },
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
        let control = self.control_root();
        match control {
            Self::Document { output, .. } => output.publish(control.request_action(info, pause)),
            Self::Worker(output) => {
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
        match self.control_root() {
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

#[cfg(test)]
mod control_root_tests {
    use super::*;
    use crate::types::{SubresourceDataReceived, SubresourceNetworkRequestHandle};
    use std::future::Future as _;
    use std::task::{Context, Waker};

    #[tokio::test]
    async fn nested_control_delegates_without_redirecting_its_network_observer() {
        let controls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let networks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = controls.clone();
        let root = ResourceInterceptionContext::worker(None, move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            true
        });
        let parent = ResourceInterceptionContext::worker(Some(root.clone()), |_| {
            panic!("control must reach its root")
        });
        let count = networks.clone();
        let child = ResourceInterceptionContext::worker(Some(parent), move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            true
        });
        assert!(child.publish_continue(
            PendingSubresourceContinueEvent::Completed { internal_id: 1 },
            None
        ));
        assert!(
            child.publish_network(ScriptNetworkOutputItem::SubresourceDataReceived(
                SubresourceDataReceived::new(SubresourceNetworkRequestHandle::new(1), 3, 3)
            ))
        );
        assert_eq!(controls.load(Ordering::SeqCst), 1);
        assert_eq!(networks.load(Ordering::SeqCst), 1);
        child.close_worker_observer();
        assert!(
            !child.worker_control_closed(),
            "observer retirement does not retire its inherited control"
        );
        let shutdown = child.worker_control_shutdown();
        tokio::pin!(shutdown);
        assert!(
            shutdown
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        root.close_worker_observer();
        shutdown.await;
        assert!(child.worker_control_closed());
    }
}
