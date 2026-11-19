//! Performance entry values, lists and legacy timing snapshots.
//! Collection, navigation lifecycle projection and observer delivery are host work.
pub mod entries;
pub mod entry_list;
pub mod legacy;
pub mod long_task;
pub mod memory;
pub mod name;
pub mod observer;
pub mod slots;
pub mod user_timing;
pub use entries::{
    performance_entry_slot_number, performance_entry_slot_string, performance_entry_slot_value,
    set_performance_entry_slot_number,
};
pub use name::PerformanceEntryName;
pub use slots::*;

pub mod event_counts;
