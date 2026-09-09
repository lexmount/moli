//! Computes the CSS block layout algorithm in the case that the block container being laid out contains only block-level boxes
use crate::geometry::{AbsoluteAxis, Line, Point, Rect, Size};
use crate::style::{AvailableSpace, CoreStyle, LengthPercentageAuto, Overflow, Position};
use crate::style_helpers::TaffyMaxContent;
use crate::tree::{
    ChildLayoutInput, CollapsibleMarginSet, Layout, LayoutInput, LayoutOutput, RunMode, SizingMode, SizingPurpose,
};
use crate::tree::{LayoutPartialTree, LayoutPartialTreeExt, NodeId};
use crate::util::debug::debug_log;
use crate::util::sys::f32_max;
use crate::util::sys::Vec;
use crate::util::MaybeMath;
use crate::util::{MaybeResolve, ResolveOrZero};
use crate::{
    AutoSizeBehavior, BlockContainerStyle, BlockItemStyle, BoxGenerationMode, BoxSizing, Direction,
    LayoutBlockContainer, RequestedAxis, TextAlign, WritingMode,
};

use super::common::absolute::fit_content_width;
use super::common::aspect_ratio::{
    resolve_size_constraints, ResolvedAxisConstraints, SizeConstraintInput, TransferredSizesMode,
};
use super::common::intrinsic_size::{
    measure_content_based_block_size, resolve_intrinsic_width_constraints, BlockSizeProperties, ContentBasedBlockSize,
};
use super::common::used_size::resolve_used_size;

#[cfg(feature = "float_layout")]
use super::float::{BfcSlot, ContentSlot, FloatContext, FloatIntrinsicWidthCalculator};
#[cfg(feature = "float_layout")]
use crate::{Clear, Float, FloatDirection};

/// Context for positioning Block and Float boxes within a Block Formatting Context
pub struct BlockFormattingContext {
    /// The float positioning context that handles positioning floats within this Block Formatting Context
    #[cfg(feature = "float_layout")]
    float_context: FloatContext,
}

impl Default for BlockFormattingContext {
    fn default() -> Self {
        Self {
            #[cfg(feature = "float_layout")]
            float_context: FloatContext::new(),
        }
    }
}

impl BlockFormattingContext {
    /// Create a new `BlockFormattingContext` with the specified width constraint
    pub fn new() -> Self {
        Default::default()
    }

    /// Create an initial `BlockContext` for this `BlockFormattingContext`
    pub fn root_block_context(&mut self) -> BlockContext<'_> {
        BlockContext {
            bfc: self,
            y_offset: 0.0,
            insets: [0.0, 0.0],
            content_box_insets: [0.0, 0.0],
            float_content_contribution: 0.0,
            is_root: true,
            #[cfg(feature = "float_layout")]
            adjoining_floats: [false, false],
            #[cfg(feature = "float_layout")]
            top_adjoining_floats: None,
        }
    }
}

/// Context for each individual Block within a Block Formatting Context
///
/// Contains a mutable reference to the BlockFormattingContext + block-specific data
pub struct BlockContext<'bfc> {
    /// A mutable reference to the root BlockFormatttingContext that this BlockContext belongs to
    bfc: &'bfc mut BlockFormattingContext,
    /// The y-offset of the border-top of the block node, relative to the to the border-top of the
    /// root node of the Block Formatting Context it belongs to.
    y_offset: f32,
    /// The x-inset of the border-box in from each side of the block node, relative to the root node of the Block Formatting Context it belongs to.
    insets: [f32; 2],
    /// The x-insets of the content box
    content_box_insets: [f32; 2],
    /// The height that floats take up in the element
    float_content_contribution: f32,
    /// Whether the node is the root of the Block Formatting Context is belongs to.
    is_root: bool,
    /// Whether a float has been placed (on each side) whose position adjoins the current
    /// margin-collapse strut of this block (i.e. whose final position can still be moved by
    /// margins that collapse into that strut). Such floats force clearance on cleared elements
    /// whose margins adjoin the same strut.
    #[cfg(feature = "float_layout")]
    adjoining_floats: [bool; 2],
    /// The value of `adjoining_floats` frozen at the first point at which in-flow content was
    /// committed within this block (resolving the position of the block's top margin strut).
    /// `None` if no in-flow content has been committed yet.
    #[cfg(feature = "float_layout")]
    top_adjoining_floats: Option<[bool; 2]>,
}

impl BlockContext<'_> {
    /// Create a sub-`BlockContext` for a child block node
    pub fn sub_context(&mut self, additional_y_offset: f32, insets: [f32; 2]) -> BlockContext<'_> {
        let insets = [self.insets[0] + insets[0], self.insets[1] + insets[1]];
        BlockContext {
            bfc: self.bfc,
            y_offset: self.y_offset + additional_y_offset,
            insets,
            content_box_insets: insets,
            float_content_contribution: 0.0,
            is_root: false,
            // Floats adjoining the parent's current strut also adjoin this block's top strut
            // (if this block's top margin collapses with its first child's, which is checked separately)
            #[cfg(feature = "float_layout")]
            adjoining_floats: self.adjoining_floats,
            #[cfg(feature = "float_layout")]
            top_adjoining_floats: None,
        }
    }

    /// Returns whether this block is the root block of it's Block Formatting Context
    pub fn is_bfc_root(&self) -> bool {
        self.is_root
    }
}

#[cfg(feature = "float_layout")]
impl BlockContext<'_> {
    /// Set the width of the overall Block Formatting Context. This is used to resolve positions
    /// that are relative to the right of the context such as right-floated boxes.
    ///
    /// Sub-blocks within a Block Formatting Context should use the `Self::sub_context` method to create
    /// a sub-`BlockContext` with `insets` instead.
    pub fn set_width(&mut self, available_width: f32) {
        self.bfc.float_context.set_width(available_width);
    }

    /// Set the x-axis content-box insets of the `BlockContext`. These are the difference between the border-box
    /// and the content-box of the box (padding + border + scrollbar_gutter).
    pub fn apply_content_box_inset(&mut self, content_box_x_insets: [f32; 2]) {
        self.content_box_insets[0] = self.insets[0] + content_box_x_insets[0];
        self.content_box_insets[1] = self.insets[1] + content_box_x_insets[1];
    }

    /// Whether the float context contains any floats
    #[inline(always)]
    pub fn has_floats(&self) -> bool {
        self.bfc.float_context.has_floats()
    }

    /// Whether the float context contains any floats that extend to or below min_y
    #[inline(always)]
    pub fn has_active_floats(&self, min_y: f32) -> bool {
        self.bfc.float_context.has_active_floats(min_y + self.y_offset)
    }

    /// Position a floated box with the context
    pub fn place_floated_box(
        &mut self,
        floated_box: Size<f32>,
        min_y: f32,
        direction: FloatDirection,
        clear: Clear,
        adjoins_unresolved_strut: bool,
    ) -> Point<f32> {
        if adjoins_unresolved_strut {
            self.adjoining_floats[direction as usize] = true;
        }
        let mut pos = self.bfc.float_context.place_floated_box(
            floated_box,
            min_y + self.y_offset,
            self.content_box_insets,
            direction,
            clear,
        );
        pos.y -= self.y_offset;
        pos.x -= self.insets[0];

        self.float_content_contribution = self.float_content_contribution.max(pos.y + floated_box.height);

        pos
    }

    /// Search a space suitable for laying out non-floated content into
    pub fn find_content_slot(&self, min_y: f32, clear: Clear, after: Option<usize>) -> ContentSlot {
        let mut slot =
            self.bfc.float_context.find_content_slot(min_y + self.y_offset, self.content_box_insets, clear, after);
        slot.y -= self.y_offset;
        slot.x -= self.insets[0];
        slot
    }

    /// Search for a space suitable for laying out a box that establishes an independent
    /// formatting context (whose border box must not overlap floats)
    pub fn find_bfc_slot(
        &self,
        min_y: f32,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        after: Option<usize>,
    ) -> BfcSlot {
        let mut slot = self.bfc.float_context.find_bfc_slot(
            min_y + self.y_offset,
            self.content_box_insets,
            margins,
            direction,
            clear,
            after,
        );
        slot.y -= self.y_offset;
        slot.x -= self.insets[0];
        slot
    }

    /// Get the bottom of lowest relevant float for the specific clear property
    pub fn cleared_threshold(&self, clear: Clear) -> Option<f32> {
        self.bfc.float_context.cleared_threshold(clear).map(|threshold| threshold - self.y_offset)
    }

    /// Whether a float that is adjoining the current margin-collapse strut has been placed
    /// on the side(s) relevant to the passed clear property
    pub fn has_adjoining_float(&self, clear: Clear) -> bool {
        match clear {
            Clear::Left => self.adjoining_floats[0],
            Clear::Right => self.adjoining_floats[1],
            Clear::Both => self.adjoining_floats[0] || self.adjoining_floats[1],
            Clear::None => false,
        }
    }

    /// Merge adjoining float flags propagated from a child block into this block's flags
    fn merge_adjoining_floats(&mut self, flags: [bool; 2]) {
        self.adjoining_floats[0] |= flags[0];
        self.adjoining_floats[1] |= flags[1];
    }

    /// Record that in-flow content has been committed within this block, resolving the position of
    /// the current margin-collapse strut. Floats placed before this point no longer adjoin the
    /// current strut. The flags for the block's top strut are frozen at the first commit.
    fn commit_strut(&mut self) {
        if self.top_adjoining_floats.is_none() {
            self.top_adjoining_floats = Some(self.adjoining_floats);
        }
        self.adjoining_floats = [false, false];
    }

    /// The adjoining float flags for this block's top margin strut: floats placed while the
    /// position of the block's top strut was still unresolved
    fn top_adjoining_floats(&self) -> [bool; 2] {
        self.top_adjoining_floats.unwrap_or(self.adjoining_floats)
    }

    /// Update the height that descendent floats with the height that floats consume
    /// within a particular child
    fn add_child_floated_content_height_contribution(&mut self, child_contribution: f32) {
        self.float_content_contribution = self.float_content_contribution.max(child_contribution);
    }

    /// Returns the height that descendent floats consume
    pub fn floated_content_height_contribution(&self) -> f32 {
        self.float_content_contribution
    }
}

#[cfg(not(feature = "float_layout"))]
impl BlockContext<'_> {
    #[inline(always)]
    /// Returns the height that descendent floats consume (always 0.0 when the float feature is disabled)
    fn float_content_contribution(&self) -> f32 {
        0.0
    }
}

