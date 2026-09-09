//! HTML fieldset formatting: the rendered legend is part of the block-start
//! border, not a line or flex/grid item in the fieldset's content.
//!
//! The anonymous content box uses the ordinary formatting algorithms. This
//! owner combines its size and baselines with the independently measured
//! legend in logical coordinates, then publishes physical child fragments.

use std::{fmt::Debug, hash::Hash};

use taffy::{
    AutoSizeBehavior, AvailableSpace, LayoutInput, LayoutOutput, LayoutPartialTree,
    LeafLayoutContext, Line, LogicalOffset, LogicalSize, MaybeMath, MaybeResolve, Point, Rect,
    ResolveOrZero, RunMode, Size, SizingMode, WritingDirection, compute_leaf_layout_with_context,
};

use crate::{LayoutBoxId, LayoutBoxKind, LayoutWorld, style::resolve_stylo_calc_value};

#[derive(Clone, Copy)]
pub(crate) struct FieldsetChildren {
    pub(crate) legend: Option<LayoutBoxId>,
    pub(crate) content: LayoutBoxId,
}

impl<N: Copy + Debug + Eq + Hash> LayoutWorld<N> {
    pub(crate) fn fieldset_children(&self, id: LayoutBoxId) -> Option<FieldsetChildren> {
        let node = &self.boxes[id.index()];
        if node.kind != LayoutBoxKind::Fieldset {
            return None;
        }
        let (&content, before_content) = node.children.split_last()?;
        debug_assert_eq!(
            self.boxes[content.index()].kind,
            LayoutBoxKind::FieldsetContent
        );
        debug_assert!(before_content.len() <= 1);
        Some(FieldsetChildren {
            content,
            legend: before_content.first().copied(),
        })
    }

    /// The content box inherits its fieldset's containing-block capability.
    /// Only descendants actually inside that box use it; descendants of the
    /// rendered legend still use the outer fieldset. Source ancestry remains
    /// unchanged for CSSOM and selector ownership.
    pub(crate) fn fieldset_descendant_containing_box(
        &self,
        containing_box: LayoutBoxId,
        descendant: LayoutBoxId,
    ) -> LayoutBoxId {
        let Some(children) = self.fieldset_children(containing_box) else {
            return containing_box;
        };
        let mut ancestor = self.boxes[descendant.index()].parent;
        while let Some(id) = ancestor {
            if id == children.content {
                return id;
            }
            if id == containing_box {
                break;
            }
            ancestor = self.boxes[id.index()].parent;
        }
        containing_box
    }
}

struct FieldsetContext {
    children: FieldsetChildren,
    inputs: LayoutInput,
    flow: WritingDirection,
    padding: Rect<f32>,
    border: Rect<f32>,
    definite_block_size: bool,
}

struct LegendLayout {
    output: LayoutOutput,
    margins: Rect<f32>,
    block_offset: f32,
}

struct FieldsetMeasurement {
    legend: Option<LegendLayout>,
    content: LayoutOutput,
    content_block_offset: f32,
    size: LogicalSize<f32>,
}

