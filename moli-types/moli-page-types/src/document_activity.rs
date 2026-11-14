//! Native activity state exposed by a Window's Document.
//!
//! This is renderer state rather than a document-start script.  The values
//! are updated synchronously by the owning Page command; observable events
//! are delivered on the Page rendering-update task source afterward.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentActivity {
    pub focused: bool,
    pub visible: bool,
}

impl DocumentActivity {
    pub const fn new(focused: bool, visible: bool) -> Self {
        Self { focused, visible }
    }
}

impl Default for DocumentActivity {
    fn default() -> Self {
        Self::new(true, true)
    }
}