use super::common::alignment::{apply_alignment_fallback, compute_alignment_offset};
#[cfg(feature = "content_size")]
use super::common::content_size::{compute_content_size_contribution, content_size_contribution_location};

/// Per-child data that is accumulated and modified over the course of the layout algorithm
struct BlockItem {
    /// The identifier for the associated node
    node_id: NodeId,

    /// The "source order" of the item. This is the index of the item within the children iterator,
    /// and controls the order in which the nodes are placed
    order: u32,

    /// Items that are tables don't have stretch sizing applied to them
    is_table: bool,

    /// Items that are replaced elements resolve an auto width to their intrinsic size
    /// rather than being stretch-sized
    /// <https://www.w3.org/TR/CSS22/visudet.html#block-replaced-width>
    is_replaced: bool,

    /// Whether this item is laid out by the block formatting algorithm.
    ///
    /// Inline-block baseline propagation uses a block child's last baseline,
    /// while other formatting contexts contribute their first baseline.
    uses_block_layout: bool,

    /// Whether the child is a non-independent block or inline node
    is_in_same_bfc: bool,

    #[cfg(feature = "float_layout")]
    /// The `float` style of the node
    float: Float,
    #[cfg(feature = "float_layout")]
    /// The `clear` style of the node
    clear: Clear,

    /// The base size of this item
    size: Size<Option<f32>>,
    /// The minimum allowable size of this item
    min_size: Size<Option<f32>>,
    /// The maximum allowable size of this item
    max_size: Size<Option<f32>>,
    /// Authored and ratio-transferred constraints for the logical block axis.
    ///
    /// The content-based automatic minimum is capped only by an authored
    /// maximum, so the used `min_size`/`max_size` pair is not sufficient here.
    block_axis_constraints: ResolvedAxisConstraints,

    /// The overflow style of the item
    overflow: Point<Overflow>,
    /// The total physical space occupied by the item's scrollbar gutters
    scrollbar_size: Size<f32>,

    /// The position style of the item
    position: Position,
    /// The final offset of this item
    inset: Rect<LengthPercentageAuto>,
    /// The margin of this item
    margin: Rect<LengthPercentageAuto>,
    /// The margin of this item
    padding: Rect<f32>,
    /// The margin of this item
    border: Rect<f32>,
    /// The sum of padding and border for this item
    padding_border_sum: Size<f32>,

    /// The computed border box size of this item
    computed_size: Size<f32>,
    /// The computed "static position" of this item. The static position is the position
    /// taking into account padding, border, margins, and scrollbar_gutters but not inset
    static_position: Point<f32>,
    /// Whether margins can be collapsed through this item
    can_be_collapsed_through: bool,

    /// Whether this item's intrinsic inline contribution depends on the
    /// containing block's block-size.
    depends_on_block_constraints: bool,

    /// Pending layout for in-flow non-floated items. Held back from `set_unrounded_layout` so the
    /// post-loop `align-content` pass in `compute_inner` can shift `location.y` before commit.
    final_layout: Option<Layout>,
}

/// Computes the layout of [`LayoutPartialTree`] according to the block layout algorithm
pub fn compute_block_layout(
    tree: &mut impl LayoutBlockContainer,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let writing_mode = tree.get_writing_mode(node_id);
    let percentage_basis = inputs.constraint_space(writing_mode).margin_padding_percentage_basis();
    let LayoutInput { known_dimensions, parent_size, run_mode, .. } = inputs;
    let resolved_aspect_ratio = tree.get_resolved_aspect_ratio(node_id);
    let style = tree.get_block_container_style(node_id);

    // Pull these out earlier to avoid borrowing issues
    let overflow = style.overflow();
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
    let aspect_ratio = if inputs.sizing_mode == SizingMode::InherentSize { resolved_aspect_ratio } else { None };
    let padding = style.padding().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let border = style.border().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let padding_border_size = (padding + border).sum_axes();
    let box_sizing = style.box_sizing();
    let box_sizing_adjustment = if box_sizing == BoxSizing::ContentBox { padding_border_size } else { Size::ZERO };

    let (min_size, max_size, clamped_style_size, preferred_inline_from_aspect_ratio) = match inputs.sizing_mode {
        SizingMode::ContentSize => (Size::NONE, Size::NONE, Size::NONE, false),
        SizingMode::InherentSize => {
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
                writing_mode,
                block_auto_behavior: inputs.block_auto_behavior,
                transferred_sizes_mode: TransferredSizesMode::Normal,
                aspect_ratio,
                padding_border: padding_border_size,
            });
            let min_size = resolved.min_size;
            let max_size = resolved.max_size;
            let preferred_size = resolved.size.maybe_clamp(min_size, max_size);
            (min_size, max_size, preferred_size, resolved.aspect_ratio_applied.width)
        }
    };

    drop(style);

    // If both min and max in a given axis are set and max <= min then this determines the size in that axis
    let min_max_definite_size = min_size.zip_map(max_size, |min, max| match (min, max) {
        (Some(min), Some(max)) if max <= min => Some(min),
        _ => None,
    });
    let applied_aspect_ratio = run_mode == RunMode::ComputeSize
        && known_dimensions.width.is_none()
        && min_max_definite_size.width.is_none()
        && preferred_inline_from_aspect_ratio;

    let styled_based_known_dimensions = resolve_used_size(
        known_dimensions,
        min_max_definite_size.or(clamped_style_size),
        Size::NONE,
        Size::NONE,
        padding_border_size,
    );

    // Short-circuit layout if the container's size is fully determined by the container's size and the run mode
    // is ComputeSize (and thus the container's size is all that we're interested in)
    if run_mode == RunMode::ComputeSize {
        if let Size { width: Some(width), height: Some(height) } = styled_based_known_dimensions {
            return LayoutOutput::from_outer_size(Size { width, height })
                .with_applied_aspect_ratio(applied_aspect_ratio);
        }

        // We can also short-circuit if the width is known and only the width has been requested.
        if inputs.axis == RequestedAxis::Horizontal {
            if let Some(width) = styled_based_known_dimensions.width {
                return LayoutOutput::from_outer_size(Size { width, height: 0.0 })
                    .with_applied_aspect_ratio(applied_aspect_ratio);
            }
        }
    }

    // Unwrap the block formatting context if one was passed, or else create a new one
    debug_log!("BLOCK");
    let output = match block_ctx {
        Some(inherited_bfc) if !is_scroll_container => compute_inner(
            tree,
            node_id,
            LayoutInput { known_dimensions: styled_based_known_dimensions, ..inputs },
            inherited_bfc,
        ),
        _ => {
            let mut root_bfc = BlockFormattingContext::new();
            let mut root_ctx = root_bfc.root_block_context();
            compute_inner(
                tree,
                node_id,
                LayoutInput { known_dimensions: styled_based_known_dimensions, ..inputs },
                &mut root_ctx,
            )
        }
    };
    output.with_applied_aspect_ratio(applied_aspect_ratio)
}

