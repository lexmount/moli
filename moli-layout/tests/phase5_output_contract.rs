use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutFlushReason, LayoutFragmentKind, LayoutNamespace, LayoutPassRequest,
    LayoutPassResult, LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch, LayoutRect,
    LayoutSource, LayoutSourceKind, LayoutStyleResolver, LayoutTransform2D, LayoutViewport,
    PaintColor, ResolvedLayoutStyle, build_layout_pass,
};
use style::Atom;
use taffy::{
    BoxSizing, Dimension, LengthPercentageAuto, Overflow, Point, Position, Rect, Size, Style,
    style_helpers::length,
};

#[derive(Clone)]
struct Node {
    label: &'static str,
    kind: LayoutSourceKind,
    text: Option<&'static str>,
    children: Vec<usize>,
    scroll: LayoutPoint,
}

impl Node {
    fn element(label: &'static str, children: Vec<usize>) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Element,
            text: None,
            children,
            scroll: LayoutPoint::ZERO,
        }
    }

    fn text(label: &'static str, text: &'static str) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Text,
            text: Some(text),
            children: Vec::new(),
            scroll: LayoutPoint::ZERO,
        }
    }
}

struct Source(Vec<Node>);

impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.0
            .iter()
            .position(|candidate| candidate.children.contains(&node))
    }

    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        self.0[node].children.iter().copied()
    }

    fn node_kind(&self, node: Self::NodeId) -> LayoutSourceKind {
        self.0[node].kind
    }

    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics> {
        (self.0[node].kind == LayoutSourceKind::Element).then(|| {
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "div",
                LayoutElementCategory::Generic,
                None,
            )
        })
    }

    fn text(&self, node: Self::NodeId) -> Option<&str> {
        self.0[node].text
    }

    fn label(&self, node: Self::NodeId) -> String {
        self.0[node].label.to_owned()
    }

    fn scroll_offset(&self, node: Self::NodeId) -> LayoutPoint {
        self.0[node].scroll
    }
}

#[derive(Default)]
struct Styles(HashMap<usize, ResolvedLayoutStyle>);

impl LayoutStyleResolver<usize> for Styles {
    fn primary_style(&mut self, node: usize) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        Ok(self.0.get(&node).cloned())
    }

    fn pseudo_style(
        &mut self,
        _node: usize,
        _pseudo: moli_layout::LayoutPseudo,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        Ok(None)
    }
}

fn resolved(display: LayoutDisplay, taffy: Style<Atom>) -> ResolvedLayoutStyle {
    ResolvedLayoutStyle::synthetic(display, taffy, PaintColor::TRANSPARENT)
}

fn fixed_size(display: LayoutDisplay, width: f32, height: f32) -> ResolvedLayoutStyle {
    resolved(
        display,
        Style {
            size: Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            },
            ..Style::default()
        },
    )
}

fn build(source: &Source, styles: &mut Styles) -> LayoutPassResult<usize> {
    build_with_request(
        source,
        styles,
        LayoutPassRequest::new(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    )
}

fn build_with_request(
    source: &Source,
    styles: &mut Styles,
    request: LayoutPassRequest,
) -> LayoutPassResult<usize> {
    build_layout_pass(source, styles, &mut DocumentLayoutServices::new(), request).unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.05,
        "expected {expected}, got {actual}"
    );
}

fn assert_rect(actual: LayoutRect, expected: LayoutRect) {
    assert_close(actual.x, expected.x);
    assert_close(actual.y, expected.y);
    assert_close(actual.width, expected.width);
    assert_close(actual.height, expected.height);
}

#[test]
fn display_none_root_uses_an_unmapped_internal_carrier() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("suppressed-child", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, resolved(LayoutDisplay::None, Style::default()));
    styles
        .0
        .insert(1, fixed_size(LayoutDisplay::Block, 100.0, 20.0));

    let output = build(&source, &mut styles);
    assert!(output.source_output(0).is_none());
    assert!(output.client_rects_for_source(0).is_empty());
    assert!(output.box_model_for_source(0).is_none());
    assert!(output.source_output(1).is_none());
}

