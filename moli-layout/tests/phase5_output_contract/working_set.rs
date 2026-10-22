use super::*;
use moli_layout::{
    EmbeddedFrameRenderer, EmbeddedFrameSnapshot, LayoutWorkingSet, LayoutWorkingSetLimits,
    WebFontFace, WebFontRegistration, build_layout_pass_with_working_set,
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

fn fixture() -> (Source, Styles, DocumentLayoutServices) {
    let source = Source(vec![
        Node::element("root", vec![1, 3]),
        Node::element("first", vec![2]),
        Node::text("first-text", "alpha beta gamma delta epsilon"),
        Node::element("second", vec![4]),
        Node::text("second-text", "one two three four five six"),
    ]);
    let (mut font_style, services) = fixed_inline_font();
    font_style.force_layout_display(LayoutDisplay::Block);
    let styles = Styles(HashMap::from([
        (0, resolved(LayoutDisplay::Block, Style::default())),
        (1, font_style.clone()),
        (3, font_style),
    ]));
    (source, styles, services)
}

fn compare_pass(
    source: &Source,
    styles: &mut Styles,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
    width: u32,
) -> LayoutPassResult<usize> {
    compare_source_pass(source, source.0.len(), styles, services, work, width)
}

fn compare_source_pass(
    source: &impl LayoutSource<NodeId = usize>,
    node_count: usize,
    styles: &mut Styles,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
    width: u32,
) -> LayoutPassResult<usize> {
    let request = LayoutPassRequest::with_paint(
        LayoutViewport::new(width, 240, 1.0),
        LayoutFlushReason::Test,
    );
    let retained = build_layout_pass_with_working_set(
        source,
        styles,
        services,
        request,
        &mut NoFrames,
        Some(work),
    )
    .unwrap();
    let fresh = build_layout_pass(source, styles, services, request).unwrap();
    assert_eq!(retained.tree.boxes, fresh.tree.boxes);
    assert_eq!(retained.tree.fragments, fresh.tree.fragments);
    assert_eq!(retained.paint_snapshot(), fresh.paint_snapshot());
    for node in 0..node_count {
        for query in [
            LayoutQuery::ElementMetrics { source: node },
            LayoutQuery::TextRangeRects {
                source: node,
                utf16_range: 0..usize::MAX,
            },
            LayoutQuery::IntersectionGeometry {
                target: node,
                root: None,
            },
        ] {
            assert_eq!(retained.answer_query(&query), fresh.answer_query(&query));
        }
    }
    for y in (0..240).step_by(8) {
        for x in (0..width).step_by(8) {
            let point = LayoutPoint::new(x as f32, y as f32);
            assert_eq!(
                retained.hit_test(point, false),
                fresh.hit_test(point, false)
            );
        }
    }
    retained
}

struct DynamicInputs {
    source: Source,
    selection: Option<moli_layout::LayoutTextSelection>,
    image_size: Option<f32>,
}

impl LayoutSource for DynamicInputs {
    type NodeId = usize;
    type ChildIter<'a> = <Source as LayoutSource>::ChildIter<'a>;

    fn root(&self) -> usize {
        self.source.root()
    }
    fn flat_parent(&self, node: usize) -> Option<usize> {
        self.source.flat_parent(node)
    }
    fn flat_children(&self, node: usize) -> Self::ChildIter<'_> {
        self.source.flat_children(node)
    }
    fn node_kind(&self, node: usize) -> LayoutSourceKind {
        self.source.node_kind(node)
    }
    fn text(&self, node: usize) -> Option<&str> {
        self.source.text(node)
    }
    fn label(&self, node: usize) -> String {
        self.source.label(node)
    }

    fn element_semantics(&self, node: usize) -> Option<LayoutElementSemantics> {
        if node == 5 {
            Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "img",
                LayoutElementCategory::Generic,
                Some(moli_layout::LayoutReplacedKind::Image),
            ))
        } else {
            self.source.element_semantics(node)
        }
    }

    fn text_selection(&self, node: usize) -> Option<moli_layout::LayoutTextSelection> {
        (node == 2).then_some(self.selection).flatten()
    }

    fn replaced_metrics(&self, node: usize) -> Option<moli_layout::ReplacedMetrics> {
        (node == 5).then(|| moli_layout::ReplacedMetrics {
            intrinsic_width: self.image_size,
            intrinsic_height: self.image_size,
            ..Default::default()
        })
    }
}

