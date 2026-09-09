//! Exact selected-Page-task execution for semantic tests.
//!
//! Production has one selected-task dispatcher, but P5 migration originally
//! accumulated one test driver per task family. Each driver repeated the same
//! protocol:
//!
//! 1. scan stable source-head descriptors;
//! 2. dequeue one concrete scheduler task;
//! 3. unpack and immediately rebuild its enum variant;
//! 4. return through the production dispatcher.
//!
//! This module is the only test authority for that protocol. Tests select a
//! strongly typed family and receive an opaque claim. They may hold the claim
//! across `document.open()` or Page replacement to exercise stale
//! authorization, but cannot extract the scheduler payload or reproduce
//! completion policy themselves. Domain body tests deliberately use separate
//! body-only helpers and must not claim that they cover a complete HTML task.

use crate::page_task_queue::{
    PageMainDocumentRuntimeActionKind, RendererPageDomManipulationTask,
    RendererPageNavigationAndTraversalHead, RendererPageNavigationAndTraversalTask,
    RendererPageNetworkingOwner, RendererPageNetworkingTask, RendererPageReadyDescriptor,
    RendererPageSchedulerTask,
};

use super::{PageDomManipulationTestFamily, PageVm};

type SelectedPageTaskTestFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<T>> + 'a>>;

/// Exact scheduler family a semantic fixture is authorized to claim.
///
/// Shared scheduler sources retain their domain variant here. Selecting
/// `ChildNavigationCommit`, `ChildRealmMaterialization`,
/// `ChildParserModuleRootStart`, `ChildClassicScriptSourceLoad`,
/// `ChildModuleDependencyFetchStart`, `ChildModuleScriptTerminal`,
/// `HistoryTraversal`, `NavigationApi`, `StyleElementEvent`,
/// `MainParserContinuation`, `ModulepreloadStart`, `ResourceCompletion`,
/// `StylesheetCompletion`,
/// `WebSocket`, `WorkerHostBridge`, or one exact `MainDocumentRuntime` action
/// therefore cannot consume a neighboring head from the same FIFO.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageSelectedTaskTestSelector {
    ChildDocumentLifecycle,
    ChildDocumentScriptReady,
    ChildHostLoad,
    ChildClassicScriptSourceLoad,
    ChildModuleDependencyFetchStart,
    ChildModuleScriptTerminal,
    ChildParserModuleRootStart,
    ChildRealmMaterialization,
    ChildNavigationCommit,
    ChildModulepreloadEventAction,
    DomManipulation(PageDomManipulationTestFamily),
    DedicatedWorkerClientEvent,
    DynamicImportOwnerAction,
    FileReading,
    MiscPlatformApi,
    HistoryTraversal,
    IndexedDbTask,
    InternalLoading,
    MainParserContinuation,
    MediaElementEvent,
    MessagePortDelivery,
    ModuleReaction,
    ModulepreloadStart,
    NavigationApi,
    OpfsTask,
    RenderingUpdate,
    ResourceCompletion,
    MainDocumentRuntime(PageMainDocumentRuntimeActionKind),
    ServiceWorkerClientMessage,
    ServiceWorkerInternal,
    SharedWorkerClientEvent,
    StyleElementEvent,
    StylesheetCompletion,
    TextTrackNetworking,
    UserInteraction,
    BitmapTask,
    WebCryptoTask,
    WebSocket,
    WindowMessage,
    WorkerHostBridge,
}

