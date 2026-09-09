//! Computes size using styles and measure functions

use crate::geometry::{Rect, Size};
use crate::style::{resolve_scrollbar_insets, AvailableSpace, Position};
use crate::tree::RunMode;
use crate::tree::{LayoutInput, LayoutOutput, SizingMode};
use crate::util::debug::debug_log;
use crate::util::sys::f32_max;
use crate::util::MaybeMath;
use crate::util::{MaybeResolve, ResolveOrZero};
use crate::{BoxSizing, CoreStyle, ResolvedAspectRatio, WritingMode};
use core::unreachable;

use super::common::aspect_ratio::{
    apply_preferred_aspect_ratio, resolve_size_constraints, SizeConstraintInput, TransferredSizesMode,
};
use super::common::used_size::{resolve_used_axis, resolve_used_size};

/// Node-level values resolved by the embedding before leaf layout begins.
///
/// These values form one adapter boundary so browser integrations do not need
/// a separate leaf-layout entry point for every combination of inherited and
/// replaced-element state.
#[derive(Copy, Clone, Debug)]
pub struct LeafLayoutContext {
    /// The inherited writing mode that defines the node's logical axes.
    writing_mode: WritingMode,
    /// The node's used preferred ratio, including its sizing-box semantics.
    resolved_aspect_ratio: Option<ResolvedAspectRatio>,
    /// Physical space occupied by resolved scrollbar gutters.
    scrollbar_insets: Rect<f32>,
}

impl LeafLayoutContext {
    /// Creates a context from values already resolved at the layout node.
    pub const fn new(
        writing_mode: WritingMode,
        resolved_aspect_ratio: Option<ResolvedAspectRatio>,
        scrollbar_insets: Rect<f32>,
    ) -> Self {
        Self { writing_mode, resolved_aspect_ratio, scrollbar_insets }
    }

    /// Builds the default context from values exposed directly by the style.
    fn from_style(style: &impl CoreStyle) -> Self {
        let resolved_aspect_ratio =
            style.aspect_ratio().and_then(|ratio| ResolvedAspectRatio::new(ratio, style.box_sizing()));
        Self::new(style.writing_mode(), resolved_aspect_ratio, resolve_scrollbar_insets(style))
    }
}

/// Compute the size of a leaf node (node with no children)
pub fn compute_leaf_layout<MeasureFunction>(
    inputs: LayoutInput,
    style: &impl CoreStyle,
    resolve_calc_value: impl Fn(*const (), f32) -> f32,
    measure_function: MeasureFunction,
) -> LayoutOutput
where
    MeasureFunction: FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> Size<f32>,
{
    compute_leaf_layout_with_context(
        inputs,
        style,
        LeafLayoutContext::from_style(style),
        resolve_calc_value,
        measure_function,
    )
}

/// Compute the size of a leaf node using a node-level resolved aspect ratio.
///
/// Browser integrations should use this entry point when the used ratio may
/// depend on natural replaced-element sizing or when the ratio constrains a
/// different sizing box than authored width and height.
pub fn compute_leaf_layout_with_aspect_ratio<MeasureFunction>(
    inputs: LayoutInput,
    style: &impl CoreStyle,
    resolved_aspect_ratio: Option<ResolvedAspectRatio>,
    resolve_calc_value: impl Fn(*const (), f32) -> f32,
    measure_function: MeasureFunction,
) -> LayoutOutput
where
    MeasureFunction: FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> Size<f32>,
{
    compute_leaf_layout_with_context(
        inputs,
        style,
        LeafLayoutContext::new(style.writing_mode(), resolved_aspect_ratio, resolve_scrollbar_insets(style)),
        resolve_calc_value,
        measure_function,
    )
}

/// Compute the size of a leaf node using scrollbar gutters that have already
/// been resolved to physical edges by the embedding.
///
/// This is the axis-independent counterpart to [`compute_leaf_layout`]. The
/// legacy entry point remains available and derives conventional end-edge
/// gutters from the style's scalar scrollbar width.
pub fn compute_leaf_layout_with_scrollbar_insets<MeasureFunction>(
    inputs: LayoutInput,
    style: &impl CoreStyle,
    scrollbar_insets: Rect<f32>,
    resolve_calc_value: impl Fn(*const (), f32) -> f32,
    measure_function: MeasureFunction,
) -> LayoutOutput
where
    MeasureFunction: FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> Size<f32>,
{
    let resolved_aspect_ratio =
        style.aspect_ratio().and_then(|ratio| ResolvedAspectRatio::new(ratio, style.box_sizing()));
    compute_leaf_layout_with_context(
        inputs,
        style,
        LeafLayoutContext::new(style.writing_mode(), resolved_aspect_ratio, scrollbar_insets),
        resolve_calc_value,
        measure_function,
    )
}

