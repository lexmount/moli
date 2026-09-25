//! Native history owned by a Window, independent of JavaScript objects.
//!
//! `moli-session-history` remains the authority for the traversable's joint
//! steps. This crate owns the Window's entries, cursor, and serialized state.
//! Bindings may retain an entry after it leaves the history (for example for
//! NavigationHistoryEntry.dispose), without retaining its former Window.

mod entry;
mod window;

pub use entry::{HistoryEntry, HistoryEntryRef, ScrollRestoration};
pub use moli_structured_clone::SerializedScriptValue;
pub use window::WindowHistory;

#[cfg(test)]
mod tests;