/// Computes the layout of [`LayoutBlockContainer`] according to the block layout algorithm
fn compute_inner(
    tree: &mut impl LayoutBlockContainer,
    node_id: NodeId,
    inputs: LayoutInput,
    #[allow(unused_mut)] mut block_ctx: &mut BlockContext<'_>,
) -> LayoutOutput {
    let writing_mode = tree.get_writing_mode(node_id);
    let percentage_basis = inputs.constraint_space(writing_mode).margin_padding_percentage_basis();
    let LayoutInput {
        known_dimensions,
        definite_dimensions,
        parent_size,
        available_space,
        run_mode,
        sizing_mode,
        vertical_margins_are_collapsible,
        ..
    } = inputs;

    let resolved_aspect_ratio = tree.get_resolved_aspect_ratio(node_id);
    let scrollbar_gutter = tree.get_scrollbar_insets(node_id);
    let style = tree.get_block_container_style(node_id);
    let raw_margin = style.margin();
    let aspect_ratio = if sizing_mode == SizingMode::InherentSize { resolved_aspect_ratio } else { None };
    let padding = style.padding().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let border = style.border().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
    let direction = style.direction();

    let padding_border = padding + border;
    let padding_border_size = padding_border.sum_axes();
    let content_box_inset = padding_border + scrollbar_gutter;

    // Apply content box inset
    #[cfg(feature = "float_layout")]
    block_ctx.apply_content_box_inset([content_box_inset.left, content_box_inset.right]);

    let box_sizing = style.box_sizing();
    let box_sizing_adjustment = if box_sizing == BoxSizing::ContentBox { padding_border_size } else { Size::ZERO };
    let (size, min_size, max_size) = match sizing_mode {
        SizingMode::ContentSize => (Size::NONE, Size::NONE, Size::NONE),
        SizingMode::InherentSize => {
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
                writing_mode,
                block_auto_behavior: inputs.block_auto_behavior,
                transferred_sizes_mode: TransferredSizesMode::Normal,
                aspect_ratio,
                padding_border: padding_border_size,
            });
            (resolved.size, resolved.min_size, resolved.max_size)
        }
    };

    // css-sizing-4: a definite size in one axis transfers through `aspect-ratio`
    // to make the other definite. Deriving it from `known_dimensions` self-gates
    // the transfer — a block parent fills an axis only when it's a real
    // constraint (e.g. the stretched width at final layout) and leaves it None
    // while probing intrinsic sizes, so measure passes stay content-based. Only a
    // newly-filled axis is adopted (and clamped); an incoming known size is left
    // as the parent resolved it (re-clamping would undo padding/border overrides).
    let known_dimensions = {
        let derived = known_dimensions
            .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border_size)
            .maybe_clamp(min_size, max_size);
        Size { width: known_dimensions.width.or(derived.width), height: known_dimensions.height.or(derived.height) }
    };
    let container_content_box_size = known_dimensions.maybe_sub(content_box_inset.sum_axes());

    let overflow = style.overflow();
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();

    // Determine margin collapsing behaviour
    let own_margins_collapse_with_children = Line {
        start: vertical_margins_are_collapsible.start
            && !is_scroll_container
            && style.position() == Position::Relative
            && padding.top == 0.0
            && border.top == 0.0,
        end: vertical_margins_are_collapsible.end
            && !is_scroll_container
            && style.position() == Position::Relative
            && padding.bottom == 0.0
            && border.bottom == 0.0
            && size.height.is_none(),
    };
    let has_styles_preventing_being_collapsed_through = !style.is_block()
        || block_ctx.is_bfc_root()
        || is_scroll_container
        || style.position() == Position::Absolute
        || padding.top > 0.0
        || padding.bottom > 0.0
        || border.top > 0.0
        || border.bottom > 0.0
        || matches!(size.height, Some(h) if h > 0.0)
        || matches!(min_size.height, Some(h) if h > 0.0);

    let text_align = style.text_align();
    let align_content = style.align_content();
    drop(style);

    // 1. Generate items
    let mut items = generate_item_list(tree, node_id, writing_mode, container_content_box_size, available_space);

    // 2. Compute container width
    let (container_outer_width, content_width_depends_on_block_constraints) = match known_dimensions.width {
        Some(width) => (width, false),
        None => {
            let available_width = available_space.width.maybe_sub(content_box_inset.horizontal_axis_sum());
            let (intrinsic_width, depends) =
                determine_content_based_container_width(tree, &mut items, available_width, writing_mode);
            (
                (intrinsic_width + content_box_inset.horizontal_axis_sum())
                    .maybe_clamp(min_size.width, max_size.width)
                    .maybe_max(Some(padding_border_size.width)),
                depends,
            )
        }
    };

    // Short-circuit if computing size and both dimensions known
    if let (RunMode::ComputeSize, Some(container_outer_height)) = (run_mode, known_dimensions.height) {
        return LayoutOutput::from_outer_size(Size { width: container_outer_width, height: container_outer_height })
            .with_block_constraint_dependency(content_width_depends_on_block_constraints);
    }

    // We can also short-circuit if the width is known and only the width has been requested.
    if run_mode == RunMode::ComputeSize && inputs.axis == RequestedAxis::Horizontal {
        return LayoutOutput::from_outer_size(Size { width: container_outer_width, height: 0.0 })
            .with_block_constraint_dependency(content_width_depends_on_block_constraints);
    }

    let container_percentage_resolution_height =
        known_dimensions.height.or(size.height.maybe_max(min_size.height)).or(min_size.height);
    // Relative block-axis percentage insets only resolve against a definite
    // containing-block height. A min-height may determine the eventual used
    // height, but it does not make an otherwise-auto height definite.
    let relative_inset_percentage_resolution_height = definite_dimensions.height.or(size.height);

    // 3. Perform final item layout and return content height
    #[cfg_attr(not(feature = "content_size"), allow(unused_mut))]
    let (
        mut inflow_content_size,
        mut intrinsic_outer_height,
        first_child_top_margin_set,
        last_child_bottom_margin_set,
        mut first_baseline,
        mut last_baseline,
    ) = perform_final_layout_on_in_flow_children(
        tree,
        &mut items,
        BlockContainerLayoutContext {
            run_mode,
            outer_width: container_outer_width,
            percentage_resolution_height: container_percentage_resolution_height,
            relative_inset_percentage_resolution_height,
            content_box_inset,
            border,
            scrollbar_insets: scrollbar_gutter,
            text_align,
            direction,
            writing_mode,
            own_margins_collapse_with_children,
        },
        block_ctx,
    );

    // Root BFCs contain floats
    #[cfg(feature = "float_layout")]
    if block_ctx.is_bfc_root() || is_scroll_container {
        intrinsic_outer_height = intrinsic_outer_height.max(block_ctx.floated_content_height_contribution());
    }

    let container_outer_height = known_dimensions
        .height
        .unwrap_or(intrinsic_outer_height.maybe_clamp(min_size.height, max_size.height))
        .maybe_max(Some(padding_border_size.height));
    let final_outer_size = Size { width: container_outer_width, height: container_outer_height };

    // CSS2 §8.3.1: the bottom margin of a block with `height: auto` collapses with its last
    // in-flow child's bottom margin only if the box's `min-height` is less than the box's
    // used height. When `min-height` determines the used height, the last child's bottom
    // margin no longer adjoins the box's bottom edge, so it stays inside the box instead of
    // collapsing with the box's own bottom margin. (`max-height` has no such effect.)
    let height_constrained_by_min_height = matches!(min_size.height, Some(h) if h > 0.0 && h >= container_outer_height);
    let own_bottom_margin_collapses_with_children =
        own_margins_collapse_with_children.end && !height_constrained_by_min_height;

    // Apply `align-content` to in-flow non-floated items if requested. The per-item layouts were
    // held back in `item.final_layout` so that this step can shift `location.y` before tree commit.
    //
    // For block layout the entire stack of in-flow children is treated as a single alignment
    // subject. That means distribution keywords (`space-between`, `space-around`,
    // `space-evenly`, `stretch`) must invoke the single-subject fallback unconditionally —
    // which is what passing `num_items = 1` to `apply_alignment_fallback` does. The whole
    // group then shifts by one offset, with zero inter-item gap.
    if let Some(align_content) = align_content {
        let container_inner_height = container_outer_height - content_box_inset.vertical_axis_sum();
        let inflow_content_height = intrinsic_outer_height - content_box_inset.vertical_axis_sum();
        let free_space = container_inner_height - inflow_content_height;
        let any_in_flow = items.iter().any(|item| item.final_layout.is_some());
        if any_in_flow {
            let keyword = apply_alignment_fallback(free_space, 1, align_content);
            let group_offset = compute_alignment_offset(free_space, 1, 0.0, keyword, false, true);
            first_baseline = first_baseline.map(|baseline| baseline + group_offset);
            last_baseline = last_baseline.map(|baseline| baseline + group_offset);
            for item in items.iter_mut() {
                if let Some(layout) = item.final_layout.as_mut() {
                    layout.location.y += group_offset;
                }
            }

            #[cfg(feature = "content_size")]
            {
                inflow_content_size = Size::ZERO;
                for item in items.iter() {
                    if let Some(layout) = item.final_layout.as_ref() {
                        let contribution_location = content_size_contribution_location(
                            layout.location,
                            layout.size,
                            container_outer_width,
                            border,
                            scrollbar_gutter,
                            direction,
                        );
                        inflow_content_size = inflow_content_size.f32_max(compute_content_size_contribution(
                            contribution_location,
                            layout.size,
                            layout.content_size,
                            item.overflow,
                        ));
                    }
                }
            }
        }
    }

    // Determine whether this node can be collapsed through
    let all_in_flow_children_can_be_collapsed_through = items.iter().all(|item| {
        #[cfg(feature = "float_layout")]
        if item.float.is_floated() {
            return true;
        }
        item.position == Position::Absolute || item.can_be_collapsed_through
    });
    let can_be_collapsed_through =
        !has_styles_preventing_being_collapsed_through && all_in_flow_children_can_be_collapsed_through;

    let mut output = LayoutOutput::from_sizes_and_baseline_sets(
        final_outer_size,
        Size::ZERO,
        Point { x: None, y: first_baseline },
        Point { x: None, y: last_baseline },
    )
    .with_block_constraint_dependency(
        content_width_depends_on_block_constraints || items.iter().any(|item| item.depends_on_block_constraints),
    );
    output.top_margin = if own_margins_collapse_with_children.start {
        first_child_top_margin_set
    } else {
        let margin_top = raw_margin.top.resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
        CollapsibleMarginSet::from_margin(margin_top)
    };
    output.bottom_margin = if own_bottom_margin_collapses_with_children {
        last_child_bottom_margin_set
    } else {
        let margin_bottom = raw_margin.bottom.resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
        CollapsibleMarginSet::from_margin(margin_bottom)
    };
    output.margins_can_collapse_through = can_be_collapsed_through;

    // Short-circuit if computing size.
    //
    // Note: it is important that we return the margin-collapsing related outputs here as Parent block containers
    // rely on the `top_margin`/`bottom_margin` of their children to compute their own intrinsic height.
    if run_mode == RunMode::ComputeSize {
        return output;
    }

    // Commit deferred in-flow layouts to the tree. Floated items already wrote their own layouts.
    for item in items.iter() {
        if let Some(layout) = item.final_layout.as_ref() {
            tree.set_unrounded_layout(item.node_id, layout);
        }
    }

    // 4. Layout absolutely positioned children
    let absolute_position_inset = border + scrollbar_gutter;
    let absolute_position_area = final_outer_size - absolute_position_inset.sum_axes();
    let absolute_position_offset = Point { x: absolute_position_inset.left, y: absolute_position_inset.top };
    let absolute_content_size = perform_absolute_layout_on_absolute_children(
        tree,
        &items,
        absolute_position_area,
        absolute_position_offset,
        direction,
        writing_mode,
    );

    #[cfg(feature = "content_size")]
    {
        // The container's own padding at the end of the content is part of its scrollable
        // overflow region, so it is included in the in-flow content size.
        inflow_content_size.width += if direction.is_rtl() { padding.left } else { padding.right };
        inflow_content_size.height += padding.bottom;
        output.content_size = inflow_content_size.f32_max(absolute_content_size);
    }

    // 5. Perform hidden layout on hidden children
    let len = tree.child_count(node_id);
    for order in 0..len {
        let child = tree.get_child_id(node_id, order);
        let child_style = tree.get_block_child_style(child);
        if child_style.box_generation_mode() == BoxGenerationMode::None {
            drop(child_style);
            tree.set_unrounded_layout(child, &Layout::with_order(order as u32));
            tree.perform_child_layout(
                child,
                ChildLayoutInput::new(
                    Size::NONE,
                    Size::NONE,
                    writing_mode,
                    Size::MAX_CONTENT,
                    SizingMode::InherentSize,
                    Line::FALSE,
                ),
            );
        }
    }

    output
}

