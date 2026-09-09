use tokio::sync::oneshot;

use crate::{
    frame_owner_model::FrameDocumentTaskOwner,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    script_vm::{MainDocumentLifecycleBody, MainDocumentLifecycleTargetEffect},
};

use super::dom_manipulation::{RendererPageDomManipulationRoute, RendererPageDomManipulationTask};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageMainDocumentLifecycleOwner {
    pub(crate) root_document: RendererDocumentToken,
    pub(crate) body: MainDocumentLifecycleBody,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageMainDocumentLifecycleCompletion {
    Executed,
    /// An earlier DOM task added a load delay after this task was admitted.
    /// Return the exact load boundary to its driver without completing it.
    LoadBlocked {
        owner: FrameDocumentTaskOwner,
    },
}

/// An admitted HTML lifecycle task, after parser/defer and load prerequisites.
/// Its receipt releases the driver's completion token only after execution.
#[derive(Debug)]
pub(crate) struct RendererPageMainDocumentLifecycleTask {
    pub(crate) owner: RendererPageMainDocumentLifecycleOwner,
    pub(crate) completion: Option<oneshot::Sender<RendererPageMainDocumentLifecycleCompletion>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageMainDocumentLifecycleSender {
    route: RendererPageDomManipulationRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageMainDocumentLifecycleSender {
    pub(super) fn new(
        route: RendererPageDomManipulationRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn send(
        &self,
        body: MainDocumentLifecycleBody,
        completion: Option<oneshot::Sender<RendererPageMainDocumentLifecycleCompletion>>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !matches!(body, MainDocumentLifecycleBody::Interactive(_)),
            "interactive readiness belongs to parser completion"
        );
        self.route
            .send(RendererPageDomManipulationTask::MainDocumentLifecycle(
                RendererPageMainDocumentLifecycleTask {
                    owner: RendererPageMainDocumentLifecycleOwner {
                        root_document: self.root_document,
                        body,
                    },
                    completion,
                },
            ))
            .map_err(|_| anyhow::anyhow!("main document lifecycle DOM task route closed"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageMainDocumentLifecycleTurnAction {
    pub(crate) owner: RendererPageMainDocumentLifecycleOwner,
    pub(crate) target: MainDocumentLifecycleTargetEffect,
}

pub(crate) type PageMainDocumentLifecycleTurnOutcome =
    PageOwnerTurnOutcome<PageMainDocumentLifecycleTurnAction>;
