use crate::{
    native_bridge::WindowExecutionContextIdentity,
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::navigation_and_traversal::{
    RendererPageNavigationAndTraversalRoute, RendererPageNavigationAndTraversalTask,
};

/// PageVm-local key for one V8-bearing Navigation API payload retained by
/// `PageScriptHost`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageNavigationApiTaskId(u64);

impl RendererPageNavigationApiTaskId {
    pub(crate) const fn first() -> Self {
        Self(1)
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageNavigationApiTaskKind {
    FinishResult,
}

/// Exact relevant global captured by `queue a global task`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageNavigationApiTaskOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
}

impl RendererPageNavigationApiTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
    ) -> Self {
        Self {
            root_document,
            execution_context,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }

    #[cfg(test)]
    pub(crate) fn target(self) -> crate::native_bridge::WindowTaskTarget {
        crate::native_bridge::WindowTaskTarget::new(
            self.execution_context.dispatch_scope(),
            self.execution_context.owner(),
        )
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageNavigationApiTask {
    owner: RendererPageNavigationApiTaskOwner,
    task_id: RendererPageNavigationApiTaskId,
    kind: RendererPageNavigationApiTaskKind,
}

impl RendererPageNavigationApiTask {
    fn new(
        owner: RendererPageNavigationApiTaskOwner,
        task_id: RendererPageNavigationApiTaskId,
        kind: RendererPageNavigationApiTaskKind,
    ) -> Self {
        Self {
            owner,
            task_id,
            kind,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageNavigationApiTaskOwner {
        self.owner
    }

    pub(crate) const fn task_id(&self) -> RendererPageNavigationApiTaskId {
        self.task_id
    }

    pub(crate) const fn kind(&self) -> RendererPageNavigationApiTaskKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageNavigationApiTaskRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageNavigationApiTaskSender {
    route: RendererPageNavigationAndTraversalRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageNavigationApiTaskSender {
    pub(super) fn new(
        route: RendererPageNavigationAndTraversalRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    pub(crate) fn bind_task(
        &self,
        execution_context: WindowExecutionContextIdentity,
        task_id: RendererPageNavigationApiTaskId,
        kind: RendererPageNavigationApiTaskKind,
    ) -> RendererPageNavigationApiTaskProducer {
        RendererPageNavigationApiTaskProducer {
            route: self.route.clone(),
            task: RendererPageNavigationApiTask::new(
                RendererPageNavigationApiTaskOwner::new(self.root_document, execution_context),
                task_id,
                kind,
            ),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageNavigationApiTaskProducer {
    route: RendererPageNavigationAndTraversalRoute,
    task: RendererPageNavigationApiTask,
}

impl RendererPageNavigationApiTaskProducer {
    pub(crate) const fn task_id(&self) -> RendererPageNavigationApiTaskId {
        self.task.task_id()
    }

    pub(crate) fn send(self) -> Result<(), RendererPageNavigationApiTaskRouteClosed> {
        self.route
            .send(RendererPageNavigationAndTraversalTask::NavigationApi(
                self.task,
            ))
            .map_err(|_| RendererPageNavigationApiTaskRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageNavigationApiTaskTargetEffect {
    /// The active attempt dispatched its success pass and scheduled its
    /// Promise settlements in the exact current Window.
    FinishResultAppliedToCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageNavigationApiTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageNavigationApiTaskTurnAction {
    pub(crate) owner: RendererPageNavigationApiTaskOwner,
    pub(crate) task_id: RendererPageNavigationApiTaskId,
    pub(crate) kind: RendererPageNavigationApiTaskKind,
    pub(crate) target_effect: PageNavigationApiTaskTargetEffect,
}

pub(crate) type PageNavigationApiTaskTurnOutcome =
    PageOwnerTurnOutcome<PageNavigationApiTaskTurnAction>;