/// Create a `Vec` of `BlockItem` structs where each item in the `Vec` represents a child of the current node
#[inline]
fn generate_item_list(
    tree: &mut impl LayoutBlockContainer,
    node: NodeId,
    writing_mode: WritingMode,
    node_inner_size: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
) -> Vec<BlockItem> {
    let child_ids: Vec<_> = tree.child_ids(node).collect();
    child_ids
        .into_iter()
        .filter_map(|child_node_id| {
            let aspect_ratio = tree.get_resolved_aspect_ratio(child_node_id);
            let child_writing_mode = tree.get_writing_mode(child_node_id);
            let scrollbar_size = tree.get_scrollbar_insets(child_node_id).sum_axes();
            let child_style = tree.get_block_child_style(child_node_id);
            if child_style.box_generation_mode() == BoxGenerationMode::None {
                return None;
            }
            // When the container's inline size depends on its contents, CSS
            // Sizing resolves cyclic percentage padding/border and min-width
            // contributions against zero. Preferred and max sizes remain
            // unresolved until final layout, so keep their original
            // containing-block basis here.
            let mut logical_contribution_parent_size = writing_mode.to_logical(node_inner_size);
            logical_contribution_parent_size.inline_size = logical_contribution_parent_size.inline_size.or(Some(0.0));
            let contribution_parent_size = writing_mode.to_physical(logical_contribution_parent_size);
            let contribution_inline_size = logical_contribution_parent_size.inline_size;
            let padding =
                child_style.padding().resolve_or_zero(contribution_inline_size, |val, basis| tree.calc(val, basis));
            let border =
                child_style.border().resolve_or_zero(contribution_inline_size, |val, basis| tree.calc(val, basis));
            let pb_sum = (padding + border).sum_axes();
            let box_sizing = child_style.box_sizing();
            let box_sizing_adjustment = if box_sizing == BoxSizing::ContentBox { pb_sum } else { Size::ZERO };
            let raw_size = child_style.size();
            let raw_min_size = child_style.min_size();
            let raw_max_size = child_style.max_size();
            let child_block_size_depends_on_parent = [raw_size.height, raw_min_size.height, raw_max_size.height]
                .into_iter()
                .any(|value| value.may_have_percentage_dependence() || value.is_stretch());
            let mut depends_on_block_constraints = child_block_size_depends_on_parent && aspect_ratio.is_some();
            let mut size = raw_size
                .maybe_resolve(node_inner_size, |val, basis| tree.calc(val, basis))
                .maybe_add(box_sizing_adjustment);
            let mut min_size = raw_min_size
                .maybe_resolve(contribution_parent_size, |val, basis| tree.calc(val, basis))
                .maybe_add(box_sizing_adjustment);
            let mut max_size = raw_max_size
                .maybe_resolve(node_inner_size, |val, basis| tree.calc(val, basis))
                .maybe_add(box_sizing_adjustment);
            let position = child_style.position();
            let overflow = child_style.overflow();
            let inset = child_style.inset();
            let margin = child_style.margin();

            #[cfg(feature = "float_layout")]
            let float = child_style.float();
            #[cfg(feature = "float_layout")]
            let clear = child_style.clear();
            #[cfg(feature = "float_layout")]
            let is_not_floated = float == Float::None;

            #[cfg(not(feature = "float_layout"))]
            let is_not_floated = true;

            let is_block = child_style.is_block();
            let uses_block_layout = child_style.uses_block_layout();
            let is_table = child_style.is_table();
            let is_replaced = child_style.is_compressible_replaced();
            let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();

            let is_in_same_bfc: bool =
                is_block && !is_table && position != Position::Absolute && is_not_floated && !is_scroll_container;

            drop(child_style);

            // Absolutely positioned boxes derive their available inline space
            // from their insets. Resolve those intrinsic keywords later, in
            // the absolute-layout seam, rather than against the whole parent.
            if position != Position::Absolute {
                let resolved_margin =
                    margin.resolve_or_zero(contribution_inline_size, |val, basis| tree.calc(val, basis));
                let child_available_space = Size {
                    width: node_inner_size.width.map(AvailableSpace::Definite).unwrap_or(available_space.width),
                    height: available_space.height,
                };
                let available_width = child_available_space.width.maybe_sub(resolved_margin.horizontal_axis_sum());
                let intrinsic_inputs = LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode: SizingMode::InherentSize,
                    sizing_purpose: SizingPurpose::IntrinsicContribution,
                    axis: RequestedAxis::Horizontal,
                    block_auto_behavior: crate::AutoSizeBehavior::FitContent,
                    known_dimensions: Size::NONE,
                    definite_dimensions: Size::NONE,
                    parent_size: node_inner_size,
                    parent_writing_mode: writing_mode,
                    available_space: child_available_space,
                    vertical_margins_are_collapsible: Line::TRUE,
                };
                let intrinsic = resolve_intrinsic_width_constraints(
                    tree,
                    child_node_id,
                    intrinsic_inputs,
                    raw_size.width,
                    raw_min_size.width,
                    raw_max_size.width,
                    available_width,
                );
                size.width = size.width.or(intrinsic.preferred);
                min_size.width = min_size.width.or(intrinsic.min);
                max_size.width = max_size.width.or(intrinsic.max);
                depends_on_block_constraints |= intrinsic.depends_on_block_constraints;
            }

            let resolved = resolve_size_constraints(SizeConstraintInput {
                size,
                min_size,
                max_size,
                size_is_auto: raw_size.map(|dimension| dimension.is_auto()),
                writing_mode: child_writing_mode,
                block_auto_behavior: AutoSizeBehavior::FitContent,
                transferred_sizes_mode: TransferredSizesMode::Normal,
                aspect_ratio,
                padding_border: pb_sum,
            });
            let block_axis_constraints = resolved.block_axis_constraints(child_writing_mode);
            size = resolved.size;
            min_size = resolved.min_size;
            max_size = resolved.max_size;

            Some(BlockItem {
                node_id: child_node_id,
                order: 0,
                is_table,
                is_replaced,
                uses_block_layout,
                is_in_same_bfc,
                #[cfg(feature = "float_layout")]
                float,
                #[cfg(feature = "float_layout")]
                clear,
                size,
                min_size,
                max_size,
                block_axis_constraints,
                overflow,
                scrollbar_size,
                position,
                inset,
                margin,
                padding,
                border,
                padding_border_sum: pb_sum,

                // Fields to be computed later (for now we initialise with dummy values)
                computed_size: Size::zero(),
                static_position: Point::zero(),
                can_be_collapsed_through: false,
                depends_on_block_constraints,
                final_layout: None,
            })
        })
        .enumerate()
        .map(|(order, mut item)| {
            item.order = order as u32;
            item
        })
        .collect()
}

/// Compute the content-based width in the case that the width of the container is not known
#[inline]
fn determine_content_based_container_width(
    tree: &mut impl LayoutPartialTree,
    items: &mut [BlockItem],
    available_width: AvailableSpace,
    parent_writing_mode: WritingMode,
) -> (f32, bool) {
    let available_space = Size { width: available_width, height: AvailableSpace::MinContent };

    let mut max_child_width = 0.0;
    #[cfg(feature = "float_layout")]
    let mut float_contribution = FloatIntrinsicWidthCalculator::new(available_width);
    let mut depends_on_block_constraints = false;
    for item in items.iter_mut().filter(|item| item.position != Position::Absolute) {
        let known_dimensions = item.size.maybe_clamp(item.min_size, item.max_size);

        // The containing block's inline size depends on this contribution, so
        // cyclic percentage margins resolve against zero rather than the
        // external available-space constraint.
        let item_x_margin_sum =
            item.margin.resolve_or_zero(Some(0.0), |val, basis| tree.calc(val, basis)).horizontal_axis_sum();
        let width = match known_dimensions.width {
            Some(width) => width,
            None => {
                let measured = tree.measure_child_size_with_metadata(
                    item.node_id,
                    ChildLayoutInput::new(
                        known_dimensions,
                        Size::NONE,
                        parent_writing_mode,
                        available_space.map_width(|w| w.maybe_sub(item_x_margin_sum)),
                        SizingMode::InherentSize,
                        Line::TRUE,
                    ),
                    RequestedAxis::Horizontal,
                );
                item.depends_on_block_constraints |= measured.depends_on_block_constraints;
                measured.size.width
            }
        }
        .maybe_clamp(item.min_size.width, item.max_size.width);
        depends_on_block_constraints |= item.depends_on_block_constraints;

        let width = f32_max(width, item.padding_border_sum.width) + item_x_margin_sum;

        #[cfg(feature = "float_layout")]
        if let Some(direction) = item.float.float_direction() {
            float_contribution.add_float(width, direction, item.clear);
            continue;
        }

        max_child_width = f32_max(max_child_width, width);
    }

    #[cfg(feature = "float_layout")]
    {
        max_child_width = max_child_width.max(float_contribution.result());
    }

    (max_child_width, depends_on_block_constraints)
}

/// Resolve an item's preferred/min/max sizes against the containing block's
/// final percentage basis.
///
/// Item generation may run while that basis is indefinite in order to compute
/// the container's intrinsic width. Numeric percentage values are therefore
/// materialized again here, after the container width is known. Intrinsic
/// keyword measurements from the contribution phase are retained when the raw
/// style cannot be reduced to a numeric value.
fn resolve_block_item_final_style(
    tree: &mut impl LayoutBlockContainer,
    item: &mut BlockItem,
    parent_size: Size<Option<f32>>,
    parent_writing_mode: WritingMode,
) {
    let percentage_basis = parent_writing_mode.to_logical(parent_size).inline_size;
    let aspect_ratio = tree.get_resolved_aspect_ratio(item.node_id);
    let writing_mode = tree.get_writing_mode(item.node_id);
    let (size, min_size, max_size, block_axis_constraints, padding, border) = {
        let style = tree.get_block_child_style(item.node_id);
        let padding = style.padding().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
        let border = style.border().resolve_or_zero(percentage_basis, |val, basis| tree.calc(val, basis));
        let padding_border_sum = (padding + border).sum_axes();
        let box_sizing = style.box_sizing();
        let box_sizing_adjustment = if box_sizing == BoxSizing::ContentBox { padding_border_sum } else { Size::ZERO };
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
            writing_mode,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            transferred_sizes_mode: TransferredSizesMode::Normal,
            aspect_ratio,
            padding_border: padding_border_sum,
        });
        (
            resolved.size,
            resolved.min_size,
            resolved.max_size,
            resolved.block_axis_constraints(writing_mode),
            padding,
            border,
        )
    };

    item.size = size.or(item.size);
    item.min_size = min_size.or(item.min_size);
    item.max_size = max_size.or(item.max_size);
    item.block_axis_constraints = block_axis_constraints;
    item.padding = padding;
    item.border = border;
    item.padding_border_sum = (padding + border).sum_axes();
}

