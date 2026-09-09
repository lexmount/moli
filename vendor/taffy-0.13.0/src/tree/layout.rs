//! Final data structures that represent the high-level UI layout
use crate::geometry::{AbsoluteAxis, Line, LogicalSize, Point, Rect, Size, WritingMode};
use crate::style::AvailableSpace;
use crate::style_helpers::TaffyMaxContent;
use crate::util::sys::{f32_max, f32_min};

/// Whether we are performing a full layout, or we merely need to size the node
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum RunMode {
    /// A full layout for this node and all children should be computed
    PerformLayout,
    /// The layout algorithm should be executed such that an accurate container size for the node can be determined.
    /// Layout steps that aren't necessary for determining the container size of the current node can be skipped.
    ComputeSize,
    /// This node should have a null layout set as it has been hidden (i.e. using `Display::None`)
    PerformHiddenLayout,
}

/// Whether styles should be taken into account when computing size
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum SizingMode {
    /// Only content contributions should be taken into account
    ContentSize,
    /// Inherent size styles should be taken into account in addition to content contributions
    InherentSize,
}

/// The purpose for which a node's size is being computed.
///
/// This is independent from [`RunMode`]: both normal layout and an intrinsic
/// contribution may only need a size, but percentage-dependent sizing rules
/// need to distinguish the two cases.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum SizingPurpose {
    /// Resolve the node's final used size and lay out its contents normally.
    Layout,
    /// Compute a min/max-content contribution for an ancestor whose size may
    /// depend on this node.
    IntrinsicContribution,
}

/// How an authored `auto` size behaves in the logical block axis.
///
/// This belongs to the constraint space rather than style: the containing
/// formatting context decides whether `auto` is content-sized or stretched
/// when it supplies known dimensions. Keeping the resolution order explicit
/// is required when a preferred aspect ratio can synthesize the block size.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum AutoSizeBehavior {
    /// Resolve `auto` from the formatting context's content contribution.
    #[default]
    FitContent,
    /// Stretch before considering a preferred aspect ratio.
    StretchExplicit,
    /// Stretch only if a preferred aspect ratio did not supply a size.
    StretchImplicit,
}

impl AutoSizeBehavior {
    /// Whether `auto` uses content/intrinsic block-size semantics here.
    #[inline(always)]
    pub const fn is_content_based(self, has_preferred_aspect_ratio: bool) -> bool {
        match self {
            Self::FitContent => true,
            Self::StretchExplicit => false,
            Self::StretchImplicit => has_preferred_aspect_ratio,
        }
    }
}

/// A set of margins that are available for collapsing with for block layout's margin collapsing
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct CollapsibleMarginSet {
    /// The largest positive margin
    positive: f32,
    /// The smallest negative margin (with largest absolute value)
    negative: f32,
}

impl CollapsibleMarginSet {
    /// A default margin set with no collapsible margins
    pub const ZERO: Self = Self { positive: 0.0, negative: 0.0 };

    /// Create a set from a single margin
    pub fn from_margin(margin: f32) -> Self {
        if margin >= 0.0 {
            Self { positive: margin, negative: 0.0 }
        } else {
            Self { positive: 0.0, negative: margin }
        }
    }

    /// Collapse a single margin with this set
    pub fn collapse_with_margin(mut self, margin: f32) -> Self {
        if margin >= 0.0 {
            self.positive = f32_max(self.positive, margin);
        } else {
            self.negative = f32_min(self.negative, margin);
        }
        self
    }

    /// Collapse another margin set with this set
    pub fn collapse_with_set(mut self, other: CollapsibleMarginSet) -> Self {
        self.positive = f32_max(self.positive, other.positive);
        self.negative = f32_min(self.negative, other.negative);
        self
    }

    /// Resolve the resultant margin from this set once all collapsible margins
    /// have been collapsed into it
    pub fn resolve(&self) -> f32 {
        self.positive + self.negative
    }
}

/// An axis that layout algorithms can be requested to compute a size for
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum RequestedAxis {
    /// The horizontal axis
    Horizontal,
    /// The vertical axis
    Vertical,
    /// Both axes
    Both,
}