impl PageSelectedTaskTestSelector {
    fn matches_descriptor(self, descriptor: RendererPageReadyDescriptor) -> bool {
        match self {
            Self::ChildDocumentLifecycle => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    if matches!(
                        owner.target(),
                        crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
                    )
            ),
            Self::ChildDocumentScriptReady => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    if matches!(
                        owner.target(),
                        crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentScriptReady(_)
                    )
            ),
            Self::ChildHostLoad => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    if matches!(
                        owner.target(),
                        crate::page_task_queue::RendererPageChildFrameTaskTarget::HostLoad(_)
                    )
            ),
            Self::ChildClassicScriptSourceLoad => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    if matches!(
                        owner.target(),
                        crate::page_task_queue::RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_)
                    )
            ),
            Self::ChildModuleDependencyFetchStart => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildModuleDependencyFetchStart { .. }
            ),
            Self::ChildModuleScriptTerminal => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildModuleScriptTerminal { .. }
            ),
            Self::ChildParserModuleRootStart => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    if matches!(
                        owner.target(),
                        crate::page_task_queue::RendererPageChildFrameTaskTarget::ParserModuleRootStart(_)
                    )
            ),
            Self::ChildRealmMaterialization => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                    if matches!(
                        owner.target(),
                        crate::page_task_queue::RendererPageChildFrameTaskTarget::RealmMaterialization(_)
                    )
            ),
            Self::ChildNavigationCommit => matches!(
                descriptor,
                RendererPageReadyDescriptor::NavigationAndTraversal {
                    head: RendererPageNavigationAndTraversalHead::ChildNavigationCommit { .. },
                    ..
                }
            ),
            Self::ChildModulepreloadEventAction => matches!(
                descriptor,
                RendererPageReadyDescriptor::ChildModulepreloadEventAction { .. }
            ),
            Self::DomManipulation(family) => matches!(
                descriptor,
                RendererPageReadyDescriptor::DomManipulation { owner, .. }
                    if family.matches_owner(owner)
            ),
            Self::DedicatedWorkerClientEvent => matches!(
                descriptor,
                RendererPageReadyDescriptor::DedicatedWorkerClientEvent { .. }
            ),
            Self::DynamicImportOwnerAction => matches!(
                descriptor,
                RendererPageReadyDescriptor::DynamicImportOwnerAction { .. }
            ),
            Self::HistoryTraversal => matches!(
                descriptor,
                RendererPageReadyDescriptor::NavigationAndTraversal {
                    head: RendererPageNavigationAndTraversalHead::HistoryTraversal { .. },
                    ..
                }
            ),
            Self::IndexedDbTask => {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::IndexedDbTask { .. }
                )
            }
            Self::InternalLoading => matches!(
                descriptor,
                RendererPageReadyDescriptor::InternalLoading { .. }
            ),
            Self::MainParserContinuation => matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::MainParserContinuation(_),
                    ..
                }
            ),
            Self::MediaElementEvent => matches!(
                descriptor,
                RendererPageReadyDescriptor::MediaElementEvent { .. }
            ),
            Self::TextTrackNetworking => matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::TextTrackLoad(_),
                    ..
                }
            ),
            Self::MessagePortDelivery => matches!(
                descriptor,
                RendererPageReadyDescriptor::MessagePortDelivery { .. }
            ),
            Self::ModuleReaction => {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::ModuleReaction { .. }
                )
            }
            Self::ModulepreloadStart => matches!(
                descriptor,
                RendererPageReadyDescriptor::ModulepreloadStart { .. }
            ),
            Self::NavigationApi => matches!(
                descriptor,
                RendererPageReadyDescriptor::NavigationAndTraversal {
                    head: RendererPageNavigationAndTraversalHead::NavigationApi { .. },
                    ..
                }
            ),
            Self::OpfsTask => {
                matches!(descriptor, RendererPageReadyDescriptor::OpfsTask { .. })
            }
            Self::RenderingUpdate => matches!(
                descriptor,
                RendererPageReadyDescriptor::RenderingUpdate { .. }
            ),
            Self::ResourceCompletion => matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::ResourceCompletion(_),
                    ..
                }
            ),
            // The production descriptor intentionally exposes only arbitration
            // metadata for this heterogeneous lane. Claiming this exact
            // variant uses the source-local head check below.
            Self::MainDocumentRuntime(_) => matches!(
                descriptor,
                RendererPageReadyDescriptor::MainDocumentRuntime { .. }
            ),
            Self::ServiceWorkerClientMessage => matches!(
                descriptor,
                RendererPageReadyDescriptor::ServiceWorkerClientMessage { .. }
            ),
            Self::ServiceWorkerInternal => matches!(
                descriptor,
                RendererPageReadyDescriptor::ServiceWorkerInternal { .. }
            ),
            Self::SharedWorkerClientEvent => matches!(
                descriptor,
                RendererPageReadyDescriptor::SharedWorkerClientEvent { .. }
            ),
            Self::StyleElementEvent => matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::StyleElementEvent(_),
                    ..
                }
            ),
            Self::StylesheetCompletion => matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::StylesheetCompletion(_),
                    ..
                }
            ),
            Self::UserInteraction => matches!(
                descriptor,
                RendererPageReadyDescriptor::UserInteraction { .. }
            ),
            Self::FileReading => {
                matches!(descriptor, RendererPageReadyDescriptor::FileReading { .. })
            }
            Self::MiscPlatformApi => {
                matches!(
                    descriptor,
                    RendererPageReadyDescriptor::MiscPlatformApi { .. }
                )
            }
            Self::BitmapTask => {
                matches!(descriptor, RendererPageReadyDescriptor::BitmapTask { .. })
            }
            Self::WebCryptoTask => matches!(
                descriptor,
                RendererPageReadyDescriptor::WebCryptoTask { .. }
            ),
            // WebSocket readiness also depends on the current root Document.
            // The claim boundary below performs that source-local check.
            Self::WebSocket => matches!(descriptor, RendererPageReadyDescriptor::WebSocket { .. }),
            Self::WindowMessage => matches!(
                descriptor,
                RendererPageReadyDescriptor::WindowMessage { .. }
            ),
            Self::WorkerHostBridge => matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::WorkerHostBridge(_),
                    ..
                }
            ),
        }
    }

    fn matches_task(self, task: &RendererPageSchedulerTask) -> bool {
        match (self, task) {
            (Self::ChildDocumentLifecycle, RendererPageSchedulerTask::ChildFrameTask(task)) => {
                matches!(
                    task.owner().target(),
                    crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
                )
            }
            (Self::ChildDocumentScriptReady, RendererPageSchedulerTask::ChildFrameTask(task)) => {
                matches!(
                    task.owner().target(),
                    crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentScriptReady(
                        _
                    )
                )
            }
            (Self::ChildHostLoad, RendererPageSchedulerTask::ChildFrameTask(task)) => matches!(
                task.owner().target(),
                crate::page_task_queue::RendererPageChildFrameTaskTarget::HostLoad(_)
            ),
            (
                Self::ChildClassicScriptSourceLoad,
                RendererPageSchedulerTask::ChildFrameTask(task),
            ) => matches!(
                task.owner().target(),
                crate::page_task_queue::RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(
                    _
                )
            ),
            (
                Self::ChildModuleDependencyFetchStart,
                RendererPageSchedulerTask::ChildModuleDependencyFetchStart(_),
            ) => true,
            (
                Self::ChildModuleScriptTerminal,
                RendererPageSchedulerTask::ChildModuleScriptTerminal(_),
            ) => true,
            (Self::ChildParserModuleRootStart, RendererPageSchedulerTask::ChildFrameTask(task)) => {
                matches!(
                    task.owner().target(),
                    crate::page_task_queue::RendererPageChildFrameTaskTarget::ParserModuleRootStart(
                        _
                    )
                )
            }
            (Self::ChildRealmMaterialization, RendererPageSchedulerTask::ChildFrameTask(task)) => {
                matches!(
                    task.owner().target(),
                    crate::page_task_queue::RendererPageChildFrameTaskTarget::RealmMaterialization(
                        _
                    )
                )
            }
            (
                Self::ChildNavigationCommit,
                RendererPageSchedulerTask::NavigationAndTraversal(
                    RendererPageNavigationAndTraversalTask::ChildNavigationCommit(_),
                ),
            ) => true,
            (
                Self::ChildModulepreloadEventAction,
                RendererPageSchedulerTask::ChildModulepreloadEventAction(_),
            ) => true,
            (Self::DomManipulation(family), RendererPageSchedulerTask::DomManipulation(task)) => {
                family.matches_owner(task.owner())
            }
            (
                Self::DedicatedWorkerClientEvent,
                RendererPageSchedulerTask::DedicatedWorkerClientEvent(_),
            )
            | (
                Self::DynamicImportOwnerAction,
                RendererPageSchedulerTask::DynamicImportOwnerAction(_),
            )
            | (Self::FileReading, RendererPageSchedulerTask::FileReading(_))
            | (Self::MiscPlatformApi, RendererPageSchedulerTask::MiscPlatformApi(_))
            | (Self::IndexedDbTask, RendererPageSchedulerTask::IndexedDbTask(_))
            | (Self::InternalLoading, RendererPageSchedulerTask::InternalLoading(_))
            | (
                Self::MainParserContinuation,
                RendererPageSchedulerTask::Networking(
                    RendererPageNetworkingTask::MainParserContinuation(_),
                ),
            )
            | (Self::MediaElementEvent, RendererPageSchedulerTask::MediaElementEvent(_))
            | (Self::MessagePortDelivery, RendererPageSchedulerTask::MessagePortDelivery { .. })
            | (Self::ModuleReaction, RendererPageSchedulerTask::ModuleReaction(_))
            | (Self::ModulepreloadStart, RendererPageSchedulerTask::ModulepreloadStart(_))
            | (Self::OpfsTask, RendererPageSchedulerTask::OpfsTask(_))
            | (Self::RenderingUpdate, RendererPageSchedulerTask::RenderingUpdate(_))
            | (
                Self::TextTrackNetworking,
                RendererPageSchedulerTask::Networking(RendererPageNetworkingTask::TextTrackLoad(_)),
            )
            | (
                Self::ServiceWorkerClientMessage,
                RendererPageSchedulerTask::ServiceWorkerClientMessage(_),
            )
            | (Self::ServiceWorkerInternal, RendererPageSchedulerTask::ServiceWorkerInternal(_))
            | (
                Self::SharedWorkerClientEvent,
                RendererPageSchedulerTask::SharedWorkerClientEvent(_),
            )
            | (Self::UserInteraction, RendererPageSchedulerTask::UserInteraction(_))
            | (Self::BitmapTask, RendererPageSchedulerTask::BitmapTask(_))
            | (Self::WebCryptoTask, RendererPageSchedulerTask::WebCryptoTask(_))
            | (Self::WindowMessage, RendererPageSchedulerTask::WindowMessage(_)) => true,
            (Self::WebSocket, RendererPageSchedulerTask::WebSocket(_)) => true,
            (
                Self::ResourceCompletion,
                RendererPageSchedulerTask::Networking(
                    RendererPageNetworkingTask::ResourceCompletion(_),
                ),
            ) => true,
            (
                Self::HistoryTraversal,
                RendererPageSchedulerTask::NavigationAndTraversal(
                    RendererPageNavigationAndTraversalTask::HistoryTraversal(_),
                ),
            )
            | (
                Self::NavigationApi,
                RendererPageSchedulerTask::NavigationAndTraversal(
                    RendererPageNavigationAndTraversalTask::NavigationApi(_),
                ),
            ) => true,
            (
                Self::MainDocumentRuntime(expected_kind),
                RendererPageSchedulerTask::MainDocumentRuntime(task),
            ) => task.action_kind() == expected_kind,
            (
                Self::WorkerHostBridge,
                RendererPageSchedulerTask::Networking(
                    RendererPageNetworkingTask::WorkerHostBridge(_),
                ),
            )
            | (
                Self::StyleElementEvent,
                RendererPageSchedulerTask::Networking(
                    RendererPageNetworkingTask::StyleElementEvent(_),
                ),
            ) => true,
            (
                Self::StylesheetCompletion,
                RendererPageSchedulerTask::Networking(
                    RendererPageNetworkingTask::StylesheetCompletion(_),
                ),
            ) => true,
            _ => false,
        }
    }
}

