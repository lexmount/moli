use super::*;
use crate::ResolvedLayoutStyle;
use taffy::WritingMode;

fn text_world(retained: bool) -> LayoutWorld<u8> {
    let text = "alpha beta gamma delta epsilon zeta eta theta";
    let mut fonts = parley::FontContext::new();
    let mut layouts = parley::LayoutContext::<crate::stylo_to_parley::TextBrush>::new();
    let mut builder = layouts.style_run_builder(&mut fonts, text, 1.0, true);
    let style = builder.push_style(parley::TextStyle::default());
    builder.push_style_run(style, ..);
    let mut inline = empty_inline_context();
    inline.text = text.to_owned();
    inline.measurement_layout = Some(builder.build(text));
    let mut style = ResolvedLayoutStyle::from_stylo(
        style::properties::ComputedValues::initial_values_with_font_override(
            style::properties::style_structs::Font::initial_values(),
        ),
    );
    style.force_layout_display(crate::LayoutDisplay::Block);
    let layout_box = LayoutWorld::new_box(
        Some(0),
        None,
        None,
        "text".to_owned(),
        None,
        None,
        None,
        LayoutBoxKind::PrincipalBlock,
        style,
        None,
    );
    let mut world = LayoutWorld::new(layout_box, false);
    world.prepare_inline_storage();
    world.inline_layouts[0] = Some(inline);
    if retained {
        world.inline_numeric_reuse = Some(Box::new(crate::inline::InlineNumericReuse::new(1)));
    }
    world
}

fn inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        sizing_purpose: SizingPurpose::Layout,
        axis: taffy::RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(60.0),
            height: Some(20.0),
        },
        definite_dimensions: Size {
            width: Some(60.0),
            height: Some(20.0),
        },
        parent_size: Size {
            width: Some(240.0),
            height: None,
        },
        parent_writing_mode: WritingMode::HorizontalTb,
        available_space: Size {
            width: AvailableSpace::Definite(60.0),
            height: AvailableSpace::MaxContent,
        },
        block_auto_behavior: AutoSizeBehavior::FitContent,
        vertical_margins_are_collapsible: taffy::Line::FALSE,
    }
}

fn measure(world: &mut LayoutWorld<u8>, inputs: LayoutInput, baselines: bool) -> LayoutOutput {
    world.measure_baselines = baselines;
    let output = world.compute_inline_formatting_context(world.root(), inputs, None);
    world.measure_baselines = false;
    output
}

#[test]
fn inline_measurement_baselines_and_purposes_do_not_alias_final_layout() {
    let mut world = text_world(true);
    let final_inputs = LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..inputs()
    };
    let accepted_output = measure(&mut world, final_inputs, false);
    let signature = |world: &LayoutWorld<u8>| {
        let inline = world.inline_layouts[0].as_ref().unwrap();
        let accepted = inline.laid_out.as_ref().unwrap();
        (
            accepted.len(),
            accepted.width(),
            accepted.height(),
            inline.line_placements.clone(),
            inline.fragments.lines.clone(),
        )
    };
    let accepted = signature(&world);
    assert!(!accepted.4.is_empty());
    let before = world
        .inline_numeric_reuse
        .as_ref()
        .unwrap()
        .estimated_retained_bytes();
    let plain = measure(&mut world, inputs(), false);
    assert_eq!(
        world
            .inline_numeric_reuse
            .as_ref()
            .unwrap()
            .estimated_retained_bytes(),
        before,
        "known-size shortcuts must not allocate measurement storage"
    );
    let baseline = measure(&mut world, inputs(), true);
    assert!(plain.first_baselines.y.is_none());
    assert!(baseline.first_baselines.y.is_some());
    assert_ne!(plain, baseline);
    world.inline_numeric_reuse.as_mut().unwrap().reset_hits();
    assert_eq!(measure(&mut world, inputs(), true), baseline);
    assert_eq!(measure(&mut world, inputs(), false), plain);
    assert_eq!(
        world
            .inline_numeric_reuse
            .as_ref()
            .unwrap()
            .measurement_hits(),
        1
    );
    assert_eq!(world.inline_numeric_reuse.as_ref().unwrap().hits(), 0);
    assert_eq!(signature(&world), accepted);

    for width in [
        AvailableSpace::MinContent,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(180.0),
    ] {
        for purpose in [SizingPurpose::IntrinsicContribution, SizingPurpose::Layout] {
            let probe = LayoutInput {
                known_dimensions: Size::NONE,
                definite_dimensions: Size::NONE,
                available_space: Size {
                    width,
                    height: AvailableSpace::MaxContent,
                },
                sizing_purpose: purpose,
                ..inputs()
            };
            let expected = measure(&mut text_world(false), probe, true);
            assert_eq!(measure(&mut world, probe, true), expected);
            world.inline_numeric_reuse.as_mut().unwrap().reset_hits();
            assert_eq!(measure(&mut world, probe, true), expected);
            assert_eq!(
                world
                    .inline_numeric_reuse
                    .as_ref()
                    .unwrap()
                    .measurement_hits(),
                1
            );
            assert_eq!(signature(&world), accepted);
        }
    }
    assert_eq!(measure(&mut world, final_inputs, false), accepted_output);
    assert_eq!(signature(&world), accepted);
    assert_eq!(world.inline_numeric_reuse.as_ref().unwrap().hits(), 1);
}

#[test]
fn inline_measurements_revalidate_full_constraints_and_numeric_style() {
    let mut world = text_world(true);
    let base = inputs();
    let probes = [
        base,
        LayoutInput {
            known_dimensions: Size {
                width: Some(180.0),
                height: None,
            },
            ..base
        },
        LayoutInput {
            definite_dimensions: Size::NONE,
            ..base
        },
        LayoutInput {
            parent_size: Size {
                width: Some(320.0),
                height: Some(80.0),
            },
            ..base
        },
        LayoutInput {
            parent_writing_mode: WritingMode::VerticalRl,
            ..base
        },
        LayoutInput {
            available_space: Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::Definite(40.0),
            },
            ..base
        },
        LayoutInput {
            axis: taffy::RequestedAxis::Horizontal,
            ..base
        },
        LayoutInput {
            vertical_margins_are_collapsible: taffy::Line {
                start: true,
                end: true,
            },
            ..base
        },
    ];
    for (index, probe) in probes.into_iter().enumerate() {
        for baselines in [false, true] {
            let mut cold = text_world(false);
            if index % 2 == 1 {
                world.boxes[0].style.taffy.min_size.width = taffy::style_helpers::length(80.0);
                cold.boxes[0].style.taffy.min_size.width = taffy::style_helpers::length(80.0);
            } else {
                world.boxes[0].style.taffy.min_size.width = taffy::Dimension::auto();
            }
            let expected = measure(&mut cold, probe, baselines);
            assert_eq!(measure(&mut world, probe, baselines), expected);
            world.inline_numeric_reuse.as_mut().unwrap().reset_hits();
            assert_eq!(measure(&mut world, probe, baselines), expected);
            if baselines {
                assert_eq!(
                    world
                        .inline_numeric_reuse
                        .as_ref()
                        .unwrap()
                        .measurement_hits(),
                    1
                );
            }
        }
    }
    let owner = world.root();
    world.table_cell_percentage_height = Some((owner, false));
    world.inline_numeric_reuse.as_mut().unwrap().reset_hits();
    measure(&mut world, base, true);
    assert_eq!(
        world
            .inline_numeric_reuse
            .as_ref()
            .unwrap()
            .measurement_hits(),
        0
    );
}