impl RequestedAxis {
    /// Whether this request includes the supplied physical axis.
    #[inline(always)]
    pub const fn contains(self, axis: AbsoluteAxis) -> bool {
        matches!(self, Self::Both)
            || matches!(
                (self, axis),
                (Self::Horizontal, AbsoluteAxis::Horizontal) | (Self::Vertical, AbsoluteAxis::Vertical)
            )
    }
}

impl From<AbsoluteAxis> for RequestedAxis {
    fn from(value: AbsoluteAxis) -> Self {
        match value {
            AbsoluteAxis::Horizontal => RequestedAxis::Horizontal,
            AbsoluteAxis::Vertical => RequestedAxis::Vertical,
        }
    }
}
impl TryFrom<RequestedAxis> for AbsoluteAxis {
    type Error = ();
    fn try_from(value: RequestedAxis) -> Result<Self, Self::Error> {
        match value {
            RequestedAxis::Horizontal => Ok(AbsoluteAxis::Horizontal),
            RequestedAxis::Vertical => Ok(AbsoluteAxis::Vertical),
            RequestedAxis::Both => Err(()),
        }
    }
}

/// A struct containing the inputs constraints/hints for laying out a node, which are passed in by the parent
#[derive(Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayoutInput {
    /// Whether we only need to know the Node's size, or whether we need to perform a full layout
    pub run_mode: RunMode,
    /// Whether a Node's style sizes should be taken into account or ignored
    pub sizing_mode: SizingMode,
    /// Whether this invocation computes final layout or an intrinsic size
    /// contribution.
    pub sizing_purpose: SizingPurpose,
    /// Which axis we need the size of
    pub axis: RequestedAxis,
    /// Resolution behavior for an authored logical `block-size: auto`.
    pub block_auto_behavior: AutoSizeBehavior,

    /// Known dimensions represent dimensions (width/height) which should be taken as fixed when performing layout.
    /// For example, if known_dimensions.width is set to Some(WIDTH) then this means something like:
    ///
    ///    "What would the height of this node be, assuming the width is WIDTH"
    ///
    /// Layout functions will be called with both known_dimensions set for final layout. Where the meaning is:
    ///
    ///   "The exact size of this node is WIDTHxHEIGHT. Please lay out your children"
    ///
    pub known_dimensions: Size<Option<f32>>,
    /// Definite dimensions of this node which descendants may use as a percentage basis.
    ///
    /// This usually matches `known_dimensions`, but differs when a parent's final
    /// used size is known without making the corresponding CSS size definite. For
    /// example, an absolutely positioned auto-height box clamped by `min-height`
    /// has a known final height, while percentage block-axis insets in its children
    /// must remain unresolved.
    pub definite_dimensions: Size<Option<f32>>,
    /// Parent size dimensions are intended to be used for percentage resolution.
    pub parent_size: Size<Option<f32>>,
    /// Writing mode of the containing block that supplied `parent_size`.
    ///
    /// This is required to recover the containing block's logical inline size
    /// after the physical constraints have been projected into an orthogonal
    /// child's coordinate space.
    pub parent_writing_mode: WritingMode,
    /// Available space represents an amount of space to layout into, and is used as a soft constraint
    /// for the purpose of wrapping.
    pub available_space: Size<AvailableSpace>,
    /// Specific to CSS Block layout. Used for correctly computing margin collapsing. You probably want to set this to `Line::FALSE`.
    pub vertical_margins_are_collapsible: Line<bool>,
}

impl LayoutInput {
    /// A LayoutInput that can be used to request hidden layout
    pub const HIDDEN: LayoutInput = LayoutInput {
        // The important property for hidden layout
        run_mode: RunMode::PerformHiddenLayout,
        // The rest will be ignored
        known_dimensions: Size::NONE,
        definite_dimensions: Size::NONE,
        parent_size: Size::NONE,
        parent_writing_mode: WritingMode::HorizontalTb,
        available_space: Size::MAX_CONTENT,
        sizing_mode: SizingMode::InherentSize,
        sizing_purpose: SizingPurpose::Layout,
        axis: RequestedAxis::Both,
        block_auto_behavior: AutoSizeBehavior::FitContent,
        vertical_margins_are_collapsible: Line::FALSE,
    };

