//! One-shot batching and semantic compaction for browser actions.
//!
//! The queue owns no timer. A host admits actions with [`ActionWindow::push`],
//! arms at most one timer for [`ActionWindow::next_deadline`], and obtains work
//! with [`ActionWindow::take_due`]. Read barriers such as screenshots call
//! [`ActionWindow::flush`] first.
//!
//! A returned [`ActionBatch`] is an execution boundary: apply all of its
//! [`PlannedAction`] values in order, then commit derived work (for example
//! IntersectionObserver delivery, layout, and rendering) once.

mod action;
mod batch;
mod window;

pub use action::{
    ClickAction, InputModifiers, MouseButton, Point, ScrollAction, ScrollDeltaMode, WindowAction,
};
pub use batch::{
    ActionBarrier, ActionBatch, ActionBatchCause, ActionBatchId, ActionSequence, PlannedAction,
    ScheduledAction, ScrollRun,
};
pub use window::{ActionAdmission, ActionCompaction, ActionWindow, AdmissionState};

#[cfg(test)]
mod tests;