pub(crate) fn compute_fieldset_layout<N>(
    world: &mut LayoutWorld<N>,
    id: LayoutBoxId,
    inputs: LayoutInput,
) -> LayoutOutput
where
    N: Copy + Debug + Eq + Hash,
{
    let children = world
        .fieldset_children(id)
        .expect("a fieldset has one content box");
    let style = world.boxes[id.index()].style.taffy.clone();
    let mode = world.boxes[id.index()].style.writing_mode();
    let flow = WritingDirection::new(mode, style.direction);
    let percentage_basis = inputs
        .constraint_space(mode)
        .margin_padding_percentage_basis();
    let padding = style
        .padding
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let resolved_padding = padding.map(taffy::LengthPercentage::length);
    // Percentage padding belongs to the fieldset's containing block, not to
    // the anonymous child. Resolve it at the owner boundary and invalidate
    // that child's numeric cache whenever the used input actually changes.
    if world.boxes[children.content.index()].style.taffy.padding != resolved_padding {
        world.boxes[children.content.index()].style.taffy.padding = resolved_padding;
        world.boxes[children.content.index()].cache.clear();
    }
    let definite_block_size = mode
        .to_logical(inputs.known_dimensions)
        .block_size
        .is_some()
        || (inputs.sizing_mode == SizingMode::InherentSize
            && mode
                .to_logical(
                    style
                        .size
                        .maybe_resolve(inputs.parent_size, resolve_stylo_calc_value),
                )
                .block_size
                .is_some());
    let context = FieldsetContext {
        children,
        inputs,
        flow,
        padding,
        border,
        definite_block_size,
    };
    let insets = mode.to_logical((padding + border).sum_axes());
    let mut measurement = None;
    let mut output = compute_leaf_layout_with_context(
        // A known outer size cannot skip legend measurement: even a zero
        // block-size must make room for the legend and the content padding.
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            ..inputs
        },
        &style,
        LeafLayoutContext::new(
            mode,
            world.boxes[id.index()].resolved_aspect_ratio(),
            Rect::ZERO,
        ),
        resolve_stylo_calc_value,
        |_, available_space| {
            let result =
                context.measure(world, mode.to_logical(available_space), definite_block_size);
            let size = mode.to_physical(LogicalSize {
                inline_size: (result.size.inline_size - insets.inline_size).max(0.0),
                block_size: (result.size.block_size - insets.block_size).max(0.0),
            });
            measurement = Some(result);
            size
        },
    );
    let mut measurement = measurement.expect("fieldset measurement cannot be skipped");
    let padding = flow.to_logical_box_strut(padding);
    let border = flow.to_logical_box_strut(border);
    let minimum_block_size = measurement.content_block_offset
        + padding.block_start
        + padding.block_end
        + border.block_end;
    let mut size = mode.to_logical(output.size);
    size.block_size = size.block_size.max(minimum_block_size);
    if size != measurement.size {
        // Preferred/min/max sizing can constrain an intrinsically measured
        // box. Reflow the real content at that accepted size before exporting
        // baselines or child locations; no fragment is stretched after layout.
        let constrained_block =
            definite_block_size || size.block_size != measurement.size.block_size;
        measurement = context.measure(
            world,
            LogicalSize {
                inline_size: AvailableSpace::Definite(
                    (size.inline_size - insets.inline_size).max(0.0),
                ),
                block_size: AvailableSpace::Definite(
                    (size.block_size - insets.block_size).max(0.0),
                ),
            },
            constrained_block,
        );
    }
    output.size = mode.to_physical(size);
    output.margins_can_collapse_through = false;
    let content_location = flow.converter(output.size).to_physical_point(
        LogicalOffset {
            inline_offset: border.inline_start,
            block_offset: measurement.content_block_offset,
        },
        measurement.content.size,
    );
    output.first_baselines =
        offset_baselines(measurement.content.first_baselines, content_location);
    output.last_baselines = offset_baselines(measurement.content.last_baselines, content_location);
    output.content_size = Size {
        width: content_location.x + measurement.content.content_size.width,
        height: content_location.y + measurement.content.content_size.height,
    };
    if inputs.run_mode == RunMode::PerformLayout {
        if let (Some(legend), Some(layout)) = (children.legend, measurement.legend) {
            let style = &world.boxes[legend.index()].style;
            let margins = flow.to_logical_box_strut(layout.margins);
            let free = (size.inline_size
                - insets.inline_size
                - mode.to_logical(layout.output.size).inline_size
                - margins.inline_start
                - margins.inline_end)
                .max(0.0);
            let auto_margins =
                flow.to_logical_box_strut(style.taffy.margin.map(|margin| margin.is_auto()));
            let alignment = if auto_margins.inline_start {
                if auto_margins.inline_end {
                    free / 2.0
                } else {
                    free
                }
            } else if auto_margins.inline_end {
                0.0
            } else {
                legend_alignment(
                    style.taffy.justify_self,
                    free,
                    style.text_align(),
                    flow.direction,
                )
            };
            let location = flow.converter(output.size).to_physical_point(
                LogicalOffset {
                    inline_offset: border.inline_start
                        + padding.inline_start
                        + margins.inline_start
                        + alignment,
                    block_offset: layout.block_offset,
                },
                layout.output.size,
            );
            let relative =
                crate::inline::relative_atomic_inset_offset(&style.taffy, output.size, flow);
            world.set_measured_child_layout(
                legend,
                Point {
                    x: location.x + relative.x,
                    y: location.y + relative.y,
                },
                relative,
                layout.output,
                0,
                Some((size.inline_size - insets.inline_size).max(0.0)),
            );
        }
        world.set_measured_child_layout(
            children.content,
            content_location,
            Point::ZERO,
            measurement.content,
            1,
            Some(size.inline_size - border.inline_start - border.inline_end),
        );
    }
    output
}

