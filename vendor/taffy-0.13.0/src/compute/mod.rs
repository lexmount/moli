//! Low-level access to the layout algorithms themselves. For a higher-level API, see the [`TaffyTree`](crate::TaffyTree) struct.
//!
//! ### Layout functions
//!
//! The layout functions all take an [`&mut impl LayoutPartialTree`](crate::LayoutPartialTree) parameter, which represents a single container node and it's direct children.
//!
//! | Function                          | Purpose                                                                                                                                                                                            |
//! | ---                               | ---                                                                                                                                                                                                |
//! | [`compute_flexbox_layout`]        | Layout a Flexbox container and it's direct children                                                                                                                                                |
//! | [`compute_grid_layout`]           | Layout a CSS Grid container and it's direct children                                                                                                                                               |
//! | [`compute_block_layout`]          | Layout a Block container and it's direct children                                                                                                                                                  |
//! | [`compute_leaf_layout`]           | Applies common properties like padding/border/aspect-ratio to a node before deferring to a passed closure to determine it's size. Can be applied to nodes like text or image nodes.                |
//! | [`compute_root_layout`]           | Layout the root node of a tree (regardless of it's layout mode). This function is typically called once to begin a layout run.                                                                     |                                                                      |
//! | [`compute_hidden_layout`]         | Mark a node as hidden during layout (like `Display::None`)                                                                                                                                         |
//! | [`compute_cached_layout`]         | Attempts to find a cached layout for the specified node and layout inputs. Uses the provided closure to compute the layout (and then stores the result in the cache) if no cached layout is found. |
//!
//! ### Other functions
//!
//! | Function                          | Requires                                                                                                                                                                                           | Purpose                                                              |
//! | ---                               | ---                                                                                                                                                                                                | ---                                                                  |
//! | [`round_layout`]                  | [`RoundTree`]                                                                                                                                                                                      | Round a tree of float-valued layouts to integer pixels               |
//! | [`round_layout_with_scale_factor`]| [`RoundTree`]                                                                                                                                                                                      | Round a tree to a caller-selected subpixel grid                      |
//! | [`print_tree`](crate::print_tree) | [`PrintTree`](crate::PrintTree)                                                                                                                                                                    | Print a debug representation of a node tree and it's computed layout |
//!
pub(crate) mod common;
pub(crate) mod leaf;

#[cfg(feature = "block_layout")]
pub(crate) mod block;

#[cfg(feature = "float_layout")]
pub(crate) mod float;

#[cfg(feature = "flexbox")]
pub(crate) mod flexbox;

#[cfg(feature = "grid")]
pub(crate) mod grid;

pub use leaf::{
    compute_leaf_layout, compute_leaf_layout_with_aspect_ratio, compute_leaf_layout_with_aspect_ratio_and_writing_mode,
    compute_leaf_layout_with_context, compute_leaf_layout_with_scrollbar_insets, LeafLayoutContext,
};

#[cfg(feature = "block_layout")]
pub use self::block::{compute_block_layout, BlockContext, BlockFormattingContext};

#[cfg(feature = "flexbox")]
pub use self::flexbox::compute_flexbox_layout;

#[cfg(feature = "grid")]
pub use self::grid::compute_grid_layout;

#[cfg(feature = "float_layout")]
pub use self::float::{BfcSlot, ContentSlot, FloatContext, FloatIntrinsicWidthCalculator};

use crate::geometry::{Line, Point, Size};
use crate::style::{AvailableSpace, CoreStyle};
use crate::tree::{
    ChildLayoutInput, IntrinsicSizeResult, Layout, LayoutInput, LayoutOutput, LayoutPartialTree, LayoutPartialTreeExt,
    NodeId, RoundTree, RunMode, SizingMode, SizingPurpose,
};
use crate::util::debug::{debug_log, debug_log_node, debug_pop_node, debug_push_node};
use crate::util::sys::round;
use crate::util::ResolveOrZero;
use crate::{CacheTree, MaybeMath, MaybeResolve, RequestedAxis};