    /// Project the physical tree-boundary inputs into `writing_mode`'s logical
    /// axes.
    #[inline(always)]
    pub fn constraint_space(self, writing_mode: WritingMode) -> ConstraintSpace {
        ConstraintSpace {
            run_mode: self.run_mode,
            sizing_mode: self.sizing_mode,
            sizing_purpose: self.sizing_purpose,
            block_auto_behavior: self.block_auto_behavior,
            writing_mode,
            parent_writing_mode: self.parent_writing_mode,
            known_size: writing_mode.to_logical(self.known_dimensions),
            definite_size: writing_mode.to_logical(self.definite_dimensions),
            percentage_resolution_size: writing_mode.to_logical(self.parent_size),
            available_size: writing_mode.to_logical(self.available_space),
            requested_axis: self.axis,
            vertical_margins_are_collapsible: self.vertical_margins_are_collapsible,
        }
    }
}

/// Inputs shared by intrinsic measurement and final layout of a child.
///
/// Formatting algorithms construct this value at the containing-block
/// boundary. The tree helpers then add only the operation-specific run mode,
/// purpose and requested axis, preventing those two paths from drifting apart
/// as constraint-space state grows.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct ChildLayoutInput {
    /// Child border-box dimensions that are already known.
    pub known_dimensions: Size<Option<f32>>,
    /// Physical containing-block dimensions used to resolve child percentages.
    pub parent_size: Size<Option<f32>>,
    /// Writing mode that establishes the containing block's logical axes.
    pub parent_writing_mode: WritingMode,
    /// Physical available space offered by the parent formatting algorithm.
    pub available_space: Size<AvailableSpace>,
    /// Whether authored size constraints participate in child sizing.
    pub sizing_mode: SizingMode,
    /// Resolution behavior for an authored logical `block-size: auto`.
    pub block_auto_behavior: AutoSizeBehavior,
    /// Whether the child's physical vertical margins may collapse through the boundary.
    pub vertical_margins_are_collapsible: Line<bool>,
}

impl ChildLayoutInput {
    /// Construct the shared inputs at a parent-to-child layout boundary.
    #[inline(always)]
    pub const fn new(
        known_dimensions: Size<Option<f32>>,
        parent_size: Size<Option<f32>>,
        parent_writing_mode: WritingMode,
        available_space: Size<AvailableSpace>,
        sizing_mode: SizingMode,
        vertical_margins_are_collapsible: Line<bool>,
    ) -> Self {
        Self {
            known_dimensions,
            parent_size,
            parent_writing_mode,
            available_space,
            sizing_mode,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            vertical_margins_are_collapsible,
        }
    }

    /// Set the containing formatting context's block-axis auto behavior.
    #[inline(always)]
    pub const fn with_block_auto_behavior(mut self, behavior: AutoSizeBehavior) -> Self {
        self.block_auto_behavior = behavior;
        self
    }

    /// Convert shared child inputs into an intrinsic measurement request.
    #[inline(always)]
    pub const fn into_measurement(self, axis: RequestedAxis) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: self.sizing_mode,
            sizing_purpose: SizingPurpose::IntrinsicContribution,
            axis,
            block_auto_behavior: self.block_auto_behavior,
            known_dimensions: self.known_dimensions,
            definite_dimensions: self.known_dimensions,
            parent_size: self.parent_size,
            parent_writing_mode: self.parent_writing_mode,
            available_space: self.available_space,
            vertical_margins_are_collapsible: self.vertical_margins_are_collapsible,
        }
    }

    /// Convert shared child inputs into a final layout request.
    #[inline(always)]
    pub const fn into_layout(self) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: self.sizing_mode,
            sizing_purpose: SizingPurpose::Layout,
            axis: RequestedAxis::Both,
            block_auto_behavior: self.block_auto_behavior,
            known_dimensions: self.known_dimensions,
            definite_dimensions: self.known_dimensions,
            parent_size: self.parent_size,
            parent_writing_mode: self.parent_writing_mode,
            available_space: self.available_space,
            vertical_margins_are_collapsible: self.vertical_margins_are_collapsible,
        }
    }
}

