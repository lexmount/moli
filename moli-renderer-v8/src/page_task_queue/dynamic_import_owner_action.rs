use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    frame_owner_model::{
        FrameDocumentDynamicImportTerminalOutcome,
        FrameDocumentDynamicImportTerminalPreparedAction, FrameDocumentTaskOwner, FrameRealmId,
    },
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::RendererOwnerWakeSender;

/// Exact owner of one child dynamic-import owner action.
///
/// The stable source is Page-owned, while executable authority remains bound
/// to the root Document namespace and the child Document's exact V8 realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageDynamicImportOwnerActionOwner {
    root_document: RendererDocumentToken,
    task_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
}

impl RendererPageDynamicImportOwnerActionOwner {
    pub(crate) fn new(
        root_document: RendererDocumentToken,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> Self {
        Self {
            root_document,
            task_owner,
            realm_id,
        }
    }

    pub(crate) fn task_owner(self) -> FrameDocumentTaskOwner {
        self.task_owner
    }

    pub(crate) fn realm_id(self) -> FrameRealmId {
        self.realm_id
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageDynamicImportOwnerActionTask {
    root_document: RendererDocumentToken,
    action: FrameDocumentDynamicImportTerminalPreparedAction,
}

impl RendererPageDynamicImportOwnerActionTask {
    fn new(
        root_document: RendererDocumentToken,
        action: FrameDocumentDynamicImportTerminalPreparedAction,
    ) -> Self {
        Self {
            root_document,
            action,
        }
    }

    pub(crate) fn owner(&self) -> RendererPageDynamicImportOwnerActionOwner {
        RendererPageDynamicImportOwnerActionOwner::new(
            self.root_document,
            self.action.task_owner(),
            self.action.realm_id(),
        )
    }

    pub(crate) fn into_action(self) -> FrameDocumentDynamicImportTerminalPreparedAction {
        self.action
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageDynamicImportOwnerActionRouteClosed;

/// Cloneable producer capability for the stable Page-owned source.
///
/// This type deliberately exposes no dequeue or clear operation. The unique
/// `RendererPageDynamicImportOwnerActionSource` remains in the owner-local
/// Page reservation/slot for its entire lifetime.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageDynamicImportOwnerActionRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageDynamicImportOwnerActionTask>,
        RendererPageDynamicImportOwnerActionReadySignal,
    >,
}

impl RendererPageDynamicImportOwnerActionRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageDynamicImportOwnerActionSender {
        RendererPageDynamicImportOwnerActionSender::new(self.task_route.clone(), root_document)
    }

    fn same_route_as(&self, source: &RendererPageDynamicImportOwnerActionSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// Document-stamped producer route into the stable Page source.
///
/// A fanout is atomically enqueued as individual ready tasks and publishes at
/// most one readiness wake. A closed owner-attached route never mirrors work
/// into another execution path.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageDynamicImportOwnerActionSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageDynamicImportOwnerActionTask>,
        RendererPageDynamicImportOwnerActionReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageDynamicImportOwnerActionSender {
    fn new(
        task_route: OwnerReadyTaskRoute<
            ReadyPageTask<RendererPageDynamicImportOwnerActionTask>,
            RendererPageDynamicImportOwnerActionReadySignal,
        >,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            task_route,
            root_document,
        }
    }

    pub(crate) fn send_all(
        &self,
        actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> Result<bool, RendererPageDynamicImportOwnerActionRouteClosed> {
        self.task_route
            .send_all_and_signal_if_newly_ready(actions.into_iter().map(|action| {
                ReadyPageTask::new(RendererPageDynamicImportOwnerActionTask::new(
                    self.root_document,
                    action,
                ))
            }))
            .map(|enqueued| enqueued != 0)
            .map_err(|_| RendererPageDynamicImportOwnerActionRouteClosed)
    }

    #[cfg(test)]
    pub(crate) fn same_route_as(&self, other: &Self) -> bool {
        self.task_route.same_route_as(&other.task_route)
    }
}

#[derive(Clone, Debug)]
struct RendererPageDynamicImportOwnerActionReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageDynamicImportOwnerActionReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_dynamic_import_owner_action();
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageDynamicImportOwnerActionSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageDynamicImportOwnerActionTask>,
        RendererPageDynamicImportOwnerActionReadySignal,
    >,
}

impl RendererPageDynamicImportOwnerActionSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageDynamicImportOwnerActionReadySignal {
                owner_wake,
            }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageDynamicImportOwnerActionRoute {
        RendererPageDynamicImportOwnerActionRoute {
            task_route: self.source.route(),
        }
    }

    #[cfg(test)]
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageDynamicImportOwnerActionSender {
        self.route().sender(root_document)
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageDynamicImportOwnerActionTask,
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
    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageDynamicImportOwnerActionOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    #[cfg(test)]
    pub(crate) fn enqueue_local_for_test(
        &mut self,
        root_document: RendererDocumentToken,
        action: FrameDocumentDynamicImportTerminalPreparedAction,
    ) {
        self.source.enqueue_local(ReadyPageTask::new(
            RendererPageDynamicImportOwnerActionTask::new(root_document, action),
        ));
    }

    pub(crate) fn route_matches(&self, route: &RendererPageDynamicImportOwnerActionRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageDynamicImportOwnerActionDocumentEffect {
    AppliedToCurrentOwner {
        outcome: FrameDocumentDynamicImportTerminalOutcome,
    },
    DiscardedStaleOwner {
        current_owner: Option<RendererPageDynamicImportOwnerActionOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageDynamicImportOwnerActionTurnAction {
    pub(crate) owner: RendererPageDynamicImportOwnerActionOwner,
    pub(crate) document_effect: PageDynamicImportOwnerActionDocumentEffect,
}

pub(crate) type PageDynamicImportOwnerActionTurnOutcome =
    PageOwnerTurnOutcome<PageDynamicImportOwnerActionTurnAction>;

#[cfg(test)]
mod tests {
    use moli_module_script_tree as module_tree;
    use url::Url;

    use crate::{
        PageId,
        frame_owner_model::{
            DocumentId, FrameDocumentDynamicImportTerminalWork, FrameDocumentTaskOwner,
            FrameRealmId, FrameSchedulerLaneId, LocalWindowId,
        },
        module_runtime::{ModuleMapKey, NativeDynamicImportSingleModuleClient},
        runtime::{RendererDocumentToken, RendererPageToken},
    };

    use super::{
        RendererPageDynamicImportOwnerActionOwner, RendererPageDynamicImportOwnerActionSource,
    };
    use crate::page_task_queue::{
        PageTaskQueue, RendererOwnerWakeSender, RendererOwnerWakeSource,
        RendererPageTaskTestResidence,
    };

    fn document_token(page_id: PageId, lifecycle_document_id: u64) -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(page_id, lifecycle_document_id)
    }

    fn prepared_action(
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        sequence: u64,
    ) -> crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction {
        let key = ModuleMapKey::java_script(
            Url::parse(&format!("https://dynamic-owner-source.test/{sequence}.mjs"))
                .expect("dynamic-import test URL"),
        );
        let client = NativeDynamicImportSingleModuleClient::new(
            module_tree::SingleModuleClientToken {
                tree_id: module_tree::ModuleTreeId(sequence),
                sequence,
            },
            module_tree::ModuleImportPhase::Evaluation,
        );
        crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction::from_terminal_work(
            FrameDocumentDynamicImportTerminalWork::from_terminal_parts(
                task_owner,
                realm_id,
                key,
                client,
            ),
        )
    }

    fn task_owner(seed: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(seed),
            LocalWindowId(seed + 1),
            DocumentId(seed + 2),
        )
    }

    #[test]
    fn batch_sender_expands_actions_in_fifo_order_and_publishes_one_wake() {
        let page_id = PageId::new_for_testing(61);
        let root_document = document_token(page_id, 7);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut source = RendererPageDynamicImportOwnerActionSource::new(
            RendererOwnerWakeSender::new(wake_tx, RendererPageToken::new_for_testing(page_id)),
        );
        let sender = source.sender(root_document);
        let first_owner = task_owner(11);
        let second_owner = task_owner(17);

        assert_eq!(
            sender.send_all(vec![
                prepared_action(first_owner, FrameRealmId(13), 1),
                prepared_action(second_owner, FrameRealmId(19), 2),
            ]),
            Ok(true)
        );
        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageDynamicImportOwnerActionOwner::new(
                root_document,
                first_owner,
                FrameRealmId(13),
            ))
        );
        source
            .pop_front()
            .expect("first action should remain queued");
        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageDynamicImportOwnerActionOwner::new(
                root_document,
                second_owner,
                FrameRealmId(19),
            ))
        );
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("accepted fanout should publish one wake")
                .source_for_test(),
            RendererOwnerWakeSource::DynamicImportOwnerAction
        );
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn empty_batch_does_not_create_work_or_publish_wake() {
        let page_id = PageId::new_for_testing(67);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut source = RendererPageDynamicImportOwnerActionSource::new(
            RendererOwnerWakeSender::new(wake_tx, RendererPageToken::new_for_testing(page_id)),
        );
        let sender = source.sender(document_token(page_id, 1));

        assert_eq!(sender.send_all(Vec::new()), Ok(false));
        assert!(!source.has_ready_task());
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn replacement_sender_shares_route_but_stamps_new_root_document() {
        let page_id = PageId::new_for_testing(71);
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let wake =
            RendererOwnerWakeSender::new(wake_tx, RendererPageToken::new_for_testing(page_id));
        let mut source = RendererPageDynamicImportOwnerActionSource::new(wake);
        let retired_document = document_token(page_id, 3);
        let current_document = document_token(page_id, 4);
        let retired_sender = source.sender(retired_document);
        let current_sender = source.sender(current_document);
        assert!(retired_sender.same_route_as(&current_sender));
        let reused_local_owner = task_owner(23);

        retired_sender
            .send_all(vec![prepared_action(
                reused_local_owner,
                FrameRealmId(29),
                3,
            )])
            .expect("retired sender should remain connected to the Page source");
        current_sender
            .send_all(vec![prepared_action(
                reused_local_owner,
                FrameRealmId(29),
                4,
            )])
            .expect("replacement sender should remain connected to the Page source");

        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageDynamicImportOwnerActionOwner::new(
                retired_document,
                reused_local_owner,
                FrameRealmId(29),
            ))
        );
        source
            .pop_front()
            .expect("retired action should remain queued");
        assert_eq!(
            source.next_ready_owner(),
            Some(RendererPageDynamicImportOwnerActionOwner::new(
                current_document,
                reused_local_owner,
                FrameRealmId(29),
            ))
        );
    }

    #[test]
    fn closed_page_route_rejects_batch_without_phantom_wake() {
        let page_id = PageId::new_for_testing(73);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source = RendererPageDynamicImportOwnerActionSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(page_id),
        ));
        let sender = source.sender(document_token(page_id, 1));
        drop(source);