#[test]
fn pass_result_owns_complete_box_models_and_answers_a_batch_from_one_pass() {
    let source = Source(vec![Node::element("root", Vec::new())]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style {
                box_sizing: BoxSizing::ContentBox,
                size: Size {
                    width: length(100.0),
                    height: length(60.0),
                },
                margin: Rect {
                    left: length(3.0),
                    right: length(3.0),
                    top: length(3.0),
                    bottom: length(3.0),
                },
                padding: Rect {
                    left: length(5.0),
                    right: length(5.0),
                    top: length(5.0),
                    bottom: length(5.0),
                },
                border: Rect {
                    left: length(2.0),
                    right: length(2.0),
                    top: length(2.0),
                    bottom: length(2.0),
                },
                ..Style::default()
            },
        ),
    );

    let output = build(&source, &mut styles);
    assert!(output.paint_snapshot().is_none());
    assert_eq!(output.metrics.paint_operation_count, 0);
    let model = output.box_model_for_source(0).unwrap();
    assert_rect(
        model.content.bounding_rect(),
        LayoutRect::new(10.0, 10.0, 100.0, 60.0),
    );
    assert_rect(
        model.padding.bounding_rect(),
        LayoutRect::new(5.0, 5.0, 110.0, 70.0),
    );
    assert_rect(
        model.border.bounding_rect(),
        LayoutRect::new(3.0, 3.0, 114.0, 74.0),
    );
    assert_rect(
        model.margin.bounding_rect(),
        LayoutRect::new(0.0, 0.0, 120.0, 80.0),
    );

    let answers = output.answer_queries(&LayoutQueryBatch::new(vec![
        LayoutQuery::DocumentMetrics,
        LayoutQuery::BoxModel { source: 0 },
        LayoutQuery::ClientRects { source: 0 },
    ]));
    assert_eq!(answers.answers.len(), 3);
    assert_eq!(answers.metrics.reason, LayoutFlushReason::Test);
    assert_eq!(answers.metrics.box_count, output.boxes.len());
    assert!(matches!(
        answers.answers[0],
        LayoutQueryAnswer::DocumentMetrics(_)
    ));
    assert!(matches!(
        answers.answers[1],
        LayoutQueryAnswer::BoxModel(Some(_))
    ));
    assert!(matches!(
        &answers.answers[2],
        LayoutQueryAnswer::ClientRects(rects) if rects.len() == 1
    ));
}

#[test]
fn own_border_is_visual_geometry_not_scrollable_content() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("bordered-child", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                box_sizing: BoxSizing::ContentBox,
                size: Size {
                    width: length(100.0),
                    height: length(60.0),
                },
                border: Rect {
                    left: length(7.0),
                    right: length(11.0),
                    top: length(5.0),
                    bottom: length(13.0),
                },
                ..Style::default()
            },
        ),
    );

    let output = build(&source, &mut styles);
    let metrics = output.element_metrics_for_source(1).unwrap();
    assert_eq!(
        metrics.client_size,
        moli_layout::LayoutSize::new(100.0, 60.0)
    );
    assert_eq!(metrics.scroll_size, metrics.client_size);
    assert_eq!(metrics.client_border, LayoutPoint::new(7.0, 5.0));
}

#[test]
fn pass_output_freezes_into_the_sole_queryable_retained_tree() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("target", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 100.0));
    styles
        .0
        .insert(1, fixed_size(LayoutDisplay::Block, 80.0, 30.0));

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(200, 100, 1.0), LayoutFlushReason::Test),
    );
    assert!(output.paint_snapshot().is_some());
    let metrics = output.metrics;
    let tree = output.into_tree();

    assert!(tree.source_output(1).is_some());
    assert_eq!(
        tree.hit_test(LayoutPoint::new(10.0, 10.0), false)
            .expect("the frozen tree derives a hit-test view")
            .source,
        1
    );
    let answers = tree.answer_queries(
        &LayoutQueryBatch::new(vec![LayoutQuery::BoxModel { source: 1 }]),
        metrics,
    );
    assert!(matches!(
        answers.answers.as_slice(),
        [LayoutQueryAnswer::BoxModel(Some(_))]
    ));
}