#[test]
fn retained_work_updates_selection_and_inline_image_sizes() {
    let (source, mut styles, mut services) = fixture();
    let mut source = DynamicInputs {
        source,
        selection: None,
        image_size: Some(20.0),
    };
    source
        .source
        .0
        .push(Node::html_element("image", "img", vec![]));
    source.source.0[1].children.push(5);
    let mut image_style = styles.0[&1].clone();
    image_style.force_layout_display(LayoutDisplay::Inline);
    styles.0.insert(5, image_style);
    let mut work = LayoutWorkingSet::default();
    compare_source_pass(&source, 6, &mut styles, &mut services, &mut work, 100);
    for size in [45.0, 0.0, 180.0, 20.0] {
        source.image_size = Some(size);
        let pass = compare_source_pass(&source, 6, &mut styles, &mut services, &mut work, 100);
        assert_eq!(work.stats().reused_inline_contexts, 2);
        assert_eq!(
            pass.tree
                .element_metrics_for_source(5)
                .unwrap()
                .offset_size
                .width,
            size
        );
    }
    for selection in [
        Some(moli_layout::LayoutTextSelection::new(0, 10)),
        Some(moli_layout::LayoutTextSelection::new(4, 4)),
        None,
    ] {
        source.selection = selection;
        compare_source_pass(&source, 6, &mut styles, &mut services, &mut work, 100);
        assert_eq!(work.stats().reused_inline_contexts, 1);
        compare_source_pass(&source, 6, &mut styles, &mut services, &mut work, 100);
        assert_eq!(work.stats().reused_inline_contexts, 2);
    }
}

#[test]
fn retained_work_tracks_active_stylesheet_fonts_across_temporary_services() {
    let (source, mut styles, mut services) = fixture();
    services
        .register_web_font(
            WebFontRegistration::new(
                "fixed",
                WebFontFace::new("Moli Ahem"),
                include_bytes!("../fixtures/moli-ahem.ttf").to_vec(),
            )
            .with_stylesheet_scope(),
        )
        .unwrap();
    let mut work = LayoutWorkingSet::default();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    for active in [false, true, false, true] {
        services.with_active_stylesheet_fonts(
            |_| active,
            |selected| {
                compare_pass(&source, &mut styles, selected, &mut work, 100);
                assert_eq!(work.stats().reused_inline_contexts, 0);
            },
        );
        services.with_active_stylesheet_fonts(
            |_| active,
            |selected| {
                compare_pass(&source, &mut styles, selected, &mut work, 100);
                assert_eq!(work.stats().reused_inline_contexts, 2);
            },
        );
    }
}

#[test]
fn retained_work_rebreaks_text_for_current_constraints_and_survives_eviction() {
    let (source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    let first = compare_pass(&source, &mut styles, &mut services, &mut work, 320);
    assert_eq!(work.stats().retained_boxes, 5);
    assert_eq!(work.stats().reused_inline_contexts, 0);
    let narrow = compare_pass(&source, &mut styles, &mut services, &mut work, 80);
    assert_eq!(work.stats().reused_box_slots, 5);
    assert_eq!(work.stats().reused_inline_contexts, 2);
    assert!(
        narrow.text_range_rects(2, 0..usize::MAX).len()
            > first.text_range_rects(2, 0..usize::MAX).len()
    );
    for width in [240, 64, 400, 80, 320] {
        compare_pass(&source, &mut styles, &mut services, &mut work, width);
        assert_eq!(work.stats().reused_inline_contexts, 2);
        work.clear();
        compare_pass(&source, &mut styles, &mut services, &mut work, width);
        assert_eq!(work.stats().reused_inline_contexts, 0);
    }
}

#[test]
fn retained_work_invalidates_changed_text_and_style_without_discarding_other_paragraphs() {
    let (mut source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    source.0[2].text = Some("changed content wraps onto different lines");
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 1);
    let changed = styles.0[&1].clone().with_text_metrics(24.0, 30.0);
    styles.0.insert(1, changed);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 1);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 2);
}

#[test]
fn retained_work_retires_box_ids_when_structure_changes() {
    let (mut source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    // Reordering keeps the same node count but gives the constructor different
    // box IDs. No old inline source map may survive this graph replacement.
    source.0[0].children.reverse();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_box_slots, 0);
    assert_eq!(work.stats().reused_inline_contexts, 0);
    styles
        .0
        .get_mut(&1)
        .unwrap()
        .force_layout_display(LayoutDisplay::Inline);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_box_slots, 0);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert!(work.stats().reused_inline_contexts > 0);
    source.0[0].children.retain(|node| *node != 1);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_box_slots, 0);
}

