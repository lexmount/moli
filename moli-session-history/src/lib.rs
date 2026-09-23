//! The pure state machine of a traversable's joint session history.
//!
//! This crate owns identities, joint steps, and validated traversal plans. It
//! does not own Documents, renderer objects, URLs, event dispatch, or protocol
//! projections. A renderer admits and schedules every participant before
//! committing the plan. Timeline mutations or changed participant transitions
//! invalidate it; an unchanged context can attach without canceling traversal.

mod entry;
mod identity;
mod joint;
mod traversal;

pub use entry::{SessionHistoryEntry, SessionHistoryPosition};
pub use identity::{
    NavigationHistoryDocumentId, NavigationHistoryEntryKey, SessionHistoryContextId,
    SessionHistoryRevision, SessionHistoryStepId,
};
pub use joint::JointSessionHistory;
pub use traversal::{SessionHistoryContextChange, SessionHistoryTraversalPlan};

#[cfg(test)]
mod tests;