/// One dequeued scheduler task whose payload remains private to this harness.
///
/// Holding the value across a replacement is intentional: the production
/// dispatcher, not the test, must decide whether the exact task is current or
/// stale when it eventually executes.
pub(crate) struct ClaimedPageSelectedTaskForTest {
    selector: PageSelectedTaskTestSelector,
    task: RendererPageSchedulerTask,
}

impl ClaimedPageSelectedTaskForTest {
    pub(crate) const fn selector(&self) -> PageSelectedTaskTestSelector {
        self.selector
    }

    pub(crate) fn child_module_script_terminal_owner(
        &self,
    ) -> Option<crate::page_task_queue::RendererPageChildModuleScriptTerminalOwner> {
        match &self.task {
            RendererPageSchedulerTask::ChildModuleScriptTerminal(task) => Some(task.owner()),
            _ => None,
        }
    }

    pub(crate) fn dedicated_worker_owner_and_event_kind(
        &self,
    ) -> Option<(
        crate::page_task_queue::RendererPageDedicatedWorkerClientEventOwner,
        crate::page_task_queue::RendererDedicatedWorkerClientEventKind,
    )> {
        match &self.task {
            RendererPageSchedulerTask::DedicatedWorkerClientEvent(task) => {
                Some((task.owner(), task.event_kind()))
            }
            _ => None,
        }
    }