#[test]
fn document_content_size_includes_visible_descendant_end_margin() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("overflowing-child", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: length(250.0),
                },
                margin: Rect {
                    left: length(0.0),
                    right: length(0.0),
                    top: length(0.0),
                    bottom: length(30.0),
                },
                ..Style::default()
            },
        ),
    );

    let output = build(&source, &mut styles);
    assert_eq!(
        output.content_size,
        moli_layout::LayoutSize::new(320.0, 280.0)
    );
}

#[test]
fn initial_containing_block_absolute_box_expands_root_scrollable_overflow() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("viewport-absolute", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(500.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(400.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(50.0),
                    height: length(50.0),
                },
                ..Style::default()
            },
        ),
    );

    let output = build(&source, &mut styles);
    assert_eq!(
        output.content_size,
        moli_layout::LayoutSize::new(550.0, 450.0)
    );
    let root_metrics = output.element_metrics_for_source(0).unwrap();
    assert_eq!(root_metrics.scroll_size.width, 550.0);
    assert_eq!(root_metrics.scroll_size.height, 450.0);
    assert_rect(
        root_metrics.scrollport.bounding_rect(),
        LayoutRect::new(0.0, 0.0, 320.0, 240.0),
    );
    assert_eq!(
        root_metrics.maximum_scroll_offset,
        LayoutPoint::new(230.0, 210.0)
    );
}

