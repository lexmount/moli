use crate::{
    native_bridge::{WindowExecutionContextIdentity, WindowTaskTarget},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::navigation_and_traversal::{
    RendererPageNavigationAndTraversalRoute, RendererPageNavigationAndTraversalTask,
};

/// PageVm-local key for one pending history-traversal payload retained by
/// `JsContextHost`.
///
/// The stable Page source carries only this key and immutable execution
/// identity. V8 Promise resolvers, Navigation API `info`, and entry seeds stay
/// in the PageVm that accepted the traversal, so a replacement PageVm can
/// never observe an old payload through a reused local id.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageHistoryTraversalTaskId(u64);

impl RendererPageHistoryTraversalTaskId {
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

/// Concrete history operation sharing the HTML navigation-and-traversal task
/// source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageHistoryTraversalTaskKind {
    ByDelta,
    SameDocument,
    ChildCrossDocument,
}

/// Exact authority captured when a history traversal becomes pending.
///
/// `execution_context` preserves the callback/Promise relevant realm used by
/// the old timer transport. `target` independently protects the traversed
/// LocalWindow, which matters for joint-session-history work produced by a
/// parent realm on behalf of a child. The root token namespaces local ids
/// across PageVm replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageHistoryTraversalOwner {
    root_document: RendererDocumentToken,
    execution_context: WindowExecutionContextIdentity,
    target: WindowTaskTarget,
}

impl RendererPageHistoryTraversalOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        execution_context: WindowExecutionContextIdentity,
        target: WindowTaskTarget,
    ) -> Self {
        Self {
            root_document,
            execution_context,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn execution_context(self) -> WindowExecutionContextIdentity {
        self.execution_context
    }

    pub(crate) const fn target(self) -> WindowTaskTarget {
        self.target
    }
}

/// One scheduler-visible history traversal. The actual V8-bearing payload is
/// retained by `JsContextHost` under `task_id`.
#[derive(Debug)]
pub(crate) struct RendererPageHistoryTraversalTask {
    owner: RendererPageHistoryTraversalOwner,
    task_id: RendererPageHistoryTraversalTaskId,
    kind: RendererPageHistoryTraversalTaskKind,
}

impl RendererPageHistoryTraversalTask {
    fn new(
        owner: RendererPageHistoryTraversalOwner,
        task_id: RendererPageHistoryTraversalTaskId,
        kind: RendererPageHistoryTraversalTaskKind,
    ) -> Self {
        Self {
            owner,
            task_id,
            kind,
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageHistoryTraversalOwner {
        self.owner
    }

    pub(crate) const fn task_id(&self) -> RendererPageHistoryTraversalTaskId {
        self.task_id
    }

    pub(crate) const fn kind(&self) -> RendererPageHistoryTraversalTaskKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageHistoryTraversalRouteClosed;

/// PageVm-stamped producer route installed atomically on `JsContextHost`.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageHistoryTraversalSender {
    route: RendererPageNavigationAndTraversalRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageHistoryTraversalSender {
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
        target: WindowTaskTarget,
        task_id: RendererPageHistoryTraversalTaskId,
        kind: RendererPageHistoryTraversalTaskKind,
    ) -> RendererPageHistoryTraversalProducer {
        RendererPageHistoryTraversalProducer {
            route: self.route.clone(),
            task: RendererPageHistoryTraversalTask::new(
                RendererPageHistoryTraversalOwner::new(
                    self.root_document,
                    execution_context,
                    target,
                ),
                task_id,
                kind,
            ),
        }
    }
}

/// Single-use admission capability paired with one local pending payload.
#[derive(Debug)]
pub(crate) struct RendererPageHistoryTraversalProducer {
    route: RendererPageNavigationAndTraversalRoute,
    task: RendererPageHistoryTraversalTask,
}

impl RendererPageHistoryTraversalProducer {
    pub(crate) const fn task_id(&self) -> RendererPageHistoryTraversalTaskId {
        self.task.task_id()
    }

    pub(crate) fn send(self) -> Result<(), RendererPageHistoryTraversalRouteClosed> {
        self.route
            .send(RendererPageNavigationAndTraversalTask::HistoryTraversal(
                self.task,
            ))
            .map_err(|_| RendererPageHistoryTraversalRouteClosed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageHistoryTraversalTargetEffect {
    AppliedToCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageHistoryTraversalOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageHistoryTraversalTurnAction {
    pub(crate) owner: RendererPageHistoryTraversalOwner,
    pub(crate) task_id: RendererPageHistoryTraversalTaskId,
    pub(crate) kind: RendererPageHistoryTraversalTaskKind,
    pub(crate) target_effect: PageHistoryTraversalTargetEffect,
}

pub(crate) type PageHistoryTraversalTurnOutcome =
    PageOwnerTurnOutcome<PageHistoryTraversalTurnAction>;