/// Complete an in-flow block item's logical block size once its inline
/// constraint is known.
///
/// A preferred ratio may provide a provisional auto block size, but the
/// ratio-dependent automatic minimum still comes from the real content
/// contribution. Resolve both at this formatting-context boundary before the
/// result is labeled as an exact child `known_dimensions` input.
fn resolve_block_item_known_dimensions(
    tree: &mut impl LayoutBlockContainer,
    item: &mut BlockItem,
    known_dimensions: Size<Option<f32>>,
    parent_size: Size<Option<f32>>,
    parent_writing_mode: WritingMode,
    available_space: Size<AvailableSpace>,
    vertical_margins_are_collapsible: Line<bool>,
) -> Size<Option<f32>> {
    let child_writing_mode = tree.get_writing_mode(item.node_id);
    let aspect_ratio = tree.get_resolved_aspect_ratio(item.node_id);
    let (properties, is_scroll_container) = {
        let style = tree.get_block_child_style(item.node_id);
        let size = child_writing_mode.to_logical(style.size());
        let min_size = child_writing_mode.to_logical(style.min_size());
        let max_size = child_writing_mode.to_logical(style.max_size());
        let overflow = style.overflow();
        (
            BlockSizeProperties::new(size.block_size, min_size.block_size, max_size.block_size),
            overflow.x.is_scroll_container() || overflow.y.is_scroll_container(),
        )
    };
    let auto_size_is_content_based = AutoSizeBehavior::FitContent.is_content_based(aspect_ratio.is_some());
    let resolver = ContentBasedBlockSize::new(
        properties,
        aspect_ratio,
        item.padding_border_sum,
        auto_size_is_content_based,
        is_scroll_container,
    );
    if !resolver.requires_intrinsic_measurement() {
        return known_dimensions;
    }

    let mut measurement_dimensions = child_writing_mode.to_logical(known_dimensions);
    if properties.preferred_is_content_based(auto_size_is_content_based) {
        measurement_dimensions.block_size = None;
    }
    let measurement_dimensions = child_writing_mode.to_physical(measurement_dimensions);
    let intrinsic = measure_content_based_block_size(
        tree,
        item.node_id,
        ChildLayoutInput::new(
            measurement_dimensions,
            parent_size,
            parent_writing_mode,
            available_space,
            SizingMode::ContentSize,
            vertical_margins_are_collapsible,
        )
        .with_block_auto_behavior(AutoSizeBehavior::FitContent),
        resolver,
    );
    item.depends_on_block_constraints |= intrinsic.depends_on_block_constraints;

    let logical_size = child_writing_mode.to_logical(item.size);
    let resolved = intrinsic.resolve_against(logical_size.block_size, item.block_axis_constraints);
    let minimum_border_box_size = child_writing_mode.to_logical(item.padding_border_sum).block_size;
    let mut used_size = child_writing_mode.to_logical(measurement_dimensions);
    used_size.block_size = used_size
        .block_size
        .or(resolved.preferred)
        .maybe_clamp(resolved.min, resolved.max)
        .maybe_max(Some(minimum_border_box_size));
    child_writing_mode.to_physical(used_size)
}

/// Immutable container state shared while positioning in-flow block children.
#[derive(Clone, Copy, Debug)]
struct BlockContainerLayoutContext {
    /// Whether this pass measures the box or commits final fragments.
    run_mode: RunMode,
    /// Used physical border-box width of the container.
    outer_width: f32,
    /// Definite physical height available for descendant percentages.
    percentage_resolution_height: Option<f32>,
    /// Definite physical height available for relative percentage insets.
    relative_inset_percentage_resolution_height: Option<f32>,
    /// Padding, border, and scrollbar inset around the content box.
    content_box_inset: Rect<f32>,
    /// Used physical border widths.
    border: Rect<f32>,
    /// Physical space reserved for scrollbar gutters on each edge.
    scrollbar_insets: Rect<f32>,
    /// Inline alignment inherited by anonymous block content.
    text_align: TextAlign,
    /// Inline direction used for physical horizontal placement.
    direction: Direction,
    /// Writing mode that owns the container's logical axes.
    writing_mode: WritingMode,
    /// Whether block-start/end margins may collapse with children.
    own_margins_collapse_with_children: Line<bool>,
}