#[test]
fn inline_output_preserves_line_text_and_utf16_source_fragments() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("inline", vec![2]),
        Node::text("text", "ab😀cd"),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 80.0));
    styles
        .0
        .insert(1, resolved(LayoutDisplay::Inline, Style::default()));
    styles
        .0
        .insert(2, resolved(LayoutDisplay::Inline, Style::default()));

    let output = build(&source, &mut styles);
    assert!(output.fragments.iter().any(|fragment| matches!(
        fragment.kind,
        LayoutFragmentKind::Line {
            owner: _,
            line_index: 0
        }
    )));
    let text_fragments = output
        .source_output(2)
        .unwrap()
        .fragments
        .iter()
        .filter_map(|id| output.fragment(*id))
        .filter_map(|fragment| match &fragment.kind {
            LayoutFragmentKind::Text {
                source_utf16_range, ..
            } => Some(source_utf16_range.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!text_fragments.is_empty());
    assert_eq!(text_fragments.first().unwrap().start, 0);
    assert_eq!(text_fragments.last().unwrap().end, 6);

    let inline_model = output.box_model_for_source(1).unwrap();
    assert!(inline_model.border.bounding_rect().width > 0.0);
    assert_eq!(output.client_rects_for_source(1).len(), 1);
    let range_rects = output.text_range_rects(2, 2..4);
    assert!(!range_rects.is_empty());
    assert!(
        range_rects
            .iter()
            .all(|quad| quad.bounding_rect().width > 0.0)
    );
    assert_eq!(
        output.text_range_rects(2, 0..6).len(),
        1,
        "adjacent Parley clusters on one directional line are one CSSOM Range rect"
    );
    let scroll_geometry = output
        .scroll_into_view_geometry_for_source(2)
        .expect("rendered text fragments should provide scroll target geometry");
    assert!(!scroll_geometry.target_rects.is_empty());
    assert_eq!(scroll_geometry.scroll_containers.len(), 1);
}

#[test]
fn inline_and_range_rects_use_font_bounds_instead_of_the_css_line_height() {
    const CJK_TEXT: &str = "台风白海豚在浙江玉环沿海登陆";
    const LATIN_TEXT: &str = "Title text";
    let source = Source(vec![
        Node::element("root", vec![1, 3]),
        Node::element("inline", vec![2]),
        Node::text("cjk-text", CJK_TEXT),
        Node::element("latin-inline", vec![4]),
        Node::text("latin-text", LATIN_TEXT),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 320.0, 80.0).with_text_metrics(14.0, 36.0),
    );
    for node in 1..=4 {
        styles.0.insert(
            node,
            resolved(LayoutDisplay::Inline, Style::default()).with_text_metrics(16.0, 36.0),
        );
    }

    let output = build(&source, &mut styles);
    let line_rect = output
        .fragments
        .iter()
        .find_map(|fragment| {
            matches!(fragment.kind, LayoutFragmentKind::Line { .. }).then_some(fragment.rect)
        })
        .expect("one line fragment");
    let cjk_inline_rect = output.client_rects_for_source(1)[0].bounding_rect();
    let cjk_range_rect =
        output.text_range_rects(2, 0..CJK_TEXT.encode_utf16().count())[0].bounding_rect();
    let latin_range_rect =
        output.text_range_rects(4, 0..LATIN_TEXT.encode_utf16().count())[0].bounding_rect();

    assert!(
        line_rect.height >= 36.0,
        "the requested CSS line height remains the line-box floor: {line_rect:?}"
    );
    assert!(
        cjk_inline_rect.height < line_rect.height,
        "inline geometry must use the font box, not the CSS line box: {cjk_inline_rect:?}"
    );
    assert_rect(cjk_range_rect, cjk_inline_rect);
    assert_close(cjk_range_rect.y, latin_range_rect.y);
    assert_close(cjk_range_rect.height, latin_range_rect.height);
    assert!(cjk_inline_rect.y > line_rect.y);
    assert!(cjk_inline_rect.bottom() < line_rect.bottom());
}

#[test]
fn display_contents_has_no_css_box_but_can_scroll_its_rendered_contents_into_view() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("contents", vec![2]),
        Node::text("text", "rendered contents"),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 80.0));
    styles
        .0
        .insert(1, resolved(LayoutDisplay::Contents, Style::default()));
    styles
        .0
        .insert(2, resolved(LayoutDisplay::Inline, Style::default()));

    let output = build(&source, &mut styles);
    let contents = output.source_output(1).expect("display: contents source");
    assert!(contents.principal_box.is_none());
    assert!(contents.fragments.is_empty());
    assert!(output.box_model_for_source(1).is_none());
    assert!(output.client_rects_for_source(1).is_empty());

    let scroll_geometry = output
        .scroll_into_view_geometry_for_source(1)
        .expect("rendered descendants should provide a scroll target");
    assert!(!scroll_geometry.target_rects.is_empty());
    assert_eq!(scroll_geometry.scroll_containers.len(), 1);
}

#[test]
fn event_offset_uses_the_shared_ifc_space_for_inline_targets() {
    let source = Source(vec![
        Node::element("root", vec![1, 2]),
        Node::text("prefix", "prefix"),
        Node::element("inline", vec![3]),
        Node::text("target", "target"),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 80.0));
    for node in 1..=3 {
        styles
            .0
            .insert(node, resolved(LayoutDisplay::Inline, Style::default()));
    }

    let output = build(&source, &mut styles);
    let inline_rect = output.client_rects_for_source(2)[0].bounding_rect();
    assert!(
        inline_rect.x > 0.0,
        "the prefix must move the inline target"
    );
    let metrics = output
        .element_metrics_for_source(2)
        .expect("the inline target has CSSOM metrics");
    assert_close(metrics.offset_position.x, inline_rect.x);
    assert_close(metrics.offset_position.y, inline_rect.y);
    let point = LayoutPoint::new(inline_rect.x + 1.0, inline_rect.y + 2.0);
    let offset = output
        .event_offset_for_source(2, point)
        .expect("an inline layout object has an IFC coordinate space");
    assert_close(offset.x, point.x);
    assert_close(offset.y, point.y);
    assert_close(point.x - inline_rect.x, 1.0);
}

