use super::*;
use moli_layout::{
    EmbeddedFrameRenderer, EmbeddedFrameSnapshot, LayoutWorkingSet,
    build_layout_pass_with_working_set,
};
use style::values::{
    computed::{
        CSSPixelLength, LengthPercentage, NonNegativeLengthPercentage, Percentage,
        length::Size as ComputedSize,
    },
    generics::NonNegative,
};
use taffy::{AlignItems, Clear, FlexDirection, Float};

struct NoFrames;

impl EmbeddedFrameRenderer<usize> for NoFrames {
    fn render_embedded_frame(
        &mut self,
        _: usize,
        _: LayoutViewport,
    ) -> Result<Option<EmbeddedFrameSnapshot<usize>>, LayoutError> {
        Ok(None)
    }
}

fn css_length(value: f32) -> NonNegativeLengthPercentage {
    NonNegative(LengthPercentage::new_length(CSSPixelLength::new(value)))
}

fn css_percent(value: f32) -> NonNegativeLengthPercentage {
    NonNegative(LengthPercentage::new_percent(Percentage(value)))
}

fn block_style_from_computed(
    base: &ResolvedLayoutStyle,
    update: impl FnOnce(&mut style::properties::ComputedValues),
) -> ResolvedLayoutStyle {
    let mut computed = (**base.stylo_computed_values().unwrap()).clone();
    update(&mut computed);
    let mut resolved = ResolvedLayoutStyle::from_stylo(style::servo_arc::Arc::new(computed));
    resolved.force_layout_display(LayoutDisplay::Block);
    resolved
}

fn compare_with_cold(
    source: &Source,
    styles: &mut Styles,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
) -> LayoutPassResult<usize> {
    let request =
        LayoutPassRequest::with_paint(LayoutViewport::new(400, 320, 1.0), LayoutFlushReason::Test);
    let retained = build_layout_pass_with_working_set(
        source,
        styles,
        services,
        request,
        &mut NoFrames,
        Some(work),
    )
    .unwrap();
    let cold = build_layout_pass(source, styles, services, request).unwrap();
    assert_eq!(retained.tree.boxes, cold.tree.boxes);
    assert_eq!(retained.tree.fragments, cold.tree.fragments);
    assert_eq!(retained.paint_snapshot(), cold.paint_snapshot());
    for node in 0..source.0.len() {
        for query in [
            LayoutQuery::ElementMetrics { source: node },
            LayoutQuery::ClientRects { source: node },
            LayoutQuery::TextRangeRects {
                source: node,
                utf16_range: 0..usize::MAX,
            },
            LayoutQuery::TextRangeRects {
                source: node,
                utf16_range: 2..2,
            },
            LayoutQuery::IntersectionGeometry {
                target: node,
                root: None,
            },
        ] {
            assert_eq!(retained.answer_query(&query), cold.answer_query(&query));
        }
    }
    for y in (0..320).step_by(23) {
        for x in (0..400).step_by(31) {
            let point = LayoutPoint::new(x as f32, y as f32);
            assert_eq!(retained.hit_test(point, false), cold.hit_test(point, false));
        }
    }
    retained
}

fn assert_evicted_matches_cold(
    source: &Source,
    styles: &mut Styles,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
) {
    work.clear();
    compare_with_cold(source, styles, services, work);
    assert_eq!(work.stats().reused_inline_contexts, 0);
    assert_eq!(work.stats().reused_inline_numeric_layouts, 0);
}