/// A flow-relative view of the constraints passed to one layout algorithm.
///
/// Like Blink's constraint space, this records both the node's writing mode and
/// whether its axes are orthogonal to its containing block. Physical sizes are
/// converted only at tree and fragment boundaries.
#[derive(Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ConstraintSpace {
    /// Whether the node is being measured or fully laid out.
    pub run_mode: RunMode,
    /// Whether authored size constraints participate in this computation.
    pub sizing_mode: SizingMode,
    /// Whether this computation produces final layout or an intrinsic
    /// contribution.
    pub sizing_purpose: SizingPurpose,
    /// Resolution behavior for an authored logical `block-size: auto`.
    pub block_auto_behavior: AutoSizeBehavior,
    /// Writing mode whose logical axes own these sizes.
    pub writing_mode: WritingMode,
    /// Writing mode of the containing block that produced this space.
    pub parent_writing_mode: WritingMode,
    /// Fixed dimensions supplied by the parent, in child logical axes.
    pub known_size: LogicalSize<Option<f32>>,
    /// Definite dimensions usable as descendant percentage bases.
    pub definite_size: LogicalSize<Option<f32>>,
    /// Containing-block percentage basis, in child logical axes.
    pub percentage_resolution_size: LogicalSize<Option<f32>>,
    /// Soft available-space constraint, in child logical axes.
    pub available_size: LogicalSize<AvailableSpace>,
    /// Physical-axis request retained for compatibility with current callers.
    requested_axis: RequestedAxis,
    /// Block-start/end margin-collapse permissions for block layout.
    pub vertical_margins_are_collapsible: Line<bool>,
}

impl ConstraintSpace {
    /// Convert this logical constraint space back to the physical input used at
    /// the tree dispatch boundary.
    #[inline(always)]
    pub fn into_layout_input(self) -> LayoutInput {
        LayoutInput {
            run_mode: self.run_mode,
            sizing_mode: self.sizing_mode,
            sizing_purpose: self.sizing_purpose,
            axis: self.requested_axis,
            block_auto_behavior: self.block_auto_behavior,
            known_dimensions: self.writing_mode.to_physical(self.known_size),
            definite_dimensions: self.writing_mode.to_physical(self.definite_size),
            parent_size: self.writing_mode.to_physical(self.percentage_resolution_size),
            parent_writing_mode: self.parent_writing_mode,
            available_space: self.writing_mode.to_physical(self.available_size),
            vertical_margins_are_collapsible: self.vertical_margins_are_collapsible,
        }
    }

    /// Whether this node establishes a flow orthogonal to its containing block.
    #[inline(always)]
    pub const fn is_orthogonal(self) -> bool {
        self.writing_mode.is_orthogonal_to(self.parent_writing_mode)
    }

    /// The containing block's logical inline size used to resolve percentage
    /// margins, padding and borders.
    ///
    /// `percentage_resolution_size` is stored in this node's logical axes, so
    /// project through physical space before reading it in the containing
    /// block's logical coordinate system.
    #[inline(always)]
    pub fn margin_padding_percentage_basis(self) -> Option<f32> {
        let physical_size = self.writing_mode.to_physical(self.percentage_resolution_size);
        self.parent_writing_mode.to_logical(physical_size).inline_size
    }

    /// Whether the caller requested this node's logical inline size.
    #[inline(always)]
    pub const fn requests_inline_size(self) -> bool {
        self.requested_axis.contains(self.writing_mode.inline_axis())
    }

    /// Whether the caller requested this node's logical block size.
    #[inline(always)]
    pub const fn requests_block_size(self) -> bool {
        self.requested_axis.contains(self.writing_mode.block_axis())
    }
}

#[cfg(test)]
mod constraint_space_tests {
    use super::*;

    #[test]
    fn orthogonal_space_recovers_the_containing_blocks_inline_percentage_basis() {
        let input = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::IntrinsicContribution,
            axis: RequestedAxis::Horizontal,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            known_dimensions: Size { width: Some(30.0), height: None },
            definite_dimensions: Size { width: Some(30.0), height: None },
            parent_size: Size { width: Some(100.0), height: Some(200.0) },
            parent_writing_mode: WritingMode::VerticalRl,
            available_space: Size { width: AvailableSpace::MinContent, height: AvailableSpace::MaxContent },
            vertical_margins_are_collapsible: Line { start: true, end: false },
        };

        let space = input.constraint_space(WritingMode::HorizontalTb);