    pub(crate) fn indexed_db_owner_and_kind(
        &self,
    ) -> Option<(
        crate::page_task_queue::RendererPageIndexedDbTaskOwner,
        crate::page_task_queue::RendererPageIndexedDbTaskKind,
    )> {
        match &self.task {
            RendererPageSchedulerTask::IndexedDbTask(task) => Some((task.owner(), task.kind())),
            _ => None,
        }
    }

    pub(crate) fn history_traversal_owner_and_task_id(
        &self,
    ) -> Option<(
        crate::page_task_queue::RendererPageHistoryTraversalOwner,
        crate::page_task_queue::RendererPageHistoryTraversalTaskId,
    )> {
        match &self.task {
            RendererPageSchedulerTask::NavigationAndTraversal(
                RendererPageNavigationAndTraversalTask::HistoryTraversal(task),
            ) => Some((task.owner(), task.task_id())),
            _ => None,
        }
    }

    pub(crate) fn media_element_event_kind(
        &self,
    ) -> Option<crate::page_task_queue::RendererPageMediaElementEventTaskKind> {
        match &self.task {
            RendererPageSchedulerTask::MediaElementEvent(task) => Some(task.kind()),
            _ => None,
        }
    }

    pub(crate) fn navigation_api_owner_and_task_id(
        &self,
    ) -> Option<(
        crate::page_task_queue::RendererPageNavigationApiTaskOwner,
        crate::page_task_queue::RendererPageNavigationApiTaskId,
    )> {
        match &self.task {
            RendererPageSchedulerTask::NavigationAndTraversal(
                RendererPageNavigationAndTraversalTask::NavigationApi(task),
            ) => Some((task.owner(), task.task_id())),
            _ => None,
        }
    }