#[test]
fn retained_numeric_output_tracks_percentage_constraints_with_an_unchanged_child_style() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("containing-block", vec![2]),
        Node::element("paragraph", vec![3]),
        Node::text("text", "alpha beta gamma delta epsilon zeta eta theta"),
    ]);
    let (font, mut services) = fixed_inline_font();
    let paragraph = block_style_from_computed(&font, |computed| {
        use style::values::specified::align::{AlignFlags, ContentDistribution};

        let position = computed.mutate_position();
        position.set_width(ComputedSize::LengthPercentage(css_percent(0.75)));
        position.set_height(ComputedSize::LengthPercentage(css_percent(0.6)));
        position.set_align_content(ContentDistribution::new(AlignFlags::END));
        let padding = computed.mutate_padding();
        padding.set_padding_left(css_percent(0.025));
        padding.set_padding_right(css_percent(0.05));
        padding.set_padding_top(css_percent(0.025));
        padding.set_padding_bottom(css_percent(0.05));
    });
    let mut styles = Styles(HashMap::from([
        (0, fixed_size(LayoutDisplay::Block, 400.0, 320.0)),
        (2, paragraph.clone()),
    ]));
    let mut work = LayoutWorkingSet::default();
    let mut previous_size = None;
    for (width, height) in [
        (200.0, 120.0),
        (320.0, 300.0),
        (240.0, 300.0),
        (180.0, 180.0),
    ] {
        styles
            .0
            .insert(1, fixed_size(LayoutDisplay::Block, width, height));
        assert!(style::servo_arc::Arc::ptr_eq(
            paragraph.stylo_computed_values().unwrap(),
            styles.0[&2].stylo_computed_values().unwrap(),
        ));
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        let size = output.element_metrics_for_source(2).unwrap().offset_size;
        if let Some(previous) = previous_size {
            assert_ne!(size, previous);
            assert_eq!(work.stats().reused_inline_contexts, 1);
        }
        previous_size = Some(size);
        assert!(!output.text_range_rects(3, 0..usize::MAX).is_empty());

        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
    }
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_keeps_parent_width_for_percentage_padding() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("containing-block", vec![2]),
        Node::element("paragraph", vec![3]),
        Node::text("text", "alpha beta gamma delta"),
    ]);
    let (font, mut services) = fixed_inline_font();
    let paragraph = block_style_from_computed(&font, |computed| {
        let position = computed.mutate_position();
        position.set_box_sizing(style::computed_values::box_sizing::T::BorderBox);
        position.set_width(ComputedSize::LengthPercentage(css_length(180.0)));
        position.set_height(ComputedSize::LengthPercentage(css_length(120.0)));
        let padding = computed.mutate_padding();
        padding.set_padding_left(css_percent(0.05));
        padding.set_padding_right(css_percent(0.05));
        padding.set_padding_top(css_percent(0.05));
        padding.set_padding_bottom(css_percent(0.05));
    });
    let mut styles = Styles(HashMap::from([
        (0, fixed_size(LayoutDisplay::Block, 400.0, 320.0)),
        (2, paragraph.clone()),
    ]));
    let mut work = LayoutWorkingSet::default();
    let mut previous_rects = None;
    for parent_width in [200.0, 320.0, 240.0] {
        styles
            .0
            .insert(1, fixed_size(LayoutDisplay::Block, parent_width, 220.0));
        assert!(style::servo_arc::Arc::ptr_eq(
            paragraph.stylo_computed_values().unwrap(),
            styles.0[&2].stylo_computed_values().unwrap(),
        ));
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        // Border-box sizing keeps the known outer dimensions unchanged;
        // only the parent's percentage basis changes the content constraints.
        assert_eq!(
            output.element_metrics_for_source(2).unwrap().offset_size,
            moli_layout::LayoutSize::new(180.0, 120.0),
        );
        let rects = output.text_range_rects(3, 0..usize::MAX);
        assert!(!rects.is_empty());
        if let Some(previous) = previous_rects {
            assert_ne!(rects, previous);
            assert_eq!(work.stats().reused_inline_contexts, 1);
        }
        assert_eq!(work.stats().reused_inline_numeric_layouts, 0);
        previous_rects = Some(rects);

        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
    }
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_keeps_parent_width_for_calc_minimum_width() {
    use style::values::computed::length_percentage::{CalcNode, ComputedLeaf, Unpacked};

    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("containing-block", vec![2]),
        Node::element("paragraph", vec![3]),
        Node::text("text", "alpha beta gamma delta"),
    ]);
    let minimum_width = LengthPercentage::new_calc(
        CalcNode::Sum(
            vec![
                CalcNode::Leaf(ComputedLeaf::Length(CSSPixelLength::new(10.0))),
                CalcNode::Leaf(ComputedLeaf::Percentage(Percentage(0.5))),
            ]
            .into(),
        ),
        Default::default(),
    );
    assert!(matches!(minimum_width.unpack(), Unpacked::Calc(_)));
    let (font, mut services) = fixed_inline_font();
    let paragraph = block_style_from_computed(&font, |computed| {
        let position = computed.mutate_position();
        position.set_box_sizing(style::computed_values::box_sizing::T::BorderBox);
        position.set_width(ComputedSize::LengthPercentage(css_length(180.0)));
        position.set_height(ComputedSize::LengthPercentage(css_length(120.0)));
        position.set_min_width(ComputedSize::LengthPercentage(NonNegative(minimum_width)));
    });
    let mut styles = Styles(HashMap::from([
        (0, fixed_size(LayoutDisplay::Block, 400.0, 320.0)),
        (2, paragraph.clone()),
    ]));
    let mut work = LayoutWorkingSet::default();
    for (index, parent_width) in [200.0, 240.0, 360.0].into_iter().enumerate() {
        styles
            .0
            .insert(1, fixed_size(LayoutDisplay::Block, parent_width, 220.0));
        assert!(style::servo_arc::Arc::ptr_eq(
            paragraph.stylo_computed_values().unwrap(),
            styles.0[&2].stylo_computed_values().unwrap(),
        ));
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        // The first width change leaves the known 180px border box unchanged.
        // A calc-bearing style must still reject parent-width normalization;
        // the final change also makes calc(50% + 10px) the active minimum.
        let expected_width = 180.0_f32.max(parent_width * 0.5 + 10.0);
        assert_eq!(
            output.element_metrics_for_source(2).unwrap().offset_size,
            moli_layout::LayoutSize::new(expected_width, 120.0),
        );
        if index > 0 {
            assert_eq!(work.stats().reused_inline_contexts, 1);
        }
        assert_eq!(work.stats().reused_inline_numeric_layouts, 0);

        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
    }
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_observes_parent_display_with_an_unchanged_child_style() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("changing-parent", vec![2]),
        Node::element("paragraph", vec![3]),
        Node::text("text", "aa bb"),
    ]);
    let (mut paragraph, mut services) = fixed_inline_font();
    paragraph.force_layout_display(LayoutDisplay::Block);
    let mut styles = Styles(HashMap::from([
        (0, fixed_size(LayoutDisplay::Block, 400.0, 320.0)),
        (2, paragraph.clone()),
    ]));
    let mut work = LayoutWorkingSet::default();
    let mut widths = Vec::new();
    for display in [
        LayoutDisplay::Block,
        LayoutDisplay::Flex,
        LayoutDisplay::Grid,
        LayoutDisplay::Block,
    ] {
        styles.0.insert(
            1,
            resolved(
                display,
                Style {
                    size: Size {
                        width: length(200.0),
                        height: length(120.0),
                    },
                    flex_direction: FlexDirection::Column,
                    align_items: Some(AlignItems::START),
                    justify_items: Some(AlignItems::START),
                    ..Style::default()
                },
            ),
        );
        assert!(style::servo_arc::Arc::ptr_eq(
            paragraph.stylo_computed_values().unwrap(),
            styles.0[&2].stylo_computed_values().unwrap(),
        ));
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        widths.push(
            output
                .element_metrics_for_source(2)
                .unwrap()
                .offset_size
                .width,
        );
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_inline_contexts, 1);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
    }
    assert!(
        widths[0] > widths[1],
        "flex's unstretched child must shrink"
    );
    assert!(
        widths[0] > widths[2],
        "grid's unstretched child must shrink"
    );
    assert_eq!(widths[0], widths[3]);
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_is_reprojected_after_parent_moves_and_scrolls() {
    let mut source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::element("positioned-parent", vec![3]),
        Node::element("paragraph", vec![4]),
        Node::text("text", "alpha beta gamma delta epsilon zeta eta theta"),
    ]);
    let (font, mut services) = fixed_inline_font();
    let paragraph = block_style_from_computed(&font, |computed| {
        computed
            .mutate_position()
            .set_width(ComputedSize::LengthPercentage(css_length(120.0)));
    });
    let scroller_style = |left, top| {
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(180.0),
                    height: length(64.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                margin: Rect {
                    left: length(left),
                    top: length(top),
                    right: length(0.0),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
        )
    };
    let parent_style = |relative_top| {
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(120.0),
                    height: length(220.0),
                },
                inset: Rect {
                    left: LengthPercentageAuto::auto(),
                    right: LengthPercentageAuto::auto(),
                    top: length(relative_top),
                    bottom: LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
        )
        .with_position(LayoutPosition::Relative)
    };
    let mut styles = Styles(HashMap::from([
        (0, fixed_size(LayoutDisplay::Block, 400.0, 320.0)),
        (1, scroller_style(0.0, 0.0)),
        (2, parent_style(0.0)),
        (3, paragraph),
    ]));
    let mut work = LayoutWorkingSet::default();
    let first = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    let mut previous_rects = first.text_range_rects(4, 0..usize::MAX);
    assert!(!previous_rects.is_empty());
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_inline_numeric_layouts > 0);

    for (left, top, relative_top, scroll) in [
        (17.0, 11.0, 5.0, 20.0),
        (3.0, 23.0, -4.0, 60.0),
        (0.0, 0.0, 0.0, 0.0),
    ] {
        styles.0.insert(1, scroller_style(left, top));
        styles.0.insert(2, parent_style(relative_top));
        source.0[1].scroll = LayoutPoint::new(0.0, scroll);
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
        let rects = output.text_range_rects(4, 0..usize::MAX);
        assert_ne!(rects, previous_rects);
        previous_rects = rects;
        let scroller_box = output.source_output(1).unwrap().principal_box.unwrap();
        assert_eq!(
            output.scroll_extent(scroller_box).unwrap().applied_offset,
            LayoutPoint::new(0.0, scroll),
        );
    }
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_is_not_reused_inside_external_float_exclusions() {
    let source = Source(vec![
        Node::element("root", vec![1, 2]),
        Node::element("preceding-float", vec![]),
        Node::element("paragraph", vec![3]),
        Node::text("text", "alpha beta gamma delta epsilon zeta eta theta"),
    ]);
    let (mut paragraph, mut services) = fixed_inline_font();
    paragraph.force_layout_display(LayoutDisplay::Block);
    let mut styles = Styles(HashMap::from([
        (0, fixed_size(LayoutDisplay::Block, 200.0, 320.0)),
        (2, paragraph),
    ]));
    let mut work = LayoutWorkingSet::default();
    let mut first_rects = None;
    for width in [40.0, 90.0, 130.0] {
        styles.0.insert(
            1,
            fixed_size(LayoutDisplay::Block, width, 100.0).with_float(Float::Left, Clear::None),
        );
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        let rects = output.text_range_rects(3, 0..usize::MAX);
        assert!(!rects.is_empty());
        if let Some(previous) = &first_rects {
            assert_ne!(
                &rects, previous,
                "the preceding float must affect line slots"
            );
        } else {
            first_rects = Some(rects);
        }
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_inline_contexts, 1);
        assert_eq!(work.stats().reused_inline_numeric_layouts, 0);
    }
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_checks_resolved_gutters_on_the_same_computed_style() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("paragraph-scroller", vec![2]),
        Node::text("text", "alpha beta gamma delta epsilon zeta eta theta"),
    ]);
    let (font, mut services) = fixed_inline_font();
    let paragraph = block_style_from_computed(&font, |computed| {
        let position = computed.mutate_position();
        position.set_width(ComputedSize::LengthPercentage(css_length(180.0)));
        position.set_height(ComputedSize::LengthPercentage(css_length(60.0)));
        let box_style = computed.mutate_box();
        box_style.set_overflow_x(style::values::computed::Overflow::Hidden);
        box_style.set_overflow_y(style::values::computed::Overflow::Hidden);
    });
    let mut styles = Styles(HashMap::from([(
        0,
        fixed_size(LayoutDisplay::Block, 400.0, 320.0),
    )]));
    let mut work = LayoutWorkingSet::default();
    let mut client_widths = Vec::new();
    for (width, gutter) in [
        (LayoutScrollbarWidth::Auto, LayoutScrollbarGutter::Stable),
        (
            LayoutScrollbarWidth::Thin,
            LayoutScrollbarGutter::StableBothEdges,
        ),
        (
            LayoutScrollbarWidth::Auto,
            LayoutScrollbarGutter::StableBothEdges,
        ),
    ] {
        let mut changed = paragraph.clone();
        changed.set_scrollbar_style(width, gutter, None);
        assert!(style::servo_arc::Arc::ptr_eq(
            paragraph.stylo_computed_values().unwrap(),
            changed.stylo_computed_values().unwrap(),
        ));
        styles.0.insert(1, changed);
        let output = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        client_widths.push(
            output
                .element_metrics_for_source(1)
                .unwrap()
                .client_size
                .width,
        );
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_inline_contexts, 1);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
    }
    assert!(
        client_widths
            .windows(2)
            .all(|widths| widths[0] != widths[1])
    );
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}