        assert!(space.is_orthogonal());
        assert_eq!(space.percentage_resolution_size.inline_size, Some(100.0));
        assert_eq!(space.percentage_resolution_size.block_size, Some(200.0));
        assert_eq!(space.margin_padding_percentage_basis(), Some(200.0));
        assert!(space.requests_inline_size());
        assert!(!space.requests_block_size());
        assert_eq!(space.into_layout_input(), input);
    }

    #[test]
    fn parallel_vertical_space_uses_its_inline_axis_directly() {
        let input = LayoutInput {
            parent_size: Size { width: Some(100.0), height: Some(200.0) },
            parent_writing_mode: WritingMode::VerticalLr,
            ..LayoutInput::HIDDEN
        };

        let space = input.constraint_space(WritingMode::VerticalRl);

        assert!(!space.is_orthogonal());
        assert_eq!(space.percentage_resolution_size.inline_size, Some(200.0));
        assert_eq!(space.margin_padding_percentage_basis(), Some(200.0));
        assert_eq!(space.into_layout_input(), input);
    }
}

/// The result of an intrinsic size probe.
///
/// Unlike [`LayoutOutput`], this type only carries state that is meaningful
/// while a parent formatting context is measuring a child. Keeping the
/// dependency metadata on the measurement protocol prevents block, flex, and
/// grid algorithms from treating a full layout result as an intrinsic sizing
/// result.
#[derive(Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct IntrinsicSizeResult {
    /// The measured outer size of the node.
    pub size: Size<f32>,
    /// Whether the measured inline contribution can change when the containing
    /// block's block-size changes.
    pub depends_on_block_constraints: bool,
    /// Whether this probe obtained its inline contribution by applying the
    /// node's preferred aspect ratio.
    ///
    /// This is operation-local provenance. A parent contribution resolver may
    /// consume it, but it must not propagate it to the parent's result.
    pub applied_aspect_ratio: bool,
}

impl IntrinsicSizeResult {
    /// Construct an independent intrinsic size result.
    pub const fn from_size(size: Size<f32>) -> Self {
        Self { size, depends_on_block_constraints: false, applied_aspect_ratio: false }
    }
}

/// A struct containing the result of laying a single node, which is returned up to the parent node
///
/// A baseline is the line on which text sits. Your node likely has a baseline if it is a text node, or contains
/// children that may be text nodes. See <https://www.w3.org/TR/css-writing-modes-3/#intro-baselines> for details.
/// If your node does not have a baseline (or you are unsure how to compute it), then simply return `Point::NONE`
/// for the baseline fields.
#[derive(Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct LayoutOutput {
    /// The size of the node
    pub size: Size<f32>,
    /// Transitional transport for the combined low-level dispatcher. Public
    /// layout consumers exchange this state through [`IntrinsicSizeResult`],
    /// not through `LayoutOutput`.
    depends_on_block_constraints: bool,
    /// Transitional transport for operation-local aspect-ratio provenance.
    /// This flag is projected through [`IntrinsicSizeResult`] and is never
    /// exposed as part of the public layout result protocol.
    applied_aspect_ratio: bool,
    #[cfg(feature = "content_size")]
    /// The size of the content within the node
    pub content_size: Size<f32>,
    /// The first baseline of the node in each dimension, if any
    pub first_baselines: Point<Option<f32>>,
    /// The last baseline of the node in each dimension, if any
    pub last_baselines: Point<Option<f32>>,
    /// Top margin that can be collapsed with. This is used for CSS block layout and can be set to
    /// `CollapsibleMarginSet::ZERO` for other layout modes that don't support margin collapsing
    pub top_margin: CollapsibleMarginSet,
    /// Bottom margin that can be collapsed with. This is used for CSS block layout and can be set to
    /// `CollapsibleMarginSet::ZERO` for other layout modes that don't support margin collapsing
    pub bottom_margin: CollapsibleMarginSet,
    /// Whether margins can be collapsed through this node. This is used for CSS block layout and can
    /// be set to `false` for other layout modes that don't support margin collapsing
    pub margins_can_collapse_through: bool,
}

impl LayoutOutput {
    /// An all-zero `LayoutOutput` for hidden nodes
    pub const HIDDEN: Self = Self {
        size: Size::ZERO,
        depends_on_block_constraints: false,
        applied_aspect_ratio: false,
        #[cfg(feature = "content_size")]
        content_size: Size::ZERO,
        first_baselines: Point::NONE,
        last_baselines: Point::NONE,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: false,
    };