/// Compute each child's final size and position.
#[inline]
fn perform_final_layout_on_in_flow_children(
    tree: &mut impl LayoutBlockContainer,
    items: &mut [BlockItem],
    context: BlockContainerLayoutContext,
    block_ctx: &mut BlockContext<'_>,
) -> (Size<f32>, f32, CollapsibleMarginSet, CollapsibleMarginSet, Option<f32>, Option<f32>) {
    let BlockContainerLayoutContext {
        run_mode,
        outer_width: container_outer_width,
        percentage_resolution_height: container_percentage_resolution_height,
        relative_inset_percentage_resolution_height,
        content_box_inset,
        border,
        scrollbar_insets,
        text_align,
        direction,
        writing_mode,
        own_margins_collapse_with_children,
    } = context;
    let container_inner_width = container_outer_width - content_box_inset.horizontal_axis_sum();
    let container_percentage_resolution_height =
        container_percentage_resolution_height.maybe_sub(content_box_inset.vertical_axis_sum());
    let parent_size = Size { width: Some(container_inner_width), height: container_percentage_resolution_height };
    let margin_percentage_basis = writing_mode.to_logical(parent_size).inline_size.unwrap_or(0.0);
    let relative_inset_parent_size = Size {
        width: Some(container_inner_width),
        height: relative_inset_percentage_resolution_height.maybe_sub(content_box_inset.vertical_axis_sum()),
    };
    // Vertical available space in block flow is indefinite, NOT a min-content
    // constraint: MaxContent is taffy's representation of "indefinite".
    // Passing MinContent here made every descendant grid believe it was being
    // sized under a min-content constraint, in which the maximize-tracks step
    // has zero free space — so auto rows containing only scroll-container
    // items (overflow != visible, automatic minimum size = 0) collapsed to
    // zero height. Browsers size such rows to the item's content.
    let available_space =
        Size { width: AvailableSpace::Definite(container_inner_width), height: AvailableSpace::MaxContent };

    // TODO: handle nested blocks with different widths
    #[cfg(feature = "float_layout")]
    if block_ctx.is_bfc_root() {
        block_ctx.set_width(container_outer_width);
        block_ctx.apply_content_box_inset([content_box_inset.left, content_box_inset.right]);
    }

    // If this block's top margin does not collapse with its children's then the position of its
    // top margin strut is resolved relative to it, and floats adjoining ancestor struts do not
    // adjoin this block's strut.
    #[cfg(feature = "float_layout")]
    if !own_margins_collapse_with_children.start {
        block_ctx.commit_strut();
    }

    #[cfg_attr(not(feature = "content_size"), allow(unused_mut))]
    let mut inflow_content_size = Size::ZERO;
    let mut committed_y_offset = content_box_inset.top;
    let mut y_offset_for_absolute = content_box_inset.top;
    let mut first_child_top_margin_set = CollapsibleMarginSet::ZERO;
    let mut active_collapsible_margin_set = CollapsibleMarginSet::ZERO;
    let mut is_collapsing_with_first_margin_set = true;
    let mut first_baseline: Option<f32> = None;
    let mut last_baseline: Option<f32> = None;
    // Whether the active margin set contains the margins of a self-collapsing element with
    // clearance. Such margins collapse with the margins of following siblings but the resulting
    // margin does not collapse with the bottom margin of the parent block.
    let mut active_margin_set_has_clearance = false;

    #[cfg(feature = "float_layout")]
    let mut has_active_floats = block_ctx.has_active_floats(committed_y_offset);
    #[cfg(not(feature = "float_layout"))]
    let has_active_floats = false;

    for item in items.iter_mut() {
        if item.position == Position::Absolute {
            let x = match direction {
                Direction::Ltr => content_box_inset.left,
                Direction::Rtl => container_outer_width - content_box_inset.right,
            };
            item.static_position = Point { x, y: y_offset_for_absolute }
        } else {
            resolve_block_item_final_style(tree, item, parent_size, writing_mode);
            let item_margin = item
                .margin
                .map(|margin| margin.resolve_to_option(margin_percentage_basis, |val, basis| tree.calc(val, basis)));
            let item_non_auto_margin = item_margin.map(|m| m.unwrap_or(0.0));
            let item_non_auto_x_margin_sum = item_non_auto_margin.horizontal_axis_sum();

            let scrollbar_size = item.scrollbar_size;

            // Handle floated boxes
            #[cfg(feature = "float_layout")]
            if let Some(float_direction) = item.float.float_direction() {
                has_active_floats = true;

                // A float with `width: auto` is shrink-to-fit (fit-content) sized: the available
                // space clamped between its min-content and max-content sizes.
                let available_width = container_inner_width - item_non_auto_x_margin_sum;
                // Item materialization has already resolved explicit intrinsic
                // and stretch keywords against that margin-adjusted space.
                // Carry those used dimensions into child layout; otherwise the
                // generic child seam would subtract the float's margins from
                // the already-adjusted width a second time.
                let known_dimensions = item.size.maybe_clamp(item.min_size, item.max_size);
                let item_layout = tree.perform_child_layout(
                    item.node_id,
                    ChildLayoutInput::new(
                        known_dimensions,
                        parent_size,
                        writing_mode,
                        Size { width: AvailableSpace::Definite(available_width), height: AvailableSpace::MaxContent },
                        SizingMode::InherentSize,
                        // A float establishes a new block formatting context: its margins do not
                        // collapse with the margins of its children
                        Line::FALSE,
                    ),
                );
                let margin_box = item_layout.size + item_non_auto_margin.sum_axes();

                // Floats that occur between collapsing margins are positioned as if they had an otherwise
                // empty anonymous block parent taking part in the flow, so the pending collapsible margins
                // contribute to the float's minimum y position (unless those margins collapse with the
                // container's own top margin, in which case they are applied outside the container).
                //
                // In the latter case the position of the float is not fully resolved: margins contributed
                // by later siblings can still collapse into the strut and move the container (and float).
                // Such floats force clearance on cleared elements whose margins adjoin the same strut.
                let adjoins_unresolved_strut =
                    is_collapsing_with_first_margin_set && own_margins_collapse_with_children.start;
                let y_offset_for_float = if adjoins_unresolved_strut {
                    committed_y_offset
                } else {
                    committed_y_offset + active_collapsible_margin_set.resolve()
                };

                let mut location = block_ctx.place_floated_box(
                    margin_box,
                    y_offset_for_float,
                    float_direction,
                    item.clear,
                    adjoins_unresolved_strut,
                );

                // Ensure that content that appears after a float does not get positioned before/above the float
                //
                // FIXME: this isn't quite right, because a second float at the same location
                // shouldn't cause content to push down to it's level
                // committed_y_offset = committed_y_offset.max(location.y);
                // y_offset_for_absolute = y_offset_for_absolute.max(location.y);

                // Convert the margin-box location returned by float placement into a border-box location
                // for the output Layout
                location.y += item_non_auto_margin.top;
                location.x += item_non_auto_margin.left;

                // println!("BLOCK FLOATED BOX ({:?}) {:?}", item.node_id, float_direction);
                // println!("w:{} h:{} x:{}, y:{}", margin_box.width, margin_box.height, location.x, location.y);

                tree.set_unrounded_layout(
                    item.node_id,
                    &Layout {
                        order: item.order,
                        size: item_layout.size,
                        #[cfg(feature = "content_size")]
                        content_size: item_layout.content_size,
                        scrollbar_size,
                        location,
                        padding: item.padding,
                        border: item.border,
                        margin: item_non_auto_margin,
                    },
                );

                #[cfg(feature = "content_size")]
                {
                    // TODO: Should content size of floated boxes count as "inflow_content_size"
                    // or should it be counted separately?
                    let contribution_location = content_size_contribution_location(
                        location,
                        item_layout.size,
                        container_outer_width,
                        border,
                        scrollbar_insets,
                        direction,
                    );
                    inflow_content_size = inflow_content_size.f32_max(compute_content_size_contribution(
                        contribution_location,
                        item_layout.size,
                        item_layout.content_size,
                        item.overflow,
                    ));
                }

                continue;
            }

            // Handle non-floated boxes

            let mut y_margin_offset: f32 = 0.0;
            #[cfg(feature = "float_layout")]
            let mut item_avoids_floats = false;
            #[cfg(feature = "float_layout")]
            let mut item_pushed_below_float = false;

            let (stretch_width, float_avoiding_position, float_avoiding_width) = if item.is_in_same_bfc {
                let stretch_width = container_inner_width - item_non_auto_x_margin_sum;
                let position = Point { x: 0.0, y: 0.0 };
                let width = 0.0;

                (stretch_width, position, width)
            } else {
                'block: {
                    // Set y_margin_offset (different bfc child)
                    if !is_collapsing_with_first_margin_set || !own_margins_collapse_with_children.start {
                        y_margin_offset =
                            active_collapsible_margin_set.collapse_with_margin(item_non_auto_margin.top).resolve();
                    };
                    let min_y = committed_y_offset + y_margin_offset;

                    // In addition to the running flag, check the float context directly:
                    // floats placed by the subtree of a preceding in-flow sibling (in the same
                    // BFC) are not reflected in the flag
                    #[cfg(feature = "float_layout")]
                    if has_active_floats || block_ctx.has_active_floats(min_y) {
                        let x_margins = [item_non_auto_margin.left, item_non_auto_margin.right];
                        // An auto width resolves to at least the negation of the margin sum
                        // (so that the margin box width is non-negative, per CSS2 §10.3.3)
                        let min_auto_width = -item_non_auto_x_margin_sum;

                        // Find the highest slot (at or below `min_y`) with enough horizontal space
                        // for the item's border box, which must not overlap any float
                        let mut slot_segment = None;
                        let slot = loop {
                            let slot = block_ctx.find_bfc_slot(min_y, x_margins, direction, item.clear, slot_segment);
                            let Some(segment_id) = slot.segment_id else { break slot };
                            let width = item
                                .size
                                .width
                                .unwrap_or(slot.stretch_width.max(min_auto_width))
                                .maybe_clamp(item.min_size.width, item.max_size.width);
                            if width <= slot.border_width + 0.001 {
                                break slot;
                            }
                            slot_segment = Some(segment_id);
                        };

                        // If the item had to move down to avoid floats then it "separates from the
                        // float": similarly to clearance, its top margin no longer collapses with
                        // the parent's margins.
                        if slot.y > min_y {
                            item_pushed_below_float = true;
                        }

                        has_active_floats = slot.segment_id.is_some();
                        item_avoids_floats = true;
                        let stretch_width = slot.stretch_width.max(min_auto_width);
                        break 'block (stretch_width, Point { x: slot.x, y: slot.y }, slot.border_width);
                    }

                    if !has_active_floats {
                        let stretch_width = container_inner_width - item_non_auto_x_margin_sum;
                        break 'block (
                            stretch_width,
                            Point { x: content_box_inset.left, y: min_y },
                            container_inner_width,
                        );
                    }

                    unreachable!("One of the above cases will always be hit");
                }
            };

            // Tables and replaced elements are not stretch-sized: they resolve their own
            // size (for replaced elements an auto width resolves to the intrinsic size
            // <https://www.w3.org/TR/CSS22/visudet.html#block-replaced-width>)
            let known_dimensions = if item.is_table || item.is_replaced {
                // Preserve auto as unknown, but carry explicit numeric or
                // intrinsic keyword sizes resolved during item materialization.
                // This also keeps margin-adjusted fit-content/stretch widths
                // from being resolved a second time inside the child seam.
                item.size.maybe_clamp(item.min_size, item.max_size)
            } else {
                item.size
                    .map_width(|width| {
                        Some(width.unwrap_or(stretch_width).maybe_clamp(item.min_size.width, item.max_size.width))
                    })
                    .maybe_clamp(item.min_size, item.max_size)
            };

            //

            let child_available_space = available_space.map_width(|_| AvailableSpace::Definite(stretch_width));
            let known_dimensions = resolve_block_item_known_dimensions(
                tree,
                item,
                known_dimensions,
                parent_size,
                writing_mode,
                child_available_space,
                if item.is_in_same_bfc { Line::TRUE } else { Line::FALSE },
            );
            let inputs = LayoutInput {
                run_mode,
                sizing_mode: SizingMode::InherentSize,
                sizing_purpose: SizingPurpose::Layout,
                axis: RequestedAxis::Both,
                block_auto_behavior: crate::AutoSizeBehavior::FitContent,
                known_dimensions,
                definite_dimensions: known_dimensions,
                parent_size,
                parent_writing_mode: writing_mode,
                available_space: child_available_space,
                vertical_margins_are_collapsible: if item.is_in_same_bfc { Line::TRUE } else { Line::FALSE },
            };

            #[cfg(feature = "float_layout")]
            let clear_threshold = block_ctx.cleared_threshold(item.clear);
            #[cfg(feature = "float_layout")]
            let clear_pos = clear_threshold.unwrap_or(f32::NEG_INFINITY);
            #[cfg(not(feature = "float_layout"))]
            let clear_pos = f32::NEG_INFINITY;

            let item_layout = if item.is_in_same_bfc {
                // Replaced elements may not have a known width (they are sized by their
                // measure function rather than stretch-sized)
                let width = known_dimensions.width.unwrap_or(stretch_width);

                // TODO: account for auto margins
                let inset_left = item_non_auto_margin.left + content_box_inset.left;
                let inset_right = container_outer_width - width - inset_left;
                let insets = [inset_left, inset_right];

                // Compute child layout
                let mut child_block_ctx =
                    block_ctx.sub_context((y_offset_for_absolute + item_non_auto_margin.top).max(clear_pos), insets);
                let output = tree.compute_block_child_layout(item.node_id, inputs, Some(&mut child_block_ctx));

                // Extract float contribution from child block context
                #[cfg(feature = "float_layout")]
                {
                    let child_contribution = child_block_ctx.floated_content_height_contribution();
                    let child_top_adjoining_floats = child_block_ctx.top_adjoining_floats();
                    block_ctx.add_child_floated_content_height_contribution(y_offset_for_absolute + child_contribution);
                    // Floats placed while the position of the child's top margin strut was unresolved
                    // also adjoin this block's current strut
                    block_ctx.merge_adjoining_floats(child_top_adjoining_floats);
                }

                output
            } else {
                tree.compute_child_layout(item.node_id, inputs)
            };
            item.depends_on_block_constraints |= item_layout.block_constraint_dependency();
            let final_size = item_layout.size;

            let top_margin_set = item_layout.top_margin.collapse_with_margin(item_margin.top.unwrap_or(0.0));
            let bottom_margin_set = item_layout.bottom_margin.collapse_with_margin(item_margin.bottom.unwrap_or(0.0));

            // Expand auto margins to fill available space
            // Note: Vertical auto-margins for relatively positioned block items simply resolve to 0.
            // See: https://www.w3.org/TR/CSS21/visudet.html#abs-non-replaced-width
            let free_x_space = f32_max(0.0, stretch_width - final_size.width);
            let x_axis_auto_margin_size = {
                let auto_margin_count = item_margin.left.is_none() as u8 + item_margin.right.is_none() as u8;
                if auto_margin_count > 0 {
                    free_x_space / auto_margin_count as f32
                } else {
                    0.0
                }
            };
            let resolved_margin = Rect {
                left: item_margin.left.unwrap_or(x_axis_auto_margin_size),
                right: item_margin.right.unwrap_or(x_axis_auto_margin_size),
                top: top_margin_set.resolve(),
                bottom: bottom_margin_set.resolve(),
            };

            // Resolve item inset
            let inset = item
                .inset
                .zip_size(relative_inset_parent_size, |p, s| p.maybe_resolve(s, |val, basis| tree.calc(val, basis)));
            let inset_offset = Point {
                x: if direction.is_rtl() {
                    inset.right.map(|x| -x).or(inset.left).unwrap_or(0.0)
                } else {
                    inset.left.or(inset.right.map(|x| -x)).unwrap_or(0.0)
                },
                y: inset.top.or(inset.bottom.map(|x| -x)).unwrap_or(0.0),
            };

            // Set y_margin_offset (same bfc child)
            if item.is_in_same_bfc
                && (!is_collapsing_with_first_margin_set || !own_margins_collapse_with_children.start)
            {
                y_margin_offset = active_collapsible_margin_set.collapse_with_set(top_margin_set).resolve()
            };

            // Compute clearance (CSS2.2 9.5.2). Clearance is introduced if the hypothetical position of the
            // item's top border edge (the position it would have with normal margin collapsing) is not past
            // the bottom of the relevant floats. When clearance is introduced the item's border edge is
            // placed at `max(float bottom, hypothetical position)`.
            #[cfg(feature = "float_layout")]
            let mut has_clearance = false;
            #[cfg(not(feature = "float_layout"))]
            let has_clearance = false;
            #[cfg(feature = "float_layout")]
            if item.is_in_same_bfc {
                if let Some(threshold) = clear_threshold {
                    // The hypothetical position always includes the item's collapsed top margin set, even
                    // when those margins collapse with the container's own top margin (and are thus applied
                    // outside the container): in that case they still move the container (and hence the item)
                    // relative to the floats.
                    let hypothetical_y =
                        committed_y_offset + active_collapsible_margin_set.collapse_with_set(top_margin_set).resolve();
                    // Clearance is forced (regardless of the hypothetical position) if a relevant float is
                    // adjoining the margin-collapse strut that the item's top margin would collapse into:
                    // if the margins were allowed to collapse they would pull the float down with the item,
                    // so clearance is inserted to separate the two, placing the item just below the float.
                    let forced_clearance = block_ctx.has_adjoining_float(item.clear);
                    if forced_clearance || hypothetical_y < threshold {
                        has_clearance = true;
                        // Clearance stops the item's top margin collapsing with preceding margins. If those
                        // preceding margins collapse with the container's own top margin they are applied
                        // outside the container (moving it down), so the item's cleared position within the
                        // container must be reduced by that amount to keep its absolute position correct.
                        let escaped_margin =
                            if is_collapsing_with_first_margin_set && own_margins_collapse_with_children.start {
                                active_collapsible_margin_set.resolve()
                            } else {
                                0.0
                            };
                        y_margin_offset = threshold - committed_y_offset - escaped_margin;
                    }
                }
            }

            item.computed_size = item_layout.size;
            item.can_be_collapsed_through = item_layout.margins_can_collapse_through && !has_clearance;
            item.static_position = if item.is_in_same_bfc {
                let uncleared_y = committed_y_offset + active_collapsible_margin_set.resolve();
                Point {
                    x: match direction {
                        Direction::Ltr => content_box_inset.left,
                        Direction::Rtl => container_outer_width - content_box_inset.right - final_size.width,
                    },
                    y: uncleared_y.max(clear_pos),
                }
            } else {
                // TODO: handle inset and margins
                Point {
                    x: match direction {
                        Direction::Ltr => float_avoiding_position.x,
                        Direction::Rtl => float_avoiding_position.x + float_avoiding_width - final_size.width,
                    },
                    y: float_avoiding_position.y,
                }
            };
            let mut location = if item.is_in_same_bfc {
                Point {
                    x: match direction {
                        Direction::Ltr => content_box_inset.left + inset_offset.x + resolved_margin.left,
                        Direction::Rtl => {
                            container_outer_width - content_box_inset.right - final_size.width - resolved_margin.right
                                + inset_offset.x
                        }
                    },
                    y: committed_y_offset + y_margin_offset + inset_offset.y,
                }
            } else {
                // When the item avoids floats, its non-auto margins are already accounted for in the
                // slot's border-box position/width (margins may overlap floats), so only the auto
                // portion of the resolved margin is added here.
                #[cfg(feature = "float_layout")]
                let (extra_margin_left, extra_margin_right) = if item_avoids_floats {
                    (
                        resolved_margin.left - item_non_auto_margin.left,
                        resolved_margin.right - item_non_auto_margin.right,
                    )
                } else {
                    (resolved_margin.left, resolved_margin.right)
                };
                #[cfg(not(feature = "float_layout"))]
                let (extra_margin_left, extra_margin_right) = (resolved_margin.left, resolved_margin.right);

                // TODO: handle inset and margins
                Point {
                    x: match direction {
                        Direction::Ltr => float_avoiding_position.x + extra_margin_left + inset_offset.x,
                        Direction::Rtl => {
                            float_avoiding_position.x + float_avoiding_width - final_size.width - extra_margin_right
                                + inset_offset.x
                        }
                    },
                    y: float_avoiding_position.y + inset_offset.y,
                }
            };

            // Apply alignment
            let item_outer_width = item_layout.size.width + resolved_margin.horizontal_axis_sum();
            if item_outer_width < container_inner_width {
                let free_x_space = container_inner_width - item_outer_width;
                match (text_align, direction) {
                    (TextAlign::Auto, _) => {
                        // Do nothing
                    }
                    (TextAlign::LegacyLeft, Direction::Ltr) => {
                        // Do nothing. Left aligned by default.
                    }
                    (TextAlign::LegacyLeft, Direction::Rtl) => location.x -= free_x_space,
                    (TextAlign::LegacyRight, Direction::Ltr) => location.x += free_x_space,
                    (TextAlign::LegacyRight, Direction::Rtl) => {
                        // Do nothing. Right aligned by default.
                    }
                    (TextAlign::LegacyCenter, Direction::Ltr) => location.x += free_x_space / 2.0,
                    (TextAlign::LegacyCenter, Direction::Rtl) => location.x -= free_x_space / 2.0,
                }
            }

            // A block container's first baseline is the first baseline of its first in-flow child
            // that has one.
            if first_baseline.is_none() {
                first_baseline = item_layout.first_baselines.y.map(|baseline| location.y + baseline);
            }

            // CSS inline-block baseline propagation walks normal-flow block
            // descendants. Block-layout children contribute their last
            // baseline; other formatting contexts contribute their first.
            // A scroll-container block instead forces synthesis at its
            // block-end margin edge (CSS2 10.8 / CSS Inline 3).
            if !item.is_table {
                let child_baseline = if item.uses_block_layout && !item.is_replaced {
                    if item.overflow.x.is_scroll_container() || item.overflow.y.is_scroll_container() {
                        Some(item_layout.size.height + resolved_margin.bottom)
                    } else {
                        item_layout.last_baselines.y
                    }
                } else {
                    item_layout.first_baselines.y
                };
                if let Some(baseline) = child_baseline {
                    last_baseline = Some(location.y + baseline);
                }
            }

            // Defer `set_unrounded_layout` to the post-loop pass in `compute_inner` so that
            // `align-content` can shift `location.y` before the layout is committed to the tree.
            item.final_layout = Some(Layout {
                order: item.order,
                size: item_layout.size,
                #[cfg(feature = "content_size")]
                content_size: item_layout.content_size,
                scrollbar_size,
                location,
                padding: item.padding,
                border: item.border,
                margin: resolved_margin,
            });

            #[cfg(feature = "content_size")]
            {
                let contribution_location = content_size_contribution_location(
                    location,
                    final_size,
                    container_outer_width,
                    border,
                    scrollbar_insets,
                    direction,
                );
                inflow_content_size = inflow_content_size.f32_max(compute_content_size_contribution(
                    contribution_location,
                    final_size,
                    item_layout.content_size,
                    item.overflow,
                ));
            }

            // Update first_child_top_margin_set
            //
            // The top margin of an item with clearance does not collapse with the container's top margin,
            // so clearance terminates collapsing without contributing the item's own margins.
            #[cfg(feature = "float_layout")]
            if is_collapsing_with_first_margin_set && item_pushed_below_float {
                // The item's top margin "separated from the float" and must not
                // propagate to the parent
                is_collapsing_with_first_margin_set = false;
            }
            if is_collapsing_with_first_margin_set && has_clearance {
                is_collapsing_with_first_margin_set = false;
            } else if is_collapsing_with_first_margin_set {
                if item.can_be_collapsed_through {
                    first_child_top_margin_set = first_child_top_margin_set
                        .collapse_with_set(top_margin_set)
                        .collapse_with_set(bottom_margin_set);
                } else {
                    first_child_top_margin_set = first_child_top_margin_set.collapse_with_set(top_margin_set);
                    is_collapsing_with_first_margin_set = false;
                }
            }

            // Update active_collapsible_margin_set
            if item.can_be_collapsed_through {
                active_collapsible_margin_set = active_collapsible_margin_set
                    .collapse_with_set(top_margin_set)
                    .collapse_with_set(bottom_margin_set);
                y_offset_for_absolute = committed_y_offset + item_layout.size.height + y_margin_offset;
            } else {
                committed_y_offset = location.y - inset_offset.y + item_layout.size.height;
                // A self-collapsing item with clearance is not collapsed through (its margins do not collapse
                // with margins of preceding siblings), but its top and bottom margins still collapse with each
                // other and with the margins of following siblings.
                if has_clearance && item_layout.margins_can_collapse_through {
                    // The element's border edge stays at the cleared position, but its collapsed margin
                    // extends below it: the border edge sits `top margin` inside the collapsed margin, so
                    // following content is offset by `collapsed margin - top margin` from the border edge.
                    committed_y_offset -= top_margin_set.resolve();
                    active_collapsible_margin_set = top_margin_set.collapse_with_set(bottom_margin_set);
                    active_margin_set_has_clearance = true;
                } else {
                    active_collapsible_margin_set = bottom_margin_set;
                    active_margin_set_has_clearance = false;
                }
                y_offset_for_absolute = committed_y_offset + active_collapsible_margin_set.resolve();
                // Committing in-flow content resolves the position of the current margin-collapse strut,
                // so floats placed before this point no longer force clearance
                #[cfg(feature = "float_layout")]
                block_ctx.commit_strut();
            }
        }
    }

    // The margins of a self-collapsing element with clearance do not collapse with the bottom
    // margin of the parent block: they extend the parent's content height instead of escaping it
    let last_child_bottom_margin_set =
        if active_margin_set_has_clearance { CollapsibleMarginSet::ZERO } else { active_collapsible_margin_set };
    let bottom_y_margin_offset = if active_margin_set_has_clearance {
        active_collapsible_margin_set.resolve()
    } else if own_margins_collapse_with_children.end {
        0.0
    } else {
        last_child_bottom_margin_set.resolve()
    };

    committed_y_offset += content_box_inset.bottom + bottom_y_margin_offset;
    let content_height = f32_max(0.0, committed_y_offset);
    (
        inflow_content_size,
        content_height,
        first_child_top_margin_set,
        last_child_bottom_margin_set,
        first_baseline,
        last_baseline,
    )
}

