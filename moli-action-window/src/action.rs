use moli_input::{ClickAction, ScrollAction};

/// An action admitted to a batching window.
///
/// `Ordered` is an extension point for renderer-specific actions. Ordered
/// actions are retained verbatim and act as ordering boundaries between scroll
/// runs.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowAction<O = ()> {
    Scroll(ScrollAction),
    Click(ClickAction),
    Ordered(O),
}