#[test]
fn retained_numeric_output_preserves_accepted_bidi_justified_and_vertical_lines() {
    use style::computed_values::writing_mode::T as WritingMode;
    use style::properties::generated::longhands::direction::computed_value::T as Direction;
    use style::values::specified::TextAlignKeyword;

    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("paragraph", vec![2]),
        Node::text("text", "alpha beta gamma delta epsilon zeta eta theta"),
    ]);
    let (base, mut services) = fixed_inline_font();
    let mut styles = Styles(HashMap::from([(
        0,
        fixed_size(LayoutDisplay::Block, 400.0, 320.0),
    )]));
    let mut work = LayoutWorkingSet::default();
    for (writing_mode, direction, alignment) in [
        (
            WritingMode::HorizontalTb,
            Direction::Ltr,
            TextAlignKeyword::Start,
        ),
        (
            WritingMode::HorizontalTb,
            Direction::Rtl,
            TextAlignKeyword::Start,
        ),
        (
            WritingMode::HorizontalTb,
            Direction::Ltr,
            TextAlignKeyword::Justify,
        ),
        (
            WritingMode::VerticalRl,
            Direction::Rtl,
            TextAlignKeyword::Start,
        ),
        (
            WritingMode::VerticalLr,
            Direction::Ltr,
            TextAlignKeyword::End,
        ),
    ] {
        let mut computed = (**base.stylo_computed_values().unwrap()).clone();
        computed
            .mutate_inherited_box()
            .set_writing_mode(writing_mode);
        computed.mutate_inherited_box().set_direction(direction);
        computed.mutate_inherited_text().set_text_align(alignment);
        computed.writing_mode =
            style::logical_geometry::WritingMode::new(computed.get_inherited_box());
        computed
            .mutate_position()
            .set_width(ComputedSize::LengthPercentage(css_length(160.0)));
        computed
            .mutate_position()
            .set_height(ComputedSize::LengthPercentage(css_length(120.0)));
        let mut paragraph = ResolvedLayoutStyle::from_stylo(style::servo_arc::Arc::new(computed));
        paragraph.force_layout_display(LayoutDisplay::Block);
        styles.0.insert(1, paragraph);
        let first = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(!first.text_range_rects(2, 0..usize::MAX).is_empty());
        assert_eq!(work.stats().reused_inline_numeric_layouts, 0);
        let warm = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_inline_numeric_layouts > 0);
        assert_eq!(first.tree.fragments, warm.tree.fragments);
        assert_eq!(first.paint_snapshot(), warm.paint_snapshot());
    }
    assert_evicted_matches_cold(&source, &mut styles, &mut services, &mut work);
}