use self::common::aspect_ratio::{resolve_size_constraints, SizeConstraintInput, TransferredSizesMode};
pub use self::common::intrinsic_size::{
    resolve_intrinsic_width_inputs, resolve_intrinsic_width_inputs_with_provenance, ResolvedIntrinsicWidthInputs,
};

/// Compute layout for the root node in the tree
pub fn compute_root_layout(tree: &mut impl LayoutPartialTree, root: NodeId, available_space: Size<AvailableSpace>) {
    let root_writing_mode = tree.get_writing_mode(root);
    // A block root only falls back to filling definite available space when
    // its preferred width is auto. Resolve intrinsic sizing keywords before
    // that fallback so they remain explicit used sizes at the root seam.
    let root_inputs = resolve_intrinsic_width_inputs(
        tree,
        root,
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            axis: RequestedAxis::Both,
            block_auto_behavior: crate::AutoSizeBehavior::FitContent,
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: available_space.into_options(),
            parent_writing_mode: root_writing_mode,
            available_space,
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    let mut known_dimensions = root_inputs.known_dimensions;
    let percentage_basis = root_inputs.constraint_space(root_writing_mode).margin_padding_percentage_basis();

    #[cfg(feature = "block_layout")]
    {
        use crate::BoxSizing;

        let parent_size = available_space.into_options();
        let aspect_ratio = tree.get_resolved_aspect_ratio(root);
        let style = tree.get_core_container_style(root);

        if style.is_block() {
            // Pull these out earlier to avoid borrowing issues
            let margin = style.margin().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
            let padding = style.padding().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
            let border = style.border().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
            let padding_border_size = (padding + border).sum_axes();
            let box_sizing = style.box_sizing();
            let box_sizing_adjustment =
                if box_sizing == BoxSizing::ContentBox { padding_border_size } else { Size::ZERO };

            let raw_size = style.size();
            let resolved = resolve_size_constraints(SizeConstraintInput {
                size: raw_size
                    .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                    .maybe_add(box_sizing_adjustment),
                min_size: style
                    .min_size()
                    .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                    .maybe_add(box_sizing_adjustment),
                max_size: style
                    .max_size()
                    .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                    .maybe_add(box_sizing_adjustment),
                size_is_auto: raw_size.map(|dimension| dimension.is_auto()),
                writing_mode: root_writing_mode,
                block_auto_behavior: root_inputs.block_auto_behavior,
                transferred_sizes_mode: TransferredSizesMode::Normal,
                aspect_ratio,
                padding_border: padding_border_size,
            });
            let min_size = resolved.min_size;
            let max_size = resolved.max_size;
            let clamped_style_size = resolved.size.maybe_clamp(min_size, max_size);

            // If both min and max in a given axis are set and max <= min then this determines the size in that axis
            let min_max_definite_size = min_size.zip_map(max_size, |min, max| match (min, max) {
                (Some(min), Some(max)) if max <= min => Some(min),
                _ => None,
            });

            // Block nodes automatically stretch fit their width to fit available space if available space is definite
            let available_space_based_size = Size {
                width: available_space.width.into_option().maybe_sub(margin.horizontal_axis_sum()),
                height: None,
            };

            let styled_based_known_dimensions = known_dimensions
                .or(min_max_definite_size)
                .or(clamped_style_size)
                .or(available_space_based_size)
                .maybe_max(padding_border_size);

            known_dimensions = styled_based_known_dimensions;
        }
    }

    // Recursively compute node layout
    let output = tree.perform_child_layout(
        root,
        ChildLayoutInput::new(
            known_dimensions,
            available_space.into_options(),
            root_writing_mode,
            available_space,
            SizingMode::InherentSize,
            Line::FALSE,
        ),
    );
    let scrollbar_size = tree.get_scrollbar_insets(root).sum_axes();
    let style = tree.get_core_container_style(root);
    let padding = style.padding().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let border = style.border().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let margin = style.margin().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let location = Point {
        x: if style.direction().is_rtl() {
            available_space.width.into_option().map_or(0.0, |available_width| available_width - output.size.width)
        } else {
            0.0
        },
        y: 0.0,
    };
    drop(style);

    tree.set_unrounded_layout(
        root,
        &Layout {
            order: 0,
            location,
            size: output.size,
            #[cfg(feature = "content_size")]
            content_size: output.content_size,
            scrollbar_size,
            padding,
            border,
            // TODO: support auto margins for root node?
            margin,
        },
    );
}

/// Attempts to find a cached layout for the specified node and layout inputs.
///
/// Uses the provided closure to compute the layout (and then stores the result in the cache) if no cached layout is found.
#[inline(always)]
pub fn compute_cached_layout<Tree: CacheTree + LayoutPartialTree + ?Sized, ComputeFunction>(
    tree: &mut Tree,
    node: NodeId,
    inputs: LayoutInput,
    compute_uncached: ComputeFunction,
) -> LayoutOutput
where
    ComputeFunction: FnOnce(&mut Tree, NodeId, LayoutInput) -> LayoutOutput,
{
    debug_push_node!(node);

    // First we check if we have a cached result for the given input
    let cache_entry = tree.cache_get(node, &inputs);
    if let Some(cached_size_and_baselines) = cache_entry {
        debug_log_node!(inputs);
        debug_log!("RESULT (CACHED)", dbg:cached_size_and_baselines.size);
        debug_pop_node!();
        return cached_size_and_baselines;
    }

    debug_log_node!(inputs);

    let mut computed_size_and_baselines = compute_uncached(tree, node, inputs);

    computed_size_and_baselines.set_block_constraint_dependency(node_block_constraint_dependency(
        tree,
        node,
        inputs,
        computed_size_and_baselines.block_constraint_dependency(),
    ));

    // Cache result
    tree.cache_store(node, &inputs, computed_size_and_baselines);

    debug_log!("RESULT", dbg:computed_size_and_baselines.size);
    debug_pop_node!();

    computed_size_and_baselines
}

/// Compute or retrieve a dedicated intrinsic size result for a node.
///
/// This is the sizing counterpart to [`compute_cached_layout`]. It keeps
/// measurement provenance out of the public layout result protocol while the
/// formatting-context implementations are incrementally moved off the legacy
/// combined dispatcher.
pub fn compute_cached_size<Tree: CacheTree + LayoutPartialTree + ?Sized, ComputeFunction>(
    tree: &mut Tree,
    node: NodeId,
    inputs: LayoutInput,
    compute_uncached: ComputeFunction,
) -> IntrinsicSizeResult
where
    ComputeFunction: FnOnce(&mut Tree, NodeId, LayoutInput) -> IntrinsicSizeResult,
{
    debug_assert_eq!(inputs.run_mode, RunMode::ComputeSize);
    debug_push_node!(node);

    if let Some(cached_result) = tree.cache_get_size(node, &inputs) {
        debug_log_node!(inputs);
        debug_log!("RESULT (CACHED)", dbg:cached_result.size);
        debug_pop_node!();
        return cached_result;
    }

    debug_log_node!(inputs);
    let mut result = compute_uncached(tree, node, inputs);
    result.depends_on_block_constraints =
        node_block_constraint_dependency(tree, node, inputs, result.depends_on_block_constraints);
    tree.cache_store_size(node, &inputs, result);

    debug_log!("RESULT", dbg:result.size);
    debug_pop_node!();
    result
}

/// Gate a formatting algorithm's reported dependency at the node sizing
/// boundary.
///
/// Content-only probes forward descendant dependency unchanged. Inherent-size
/// probes report it to their parent only when this node consumes the parent's
/// block constraint and the requested result can observe the change.
fn node_block_constraint_dependency(
    tree: &(impl LayoutPartialTree + ?Sized),
    node: NodeId,
    inputs: LayoutInput,
    reported_dependency: bool,
) -> bool {
    if inputs.run_mode != RunMode::ComputeSize {
        return false;
    }
    if inputs.sizing_mode != SizingMode::InherentSize {
        return reported_dependency;
    }

    let writing_mode = tree.get_writing_mode(node);
    let has_aspect_ratio = tree.get_resolved_aspect_ratio(node).is_some();
    let style_depends_on_parent_block_size = {
        let style = tree.get_core_container_style(node);
        let size = writing_mode.to_logical(style.size());
        let min_size = writing_mode.to_logical(style.min_size());
        let max_size = writing_mode.to_logical(style.max_size());
        [size.block_size, min_size.block_size, max_size.block_size]
            .into_iter()
            .any(|value| value.may_have_percentage_dependence() || value.is_stretch())
    };
    let requested_block_size = inputs.axis.contains(writing_mode.block_axis());
    style_depends_on_parent_block_size && (requested_block_size || has_aspect_ratio || reported_dependency)
}

/// Rounds the calculated layout to exact pixel values
///
/// In order to ensure that no gaps in the layout are introduced we:
///   - Always round based on the cumulative x/y coordinates (relative to the viewport) rather than
///     parent-relative coordinates
///   - Compute width/height by first rounding the top/bottom/left/right and then computing the difference
///     rather than rounding the width/height directly
///
/// See <https://github.com/facebook/yoga/commit/aa5b296ac78f7a22e1aeaf4891243c6bb76488e2> for more context
///
/// In order to prevent innacuracies caused by rounding already-rounded values, we read from `unrounded_layout`
/// and write to `final_layout`.
pub fn round_layout(tree: &mut impl RoundTree, node_id: NodeId) {
    round_layout_with_scale_factor(tree, node_id, 1.0);
}

/// Rounds the calculated layout to a caller-selected subpixel grid.
///
/// `scale_factor` is the number of grid steps per layout unit. For example,
/// `1.0` rounds to integer layout units and `64.0` rounds to 1/64th of a
/// layout unit. The scale factor must be finite and greater than zero.
///
/// Like [`round_layout`], endpoints are rounded in the cumulative coordinate
/// space so that rounded sizes are derived from rounded edges rather than by
/// rounding sizes independently.
pub fn round_layout_with_scale_factor(tree: &mut impl RoundTree, node_id: NodeId, scale_factor: f32) {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "layout rounding scale factor must be finite and greater than zero"
    );
    round_layout_inner(tree, node_id, 0.0, 0.0, scale_factor);

    /// Recursive function to apply rounding to all descendents
    fn round_layout_inner(
        tree: &mut impl RoundTree,
        node_id: NodeId,
        cumulative_x: f32,
        cumulative_y: f32,
        scale_factor: f32,
    ) {
        let unrounded_layout = tree.get_unrounded_layout(node_id);
        let mut layout = unrounded_layout;

        let cumulative_x = cumulative_x + unrounded_layout.location.x;
        let cumulative_y = cumulative_y + unrounded_layout.location.y;

        layout.location.x = round_to_scale(unrounded_layout.location.x, scale_factor);
        layout.location.y = round_to_scale(unrounded_layout.location.y, scale_factor);
        layout.size.width = round_to_scale(cumulative_x + unrounded_layout.size.width, scale_factor)
            - round_to_scale(cumulative_x, scale_factor);
        layout.size.height = round_to_scale(cumulative_y + unrounded_layout.size.height, scale_factor)
            - round_to_scale(cumulative_y, scale_factor);
        layout.scrollbar_size.width = round_to_scale(unrounded_layout.scrollbar_size.width, scale_factor);
        layout.scrollbar_size.height = round_to_scale(unrounded_layout.scrollbar_size.height, scale_factor);
        layout.border.left = round_to_scale(cumulative_x + unrounded_layout.border.left, scale_factor)
            - round_to_scale(cumulative_x, scale_factor);
        layout.border.right = round_to_scale(cumulative_x + unrounded_layout.size.width, scale_factor)
            - round_to_scale(cumulative_x + unrounded_layout.size.width - unrounded_layout.border.right, scale_factor);
        layout.border.top = round_to_scale(cumulative_y + unrounded_layout.border.top, scale_factor)
            - round_to_scale(cumulative_y, scale_factor);
        layout.border.bottom = round_to_scale(cumulative_y + unrounded_layout.size.height, scale_factor)
            - round_to_scale(
                cumulative_y + unrounded_layout.size.height - unrounded_layout.border.bottom,
                scale_factor,
            );
        layout.padding.left = round_to_scale(cumulative_x + unrounded_layout.padding.left, scale_factor)
            - round_to_scale(cumulative_x, scale_factor);
        layout.padding.right = round_to_scale(cumulative_x + unrounded_layout.size.width, scale_factor)
            - round_to_scale(cumulative_x + unrounded_layout.size.width - unrounded_layout.padding.right, scale_factor);
        layout.padding.top = round_to_scale(cumulative_y + unrounded_layout.padding.top, scale_factor)
            - round_to_scale(cumulative_y, scale_factor);
        layout.padding.bottom = round_to_scale(cumulative_y + unrounded_layout.size.height, scale_factor)
            - round_to_scale(
                cumulative_y + unrounded_layout.size.height - unrounded_layout.padding.bottom,
                scale_factor,
            );

        #[cfg(feature = "content_size")]
        round_content_size(&mut layout, unrounded_layout.content_size, cumulative_x, cumulative_y, scale_factor);

        tree.set_final_layout(node_id, &layout);

        let child_count = tree.child_count(node_id);
        for index in 0..child_count {
            let child = tree.get_child_id(node_id, index);
            round_layout_inner(tree, child, cumulative_x, cumulative_y, scale_factor);
        }
    }

    #[inline(always)]
    fn round_to_scale(value: f32, scale_factor: f32) -> f32 {
        round(value * scale_factor) / scale_factor
    }

    #[cfg(feature = "content_size")]
    #[inline(always)]
    /// Round content size variables.
    /// This is split into a separate function to make it easier to feature flag.
    fn round_content_size(
        layout: &mut Layout,
        unrounded_content_size: Size<f32>,
        cumulative_x: f32,
        cumulative_y: f32,
        scale_factor: f32,
    ) {
        layout.content_size.width = round_to_scale(cumulative_x + unrounded_content_size.width, scale_factor)
            - round_to_scale(cumulative_x, scale_factor);
        layout.content_size.height = round_to_scale(cumulative_y + unrounded_content_size.height, scale_factor)
            - round_to_scale(cumulative_y, scale_factor);
    }
}