/// Resolve auto margins in one axis of an absolutely positioned box.
///
/// Auto margins only participate when both insets in the axis are definite.
/// In the inline axis, negative free space is assigned to the non-dominant
/// side so the direction's start edge remains visible. In the block axis it
/// is shared equally, matching CSS Positioned Layout and browser behavior.
#[inline]
fn resolve_absolute_axis_margins(
    margin: Line<Option<f32>>,
    inset: Line<Option<f32>>,
    area_size: f32,
    box_size: f32,
    share_negative_space: bool,
    start_is_dominant: bool,
) -> Line<f32> {
    if inset.start.is_none() || inset.end.is_none() {
        return Line { start: margin.start.unwrap_or(0.0), end: margin.end.unwrap_or(0.0) };
    }

    let free_space = area_size
        - inset.start.unwrap()
        - inset.end.unwrap()
        - box_size
        - margin.start.unwrap_or(0.0)
        - margin.end.unwrap_or(0.0);

    match (margin.start, margin.end) {
        (Some(start), Some(end)) => Line { start, end },
        (None, Some(end)) => Line { start: free_space, end },
        (Some(start), None) => Line { start, end: free_space },
        (None, None) if free_space > 0.0 || share_negative_space => {
            let start = free_space / 2.0;
            Line { start, end: free_space - start }
        }
        (None, None) if start_is_dominant => Line { start: 0.0, end: free_space },
        (None, None) => Line { start: free_space, end: 0.0 },
    }
}