#[test]
fn caret_query_uses_parley_cluster_sides_and_inline_direction() {
    fn assert_cluster_sides(text: &'static str, expected_rtl: bool) {
        let source = Source(vec![
            Node::element("root", vec![1]),
            Node::text("text", text),
        ]);
        let mut styles = Styles::default();
        styles
            .0
            .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 80.0));
        styles
            .0
            .insert(1, resolved(LayoutDisplay::Inline, Style::default()));

        let output = build(&source, &mut styles);
        let fragment = output
            .source_output(1)
            .into_iter()
            .flat_map(|source| source.fragments)
            .filter_map(|id| output.fragment(id))
            .find(|fragment| {
                matches!(
                    fragment.kind,
                    LayoutFragmentKind::Text {
                        source_utf16_range: ref range,
                        ..
                    } if *range == (0..1)
                )
            })
            .expect("one UTF-16 code-unit text fragment");
        let LayoutFragmentKind::Text { rtl, .. } = &fragment.kind else {
            unreachable!();
        };
        assert_eq!(*rtl, expected_rtl);
        assert!(fragment.rect.width > 0.0);

        let left = output
            .caret_position(LayoutPoint::new(
                fragment.rect.x + fragment.rect.width * 0.25,
                fragment.rect.y + fragment.rect.height * 0.5,
            ))
            .expect("left cluster half should resolve a caret");
        let right = output
            .caret_position(LayoutPoint::new(
                fragment.rect.x + fragment.rect.width * 0.75,
                fragment.rect.y + fragment.rect.height * 0.5,
            ))
            .expect("right cluster half should resolve a caret");
        assert_eq!(left.source, 1);
        assert_eq!(right.source, 1);
        if expected_rtl {
            assert_eq!(left.utf16_offset, Some(1));
            assert_eq!(right.utf16_offset, Some(0));
            assert_close(left.rect.bounding_rect().x, fragment.rect.x);
            assert_close(right.rect.bounding_rect().x, fragment.rect.right());
        } else {
            assert_eq!(left.utf16_offset, Some(0));
            assert_eq!(right.utf16_offset, Some(1));
            assert_close(left.rect.bounding_rect().x, fragment.rect.x);
            assert_close(right.rect.bounding_rect().x, fragment.rect.right());
        }
        assert!(
            left.ancestor_boxes.iter().any(|(source, _)| *source == 0),
            "caret retargeting must receive ancestor box models from the same pass"
        );
    }

    assert_cluster_sides("a", false);
    assert_cluster_sides("א", true);
}

#[test]
fn split_inline_continuations_remain_mapped_to_the_originating_element() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("split-inline", vec![2, 3, 4]),
        Node::text("before", "AA"),
        Node::element("block", Vec::new()),
        Node::text("after", "BB"),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 120.0));
    styles
        .0
        .insert(1, resolved(LayoutDisplay::Inline, Style::default()));
    styles
        .0
        .insert(2, resolved(LayoutDisplay::Inline, Style::default()));
    styles
        .0
        .insert(3, fixed_size(LayoutDisplay::Block, 50.0, 20.0));
    styles
        .0
        .insert(4, resolved(LayoutDisplay::Inline, Style::default()));

    let output = build(&source, &mut styles);
    let rects = output.client_rects_for_source(1);
    assert_eq!(rects.len(), 2, "{rects:?}");
    let first = rects[0].bounding_rect();
    let second = rects[1].bounding_rect();
    assert!(first.width > 0.0 && second.width > 0.0);
    assert!(second.y > first.y, "{rects:?}");
    let union = output
        .box_model_for_source(1)
        .expect("split inline box model")
        .border
        .bounding_rect();
    assert!(union.height >= second.bottom() - first.y);
}