    pub(crate) fn rendering_update_owner_and_kind(
        &self,
    ) -> Option<(
        crate::page_task_queue::RendererPageRenderingUpdateOwner,
        crate::page_task_queue::RendererPageRenderingUpdateTaskKind,
    )> {
        match &self.task {
            RendererPageSchedulerTask::RenderingUpdate(task) => Some((task.owner(), task.kind())),
            _ => None,
        }
    }

    pub(crate) fn view_transition_update_owner_and_task_id(
        &self,
    ) -> Option<(
        crate::page_task_queue::RendererPageViewTransitionUpdateOwner,
        crate::page_task_queue::RendererPageViewTransitionUpdateTaskId,
    )> {
        match &self.task {
            RendererPageSchedulerTask::DomManipulation(
                RendererPageDomManipulationTask::ViewTransitionUpdate(task),
            ) => Some((task.owner(), task.task_id())),
            _ => None,
        }
    }

    pub(crate) fn websocket_owner(
        &self,
    ) -> Option<crate::page_task_queue::RendererPageWebSocketOwner> {
        match &self.task {
            RendererPageSchedulerTask::WebSocket(task) => Some(task.owner()),
            _ => None,
        }
    }
}

impl PageVm {
    pub(crate) fn claim_exact_selected_page_task_for_test(
        &mut self,
        selector: PageSelectedTaskTestSelector,
    ) -> Option<ClaimedPageSelectedTaskForTest> {
        let sources = self.page_task_executor_sources_for_test();
        let task = match selector {
            PageSelectedTaskTestSelector::ChildModuleScriptTerminal => sources
                .take_child_module_script_terminal_for_executor_test(|owner| {
                    self.page_child_module_script_terminal_is_eligible_for_owner_turn(owner)
                })
                .map(RendererPageSchedulerTask::ChildModuleScriptTerminal),
            PageSelectedTaskTestSelector::ChildModulepreloadEventAction => sources
                .take_child_modulepreload_event_action_for_executor_test(|owner| {
                    self.page_child_modulepreload_event_action_is_eligible_for_owner_turn(owner)
                })
                .map(RendererPageSchedulerTask::ChildModulepreloadEventAction),
            PageSelectedTaskTestSelector::MainDocumentRuntime(kind) => sources
                .take_main_document_runtime_action_for_executor_test(kind)
                .map(RendererPageSchedulerTask::MainDocumentRuntime),
            PageSelectedTaskTestSelector::WindowMessage => sources
                .take_window_message_for_executor_test(|owner, task_id| {
                    self.page_window_message_is_eligible_for_owner_turn(owner, task_id)
                })
                .map(RendererPageSchedulerTask::WindowMessage),
            _ => sources.take_scheduler_task_for_executor_test(|descriptor| {
                selector.matches_descriptor(descriptor)
                    && self.page_ready_descriptor_is_eligible(descriptor)
            }),
        }?;
        assert!(
            selector.matches_task(&task),
            "exact test selector {selector:?} dequeued a different scheduler task variant"
        );
        Some(ClaimedPageSelectedTaskForTest { selector, task })
    }

    pub(crate) fn run_claimed_selected_page_task_for_test<'a>(
        &'a mut self,
        claimed: ClaimedPageSelectedTaskForTest,
        loader: &'a crate::network::ResourceRequestClient,
    ) -> SelectedPageTaskTestFuture<'a, ()> {
        Box::pin(async move {
            assert!(
                claimed.selector.matches_task(&claimed.task),
                "opaque selected-task claim changed variant before execution"
            );
            self.apply_selected_page_scheduler_task_on_owner_lane_for_test(
                claimed.task,
                loader.clone(),
            )
            .await?;
            Ok(())
        })
    }

    pub(crate) fn run_exact_selected_page_task_for_test<'a>(
        &'a mut self,
        selector: PageSelectedTaskTestSelector,
        loader: &'a crate::network::ResourceRequestClient,
    ) -> SelectedPageTaskTestFuture<'a, bool> {
        Box::pin(async move {
            let Some(claimed) = self.claim_exact_selected_page_task_for_test(selector) else {
                return Ok(false);
            };
            self.run_claimed_selected_page_task_for_test(claimed, loader)
                .await?;
            Ok(true)
        })
    }
}
