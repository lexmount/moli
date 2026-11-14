//! Seeds and committed updates exchanged across page/renderer boundaries.

use moli_session_history::{
    JointSessionHistory, SessionHistoryEntry, SessionHistoryPosition, SessionHistoryStepId,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionHistoryCommit {
    #[default]
    Attach,
    Push,
    Replace,
    Traverse,
}

/// The history effect of a document commit, carried separately from the
/// document's entry view. A top-level renderer replacement also carries its
/// traversable; subframe commits keep using the page's existing owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionHistorySeed {
    pub commit: SessionHistoryCommit,
    pub traversable: Option<Box<JointSessionHistory>>,
    pub target_step: Option<SessionHistoryStepId>,
    /// A cross-Document participant of an already accepted joint traversal.
    /// Its load may materialize this entry, but may not move the shared cursor.
    pub admitted_entry: Option<SessionHistoryEntry>,
}

/// One already-committed traversable mutation, published in renderer FIFO
/// order before navigation observers can initiate another mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryUpdate {
    pub position: SessionHistoryPosition,
    pub update: crate::SessionHistoryUpdateKind,
    pub root_url: String,
    /// Joint steps that share the current root entry. A root replacement also
    /// changes the URL exposed by steps introduced by child navigations.
    pub root_entry_steps: Vec<usize>,
}