#[test]
fn scroll_is_sampled_per_pass_and_updates_geometry_clip_and_hit_testing() {
    let mut source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::element("wide-child", Vec::new()),
    ]);
    source.0[1].scroll = LayoutPoint::new(40.0, 30.0);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: length(80.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        ),
    );
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 300.0, 200.0));

    let first = build(&source, &mut styles);
    let scroll_box = first.source_output(1).unwrap().principal_box.unwrap();
    let scroll_extent = first.scroll_extent(scroll_box).unwrap();
    assert_eq!(scroll_extent.applied_offset, LayoutPoint::new(40.0, 30.0));
    assert_eq!(scroll_extent.minimum_offset, LayoutPoint::ZERO);
    assert_eq!(scroll_extent.maximum_offset, LayoutPoint::new(200.0, 120.0));
    assert_close(
        first
            .box_model_for_source(2)
            .unwrap()
            .border
            .bounding_rect()
            .x,
        -40.0,
    );
    assert_eq!(
        first
            .hit_test(LayoutPoint::new(10.0, 10.0), false)
            .unwrap()
            .source,
        2
    );
    assert_eq!(
        first
            .hit_test(LayoutPoint::new(150.0, 10.0), false)
            .unwrap()
            .source,
        0
    );

    source.0[1].scroll = LayoutPoint::new(10.0, 0.0);
    let second = build(&source, &mut styles);
    assert_eq!(first.viewport_scroll, LayoutPoint::ZERO);
    assert_eq!(second.viewport_scroll, LayoutPoint::ZERO);
    assert_close(
        second
            .box_model_for_source(2)
            .unwrap()
            .border
            .bounding_rect()
            .x,
        -10.0,
    );
}

#[test]
fn transforms_and_semantic_paint_order_share_the_hit_test_projection() {
    let source = Source(vec![
        Node::element("root", vec![1, 2]),
        Node::element("under", Vec::new()),
        Node::element("over", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 240.0, 180.0)
            .with_position(moli_layout::LayoutPosition::Relative),
    );
    let overlay = |transform| {
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(20.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(20.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(80.0),
                    height: length(80.0),
                },
                ..Style::default()
            },
        )
        .with_2d_transform(transform)
    };
    styles.0.insert(1, overlay(LayoutTransform2D::IDENTITY));
    styles
        .0
        .insert(2, overlay(LayoutTransform2D::translation(10.0, 5.0)));

    let mut output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    assert!(output.paint_snapshot().is_some());
    assert_rect(
        output
            .box_model_for_source(2)
            .unwrap()
            .border
            .bounding_rect(),
        LayoutRect::new(30.0, 25.0, 80.0, 80.0),
    );
    assert_eq!(
        output
            .hit_test(LayoutPoint::new(40.0, 40.0), false)
            .unwrap()
            .source,
        2
    );
    assert!(
        output
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "transform-paint-deferred")
    );
    let retention = output.retention_metrics();
    assert_eq!(retention.box_count, output.boxes.len());
    assert_eq!(retention.fragment_count, output.fragments.len());
    assert!(retention.estimated_geometry_bytes > 0);
    let _paint = output
        .take_paint_snapshot()
        .expect("a paint request should expose one movable paint snapshot");
    assert!(output.paint_snapshot().is_none());
    assert_eq!(output.retention_metrics(), retention);
    assert_eq!(
        output
            .hit_test(LayoutPoint::new(40.0, 40.0), false)
            .unwrap()
            .source,
        2,
        "taking paint resources must leave geometry queries intact"
    );
}

#[test]
fn viewport_fixed_geometry_does_not_move_with_root_scroll() {
    let mut source = Source(vec![
        Node::element("root", vec![1, 2]),
        Node::element("document-flow", Vec::new()),
        Node::element("fixed", Vec::new()),
    ]);
    source.0[0].scroll = LayoutPoint::new(0.0, 50.0);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 120.0));
    styles
        .0
        .insert(1, fixed_size(LayoutDisplay::Block, 200.0, 400.0));
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(10.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(15.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(40.0),
                    height: length(30.0),
                },
                ..Style::default()
            },
        )
        .with_position(moli_layout::LayoutPosition::Fixed),
    );

    let output = build(&source, &mut styles);
    let fixed = output
        .box_model_for_source(2)
        .unwrap()
        .border
        .bounding_rect();
    assert_close(fixed.x, 10.0);
    assert_close(fixed.y, 15.0);
    assert_close(
        output
            .box_model_for_source(1)
            .unwrap()
            .border
            .bounding_rect()
            .y,
        -50.0,
    );
}

