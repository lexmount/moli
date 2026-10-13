use super::*;
use moli_layout::{
    EmbeddedFrameRenderer, EmbeddedFrameSnapshot, LayoutPseudo, LayoutWorkingSet,
    build_layout_pass_with_working_set,
};
use style::values::{
    computed::{
        CSSPixelLength, LengthPercentage, NonNegativeLengthPercentage, length::Size as ComputedSize,
    },
    generics::NonNegative,
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

fn block_width(style: &ResolvedLayoutStyle, width: f32) -> ResolvedLayoutStyle {
    let mut computed = (**style.stylo_computed_values().unwrap()).clone();
    let width: NonNegativeLengthPercentage =
        NonNegative(LengthPercentage::new_length(CSSPixelLength::new(width)));
    computed
        .mutate_position()
        .set_width(ComputedSize::LengthPercentage(width));
    let mut style = ResolvedLayoutStyle::from_stylo(style::servo_arc::Arc::new(computed));
    style.force_layout_display(LayoutDisplay::Block);
    style
}

fn fixture() -> (Source, Styles, DocumentLayoutServices) {
    let source = Source(vec![
        Node::element("root", vec![1, 3, 5, 7]),
        Node::element("changing", vec![2]),
        Node::text("changing-text", "alpha beta gamma delta epsilon"),
        Node::element("stable", vec![4]),
        Node::text("stable-text", "one two three four five six"),
        Node::element("fixed-scroller", vec![6]),
        Node::element("empty-block", vec![]),
        Node::element("hidden-frontier", vec![8]),
        Node::text("hidden-text", "initial hidden text"),
    ]);
    let (font, services) = fixed_inline_font();
    let mut hidden = font.clone();
    hidden.force_layout_display(LayoutDisplay::None);
    let styles = Styles(HashMap::from([
        (0, block_width(&font, 320.0)),
        (1, block_width(&font, 100.0)),
        (3, block_width(&font, 160.0)),
        (
            5,
            resolved(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: length(60.0),
                        height: length(40.0),
                    },
                    overflow: Point {
                        // The synthetic adapter maps Taffy's Scroll to CSS auto.
                        x: Overflow::Scroll,
                        y: Overflow::Scroll,
                    },
                    ..Style::default()
                },
            )
            .with_position(LayoutPosition::Fixed),
        ),
        (6, fixed_size(LayoutDisplay::Block, 40.0, 400.0)),
        (7, hidden),
    ]));
    (source, styles, services)
}

fn assert_same_output(
    retained: &LayoutPassResult<usize>,
    cold: &LayoutPassResult<usize>,
    node_count: usize,
) {
    assert_eq!(retained.tree.boxes, cold.tree.boxes);
    assert_eq!(retained.tree.fragments, cold.tree.fragments);
    assert_eq!(retained.paint_snapshot(), cold.paint_snapshot());
    assert_eq!(
        retained.answer_query(&LayoutQuery::DocumentMetrics),
        cold.answer_query(&LayoutQuery::DocumentMetrics),
    );
    for source in 0..node_count {
        for query in [
            LayoutQuery::BoxModel { source },
            LayoutQuery::ClientRects { source },
            LayoutQuery::ContentQuads { source },
            LayoutQuery::ElementMetrics { source },
            LayoutQuery::UsedGridTracks { source },
            LayoutQuery::ScrollIntoViewGeometry { source },
            LayoutQuery::TextRangeRects {
                source,
                utf16_range: 0..usize::MAX,
            },
            LayoutQuery::TextRangeRects {
                source,
                utf16_range: 2..2,
            },
            LayoutQuery::IntersectionGeometry {
                target: source,
                root: None,
            },
            LayoutQuery::EventOffset {
                source,
                point: LayoutPoint::new(32.0, 48.0),
            },
        ] {
            assert_eq!(
                retained.answer_query(&query),
                cold.answer_query(&query),
                "{query:?}",
            );
        }
    }
    for y in (0..240).step_by(19) {
        for x in (0..320).step_by(23) {
            let point = LayoutPoint::new(x as f32, y as f32);
            for query in [
                LayoutQuery::HitTest {
                    point,
                    ignore_pointer_events_none: false,
                },
                LayoutQuery::HitTestAll {
                    point,
                    ignore_pointer_events_none: true,
                },
                LayoutQuery::CaretPosition { point },
            ] {
                assert_eq!(retained.answer_query(&query), cold.answer_query(&query));
            }
        }
    }
}

