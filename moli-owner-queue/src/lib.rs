//! Owner-lane queue primitives shared by renderer scheduling code.
//!
//! This crate keeps single-owner task arrival and wake tracking independent from
//! the V8 renderer so parser, document, and runtime scheduling can reuse the
//! same FIFO/wake behavior without depending on JS internals.

mod owner_ready_task_source;
mod owner_task_source;
mod owner_wake_queue;

pub use owner_ready_task_source::{
    OwnerReadyTaskRoute, OwnerReadyTaskRouteClosed, OwnerReadyTaskSource, OwnerTaskReadySignal,
};
pub use owner_task_source::OwnerTaskSource;
pub use owner_wake_queue::OwnerWakeQueue;
