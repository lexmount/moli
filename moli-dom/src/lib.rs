//! Native DOM model and host mutation layer for Moli.
//!
//! This crate owns the browser-facing document tree, node types, and host-side
//! mutation utilities shared by parser, selector, and renderer code.

pub mod accessibility;
pub mod custom_elements;
pub mod forms;
pub mod html_microsyntax;
pub mod native;

pub use native::{NativeNodeId as NodeId, NodeData};

// Temporary self-alias so the extracted module tree can keep compiling while
// `moli` migrates to the new crate boundary.
pub mod dom {
    pub use crate::{NodeData, NodeId, native};
}
