// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::util::TestEnv;
use crate::{
    test_name,
    util::{ColorBrush, draw_layout, render_to_pixmap, samples::LOREM_IPSUM},
};
use parley::VerticalAlign;
use parley::{Alignment, AlignmentOptions, InlineBox, InlineBoxKind, Layout, YieldData};
use peniko::{Color, kurbo::Rect};
use taffy::{Clear, FloatContext, FloatDirection};

struct TestFloatedBox {
    side: FloatDirection,
    clear: Clear,
    width: f32,
    height: f32,
    text_index: usize,

    // Computed as part of layout
    x: f32,
    y: f32,
}

impl TestFloatedBox {
    fn new(side: FloatDirection, clear: Clear, width: f32, height: f32, text_index: usize) -> Self {
        Self {
            side,
            clear,
            width,
            height,
            text_index,

            // Computed as part of layout
            x: 0.0,
            y: 0.0,
        }
    }
}

#[test]
fn float_simple() {
    let mut env = TestEnv::new(test_name!(), None);
    *env.max_screenshot_size() = Some(500_000);

    let mut floated_boxes: Vec<TestFloatedBox> = vec![
        TestFloatedBox::new(FloatDirection::Left, Clear::None, 100.0, 40.0, 100),
        TestFloatedBox::new(FloatDirection::Right, Clear::None, 80.0, 80.0, 300),
    ];

    let text = LOREM_IPSUM;
    let mut builder = env.ranged_builder(text);

    for (id, fbox) in floated_boxes.iter().enumerate() {
        builder.push_inline_box(InlineBox {
            id: id as u64,
            kind: InlineBoxKind::CustomOutOfFlow,
            index: fbox.text_index,
            width: 0.0,
            height: 0.0,
            baseline: None,
            vertical_align: VerticalAlign::BASELINE,
        });
    }

    let mut layout = builder.build(text);

    layout_floats(&mut layout, &mut floated_boxes, 300.0);
    layout.align(Alignment::Start, AlignmentOptions::default());

    render_and_check_float_layout(&mut env, &layout, &floated_boxes);
}

fn render_and_check_float_layout(
    env: &mut TestEnv,
    layout: &Layout<ColorBrush>,
    floated_boxes: &[TestFloatedBox],
) {
    let mut renderer = draw_layout(&*env.rendering_config(), layout, None, &[]);
    renderer.ctx.set_paint(Color::from_rgb8(255, 105, 180));

    for fbox in floated_boxes {
        let x = fbox.x as f64;
        let y = fbox.y as f64;
        renderer.ctx.fill_rect(&Rect::new(
            x,
            y,
            x + fbox.width as f64,
            y + fbox.height as f64,
        ));
    }

    let current_img = render_to_pixmap(renderer);
    env.check_image(&current_img);
}

fn layout_floats(
    layout: &mut Layout<ColorBrush>,
    floated_boxes: &mut [TestFloatedBox],
    max_advance: f32,
) {
    // Setup Taffy FloatContext
    let mut float_context = FloatContext::new();
    float_context.set_width(max_advance);
    let initial_slot = float_context.find_content_slot(0.0, [0.0, 0.0], Clear::None, None);
    let mut has_active_floats = initial_slot.segment_id.is_some();

    // Configure initial breaker state
    let mut breaker = layout.break_lines();
    let state = breaker.state_mut();
    state.set_layout_max_advance(max_advance);
    state.set_line_max_advance(initial_slot.width);
    state.set_line_x(initial_slot.x);
    state.set_line_y(initial_slot.y as f64);

    // TODO: revert state and retry layout if a line doesn't fit
    //
    // Save initial state. Saved state is used to revert the layout to a previous state if needed
    // (e.g. to revert a line that doesn't fit in the space it was laid out into)
    //
    // let mut saved_state = breaker.state().clone();
    while let Some(yield_data) = breaker.break_next() {
        match yield_data {
            YieldData::LineBreak(_) => {
                let state = breaker.state_mut();

                if has_active_floats {
                    // TODO: revert state and retry layout if a line doesn't fit
                    // saved_state = state.clone();

                    let min_y = state.line_y();
                    let next_slot = float_context.find_content_slot(
                        min_y as f32,
                        [0.0, 0.0],
                        Clear::None,
                        None,
                    );
                    has_active_floats = next_slot.segment_id.is_some();

                    state.set_line_max_advance(next_slot.width);
                    state.set_line_x(next_slot.x);
                    state.set_line_y((next_slot.y) as f64);
                } else {
                    state.set_line_x(0.0);
                    state.set_line_max_advance(max_advance);
                }

                continue;
            }
            YieldData::MaxHeightExceeded(_data) => {
                // TODO
                continue;
            }
            YieldData::InlineBoxBreak(box_break_data) => {
                let state = breaker.state_mut();
                let box_id = box_break_data.inline_box_id as usize;
                let fbox = &mut floated_boxes[box_id];

                let direction = fbox.side;
                let clear = fbox.clear;
                let size = taffy::Size {
                    width: fbox.width,
                    height: fbox.height,
                };

                let min_y = state.line_y() as f32;
                let pos =
                    float_context.place_floated_box(size, min_y, [0.0, 0.0], direction, clear);

                // Record float position
                fbox.x = pos.x;
                fbox.y = pos.y;

                let next_slot =
                    float_context.find_content_slot(min_y, [0.0, 0.0], Clear::None, None);
                has_active_floats = next_slot.segment_id.is_some();

                state.set_line_max_advance(next_slot.width);
                state.set_line_x(next_slot.x);
                state.set_line_y((next_slot.y) as f64);

                state.append_inline_box_to_line(box_break_data.advance, 0.0, 0.0);
            }
        }
    }
    breaker.finish();
}
