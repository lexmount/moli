use crate::runtime::PageOwnerTurnOutcome;

use super::{
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    window_document_task_source::{
        RendererPageWindowDocumentTaskRoute, RendererPageWindowDocumentTaskSender,
        RendererPageWindowDocumentTaskSource,
    },
};

/// Host-local identity for one admitted file-reading task.
///
/// For an active enumeration this id is also written into the exact reader
/// residence. Completion may clear `reading` and advance the offset only when
/// the selected task still matches that id.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageFileReadingTaskId(u64);

impl RendererPageFileReadingTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Concrete outcome carried by the HTML file-reading task source.
///
/// These variants describe already-admitted FileReader and Entries API operations. They
/// are not callback-presence flags and do not decide scheduler policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageFileReadingTaskKind {
    BlobRead,
    FileReader(RendererPageFileReaderPhase),
    /// Complete one active enumeration request, possibly with the first empty
    /// batch that transitions the reader to `done`.
    DirectoryBatch,
    /// The reader was already `done`; deliver another asynchronous empty
    /// sequence without reopening enumeration state.
    DirectoryTerminalEmpty,
    /// A second call observed an active request and must report
    /// `InvalidStateError` without touching that active request.
    DirectoryOverlappingReadError,
    /// The reader had already recorded a terminal enumeration error.
    DirectoryTerminalError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageFileReaderPhase {
    LoadStart,
    Progress,
    Complete,
}

pub(crate) type RendererPageFileReadingOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageFileReadingTask =
    RendererPageWindowDocumentTask<RendererPageFileReadingTaskId, RendererPageFileReadingTaskKind>;
pub(super) type RendererPageFileReadingRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageFileReadingTaskId,
    RendererPageFileReadingTaskKind,
>;
pub(crate) type RendererPageFileReadingSender = RendererPageWindowDocumentTaskSender<
    RendererPageFileReadingTaskId,
    RendererPageFileReadingTaskKind,
>;
pub(super) type RendererPageFileReadingSource = RendererPageWindowDocumentTaskSource<
    RendererPageFileReadingTaskId,
    RendererPageFileReadingTaskKind,
>;

/// Execution fact produced after exact Page/Document and reader-request
/// authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageFileReadingTargetEffect {
    CallbackInvokedForCurrentOwner,
    FileReaderPhaseAdvanced,
    CurrentOwnerCallbackRetired,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageFileReadingOwner>,
    },
    DiscardedStaleReaderRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageFileReadingTurnAction {
    pub(crate) owner: RendererPageFileReadingOwner,
    pub(crate) task_id: RendererPageFileReadingTaskId,
    pub(crate) kind: RendererPageFileReadingTaskKind,
    pub(crate) target_effect: PageFileReadingTargetEffect,
}

pub(crate) type PageFileReadingTurnOutcome = PageOwnerTurnOutcome<PageFileReadingTurnAction>;
