use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    frame_owner_model::{
        ChildDocumentModuleFetchTarget, FrameDocumentModulepreloadFetchTask,
        FrameDocumentModulepreloadStartOutcome,
    },
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::RendererOwnerWakeSender;

/// Exact owner of one modulepreload start task in the stable Page source.
///
/// The root token prevents PageVm-local owner/realm counter reuse after a
/// cross-Document replacement from authorizing an old start task. The child
/// target remains atomic so routing fields cannot be spliced together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageModulepreloadStartOwner {
    root_document: RendererDocumentToken,
    target: ChildDocumentModuleFetchTarget,
}

impl RendererPageModulepreloadStartOwner {
    pub(crate) fn new(
        root_document: RendererDocumentToken,
        target: ChildDocumentModuleFetchTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) fn target(self) -> ChildDocumentModuleFetchTarget {
        self.target
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageModulepreloadStartTask {
    root_document: RendererDocumentToken,
    task: FrameDocumentModulepreloadFetchTask,
}

impl RendererPageModulepreloadStartTask {
    fn new(
        root_document: RendererDocumentToken,
        task: FrameDocumentModulepreloadFetchTask,
    ) -> Self {
        Self {
            root_document,
            task,
        }
    }

    pub(crate) fn owner(&self) -> RendererPageModulepreloadStartOwner {
        RendererPageModulepreloadStartOwner::new(self.root_document, self.task.target())
    }

    pub(crate) fn into_task(self) -> FrameDocumentModulepreloadFetchTask {
        self.task
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageModulepreloadStartRouteClosed;

/// Cloneable producer capability for the stable Page-owned source.
///
/// Dequeue and retirement remain available only on the unique source stored
/// in the owner-local Page reservation/slot.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageModulepreloadStartRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageModulepreloadStartTask>,
        RendererPageModulepreloadStartReadySignal,
    >,
}

impl RendererPageModulepreloadStartRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageModulepreloadStartSender {
        RendererPageModulepreloadStartSender::new(self.task_route.clone(), root_document)
    }

    fn same_route_as(&self, source: &RendererPageModulepreloadStartSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// Document-stamped producer route into the stable Page source.
///
/// A replacement PageVm gets a new sender with the same Page queue route but a
/// new root token. A closed production route never falls back to another
/// PageVm-local executor. Route clones share one readiness epoch, so queued
/// starts wake the owner once and subsequent turns follow scheduler readiness.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageModulepreloadStartSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageModulepreloadStartTask>,
        RendererPageModulepreloadStartReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageModulepreloadStartSender {
    fn new(
        task_route: OwnerReadyTaskRoute<
            ReadyPageTask<RendererPageModulepreloadStartTask>,
            RendererPageModulepreloadStartReadySignal,
        >,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            task_route,
            root_document,
        }
    }

    pub(crate) fn send(
        &self,
        task: FrameDocumentModulepreloadFetchTask,
    ) -> Result<(), RendererPageModulepreloadStartRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageModulepreloadStartTask::new(self.root_document, task),
            ))
            .map_err(|_| RendererPageModulepreloadStartRouteClosed)
    }

    #[cfg(test)]
    pub(crate) fn same_route_as(&self, other: &Self) -> bool {
        self.task_route.same_route_as(&other.task_route)
    }
}

#[derive(Clone, Debug)]
struct RendererPageModulepreloadStartReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageModulepreloadStartReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_modulepreload_start();
    }
}

/// Unique owner-side source shared by all PageVm generations of one Page.
#[derive(Debug)]
pub(crate) struct RendererPageModulepreloadStartSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageModulepreloadStartTask>,
        RendererPageModulepreloadStartReadySignal,
    >,
}

impl RendererPageModulepreloadStartSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageModulepreloadStartReadySignal {
                owner_wake,
            }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageModulepreloadStartRoute {
        RendererPageModulepreloadStartRoute {
            task_route: self.source.route(),
        }
    }

    #[cfg(test)]
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageModulepreloadStartSender {
        self.route().sender(root_document)
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    #[cfg(test)]
    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageModulepreloadStartOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageModulepreloadStartTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    #[cfg(test)]
    pub(crate) fn enqueue_local_for_test(
        &mut self,
        root_document: RendererDocumentToken,
        task: FrameDocumentModulepreloadFetchTask,
    ) {
        self.source
            .enqueue_local(ReadyPageTask::new(RendererPageModulepreloadStartTask::new(
                root_document,
                task,
            )));
    }

    pub(crate) fn route_matches(&self, route: &RendererPageModulepreloadStartRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageModulepreloadStartDocumentEffect {
    AppliedToCurrentOwner {
        outcome: FrameDocumentModulepreloadStartOutcome,
    },
    DiscardedStaleOwner {
        current_owner: Option<RendererPageModulepreloadStartOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageModulepreloadStartTurnAction {
    pub(crate) owner: RendererPageModulepreloadStartOwner,
    pub(crate) document_effect: PageModulepreloadStartDocumentEffect,
}

pub(crate) type PageModulepreloadStartTurnOutcome =
    PageOwnerTurnOutcome<PageModulepreloadStartTurnAction>;

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::{
        PageId,
        document_runtime::DomHandle,
        frame_owner_model::{
            DocumentId, FrameDocumentModulepreloadFetchTask, FrameDocumentModulepreloadLinkClient,
            FrameDocumentTaskOwner, FrameRealmId, FrameSchedulerLaneId, LocalWindowId,
        },
        module_runtime::{ModuleFetchMetadata, ModuleMapKey, NativeModuleSingleFetchRequest},
        runtime::{RendererDocumentToken, RendererPageToken},
    };

    use super::{RendererPageModulepreloadStartOwner, RendererPageModulepreloadStartSource};
    use crate::page_task_queue::{
        PageTaskQueue, RendererOwnerWakeSender, RendererOwnerWakeSource,
        RendererPageTaskTestResidence,
    };

    fn document_token(page_id: PageId, lifecycle_document_id: u64) -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(page_id, lifecycle_document_id)
    }

    fn start_task(
        child_handle: usize,
        document_id: u64,
        realm_id: i64,
    ) -> FrameDocumentModulepreloadFetchTask {
        let child_handle_raw = child_handle;
        let child_handle = DomHandle::new(child_handle_raw);
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(document_id),
            LocalWindowId(document_id + 1),
            DocumentId(document_id + 2),
        );
        let source_url = Url::parse(&format!(
            "https://modulepreload-source.test/{document_id}.js"
        ))
        .expect("modulepreload source URL");
        FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
            owner,
            FrameRealmId(realm_id),
            FrameDocumentModulepreloadLinkClient::new(
                child_handle,
                DomHandle::new(child_handle_raw + 100),
            ),
            NativeModuleSingleFetchRequest::new(
                source_url.clone(),
                source_url.clone(),
                source_url.clone(),
                ModuleMapKey::java_script(source_url),
                ModuleFetchMetadata::default(),
            ),
        )
    }

    #[test]
    fn lifecycle_document_senders_share_one_route_but_stamp_distinct_root_documents() {
        let page_id = PageId::new_for_testing(41);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let wake =
            RendererOwnerWakeSender::new(wake_tx, RendererPageToken::new_for_testing(page_id));
        let mut source = RendererPageModulepreloadStartSource::new(wake);
        let first_document = document_token(page_id, 7);
        let replacement_document = document_token(page_id, 8);
        let first_sender = source.sender(first_document);
        let replacement_sender = source.sender(replacement_document);
        assert!(first_sender.same_route_as(&replacement_sender));

        let first_task = start_task(11, 13, 17);
        let first_target = first_task.target();
        first_sender
            .send(first_task)
            .expect("first Document route should remain open");
        let replacement_task = start_task(19, 23, 29);
        let replacement_target = replacement_task.target();
        replacement_sender
            .send(replacement_task)
            .expect("replacement Document route should remain open");

        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageModulepreloadStartOwner::new(
                first_document,
                first_target,
            ))
        );
        source.pop_front().expect("first task should remain queued");
        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageModulepreloadStartOwner::new(
                replacement_document,
                replacement_target,
            ))
        );
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("the first accepted start should publish the readiness wake")
                .source_for_test(),
            RendererOwnerWakeSource::ModulepreloadStart
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "the replacement enqueue must share the existing readiness epoch"
        );

        source
            .pop_front()
            .expect("replacement task should drain the source");
        let rearmed_task = start_task(31, 37, 41);
        let rearmed_target = rearmed_task.target();
        replacement_sender
            .send(rearmed_task)
            .expect("a drained Page source should accept a new readiness epoch");
        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageModulepreloadStartOwner::new(
                replacement_document,
                rearmed_target,
            ))
        );
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("the first enqueue after drain should publish a new wake")
                .source_for_test(),
            RendererOwnerWakeSource::ModulepreloadStart
        );
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn closed_page_route_rejects_start_without_publishing_phantom_wake() {
        let page_id = PageId::new_for_testing(43);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageModulepreloadStartSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(page_id),
        ));
        let sender = source.sender(document_token(page_id, 1));
        drop(source);

        assert!(sender.send(start_task(31, 37, 41)).is_err());
        assert!(
            wake_rx.try_recv().is_err(),
            "a rejected payload must not publish a Page wake"
        );
    }

    #[test]
    fn document_queue_clear_retains_page_owned_start_payload_for_replacement() {
        let page_id = PageId::new_for_testing(47);
        let root_document = document_token(page_id, 5);
        let residence = RendererPageTaskTestResidence::new(None);
        let page_source = residence.runtime_source();
        let start_source = residence.task_sources().modulepreload_start();
        let mut old_document_queue =
            PageTaskQueue::new_with_page_runtime_task_source(page_source.clone());
        let task = start_task(53, 59, 61);
        let target = task.target();
        start_source.enqueue_local_for_test(root_document, task);

        old_document_queue.clear_document_owned_tasks();
        let _replacement_queue = PageTaskQueue::new_with_page_runtime_task_source(page_source);
        assert_eq!(
            start_source.next_ready_owner(),
            Some(RendererPageModulepreloadStartOwner::new(
                root_document,
                target,
            )),
            "dropping one Document's queues must not clear the stable Page source"
        );
    }
}
