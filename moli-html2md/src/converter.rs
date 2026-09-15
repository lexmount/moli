//! Convert an existing DOM to Markdown without parsing HTML or modifying nodes.
//!
//! Implement [`Dom`] for your tree, then pass a borrowed tree and a root node to
//! [`convert`]. Traversal and formatting use explicit stacks, including code,
//! lists, quotes, and tables; DOM depth never becomes Rust call-stack depth.

use crate::{Dom, Options, machine};

/// A reusable converter. All per-document state lives inside [`Self::convert`].
#[derive(Clone, Debug, Default)]
pub struct Converter {
    options: Options,
}

impl Converter {
    pub fn new(options: Options) -> Self {
        Self { options }
    }

    /// Convert a subtree, borrowing the DOM for this call only.
    pub fn convert<D: Dom + ?Sized>(&self, dom: &D, root: D::NodeId) -> String {
        machine::convert(dom, root, &self.options)
    }
}

/// Convert a subtree using the default options.
pub fn convert<D: Dom + ?Sized>(dom: &D, root: D::NodeId) -> String {
    Converter::default().convert(dom, root)
}