fn compare_with_cold(
    source: &Source,
    styles: &mut impl LayoutStyleResolver<usize>,
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
    assert_same_output(&retained, &cold, source.0.len());
    retained
}

fn assert_eviction_preserves_output(
    source: &Source,
    styles: &mut impl LayoutStyleResolver<usize>,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
    expected: &LayoutPassResult<usize>,
) {
    work.clear();
    let evicted = compare_with_cold(source, styles, services, work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    assert_same_output(&evicted, expected, source.0.len());
}

fn prime_graph_reuse(
    source: &Source,
    styles: &mut Styles,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
) -> LayoutPassResult<usize> {
    compare_with_cold(source, styles, services, work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    let old = styles.0[&1].clone();
    let changed = block_width(&old, 180.0);
    assert!(!style::servo_arc::Arc::ptr_eq(
        old.stylo_computed_values().unwrap(),
        changed.stylo_computed_values().unwrap(),
    ));
    styles.0.insert(1, changed);
    let warm = compare_with_cold(source, styles, services, work);
    assert!(
        work.stats().reused_construction_boxes > 0,
        "{:?}",
        work.stats()
    );
    assert_eq!(
        warm.element_metrics_for_source(1)
            .unwrap()
            .offset_size
            .width,
        180.0,
    );
    warm
}

#[test]
fn width_changes_skip_construction_with_hidden_and_out_of_flow_sources() {
    let (source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    let warm = prime_graph_reuse(&source, &mut styles, &mut services, &mut work);
    assert_eq!(
        work.stats().reused_construction_boxes,
        work.stats().retained_boxes
    );
    assert!(!warm.text_range_rects(2, 0..usize::MAX).is_empty());
    assert!(warm.element_metrics_for_source(5).is_some());
    assert!(warm.element_metrics_for_source(6).is_some());
    assert!(warm.element_metrics_for_source(7).is_none());
    assert_eviction_preserves_output(&source, &mut styles, &mut services, &mut work, &warm);
}

#[test]
fn hidden_and_missing_style_frontiers_are_rechecked_before_graph_reuse() {
    for missing_style in [false, true] {
        let (mut source, mut styles, mut services) = fixture();
        if missing_style {
            styles.0.remove(&7);
        }
        let mut work = LayoutWorkingSet::default();
        let hidden = prime_graph_reuse(&source, &mut styles, &mut services, &mut work);
        assert!(hidden.text_range_rects(8, 0..usize::MAX).is_empty());

        // A suppressed subtree need not be walked while hidden. Once its
        // frontier becomes visible, its current descendants must be built.
        source.0[8].text = Some("text changed while the subtree was hidden");
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        let visible_style = block_width(&styles.0[&1], 120.0);
        styles.0.insert(7, visible_style);
        let visible = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_construction_boxes, 0);
        assert!(visible.element_metrics_for_source(7).is_some());
        assert!(!visible.text_range_rects(8, 0..usize::MAX).is_empty());
        assert_eviction_preserves_output(&source, &mut styles, &mut services, &mut work, &visible);
    }
}

#[test]
fn absent_empty_and_collapsible_text_cannot_hide_new_inline_runs() {
    for (initial_text, preserve_whitespace) in [
        (None, false),
        (Some(""), false),
        (Some(" \t\n"), false),
        (Some(" \t\n"), true),
    ] {
        let (mut source, mut styles, mut services) = fixture();
        source.0[0].children.push(9);
        let mut text = Node::text("suppressed-text", "");
        text.text = initial_text;
        source.0.push(text);
        let mut work = LayoutWorkingSet::default();
        let initial = prime_graph_reuse(&source, &mut styles, &mut services, &mut work);
        assert!(initial.text_range_rects(9, 0..usize::MAX).is_empty());

        if preserve_whitespace {
            let mut computed = (**styles.0[&0].stylo_computed_values().unwrap()).clone();
            computed.mutate_inherited_text().set_white_space_collapse(
                style::computed_values::white_space_collapse::T::Preserve,
            );
            let mut changed = ResolvedLayoutStyle::from_stylo(style::servo_arc::Arc::new(computed));
            changed.force_layout_display(LayoutDisplay::Block);
            styles.0.insert(0, changed);
        } else {
            // NBSP is meaningful inline text even though CSS whitespace was
            // discarded at the same source position in the previous pass.
            source.0[9].text = Some("\u{00a0} visible");
        }
        let changed = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_construction_boxes, 0);
        assert!(changed.tree.boxes.len() > initial.tree.boxes.len());
        assert_eviction_preserves_output(&source, &mut styles, &mut services, &mut work, &changed);
    }
}

#[test]
fn a_contents_wrapper_rebuilds_principal_and_descendant_source_mapping() {
    let (source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    let initial = prime_graph_reuse(&source, &mut styles, &mut services, &mut work);
    assert!(initial.source_output(1).unwrap().principal_box.is_some());

    styles
        .0
        .get_mut(&1)
        .unwrap()
        .force_layout_display(LayoutDisplay::Contents);
    let contents = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    assert!(
        contents
            .source_output(1)
            .is_none_or(|node| node.principal_box.is_none())
    );
    assert!(!contents.text_range_rects(2, 0..usize::MAX).is_empty());
    assert_eviction_preserves_output(&source, &mut styles, &mut services, &mut work, &contents);
}

struct PseudoStyles {
    primary: Styles,
    pseudo: HashMap<(usize, LayoutPseudo), ResolvedLayoutStyle>,
}

impl LayoutStyleResolver<usize> for PseudoStyles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok(self.primary.0.get(&node).cloned().map(|primary| {
            ResolvedLayoutElementStyles::new(
                primary,
                self.pseudo.get(&(node, LayoutPseudo::Before)).cloned(),
                self.pseudo.get(&(node, LayoutPseudo::After)).cloned(),
            )
        }))
    }
}

