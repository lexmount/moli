//! Input data, scroll coalescing, and wheel transactions for browser hosts.
//!
//! Hosts supply timestamps and targets, perform input dispatch, and acknowledge the result.
//! [WheelInputController] exposes the deadline for the host's scheduler.

mod scroll;
mod types;
mod wheel;

pub use scroll::{ScrollAction, ScrollDeltaMode};
pub use types::{ClickAction, InputModifiers, MouseButton, Point};
pub use wheel::{
    PreparedWheel, WheelDispatchOutcome, WheelInput, WheelInputController, WheelPhase, WheelSource,
};
