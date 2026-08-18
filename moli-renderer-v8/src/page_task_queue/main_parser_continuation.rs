use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    frame_owner_model::FrameDocumentTaskOwner,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{
    RendererPageMainDocumentTaskOwner, RendererPageNetworkingRoute, RendererPageNetworkingTask,
};

pub(crate) type RendererPageMainParserContinuationOwner = RendererPageMainDocumentTaskOwner;

/// Whether a parser-resume request created a new Networking task or merged
/// into the task already resident for the same exact parser owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainParserContinuationRequest {
    Enqueued,
    AlreadyQueued,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMainParserContinuationRouteClosed;

/// Page-scoped route that can be bound to one exact main parser Document.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageMainParserContinuationSender {
    networking: RendererPageNetworkingRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageMainParserContinuationSender {
    pub(crate) fn new(
        networking: RendererPageNetworkingRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            networking,
            root_document,
        }
    }

    pub(crate) fn bind_producer(
        &self,
        document_owner: FrameDocumentTaskOwner,
    ) -> RendererPageMainParserContinuationProducer {
        RendererPageMainParserContinuationProducer {
            networking: self.networking.clone(),
            owner: RendererPageMainDocumentTaskOwner::new(self.root_document, document_owner),
            admission: Arc::new(MainParserContinuationAdmission::default()),
        }
    }
}

/// Cloneable producer for one exact main parser.
///
/// Parser input, stylesheet state, and script fetch results remain in their
/// authoritative stores. This producer only coalesces the notification that
/// those stores are worth observing again.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageMainParserContinuationProducer {
    networking: RendererPageNetworkingRoute,
    owner: RendererPageMainParserContinuationOwner,
    admission: Arc<MainParserContinuationAdmission>,
}

impl RendererPageMainParserContinuationProducer {
    pub(crate) fn request(
        &self,
    ) -> Result<MainParserContinuationRequest, RendererPageMainParserContinuationRouteClosed> {
        if self
            .admission
            .queued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(MainParserContinuationRequest::AlreadyQueued);
        }

        let task = RendererPageMainParserContinuationTask {
            owner: self.owner,
            permit: MainParserContinuationPermit::new(self.admission.clone()),
        };
        self.networking
            .send(RendererPageNetworkingTask::MainParserContinuation(task))
            .map(|()| MainParserContinuationRequest::Enqueued)
            .map_err(|_| RendererPageMainParserContinuationRouteClosed)
    }

    pub(crate) const fn owner(&self) -> RendererPageMainParserContinuationOwner {
        self.owner
    }
}

#[derive(Debug, Default)]
struct MainParserContinuationAdmission {
    queued: AtomicBool,
}

/// One resident Networking task for a main parser.
///
/// The task contains only an exact owner locator and a coalescing permit. It
/// never owns parser input, parser state, `PageVm`, or a phase-one runtime.
#[derive(Debug)]
pub(crate) struct RendererPageMainParserContinuationTask {
    owner: RendererPageMainParserContinuationOwner,
    permit: MainParserContinuationPermit,
}

impl RendererPageMainParserContinuationTask {
    pub(crate) const fn owner(&self) -> RendererPageMainParserContinuationOwner {
        self.owner
    }

    /// Release the resident-task permit at dequeue, before executing the task.
    ///
    /// A producer that observes new input while this task is executing must be
    /// able to queue the next bounded parser opportunity. Releasing only after
    /// execution would merge that request into a task that can no longer
    /// observe the newly arrived fact.
    pub(super) fn release_permit_at_dequeue(&mut self) {
        self.permit.release();
    }

    pub(crate) fn into_owner(self) -> RendererPageMainParserContinuationOwner {
        debug_assert!(
            !self.permit.armed,
            "main parser continuation permit must be released by Networking dequeue"
        );
        self.owner
    }
}

#[derive(Debug)]
struct MainParserContinuationPermit {
    admission: Arc<MainParserContinuationAdmission>,
    armed: bool,
}

impl MainParserContinuationPermit {
    fn new(admission: Arc<MainParserContinuationAdmission>) -> Self {
        Self {
            admission,
            armed: true,
        }
    }

    fn release(&mut self) {
        if self.armed {
            self.admission.queued.store(false, Ordering::Release);
            self.armed = false;
        }
    }
}

impl Drop for MainParserContinuationPermit {
    fn drop(&mut self) {
        // Queue teardown or stale-task disposal must not permanently leave the
        // producer in the queued state.
        self.release();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageMainParserContinuationTargetEffect {
    AdmittedCurrentParser,
    DiscardedStaleOrInactiveParser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageMainParserContinuationTurnAction {
    pub(crate) owner: RendererPageMainParserContinuationOwner,
    pub(crate) target_effect: PageMainParserContinuationTargetEffect,
}

pub(crate) type PageMainParserContinuationTurnOutcome =
    PageOwnerTurnOutcome<PageMainParserContinuationTurnAction>;

#[cfg(test)]
mod tests {
    use crate::{
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        page_task_queue::{
            PageRuntimeWakeSignal, RendererOwnerWakeSender, RendererPageNetworkingSource,
            RendererPageNetworkingTask,
        },
        runtime::{RendererDocumentToken, RendererPageToken},
    };

    use super::*;

    fn test_document_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
    }

    fn fixture() -> (
        RendererPageNetworkingSource,
        RendererPageMainParserContinuationProducer,
    ) {
        let root_document =
            RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(11), 7);
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageNetworkingSource::new_owner_attached(
            PageRuntimeWakeSignal::default(),
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(root_document.page_id),
            ),
        );
        let producer = RendererPageMainParserContinuationSender::new(source.route(), root_document)
            .bind_producer(test_document_owner());
        (source, producer)
    }

    #[test]
    fn requests_coalesce_until_the_resident_task_is_dequeued() {
        let (mut source, producer) = fixture();

        assert_eq!(
            producer.request().expect("first request"),
            MainParserContinuationRequest::Enqueued
        );
        assert_eq!(
            producer.request().expect("coalesced request"),
            MainParserContinuationRequest::AlreadyQueued
        );

        let (_, task) = source.pop_front_task().expect("one continuation task");
        let RendererPageNetworkingTask::MainParserContinuation(task) = task else {
            panic!("continuation producer must use the Networking continuation variant");
        };
        // The permit is released by dequeue, before the selected task enters
        // its executor. A producer fact in that window must create another
        // resident task instead of being absorbed by the dequeued one.
        assert_eq!(
            producer
                .request()
                .expect("request before dequeued task executes"),
            MainParserContinuationRequest::Enqueued
        );
        assert!(source.has_ready_task());
        assert_eq!(task.into_owner(), producer.owner());
    }

    #[test]
    fn dropping_a_resident_task_releases_the_coalescing_permit() {
        let (mut source, producer) = fixture();
        assert_eq!(
            producer.request().expect("first request"),
            MainParserContinuationRequest::Enqueued
        );

        source.clear();

        assert_eq!(
            producer.request().expect("request after source clear"),
            MainParserContinuationRequest::Enqueued
        );
    }
}
