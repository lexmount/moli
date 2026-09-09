use crate::geometry::AbsoluteAxis;
use crate::style::AvailableSpace;
use crate::tree::{ChildLayoutInput, LayoutPartialTree, LayoutPartialTreeExt, NodeId};

/// Resolves the fit-content width used by an auto-width absolutely positioned box.
///
/// CSS 2 defines this as `min(max(min-content, available), max-content)`. A single
/// measurement with definite available space is insufficient: nested block and flex
/// containers may return their max-content contribution while they are being measured.
#[inline]
pub(crate) fn fit_content_width(
    tree: &mut impl LayoutPartialTree,
    node: NodeId,
    mut inputs: ChildLayoutInput,
    available_width: f32,
) -> f32 {
    inputs.available_space.width = AvailableSpace::MinContent;
    let min_content = tree.measure_child_size(node, inputs, AbsoluteAxis::Horizontal);
    inputs.available_space.width = AvailableSpace::MaxContent;
    let max_content = tree.measure_child_size(node, inputs, AbsoluteAxis::Horizontal);

    available_width.max(0.0).max(min_content).min(max_content)
}