impl FieldsetContext {
    fn measure<N: Copy + Debug + Eq + Hash>(
        &self,
        world: &mut LayoutWorld<N>,
        available: LogicalSize<AvailableSpace>,
        constrain_block: bool,
    ) -> FieldsetMeasurement {
        let mode = self.flow.mode;
        let padding = self.flow.to_logical_box_strut(self.padding);
        let border = self.flow.to_logical_box_strut(self.border);
        let padding_inline = padding.inline_start + padding.inline_end;
        let padding_block = padding.block_start + padding.block_end;
        let parent_size = mode.to_physical(LogicalSize {
            inline_size: available.inline_size.into_option(),
            block_size: self
                .definite_block_size
                .then(|| available.block_size.into_option())
                .flatten(),
        });
        let child_inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            // ContentSize suppresses the fieldset's own preferred size, not
            // the sizing properties of its legend or ordinary descendants.
            sizing_mode: SizingMode::InherentSize,
            parent_size,
            parent_writing_mode: mode,
            available_space: mode.to_physical(available),
            block_auto_behavior: AutoSizeBehavior::FitContent,
            block_margins_are_collapsible: Line::FALSE,
            ..self.inputs
        };
        let legend = self.children.legend.map(|legend| {
            let child_mode = world.boxes[legend.index()].style.writing_mode();
            let child_style = &world.boxes[legend.index()].style.taffy;
            let margins = child_style.margin.resolve_or_zero(
                available.inline_size.into_option(),
                resolve_stylo_calc_value,
            );
            let mut inputs = child_inputs;
            if child_mode
                .to_logical(child_style.size)
                .inline_size
                .is_auto()
                && let AvailableSpace::Definite(available_inline) =
                    child_mode.to_logical(inputs.available_space).inline_size
            {
                let inline_size = world.measure_fit_content_inline_size(
                    legend,
                    inputs,
                    child_mode,
                    available_inline - child_mode.to_logical(margins.sum_axes()).inline_size,
                );
                inputs.known_dimensions = child_mode.to_physical(LogicalSize {
                    inline_size: Some(inline_size),
                    block_size: None,
                });
                inputs.definite_dimensions = inputs.known_dimensions;
            }
            let output = world.compute_child_layout(legend.to_taffy(), inputs);
            let block_offset =
                ((border.block_start - mode.to_logical(output.size).block_size) / 2.0).max(0.0);
            LegendLayout {
                output,
                margins,
                block_offset,
            }
        });
        let content_block_offset = legend.as_ref().map_or(border.block_start, |legend| {
            let margins = self.flow.to_logical_box_strut(legend.margins);
            border.block_start.max(
                legend.block_offset
                    + mode.to_logical(legend.output.size).block_size
                    + margins.block_end,
            )
        });
        let legend_excess = content_block_offset - border.block_start;
        let content_known = mode.to_physical(LogicalSize {
            inline_size: available
                .inline_size
                .into_option()
                .map(|size| size + padding_inline),
            block_size: constrain_block
                .then(|| available.block_size.into_option())
                .flatten()
                .map(|size| (size - legend_excess).max(0.0) + padding_block),
        });
        let content_available = mode.to_physical(LogicalSize {
            inline_size: available.inline_size.maybe_add(padding_inline),
            block_size: mode
                .to_logical(content_known)
                .block_size
                .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
        });
        let content = world.compute_child_layout(
            self.children.content.to_taffy(),
            LayoutInput {
                known_dimensions: content_known,
                definite_dimensions: content_known,
                available_space: content_available,
                parent_size: content_known,
                ..child_inputs
            },
        );
        let content_size = mode.to_logical(content.size);
        let legend_inline = legend.as_ref().map_or(0.0, |legend| {
            mode.to_logical(legend.output.size + legend.margins.sum_axes())
                .inline_size
        });
        FieldsetMeasurement {
            legend,
            content,
            content_block_offset,
            size: LogicalSize {
                inline_size: content_size.inline_size.max(legend_inline + padding_inline)
                    + border.inline_start
                    + border.inline_end,
                block_size: content_block_offset + content_size.block_size + border.block_end,
            },
        }
    }
}

fn offset_baselines(baselines: Point<Option<f32>>, offset: Point<f32>) -> Point<Option<f32>> {
    Point {
        x: baselines.x.map(|baseline| baseline + offset.x),
        y: baselines.y.map(|baseline| baseline + offset.y),
    }
}

fn legend_alignment(
    justify: Option<taffy::AlignSelf>,
    free: f32,
    text_align: parley::Alignment,
    direction: taffy::Direction,
) -> f32 {
    use taffy::AlignItemsKeyword;
    match justify.map(|alignment| alignment.keyword) {
        Some(AlignItemsKeyword::Center) => free / 2.0,
        Some(AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd) => free,
        Some(AlignItemsKeyword::Start | AlignItemsKeyword::FlexStart) => 0.0,
        _ => match text_align {
            parley::Alignment::Center => free / 2.0,
            parley::Alignment::Right if direction == taffy::Direction::Ltr => free,
            parley::Alignment::Left if direction == taffy::Direction::Rtl => free,
            _ => 0.0,
        },
    }
}
