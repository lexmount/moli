//! Input data and scroll coalescing rules for browser hosts.

mod scroll;
mod types;

pub use scroll::{ScrollAction, ScrollDeltaMode};
pub use types::{ClickAction, InputModifiers, MouseButton, Point};