/// Creates a layout for this node and its children, recursively.
/// Each hidden node has zero size and is placed at the origin
pub fn compute_hidden_layout(tree: &mut (impl LayoutPartialTree + CacheTree), node: NodeId) -> LayoutOutput {
    // Clear cache and set zeroed-out layout for the node
    tree.cache_clear(node);
    tree.set_unrounded_layout(node, &Layout::with_order(0));

    // Perform hidden layout on all children
    for index in 0..tree.child_count(node) {
        let child_id = tree.get_child_id(node, index);
        tree.compute_child_layout(child_id, LayoutInput::HIDDEN);
    }

    LayoutOutput::HIDDEN
}

/// A module for unified re-exports of detailed layout info structs, used by low level API
#[cfg(feature = "detailed_layout_info")]
pub mod detailed_info {
    #[cfg(feature = "grid")]
    pub use super::grid::{DetailedGridInfo, DetailedGridItemsInfo, DetailedGridTracksInfo};
}

#[cfg(test)]
mod tests {
    use super::{compute_hidden_layout, round_layout_with_scale_factor};
    use crate::geometry::{Point, Size};
    use crate::style::{Display, Style};
    use crate::{Layout, NodeId, RoundTree, TaffyTree, TraversePartialTree, TraverseTree};

    struct RoundNode {
        children: Vec<NodeId>,
        unrounded: Layout,
        final_layout: Layout,
    }

