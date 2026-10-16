use super::*;
use moli_layout::{
    EmbeddedFrameRenderer, EmbeddedFrameSnapshot, LayoutWorkingSet,
    build_layout_pass_with_working_set,
};

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

fn compare_with_cold_layout(
    source: &Source,
    styles: &mut Styles,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
) -> LayoutPassResult<usize> {
    let request =
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test);
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
        let query = LayoutQuery::TextRangeRects {
            source: node,
            utf16_range: 0..usize::MAX,
        };
        assert_eq!(retained.answer_query(&query), cold.answer_query(&query));
    }
    retained
}

#[test]
fn retained_structural_inline_checks_overrides_on_the_same_computed_style() {
    let source = Source(vec![
        Node::element("root", vec![1, 6]),
        Node::element("changing-paragraph", vec![2, 3, 5]),
        Node::text("baseline-reference", "A"),
        Node::html_element("shifted-inline", "span", vec![4]),
        Node::text("shifted-text", "B"),
        Node::text("trailing-text", "C"),
        Node::element("stable-paragraph", vec![7]),
        Node::text("stable-text", "unchanged"),
    ]);
    let (mut paragraph_style, mut services) = fixed_inline_font();
    let mut inline_style = paragraph_style.clone();
    paragraph_style.force_layout_display(LayoutDisplay::Block);
    inline_style.force_layout_display(LayoutDisplay::Inline);
    let mut styles = Styles(HashMap::from([
        (0, resolved(LayoutDisplay::Block, Style::default())),
        (1, paragraph_style.clone()),
        (3, inline_style.clone()),
        (6, paragraph_style),
    ]));
    let mut work = LayoutWorkingSet::default();
    compare_with_cold_layout(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_inline_contexts, 0);
    compare_with_cold_layout(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_inline_contexts, 2);

    // The retained paragraph includes StartEdge/EndEdge objects whose
    // structural box state stores vertical-align. A computed-style pointer
    // alone must not authorize reusing that state after an adapter override.
    for shift in [12.0, -7.0, 20.0, 0.0] {
        let changed_style = inline_style.clone().with_inline_baseline_shift(shift);
        assert!(style::servo_arc::Arc::ptr_eq(
            inline_style.stylo_computed_values().unwrap(),
            changed_style.stylo_computed_values().unwrap(),
        ));
        styles.0.insert(3, changed_style);
        let changed = compare_with_cold_layout(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_inline_contexts, 1);
        let reference = changed.text_range_rects(2, 0..1);
        let shifted = changed.text_range_rects(4, 0..1);
        assert_eq!(reference.len(), 1);
        assert_eq!(shifted.len(), 1);
        assert_close(
            reference[0].bounding_rect().y - shifted[0].bounding_rect().y,
            shift,
        );

        compare_with_cold_layout(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_inline_contexts, 2);
    }
}