#[test]
fn new_pseudos_invalidate_construction_even_with_the_same_principal_style() {
    for (pseudo, text) in [
        (LayoutPseudo::Before, ""),
        (LayoutPseudo::Before, "prefix"),
        (LayoutPseudo::After, "suffix"),
    ] {
        let (source, mut primary, mut services) = fixture();
        let mut work = LayoutWorkingSet::default();
        let initial = prime_graph_reuse(&source, &mut primary, &mut services, &mut work);
        let principal = primary.0[&1].clone();
        let mut generated = principal.clone().with_generated_text(text);
        generated.force_layout_display(LayoutDisplay::Inline);
        let mut styles = PseudoStyles {
            primary,
            pseudo: HashMap::from([((1, pseudo), generated)]),
        };
        assert!(style::servo_arc::Arc::ptr_eq(
            principal.stylo_computed_values().unwrap(),
            styles.primary.0[&1].stylo_computed_values().unwrap(),
        ));
        let changed = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_construction_boxes, 0);
        assert!(changed.tree.boxes.len() > initial.tree.boxes.len());
        assert_eviction_preserves_output(&source, &mut styles, &mut services, &mut work, &changed);
    }
}

#[test]
fn flattened_children_order_and_parent_changes_retire_the_old_graph() {
    let (mut source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    let initial = prime_graph_reuse(&source, &mut styles, &mut services, &mut work);

    // LayoutSource exposes the flattened tree. Reordering the same identities
    // models slot reassignment without inventing a second DOM implementation.
    source.0[0].children.swap(0, 1);
    let reordered = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    assert_ne!(
        reordered.text_range_rects(2, 0..usize::MAX),
        initial.text_range_rects(2, 0..usize::MAX),
    );
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_construction_boxes > 0);

    // Preserve node IDs, text and element styles but change each text node's
    // flattened parent. Its inherited width and IFC source owner must follow.
    source.0[1].children = vec![4];
    source.0[3].children = vec![2];
    let reparented = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    assert_ne!(
        reparented.text_range_rects(2, 0..usize::MAX),
        reordered.text_range_rects(2, 0..usize::MAX),
    );
    assert_eviction_preserves_output(&source, &mut styles, &mut services, &mut work, &reparented);
}