#[test]
fn viewport_fixed_box_escapes_intermediate_overflow_clip_for_paint_and_hit_test() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("overflow-ancestor", vec![2]),
        Node::element("viewport-fixed", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(80.0),
                    height: length(40.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(120.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(80.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(60.0),
                    height: length(40.0),
                },
                ..Style::default()
            },
        )
        .with_position(moli_layout::LayoutPosition::Fixed),
    );

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    let fixed_box = output
        .source_output(2)
        .and_then(|source| source.principal_box)
        .expect("fixed principal box");
    let fixed_clip = output.boxes[fixed_box.index()]
        .clip_chain
        .expect("viewport clip");
    assert_eq!(output.clip_chain[fixed_clip.index()].owner, None);
    assert_rect(
        output
            .box_model_for_source(2)
            .unwrap()
            .border
            .bounding_rect(),
        LayoutRect::new(120.0, 80.0, 60.0, 40.0),
    );
    assert_eq!(
        output
            .hit_test(LayoutPoint::new(130.0, 90.0), false)
            .expect("fixed box outside the intermediate overflow clip remains hittable")
            .source,
        2
    );
}

#[test]
fn absolute_box_escapes_overflow_clip_between_it_and_its_containing_block() {
    let source = Source(vec![
        Node::element("positioned-root", vec![1]),
        Node::element("overflow-ancestor", vec![2]),
        Node::element("absolute", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 320.0, 240.0)
            .with_position(moli_layout::LayoutPosition::Relative),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(80.0),
                    height: length(40.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(120.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(80.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(60.0),
                    height: length(40.0),
                },
                ..Style::default()
            },
        ),
    );

    let output = build(&source, &mut styles);
    let absolute_box = output
        .source_output(2)
        .and_then(|source| source.principal_box)
        .expect("absolute principal box");
    let absolute_clip = output.boxes[absolute_box.index()]
        .clip_chain
        .expect("root clip");
    assert_eq!(output.clip_chain[absolute_clip.index()].owner, None);
    assert_eq!(
        output
            .hit_test(LayoutPoint::new(130.0, 90.0), false)
            .expect("absolute box outside the intermediate overflow clip remains hittable")
            .source,
        2
    );
}

#[test]
fn transformed_fixed_containing_block_still_clips_its_fixed_descendant() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("transformed-overflow-containing-block", vec![2]),
        Node::element("contained-fixed", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: length(60.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        )
        .with_transform_containing_block(),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(80.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(10.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(50.0),
                    height: length(30.0),
                },
                ..Style::default()
            },
        )
        .with_position(moli_layout::LayoutPosition::Fixed),
    );

    let output = build(&source, &mut styles);
    let containing_block = output
        .source_output(1)
        .and_then(|source| source.principal_box)
        .expect("fixed containing block");
    let fixed_box = output
        .source_output(2)
        .and_then(|source| source.principal_box)
        .expect("fixed principal box");
    assert_eq!(
        output.clip_chain[output.boxes[fixed_box.index()].clip_chain.unwrap().index()].owner,
        Some(containing_block)
    );
    assert_eq!(
        output
            .hit_test(LayoutPoint::new(90.0, 20.0), false)
            .expect("the visible part of the fixed box remains hittable")
            .source,
        2
    );
    assert_ne!(
        output
            .hit_test(LayoutPoint::new(110.0, 20.0), false)
            .expect("the root remains underneath the clipped fixed box")
            .source,
        2,
        "the fixed containing block's overflow clip must still apply"
    );
}