#[test]
fn numeric_static_placeholders_do_not_retire_unchanged_text_inputs() {
    let (mut source, mut styles, mut services) = fixture();
    source.0[0].children.push(5);
    source.0.extend([
        Node::element("static-block", vec![6, 7]),
        Node::element("content", vec![]),
        Node::element("absolute", vec![]),
    ]);
    styles
        .0
        .insert(5, fixed_size(LayoutDisplay::Block, 100.0, 50.0));
    styles
        .0
        .insert(6, fixed_size(LayoutDisplay::Block, 80.0, 30.0));
    styles.0.insert(
        7,
        fixed_size(LayoutDisplay::Block, 20.0, 20.0).with_position(LayoutPosition::Absolute),
    );
    let mut work = LayoutWorkingSet::default();
    for width in [320, 100, 320] {
        let pass = compare_pass(&source, &mut styles, &mut services, &mut work, width);
        assert_eq!(
            pass.metrics.box_count, 9,
            "numeric layout adds one placeholder"
        );
        assert_eq!(work.stats().retained_boxes, 8, "placeholder is pass-local");
        if width == 100 {
            assert_eq!(work.stats().reused_inline_contexts, 2);
        }
    }
}

#[test]
fn retained_work_invalidates_fonts_even_when_family_and_slot_are_unchanged() {
    let (source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    services
        .register_web_font(WebFontRegistration::new(
            "fixed",
            WebFontFace::new("Moli Ahem"),
            include_bytes!("../fixtures/moli-hebrew-emoji.ttf").to_vec(),
        ))
        .unwrap();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 0);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 2);
    assert!(services.remove_web_font("fixed"));
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 0);
}

#[test]
fn retained_work_tracks_font_descriptors_with_identical_payloads() {
    let (source, mut styles, mut services) = fixture();
    let mut work = LayoutWorkingSet::default();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    for end in [0x60, 0x7f, 0x60, 0x7f] {
        services
            .register_web_font(WebFontRegistration::new(
                "fixed",
                WebFontFace::new("Moli Ahem")
                    .with_unicode_ranges([moli_layout::WebFontUnicodeRange::new(0, end)]),
                include_bytes!("../fixtures/moli-ahem.ttf").to_vec(),
            ))
            .unwrap();
        compare_pass(&source, &mut styles, &mut services, &mut work, 100);
        assert_eq!(work.stats().reused_inline_contexts, 0);
        compare_pass(&source, &mut styles, &mut services, &mut work, 100);
        assert_eq!(work.stats().reused_inline_contexts, 2);
    }
}

#[test]
fn retained_work_limits_and_failed_passes_do_not_leave_stale_state() {
    let (source, mut styles, mut services) = fixture();
    for limits in [
        LayoutWorkingSetLimits {
            max_boxes: 0,
            max_text_bytes: usize::MAX,
            ..Default::default()
        },
        LayoutWorkingSetLimits {
            max_boxes: usize::MAX,
            max_text_bytes: 1,
            ..Default::default()
        },
        LayoutWorkingSetLimits {
            max_estimated_retained_bytes: 1,
            ..Default::default()
        },
    ] {
        let mut work = LayoutWorkingSet::new(limits);
        for _ in 0..2 {
            compare_pass(&source, &mut styles, &mut services, &mut work, 100);
            assert_eq!(work.stats().retained_boxes, 0);
            assert_eq!(work.stats().reused_inline_contexts, 0);
        }
    }
    let mut work = LayoutWorkingSet::default();
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    let root_style = styles.0.remove(&0).unwrap();
    let error = build_layout_pass_with_working_set(
        &source,
        &mut styles,
        &mut services,
        LayoutPassRequest::new(LayoutViewport::new(100, 240, 1.0), LayoutFlushReason::Test),
        &mut NoFrames,
        Some(&mut work),
    )
    .err()
    .expect("missing root style must fail");
    assert!(matches!(error, LayoutError::MissingRootStyle { .. }));
    assert_eq!(work.stats().retained_boxes, 0);
    styles.0.insert(0, root_style);
    compare_pass(&source, &mut styles, &mut services, &mut work, 100);
    assert_eq!(work.stats().reused_inline_contexts, 0);
}
