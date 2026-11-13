use std::convert::Infallible;

use crate::{
    document_runtime::DomHandle,
    dom::NodeId,
    frame_owner_model::FrameDocumentTaskOwner,
    parser_script::action::{
        ParserClassicScriptReadyAction, ParserClassicScriptSourceFailureAction,
    },
};

use super::{DocumentScriptReadyActionDispatchRoute, DocumentScriptReadyActionRoute};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentReadyActionRoute {
    script_node_id: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDocumentClassicScriptTarget {
    owner: FrameDocumentTaskOwner,
    route: MainDocumentReadyActionRoute,
}

pub(crate) type MainDocumentClassicReadyWork =
    ParserClassicScriptReadyAction<MainDocumentClassicScriptTarget>;
pub(crate) type MainDocumentClassicSourceFailureWork =
    ParserClassicScriptSourceFailureAction<MainDocumentClassicScriptTarget, Infallible>;

impl MainDocumentReadyActionRoute {
    pub(crate) fn new(script_node_id: NodeId) -> Self {
        Self { script_node_id }
    }

    pub(crate) fn from_script_handle(script_handle: DomHandle) -> Self {
        Self::new(NodeId::new(script_handle.index()))
    }

    pub(crate) fn script_node_id(&self) -> NodeId {
        self.script_node_id
    }
}

impl MainDocumentClassicScriptTarget {
    pub(crate) fn new(owner: FrameDocumentTaskOwner, script_handle: DomHandle) -> Self {
        Self {
            owner,
            route: MainDocumentReadyActionRoute::from_script_handle(script_handle),
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn route(&self) -> MainDocumentReadyActionRoute {
        self.route
    }
}

impl DocumentScriptReadyActionRoute<FrameDocumentTaskOwner> for MainDocumentClassicReadyWork {
    fn payload_document_owner(&self) -> FrameDocumentTaskOwner {
        self.target().owner()
    }
}

impl DocumentScriptReadyActionDispatchRoute<MainDocumentReadyActionRoute>
    for MainDocumentClassicReadyWork
{
    fn dispatch_route(&self) -> MainDocumentReadyActionRoute {
        self.target().route()
    }
}

impl DocumentScriptReadyActionRoute<FrameDocumentTaskOwner>
    for MainDocumentClassicSourceFailureWork
{
    fn payload_document_owner(&self) -> FrameDocumentTaskOwner {
        self.target().owner()
    }
}

impl DocumentScriptReadyActionDispatchRoute<MainDocumentReadyActionRoute>
    for MainDocumentClassicSourceFailureWork
{
    fn dispatch_route(&self) -> MainDocumentReadyActionRoute {
        self.target().route()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::document_script_scheduler::DocumentOwnedScriptReadyAction;
    use crate::frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId};
    use crate::parser_script::action::{
        ParserClassicScriptReadyAction, ParserClassicScriptSourceFailureAction,
        ParserPendingClassicScriptReadyKind,
    };
    use crate::parser_script::payload::{
        ParserClassicScriptMetadata, ParserClassicScriptSourceFailure, ParserReadyClassicScript,
    };

    fn main_task_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(0), LocalWindowId(0), DocumentId(1))
    }

    #[test]
    fn main_parser_blocking_classic_actions_match_payload_owner_and_route() {
        let ready_handle = DomHandle::new(41);
        let ready_target = MainDocumentClassicScriptTarget::new(main_task_owner(), ready_handle);
        let ready = ParserClassicScriptReadyAction::new(
            ready_target,
            ParserReadyClassicScript::new(
                ParserClassicScriptMetadata::new(ready_handle, 9),
                url::Url::parse("https://example.test/main-classic.js").expect("script url"),
            ),
            ParserPendingClassicScriptReadyKind::ParserConnected,
        );
        let ready_dispatch = DocumentOwnedScriptReadyAction::new(main_task_owner(), ready)
            .into_dispatch::<MainDocumentReadyActionRoute>()
            .expect("main parser-blocking classic ready should route to its owner");
        assert_eq!(*ready_dispatch.queued_owner(), main_task_owner());
        let ready_route = ready_dispatch.route();
        assert_eq!(
            ready_route.script_node_id(),
            NodeId::new(ready_handle.index())
        );

        let failure_handle = DomHandle::new(42);
        let failure_target =
            MainDocumentClassicScriptTarget::new(main_task_owner(), failure_handle);
        let failure = ParserClassicScriptSourceFailureAction::new(
            failure_target,
            ParserClassicScriptSourceFailure {
                metadata: ParserClassicScriptMetadata::new(failure_handle, 10),
                script_url: url::Url::parse("https://example.test/missing-classic.js")
                    .expect("script url"),
                error: "network error".to_owned(),
            },
            None,
        );
        let failure_dispatch = DocumentOwnedScriptReadyAction::new(main_task_owner(), failure)
            .into_dispatch::<MainDocumentReadyActionRoute>()
            .expect("main parser-blocking classic source failure should route to its owner");
        assert_eq!(*failure_dispatch.queued_owner(), main_task_owner());
        let failure_route = failure_dispatch.route();
        assert_eq!(
            failure_route.script_node_id(),
            NodeId::new(failure_handle.index())
        );
    }
}