        assert!(
            sender
                .send_all(vec![prepared_action(task_owner(31), FrameRealmId(37), 5,)])
                .is_err()
        );
        assert!(wake_rx.try_recv().is_err());
    }

    #[test]
    fn document_queue_clear_preserves_page_owned_action_for_replacement() {
        let page_id = PageId::new_for_testing(79);
        let root_document = document_token(page_id, 5);
        let residence = RendererPageTaskTestResidence::new(None);
        let page_source = residence.runtime_source();
        let action_source = residence.task_sources().dynamic_import_owner_action();
        let mut retired_document_queue =
            PageTaskQueue::new_with_page_runtime_task_source(page_source.clone());
        let owner = task_owner(41);
        action_source
            .enqueue_local_for_test(root_document, prepared_action(owner, FrameRealmId(43), 6));

        retired_document_queue.clear_document_owned_tasks();
        let _replacement_queue = PageTaskQueue::new_with_page_runtime_task_source(page_source);
        assert_eq!(
            action_source.next_ready_owner(),
            Some(RendererPageDynamicImportOwnerActionOwner::new(
                root_document,
                owner,
                FrameRealmId(43),
            ))
        );
    }

    #[test]
    fn only_stable_page_source_retirement_clears_dynamic_import_actions() {
        let page_id = PageId::new_for_testing(83);
        let root_document = document_token(page_id, 9);
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let residence = RendererPageTaskTestResidence::new(Some(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(page_id),
        )));
        let page_source = residence.runtime_source();
        let source_harness = residence.task_sources();
        page_source
            .dynamic_import_owner_action_sender(root_document)
            .expect("owner-attached Page source should expose a document-stamped sender")
            .send_all(vec![prepared_action(task_owner(47), FrameRealmId(53), 7)])
            .expect("live Page source should accept its dynamic-import action");
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("accepted action should publish its owner wake")
                .source_for_test(),
            RendererOwnerWakeSource::DynamicImportOwnerAction
        );

        // PageVm/runtime cleanup owns no consumer authority and therefore
        // must not discard a stable Page task.
        page_source.clear();
        assert!(
            source_harness
                .dynamic_import_owner_action()
                .has_ready_task()
        );

        source_harness.clear();
        assert!(
            !source_harness
                .dynamic_import_owner_action()
                .has_ready_task()
        );
    }
}
