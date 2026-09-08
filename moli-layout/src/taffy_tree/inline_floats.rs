//! Line breaking and float placement share a commit boundary. A float which
//! cannot accompany the current prefix must not change that line's exclusion
//! space; it is positioned once the line's block-end is known.

use std::{fmt::Debug, hash::Hash};

use parley::{BreakerState, YieldData};
use taffy::{
    BlockContext, Clear, FloatDirection, LayoutInput, LayoutOutput, LayoutPartialTree, MaybeMath,
    Point, ResolveOrZero, Size, WritingMode,
};

use crate::{
    LayoutBoxId, LayoutWorld,
    inline::{InlineFormattingContext, InlineObjectRole, measure_inline_line_height},
    style::resolve_stylo_calc_value,
    stylo_to_parley::TextBrush,
};

use super::InlineFloatPlacement;

struct MeasuredInlineFloat {
    child: LayoutBoxId,
    output: LayoutOutput,
    margin: taffy::Rect<f32>,
    direction: FloatDirection,
    clear: Clear,
    order: usize,
    parent_width: Option<f32>,
}

impl MeasuredInlineFloat {
    fn margin_size(&self) -> Size<f32> {
        self.output.size + self.margin.sum_axes()
    }

    fn place(
        self,
        block_context: &mut BlockContext<'_>,
        min_y: f32,
        content_offset: Point<f32>,
    ) -> InlineFloatPlacement {
        let position = block_context.place_floated_box(
            WritingMode::HorizontalTb.to_logical(self.margin_size()),
            min_y,
            self.direction,
            self.clear,
            false,
        );
        InlineFloatPlacement {
            child: self.child,
            location: Point {
                x: content_offset.x + position.line_offset + self.margin.left,
                y: content_offset.y + position.block_offset + self.margin.top,
            },
            output: self.output,
            order: self.order,
            parent_width: self.parent_width,
        }
    }
}

fn update_line_slot(state: &mut BreakerState, block_context: &BlockContext<'_>) {
    let slot = block_context.find_content_slot(state.line_y() as f32, Clear::None, None);
    state.set_line_max_advance(slot.width.max(0.0));
    state.set_line_x(slot.x);
    state.set_line_y(f64::from(slot.y));
}

impl<N: Copy + Debug + Eq + Hash> LayoutWorld<N> {
    pub(super) fn break_inline_lines_with_floats(
        &mut self,
        context: &InlineFormattingContext,
        layout: &mut parley::Layout<TextBrush>,
        width: f32,
        child_inputs: LayoutInput,
        block_context: &mut BlockContext<'_>,
        content_offset: Point<f32>,
        floats: &mut Vec<InlineFloatPlacement>,
        atomic_baseline_ascents: &[Option<f32>],
        structural_edge_contributions: &[bool],
    ) {
        let mut breaker = layout.break_lines();
        breaker.state_mut().set_layout_max_advance(width);
        update_line_slot(breaker.state_mut(), block_context);
        let mut pending = Vec::<MeasuredInlineFloat>::new();
        // A soft break can rewind across a placeholder. Its float is owned by
        // this placement pass, not by the number of times the breaker sees it.
        let mut handled = vec![false; context.objects.len()];

        while let Some(yield_data) = breaker.break_next() {
            match yield_data {
                YieldData::LineBreak(_) => {
                    let line_height = measure_inline_line_height(
                        context,
                        breaker.last_line().expect("completed inline line"),
                        atomic_baseline_ascents,
                        structural_edge_contributions,
                    );
                    breaker.set_last_line_block_advance(line_height);
                    for candidate in pending.drain(..) {
                        floats.push(candidate.place(
                            block_context,
                            breaker.state().line_y() as f32,
                            content_offset,
                        ));
                    }
                    update_line_slot(breaker.state_mut(), block_context);
                }
                YieldData::MaxHeightExceeded(_) => {
                    unreachable!("float line breaking does not constrain line height");
                }
                YieldData::InlineBoxBreak(data) => {
                    // Consume every yielded placeholder, including a revisited
                    // one, without adding an artificial text break opportunity.
                    let state = breaker.state_mut();
                    state.append_inline_box_to_line(data.advance, 0.0);
                    let Some(object) = context.object(data.inline_box_id) else {
                        continue;
                    };
                    if object.role != InlineObjectRole::Float {
                        continue;
                    }
                    let order = usize::try_from(data.inline_box_id).expect("inline object index");
                    if std::mem::replace(&mut handled[order], true) {
                        continue;
                    }
                    let candidate = self.measure_inline_float(object.box_id, order, child_inputs);
                    let line_y = state.line_y() as f32;
                    let must_defer = !pending.is_empty()
                        || (!data.is_line_start
                            && (data.fit_advance + candidate.margin_size().width
                                > state.line_max_advance()
                                || block_context
                                    .cleared_threshold(candidate.clear)
                                    .is_some_and(|clear_y| clear_y > line_y)));
                    if must_defer {
                        pending.push(candidate);
                    } else {
                        floats.push(candidate.place(block_context, line_y, content_offset));
                        update_line_slot(state, block_context);
                    }
                }
            }
        }
        debug_assert!(
            pending.is_empty(),
            "the final line commits deferred floats too"
        );
        breaker.finish();
    }

    fn measure_inline_float(
        &mut self,
        child: LayoutBoxId,
        order: usize,
        child_inputs: LayoutInput,
    ) -> MeasuredInlineFloat {
        let style = &self.boxes[child.index()].style.taffy;
        let direction = match style.float {
            taffy::Float::Left => FloatDirection::Left,
            taffy::Float::Right => FloatDirection::Right,
            taffy::Float::None => unreachable!("inline float objects have a float direction"),
        };
        let clear = style.clear;
        let margin = style
            .margin
            .resolve_or_zero(child_inputs.parent_size.width, resolve_stylo_calc_value);
        // Non-replaced formatting contexts receive the space inside margins;
        // the replaced leaf adapter consumes the full slot and owns subtraction.
        let layout_inputs = if self.boxes[child.index()].is_replaced() {
            child_inputs
        } else {
            LayoutInput {
                available_space: child_inputs
                    .available_space
                    .map_width(|width| width.maybe_sub(margin.left + margin.right)),
                ..child_inputs
            }
        };
        MeasuredInlineFloat {
            child,
            output: self.compute_child_layout(child.to_taffy(), layout_inputs),
            margin,
            direction,
            clear,
            order,
            parent_width: child_inputs.parent_size.width,
        }
    }
}