    /// A blank layout output
    pub const DEFAULT: Self = Self::HIDDEN;

    /// Constructor to create a `LayoutOutput` from just the size and baselines
    pub fn from_sizes_and_baselines(
        size: Size<f32>,
        #[cfg_attr(not(feature = "content_size"), allow(unused_variables))] content_size: Size<f32>,
        first_baselines: Point<Option<f32>>,
    ) -> Self {
        Self::from_sizes_and_baseline_sets(size, content_size, first_baselines, first_baselines)
    }

    /// Constructor to create a `LayoutOutput` from the size and distinct first/last baselines
    pub fn from_sizes_and_baseline_sets(
        size: Size<f32>,
        #[cfg_attr(not(feature = "content_size"), allow(unused_variables))] content_size: Size<f32>,
        first_baselines: Point<Option<f32>>,
        last_baselines: Point<Option<f32>>,
    ) -> Self {
        Self {
            size,
            depends_on_block_constraints: false,
            applied_aspect_ratio: false,
            #[cfg(feature = "content_size")]
            content_size,
            first_baselines,
            last_baselines,
            top_margin: CollapsibleMarginSet::ZERO,
            bottom_margin: CollapsibleMarginSet::ZERO,
            margins_can_collapse_through: false,
        }
    }

    /// Construct a `LayoutOutput` from just the container and content sizes
    pub fn from_sizes(size: Size<f32>, content_size: Size<f32>) -> Self {
        Self::from_sizes_and_baselines(size, content_size, Point::NONE)
    }

    /// Construct a `LayoutOutput` from just the container's size.
    pub fn from_outer_size(size: Size<f32>) -> Self {
        Self::from_sizes(size, Size::zero())
    }

    /// Add block-constraint dependency metadata to a measured result.
    #[inline(always)]
    pub(crate) fn with_block_constraint_dependency(mut self, depends: bool) -> Self {
        self.depends_on_block_constraints |= depends;
        self
    }

    /// Return whether this combined dispatcher result depends on the parent
    /// block constraint.
    #[inline(always)]
    pub(crate) fn block_constraint_dependency(&self) -> bool {
        self.depends_on_block_constraints
    }

    /// Replace the dependency state at the node sizing boundary.
    #[inline(always)]
    pub(crate) fn set_block_constraint_dependency(&mut self, depends: bool) {
        self.depends_on_block_constraints = depends;
    }

    /// Record operation-local aspect-ratio provenance on a transitional
    /// combined dispatcher result.
    #[inline(always)]
    pub(crate) fn with_applied_aspect_ratio(mut self, applied: bool) -> Self {
        self.applied_aspect_ratio |= applied;
        self
    }

    /// Construct the transitional combined result from a dedicated intrinsic
    /// sizing result.
    #[inline(always)]
    pub(crate) fn from_intrinsic_size_result(result: IntrinsicSizeResult) -> Self {
        let mut output = Self::from_outer_size(result.size);
        output.depends_on_block_constraints = result.depends_on_block_constraints;
        output.applied_aspect_ratio = result.applied_aspect_ratio;
        output
    }

    /// Project the measurement portion of this transitional combined result.
    ///
    /// Layout algorithms should exchange [`IntrinsicSizeResult`] through
    /// [`LayoutPartialTree::compute_child_size`](super::LayoutPartialTree::compute_child_size)
    /// instead of consuming this projection directly. It exists while the
    /// low-level cached layout dispatcher still transports both operation
    /// results through `LayoutOutput`.
    #[inline(always)]
    pub fn into_intrinsic_size_result(self) -> IntrinsicSizeResult {
        IntrinsicSizeResult {
            size: self.size,
            depends_on_block_constraints: self.depends_on_block_constraints,
            applied_aspect_ratio: self.applied_aspect_ratio,
        }
    }
}