/// Perform absolute layout on all absolutely positioned children.
#[inline]
fn perform_absolute_layout_on_absolute_children(
    tree: &mut impl LayoutBlockContainer,
    items: &[BlockItem],
    area_size: Size<f32>,
    area_offset: Point<f32>,
    direction: Direction,
    writing_mode: WritingMode,
) -> Size<f32> {
    let area_width = area_size.width;
    let area_height = area_size.height;
    let percentage_basis = writing_mode.to_logical(area_size).inline_size;

    #[cfg_attr(not(feature = "content_size"), allow(unused_mut))]
    let mut absolute_content_size = Size::ZERO;

    for item in items.iter().filter(|item| item.position == Position::Absolute) {
        let aspect_ratio = tree.get_resolved_aspect_ratio(item.node_id);
        let child_writing_mode = tree.get_writing_mode(item.node_id);
        let child_style = tree.get_block_child_style(item.node_id);

        // Skip items that are display:none or are not position:absolute
        if child_style.box_generation_mode() == BoxGenerationMode::None || child_style.position() != Position::Absolute
        {
            continue;
        }

        let margin = child_style
            .margin()
            .map(|margin| margin.resolve_to_option(percentage_basis, |val, basis| tree.calc(val, basis)));
        let padding = child_style.padding().resolve_or_zero(Some(percentage_basis), |val, basis| tree.calc(val, basis));
        let border = child_style.border().resolve_or_zero(Some(percentage_basis), |val, basis| tree.calc(val, basis));
        let padding_border_sum = (padding + border).sum_axes();
        let box_sizing = child_style.box_sizing();
        let box_sizing_adjustment = if box_sizing == BoxSizing::ContentBox { padding_border_sum } else { Size::ZERO };

        // Resolve inset
        let left = child_style.inset().left.maybe_resolve(area_width, |val, basis| tree.calc(val, basis));
        let right = child_style.inset().right.maybe_resolve(area_width, |val, basis| tree.calc(val, basis));
        let top = child_style.inset().top.maybe_resolve(area_height, |val, basis| tree.calc(val, basis));
        let bottom = child_style.inset().bottom.maybe_resolve(area_height, |val, basis| tree.calc(val, basis));
        let block_auto_behavior = match child_writing_mode.block_axis() {
            AbsoluteAxis::Horizontal if left.is_some() && right.is_some() => AutoSizeBehavior::StretchExplicit,
            AbsoluteAxis::Vertical if top.is_some() && bottom.is_some() => AutoSizeBehavior::StretchExplicit,
            _ => AutoSizeBehavior::FitContent,
        };

        // Keep the unresolved values: intrinsic widths need the absolute
        // containing block after insets and non-auto margins are known.
        let raw_size = child_style.size();
        let raw_min_size = child_style.min_size();
        let raw_max_size = child_style.max_size();

        // Compute numeric known dimensions from min/max/inherent size styles.
        let mut style_size =
            raw_size.maybe_resolve(area_size, |val, basis| tree.calc(val, basis)).maybe_add(box_sizing_adjustment);
        let mut min_size =
            raw_min_size.maybe_resolve(area_size, |val, basis| tree.calc(val, basis)).maybe_add(box_sizing_adjustment);
        let mut max_size =
            raw_max_size.maybe_resolve(area_size, |val, basis| tree.calc(val, basis)).maybe_add(box_sizing_adjustment);

        drop(child_style);

        let non_auto_margin_width = margin.left.unwrap_or(0.0) + margin.right.unwrap_or(0.0);
        let static_position_in_area = item.static_position.x - area_offset.x;
        let available_width = match (left, right) {
            (Some(left), Some(right)) => area_width - left - right,
            (Some(left), None) => area_width - left,
            (None, Some(right)) => area_width - right,
            (None, None) if direction.is_rtl() => static_position_in_area,
            (None, None) => area_width - static_position_in_area,
        } - non_auto_margin_width;
        let intrinsic_inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::IntrinsicContribution,
            axis: RequestedAxis::Horizontal,
            block_auto_behavior: crate::AutoSizeBehavior::FitContent,
            known_dimensions: Size { width: None, height: style_size.height },
            definite_dimensions: Size::NONE,
            parent_size: area_size.map(Some),
            parent_writing_mode: writing_mode,
            available_space: Size {
                width: AvailableSpace::Definite(f32_max(available_width, 0.0)),
                height: AvailableSpace::Definite(area_height),
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let intrinsic = resolve_intrinsic_width_constraints(
            tree,
            item.node_id,
            intrinsic_inputs,
            raw_size.width,
            raw_min_size.width,
            raw_max_size.width,
            intrinsic_inputs.available_space.width,
        );
        style_size.width = style_size.width.or(intrinsic.preferred);
        min_size.width = min_size.width.or(intrinsic.min);
        max_size.width = max_size.width.or(intrinsic.max);

        let resolved = resolve_size_constraints(SizeConstraintInput {
            size: style_size,
            min_size,
            max_size,
            size_is_auto: raw_size.map(|dimension| dimension.is_auto()),
            writing_mode: child_writing_mode,
            block_auto_behavior,
            transferred_sizes_mode: TransferredSizesMode::Normal,
            aspect_ratio,
            padding_border: padding_border_sum,
        });
        let min_size = resolved.min_size.or(padding_border_sum.map(Some)).maybe_max(padding_border_sum);
        let max_size = resolved.max_size;
        let mut known_dimensions = resolved.size.maybe_clamp(min_size, max_size);

        // Fill in width from left/right and reapply aspect ratio if:
        //   - Width is not already known
        //   - Item has both left and right inset properties set
        if let (None, Some(left), Some(right)) = (known_dimensions.width, left, right) {
            let new_width_raw = area_width.maybe_sub(margin.left).maybe_sub(margin.right) - left - right;
            known_dimensions.width = Some(f32_max(new_width_raw, 0.0));
            known_dimensions = known_dimensions
                .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border_sum)
                .maybe_clamp(min_size, max_size);
        }

        // Fill in height from top/bottom and reapply aspect ratio if:
        //   - Height is not already known
        //   - Item has both top and bottom inset properties set
        if let (None, Some(top), Some(bottom)) = (known_dimensions.height, top, bottom) {
            let new_height_raw = area_height.maybe_sub(margin.top).maybe_sub(margin.bottom) - top - bottom;
            known_dimensions.height = Some(f32_max(new_height_raw, 0.0));
            known_dimensions = known_dimensions
                .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border_sum)
                .maybe_clamp(min_size, max_size);
        }

        // If width is still auto then one or both horizontal insets are also auto. CSS 2.2
        // 10.3.7 requires a shrink-to-fit width rather than the unconstrained content width.
        // Account for the specified inset (or the static position when both are auto) and
        // non-auto margins before clamping the available width between min/max-content.
        if known_dimensions.width.is_none() {
            known_dimensions.width = Some(fit_content_width(
                tree,
                item.node_id,
                ChildLayoutInput::new(
                    known_dimensions,
                    area_size.map(Some),
                    writing_mode,
                    Size {
                        width: AvailableSpace::Definite(available_width),
                        height: AvailableSpace::Definite(area_height.maybe_clamp(min_size.height, max_size.height)),
                    },
                    SizingMode::ContentSize,
                    Line::FALSE,
                ),
                available_width,
            ));
            known_dimensions = known_dimensions
                .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, padding_border_sum)
                .maybe_clamp(min_size, max_size);
        }

        let measured_size = tree.measure_child_size_both(
            item.node_id,
            ChildLayoutInput::new(
                known_dimensions,
                area_size.map(Some),
                writing_mode,
                Size {
                    width: AvailableSpace::Definite(area_width.maybe_clamp(min_size.width, max_size.width)),
                    height: AvailableSpace::Definite(area_height.maybe_clamp(min_size.height, max_size.height)),
                },
                SizingMode::ContentSize,
                Line::FALSE,
            ),
        );

        let final_size = known_dimensions.unwrap_or(measured_size).maybe_clamp(min_size, max_size);

        let layout_output = tree.compute_child_layout(
            item.node_id,
            LayoutInput {
                known_dimensions: final_size.map(Some),
                definite_dimensions: known_dimensions,
                parent_size: area_size.map(Some),
                parent_writing_mode: writing_mode,
                available_space: Size {
                    width: AvailableSpace::Definite(area_width.maybe_clamp(min_size.width, max_size.width)),
                    height: AvailableSpace::Definite(area_height.maybe_clamp(min_size.height, max_size.height)),
                },
                sizing_mode: SizingMode::ContentSize,
                sizing_purpose: SizingPurpose::Layout,
                axis: RequestedAxis::Both,
                block_auto_behavior: crate::AutoSizeBehavior::FitContent,
                run_mode: RunMode::PerformLayout,
                vertical_margins_are_collapsible: Line::FALSE,
            },
        );

        let horizontal_margin = resolve_absolute_axis_margins(
            Line { start: margin.left, end: margin.right },
            Line { start: left, end: right },
            area_width,
            final_size.width,
            false,
            !direction.is_rtl(),
        );
        let vertical_margin = resolve_absolute_axis_margins(
            Line { start: margin.top, end: margin.bottom },
            Line { start: top, end: bottom },
            area_height,
            final_size.height,
            true,
            true,
        );
        let resolved_margin = Rect {
            left: horizontal_margin.start,
            right: horizontal_margin.end,
            top: vertical_margin.start,
            bottom: vertical_margin.end,
        };

        let x_offset = match (left, right) {
            (Some(left), Some(right)) => {
                if direction.is_rtl() {
                    area_size.width - final_size.width - right - resolved_margin.right
                } else {
                    left + resolved_margin.left
                }
            }
            (Some(left), None) => left + resolved_margin.left,
            (None, Some(right)) => area_size.width - final_size.width - right - resolved_margin.right,
            (None, None) => {
                if direction.is_rtl() {
                    item.static_position.x - final_size.width - resolved_margin.right - area_offset.x
                } else {
                    item.static_position.x + resolved_margin.left - area_offset.x
                }
            }
        };
        let location = Point {
            x: x_offset + area_offset.x,
            y: top
                .map(|top| top + resolved_margin.top)
                .or(bottom.map(|bottom| area_size.height - final_size.height - bottom - resolved_margin.bottom))
                .maybe_add(area_offset.y)
                .unwrap_or(item.static_position.y + resolved_margin.top),
        };
        let scrollbar_size = item.scrollbar_size;

        tree.set_unrounded_layout(
            item.node_id,
            &Layout {
                order: item.order,
                size: final_size,
                #[cfg(feature = "content_size")]
                content_size: layout_output.content_size,
                scrollbar_size,
                location,
                padding,
                border,
                margin: resolved_margin,
            },
        );

        #[cfg(feature = "content_size")]
        {
            let relative_location = Point { x: location.x - area_offset.x, y: location.y - area_offset.y };
            absolute_content_size = absolute_content_size.f32_max(compute_content_size_contribution(
                relative_location,
                final_size,
                layout_output.content_size,
                item.overflow,
            ));
        }
    }

    absolute_content_size
}