/// Compute a leaf layout using an explicit node writing mode.
///
/// This is the browser-adapter seam for integrations that retain inherited
/// properties outside their numeric [`CoreStyle`] projection. The supplied
/// mode must match [`LayoutPartialTree::get_writing_mode`](crate::LayoutPartialTree::get_writing_mode)
/// for the same node.
pub fn compute_leaf_layout_with_aspect_ratio_and_writing_mode<MeasureFunction>(
    inputs: LayoutInput,
    style: &impl CoreStyle,
    writing_mode: WritingMode,
    resolved_aspect_ratio: Option<ResolvedAspectRatio>,
    resolve_calc_value: impl Fn(*const (), f32) -> f32,
    measure_function: MeasureFunction,
) -> LayoutOutput
where
    MeasureFunction: FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> Size<f32>,
{
    compute_leaf_layout_with_context(
        inputs,
        style,
        LeafLayoutContext::new(writing_mode, resolved_aspect_ratio, resolve_scrollbar_insets(style)),
        resolve_calc_value,
        measure_function,
    )
}

/// Computes a leaf from the complete set of embedding-resolved inputs.
pub fn compute_leaf_layout_with_context<MeasureFunction>(
    inputs: LayoutInput,
    style: &impl CoreStyle,
    context: LeafLayoutContext,
    resolve_calc_value: impl Fn(*const (), f32) -> f32,
    measure_function: MeasureFunction,
) -> LayoutOutput
where
    MeasureFunction: FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> Size<f32>,
{
    let LeafLayoutContext { writing_mode, resolved_aspect_ratio, scrollbar_insets } = context;
    let percentage_basis = inputs.constraint_space(writing_mode).margin_padding_percentage_basis();
    let LayoutInput { known_dimensions, parent_size, available_space, sizing_mode, run_mode, .. } = inputs;

    let margin = style.margin().resolve_or_zero(percentage_basis, &resolve_calc_value);
    let padding = style.padding().resolve_or_zero(percentage_basis, &resolve_calc_value);
    let border = style.border().resolve_or_zero(percentage_basis, &resolve_calc_value);
    let padding_border = padding + border;
    let pb_sum = padding_border.sum_axes();
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox { pb_sum } else { Size::ZERO };

    // Resolve node's preferred/min/max sizes (width/heights) against the available space (percentages resolve to pixel values)
    // For ContentSize mode, we pretend that the node has no size styles as these should be ignored.
    let (node_size, node_min_size, node_max_size, aspect_ratio, applied_aspect_ratio) = match sizing_mode {
        SizingMode::ContentSize => {
            let node_size = known_dimensions;
            let node_min_size = Size::NONE;
            let node_max_size = Size::NONE;
            (node_size, node_min_size, node_max_size, None, false)
        }
        SizingMode::InherentSize => {
            let raw_size = style.size();
            let resolved = resolve_size_constraints(SizeConstraintInput {
                size: raw_size.maybe_resolve(parent_size, &resolve_calc_value).maybe_add(box_sizing_adjustment),
                min_size: style
                    .min_size()
                    .maybe_resolve(parent_size, &resolve_calc_value)
                    .maybe_add(box_sizing_adjustment),
                max_size: style
                    .max_size()
                    .maybe_resolve(parent_size, &resolve_calc_value)
                    .maybe_add(box_sizing_adjustment),
                size_is_auto: raw_size.map(|dimension| dimension.is_auto()),
                writing_mode,
                block_auto_behavior: inputs.block_auto_behavior,
                transferred_sizes_mode: TransferredSizesMode::Normal,
                aspect_ratio: resolved_aspect_ratio,
                padding_border: pb_sum,
            });
            let style_size = resolved.size;
            let style_min_size = resolved.min_size;
            let style_max_size = resolved.max_size;
            let preferred_inline_from_aspect_ratio = resolved.aspect_ratio_applied.width;

            // A parent formatting context may make exactly one border-box axis
            // definite (for example a stretched flex cross size). Resolve the
            // other axis through the preferred ratio at the leaf boundary just
            // like an authored one-axis size.
            let size_before_ratio = known_dimensions.or(style_size);
            let node_size = apply_preferred_aspect_ratio(
                size_before_ratio,
                raw_size.map(|dimension| dimension.is_auto()),
                writing_mode,
                inputs.block_auto_behavior,
                resolved_aspect_ratio,
                pb_sum,
            );
            let applied_aspect_ratio = run_mode == RunMode::ComputeSize
                && known_dimensions.width.is_none()
                && (preferred_inline_from_aspect_ratio
                    || (size_before_ratio.width.is_none() && node_size.width.is_some()));
            (node_size, style_min_size, style_max_size, resolved_aspect_ratio, applied_aspect_ratio)
        }
    };

    let content_box_inset = padding_border + scrollbar_insets;

    let has_styles_preventing_being_collapsed_through = !style.is_block()
        || style.overflow().x.is_scroll_container()
        || style.overflow().y.is_scroll_container()
        || style.position() == Position::Absolute
        || padding.top > 0.0
        || padding.bottom > 0.0
        || border.top > 0.0
        || border.bottom > 0.0
        || matches!(node_size.height, Some(h) if h > 0.0)
        || matches!(node_min_size.height, Some(h) if h > 0.0);

    debug_log!("LEAF");
    debug_log!("node_size", dbg:node_size);
    debug_log!("min_size ", dbg:node_min_size);
    debug_log!("max_size ", dbg:node_max_size);

    // Return early if both width and height are known
    if run_mode == RunMode::ComputeSize && has_styles_preventing_being_collapsed_through {
        let used_size = resolve_used_size(known_dimensions, node_size, node_min_size, node_max_size, pb_sum);
        if let Size { width: Some(width), height: Some(height) } = used_size {
            let size = Size { width, height };
            return LayoutOutput::from_outer_size(size).with_applied_aspect_ratio(applied_aspect_ratio);
        };
    }

    // Compute available space
    let resolve_available_axis = |known_dimension: Option<f32>,
                                  node_size: Option<f32>,
                                  available_space: AvailableSpace,
                                  margin_sum: f32,
                                  min_size: Option<f32>,
                                  max_size: Option<f32>,
                                  minimum_border_box_size: f32,
                                  content_box_inset: f32| {
        let resolved_size = resolve_used_axis(known_dimension, node_size, min_size, max_size, minimum_border_box_size);
        available_space.maybe_sub(margin_sum).maybe_set(resolved_size).map_definite_value(|size| {
            let outer_size = if resolved_size.is_some() {
                size
            } else {
                size.maybe_clamp(min_size, max_size).max(minimum_border_box_size)
            };
            outer_size - content_box_inset
        })
    };
    let available_space = Size {
        width: resolve_available_axis(
            known_dimensions.width,
            node_size.width,
            available_space.width,
            margin.horizontal_axis_sum(),
            node_min_size.width,
            node_max_size.width,
            pb_sum.width,
            content_box_inset.horizontal_axis_sum(),
        ),
        height: resolve_available_axis(
            known_dimensions.height,
            node_size.height,
            available_space.height,
            margin.vertical_axis_sum(),
            node_min_size.height,
            node_max_size.height,
            pb_sum.height,
            content_box_inset.vertical_axis_sum(),
        ),
    };

    // Measure node
    let measured_size = measure_function(
        match run_mode {
            RunMode::ComputeSize => known_dimensions,
            RunMode::PerformLayout => Size::NONE,
            RunMode::PerformHiddenLayout => unreachable!(),
        },
        available_space,
    );
    let measured_outer_size = measured_size + content_box_inset.sum_axes();
    let used_size = resolve_used_size(
        known_dimensions,
        node_size.or(measured_outer_size.map(Some)),
        node_min_size,
        node_max_size,
        pb_sum,
    )
    .unwrap_or(measured_outer_size);
    let ratio_height = Size { width: Some(used_size.width), height: None }
        .maybe_apply_aspect_ratio_with_box_sizing(aspect_ratio, BoxSizing::BorderBox, pb_sum)
        .height
        .unwrap_or(0.0);
    let size = Size {
        width: used_size.width,
        height: if known_dimensions.height.is_some() {
            used_size.height
        } else {
            f32_max(used_size.height, ratio_height)
        },
    };

    let mut output = LayoutOutput::from_sizes(size, measured_size + padding.sum_axes());
    output.margins_can_collapse_through =
        !has_styles_preventing_being_collapsed_through && size.height == 0.0 && measured_size.height == 0.0;
    output.with_applied_aspect_ratio(applied_aspect_ratio)
}