/// The final result of a layout algorithm for a single node.
#[derive(Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Layout {
    /// The relative ordering of the node
    ///
    /// Nodes with a higher order should be rendered on top of those with a lower order.
    /// This is effectively a topological sort of each tree.
    pub order: u32,
    /// The top-left corner of the node
    pub location: Point<f32>,
    /// The width and height of the node
    pub size: Size<f32>,
    #[cfg(feature = "content_size")]
    /// The width and height of the content inside the node. This may be larger than the size of the node in the case of
    /// overflowing content and is useful for computing a "scroll width/height" for scrollable nodes
    pub content_size: Size<f32>,
    /// The size of the scrollbars in each dimension. If there is no scrollbar then the size will be zero.
    pub scrollbar_size: Size<f32>,
    /// The size of the borders of the node
    pub border: Rect<f32>,
    /// The size of the padding of the node
    pub padding: Rect<f32>,
    /// The size of the margin of the node
    pub margin: Rect<f32>,
}

impl Default for Layout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout {
    /// Creates a new zero-[`Layout`].
    ///
    /// The Zero-layout has size and location set to ZERO.
    /// The `order` value of this layout is set to the minimum value of 0.
    /// This means it should be rendered below all other [`Layout`]s.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            order: 0,
            location: Point::ZERO,
            size: Size::zero(),
            #[cfg(feature = "content_size")]
            content_size: Size::zero(),
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        }
    }

    /// Creates a new zero-[`Layout`] with the supplied `order` value.
    ///
    /// Nodes with a higher order should be rendered on top of those with a lower order.
    /// The Zero-layout has size and location set to ZERO.
    #[must_use]
    pub const fn with_order(order: u32) -> Self {
        Self {
            order,
            size: Size::zero(),
            location: Point::ZERO,
            #[cfg(feature = "content_size")]
            content_size: Size::zero(),
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        }
    }

    /// Get the width of the node's content box
    #[inline]
    pub fn content_box_width(&self) -> f32 {
        self.size.width - self.padding.left - self.padding.right - self.border.left - self.border.right
    }

    /// Get the height of the node's content box
    #[inline]
    pub fn content_box_height(&self) -> f32 {
        self.size.height - self.padding.top - self.padding.bottom - self.border.top - self.border.bottom
    }

    /// Get the size of the node's content box
    #[inline]
    pub fn content_box_size(&self) -> Size<f32> {
        Size { width: self.content_box_width(), height: self.content_box_height() }
    }

    /// Get x offset of the node's content box relative to it's parent's border box
    pub fn content_box_x(&self) -> f32 {
        self.location.x + self.border.left + self.padding.left
    }

    /// Get x offset of the node's content box relative to it's parent's border box
    pub fn content_box_y(&self) -> f32 {
        self.location.y + self.border.top + self.padding.top
    }
}

#[cfg(feature = "content_size")]
impl Layout {
    /// Return the maximum horizontal scroll offset of the node.
    /// This is the content width less the width of the padding box, floored at zero.
    pub fn scroll_width(&self) -> f32 {
        f32_max(
            0.0,
            self.content_size.width + f32_min(self.scrollbar_size.width, self.size.width) - self.size.width
                + self.border.left
                + self.border.right,
        )
    }

    /// Return the maximum vertical scroll offset of the node.
    /// This is the content height less the height of the padding box, floored at zero.
    pub fn scroll_height(&self) -> f32 {
        f32_max(
            0.0,
            self.content_size.height + f32_min(self.scrollbar_size.height, self.size.height) - self.size.height
                + self.border.top
                + self.border.bottom,
        )
    }
}

/// The additional information from layout algorithm
#[cfg(feature = "detailed_layout_info")]
#[derive(Debug, Clone, PartialEq)]
pub enum DetailedLayoutInfo {
    /// Enum variant for [`DetailedGridInfo`](crate::compute::grid::DetailedGridInfo)
    #[cfg(feature = "grid")]
    Grid(Box<crate::compute::grid::DetailedGridInfo>),
    /// For node that hasn't had any detailed information yet
    None,
}
#[test]
fn block_auto_behavior_preserves_aspect_ratio_resolution_order() {
    assert!(AutoSizeBehavior::FitContent.is_content_based(false));
    assert!(AutoSizeBehavior::FitContent.is_content_based(true));
    assert!(!AutoSizeBehavior::StretchExplicit.is_content_based(true));
    assert!(!AutoSizeBehavior::StretchImplicit.is_content_based(false));
    assert!(AutoSizeBehavior::StretchImplicit.is_content_based(true));
}