    struct TestRoundTree(Vec<RoundNode>);

    impl TraversePartialTree for TestRoundTree {
        type ChildIter<'a> = core::iter::Copied<core::slice::Iter<'a, NodeId>>;

        fn child_ids(&self, parent_node_id: NodeId) -> Self::ChildIter<'_> {
            self.0[usize::from(parent_node_id)].children.iter().copied()
        }

        fn child_count(&self, parent_node_id: NodeId) -> usize {
            self.0[usize::from(parent_node_id)].children.len()
        }

        fn get_child_id(&self, parent_node_id: NodeId, child_index: usize) -> NodeId {
            self.0[usize::from(parent_node_id)].children[child_index]
        }
    }

    impl TraverseTree for TestRoundTree {}

    impl RoundTree for TestRoundTree {
        fn get_unrounded_layout(&self, node_id: NodeId) -> Layout {
            self.0[usize::from(node_id)].unrounded
        }

        fn set_final_layout(&mut self, node_id: NodeId, layout: &Layout) {
            self.0[usize::from(node_id)].final_layout = *layout;
        }
    }

    #[test]
    fn scaled_rounding_preserves_subpixel_geometry() {
        let root = NodeId::new(0);
        let child = NodeId::new(1);
        let layout = |x, width| Layout {
            location: Point { x, y: 0.0 },
            size: Size { width, height: 10.0 },
            ..Layout::with_order(0)
        };
        let mut tree = TestRoundTree(vec![
            RoundNode { children: vec![child], unrounded: layout(0.2, 100.3), final_layout: Layout::with_order(0) },
            RoundNode { children: Vec::new(), unrounded: layout(0.333, 10.333), final_layout: Layout::with_order(0) },
        ]);

        round_layout_with_scale_factor(&mut tree, root, 64.0);

        assert_eq!(tree.0[0].final_layout.location.x, 0.203125);
        assert_eq!(tree.0[0].final_layout.size.width, 100.296875);
        assert_eq!(tree.0[1].final_layout.location.x, 0.328125);
        assert_eq!(tree.0[1].final_layout.size.width, 10.328125);
    }

    #[test]
    fn hidden_layout_should_hide_recursively() {
        let mut taffy: TaffyTree<()> = TaffyTree::new();

        let style: Style = Style { display: Display::Flex, size: Size::from_lengths(50.0, 50.0), ..Default::default() };

        let grandchild_00 = taffy.new_leaf(style.clone()).unwrap();
        let grandchild_01 = taffy.new_leaf(style.clone()).unwrap();
        let child_00 = taffy.new_with_children(style.clone(), &[grandchild_00, grandchild_01]).unwrap();

        let grandchild_02 = taffy.new_leaf(style.clone()).unwrap();
        let child_01 = taffy.new_with_children(style.clone(), &[grandchild_02]).unwrap();

        let root = taffy
            .new_with_children(
                Style { display: Display::None, size: Size::from_lengths(50.0, 50.0), ..Default::default() },
                &[child_00, child_01],
            )
            .unwrap();

        compute_hidden_layout(&mut taffy.as_layout_tree(), root);

        // Whatever size and display-mode the nodes had previously,
        // all layouts should resolve to ZERO due to the root's DISPLAY::NONE

        for node in [root, child_00, child_01, grandchild_00, grandchild_01, grandchild_02] {
            let layout = taffy.layout(node).unwrap();
            assert_eq!(layout.size, Size::zero());
            assert_eq!(layout.location, Point::zero());
        }
    }
}
