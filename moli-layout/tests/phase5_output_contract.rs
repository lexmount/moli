use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutControlSurfaceHit, LayoutDisplay, LayoutElementCategory,
    LayoutElementSemantics, LayoutError, LayoutFlushReason, LayoutFragmentKind, LayoutNamespace,
    LayoutPaintedSurfaceHit, LayoutPassRequest, LayoutPassResult, LayoutPoint, LayoutPosition,
    LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch, LayoutRect, LayoutScrollbarColors,
    LayoutScrollbarGutter, LayoutScrollbarPart, LayoutScrollbarWidth, LayoutSource,
    LayoutSourceKind, LayoutStyleResolver, LayoutTransform2D, LayoutViewport, PaintBrush,
    PaintCaptureRequest, PaintColor, PaintFragment, PaintShape, ResolvedLayoutElementStyles,
    ResolvedLayoutStyle, build_layout_pass,
};
use style::Atom;
use taffy::{
    BoxSizing, Dimension, LengthPercentageAuto, Overflow, Point, Position, Rect, Size, Style,
    style_helpers::{length, percent},
};

#[derive(Clone)]
struct Node {
    label: &'static str,
    local_name: &'static str,
    kind: LayoutSourceKind,
    text: Option<&'static str>,
    children: Vec<usize>,
    scroll: LayoutPoint,
}

impl Node {
    fn element(label: &'static str, children: Vec<usize>) -> Self {
        Self {
            label,
            local_name: "div",
            kind: LayoutSourceKind::Element,
            text: None,
            children,
            scroll: LayoutPoint::ZERO,
        }
    }

    fn html_element(label: &'static str, local_name: &'static str, children: Vec<usize>) -> Self {
        Self {
            label,
            local_name,
            kind: LayoutSourceKind::Element,
            text: None,
            children,
            scroll: LayoutPoint::ZERO,
        }
    }

    fn text(label: &'static str, text: &'static str) -> Self {
        Self {
            label,
            local_name: "#text",
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
                self.0[node].local_name,
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
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok(self
            .0
            .get(&node)
            .cloned()
            .map(ResolvedLayoutElementStyles::from_primary))
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

fn assert_balanced_paint_stack(fragments: &[PaintFragment]) {
    let mut depth = 0usize;
    for (index, fragment) in fragments.iter().enumerate() {
        match fragment {
            PaintFragment::PushLayer { .. } | PaintFragment::PushClip { .. } => depth += 1,
            PaintFragment::PopLayer => {
                assert!(depth > 0, "paint stack underflow at fragment {index}");
                depth -= 1;
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0, "paint stream left layers or clips open");
}

fn solid_fill_index(fragments: &[PaintFragment], expected: PaintColor) -> usize {
    fragments
        .iter()
        .position(|fragment| {
            fragment
                .solid_fill()
                .is_some_and(|(_, color, _)| color == expected)
        })
        .unwrap_or_else(|| panic!("missing solid fill {expected:?}"))
}

fn matching_paint_stack_pop(fragments: &[PaintFragment], push_index: usize) -> usize {
    assert!(matches!(
        fragments.get(push_index),
        Some(PaintFragment::PushLayer { .. } | PaintFragment::PushClip { .. })
    ));
    let mut depth = 0usize;
    for (index, fragment) in fragments.iter().enumerate().skip(push_index) {
        match fragment {
            PaintFragment::PushLayer { .. } | PaintFragment::PushClip { .. } => depth += 1,
            PaintFragment::PopLayer => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    panic!("push at fragment {push_index} has no matching pop")
}

fn innermost_active_clip_at(
    fragments: &[PaintFragment],
    fragment_index: usize,
) -> (PaintShape, LayoutTransform2D) {
    let mut stack = Vec::new();
    for fragment in &fragments[..=fragment_index] {
        match fragment {
            PaintFragment::PushLayer { .. } => stack.push(None),
            PaintFragment::PushClip { shape, transform } => {
                stack.push(Some((shape.clone(), *transform)));
            }
            PaintFragment::PopLayer => {
                stack.pop().expect("paint stack must remain balanced");
            }
            _ => {}
        }
    }
    stack
        .into_iter()
        .rev()
        .flatten()
        .next()
        .expect("painted fragment must have an active clip")
}

#[test]
fn classic_scrollbars_share_layout_paint_and_hit_test_geometry() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::element("oversized-content", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    let scroller = Style {
        size: Size {
            width: length(200.0),
            height: length(100.0),
        },
        overflow: Point {
            x: Overflow::Scroll,
            y: Overflow::Scroll,
        },
        ..Style::default()
    };
    let mut scroller = resolved(LayoutDisplay::Block, scroller);
    let thumb = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let track = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    scroller.set_scrollbar_style(
        LayoutScrollbarWidth::Auto,
        moli_layout::LayoutScrollbarGutter::Auto,
        Some(LayoutScrollbarColors::new(thumb, track)),
    );
    styles.0.insert(1, scroller);
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 400.0, 300.0));

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    let metrics = output.element_metrics_for_source(1).unwrap();
    assert_eq!(
        metrics.client_size,
        moli_layout::LayoutSize::new(185.0, 85.0)
    );
    assert_eq!(
        metrics.scroll_size,
        moli_layout::LayoutSize::new(400.0, 300.0)
    );
    assert_eq!(
        metrics.maximum_scroll_offset,
        LayoutPoint::new(215.0, 215.0)
    );

    let box_id = output.source_output(1).unwrap().principal_box.unwrap();
    let extent = output.scroll_extent(box_id).unwrap();
    let horizontal = extent.horizontal_scrollbar.expect("horizontal control");
    let vertical = extent.vertical_scrollbar.expect("vertical control");
    assert_rect(horizontal.frame, LayoutRect::new(0.0, 85.0, 185.0, 15.0));
    assert_rect(vertical.frame, LayoutRect::new(185.0, 0.0, 15.0, 85.0));
    assert_rect(horizontal.track, LayoutRect::new(18.0, 85.0, 149.0, 15.0));
    assert_rect(horizontal.thumb, LayoutRect::new(18.0, 85.0, 69.0, 15.0));
    assert_rect(
        horizontal.painted_thumb,
        LayoutRect::new(18.0, 88.0, 69.0, 9.0),
    );
    assert_rect(vertical.track, LayoutRect::new(185.0, 18.0, 15.0, 49.0));
    assert_rect(vertical.thumb, LayoutRect::new(185.0, 18.0, 15.0, 17.0));
    assert_rect(
        vertical.painted_thumb,
        LayoutRect::new(188.0, 18.0, 9.0, 17.0),
    );
    assert_rect(
        extent.scrollbar_corner.expect("two-axis corner"),
        LayoutRect::new(185.0, 85.0, 15.0, 15.0),
    );
    let hit = output
        .scrollbar_hit_test(LayoutPoint::new(190.0, 30.0))
        .expect("thumb should use the frozen paint geometry");
    assert_eq!(hit.source, 1);
    assert_eq!(hit.part, LayoutScrollbarPart::Thumb);
    let corner = output
        .control_surface_hit_test(LayoutPoint::new(190.0, 90.0))
        .expect("painted corner must consume UA-control input");
    assert!(matches!(
        corner,
        LayoutControlSurfaceHit::ScrollbarCorner(hit)
            if hit.source == 1 && hit.rect == LayoutRect::new(185.0, 85.0, 15.0, 15.0)
    ));

    let snapshot = output.paint_snapshot().expect("paint request");
    assert!(snapshot.fragments.iter().any(|fragment| matches!(
        fragment,
        PaintFragment::Fill {
            shape: PaintShape::RoundedRect { .. },
            brush: PaintBrush::Solid(color),
            ..
        } if *color == thumb
    )));
    assert_eq!(
        snapshot
            .fragments
            .iter()
            .filter(|fragment| matches!(
                fragment,
                PaintFragment::Fill {
                    shape: PaintShape::Path(_),
                    brush: PaintBrush::Solid(color),
                    ..
                } if *color == thumb
            ))
            .count(),
        4,
        "each axis paints Chromium's back and forward arrow glyphs"
    );
    assert!(snapshot.fragments.iter().any(|fragment| matches!(
        fragment,
        PaintFragment::Fill {
            brush: PaintBrush::Solid(color),
            ..
        } if *color == track
    )));
}

#[test]
fn painted_surface_order_keeps_an_overlay_above_a_scrollbar() {
    let source = Source(vec![
        Node::element("root", vec![1, 3]),
        Node::element("scroller", vec![2]),
        Node::element("oversized-content", Vec::new()),
        Node::element("overlay", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    let positioned = |left, top, width, height| Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(left),
            right: LengthPercentageAuto::auto(),
            top: length(top),
            bottom: LengthPercentageAuto::auto(),
        },
        size: Size {
            width: length(width),
            height: length(height),
        },
        ..Style::default()
    };
    let mut scroller = resolved(
        LayoutDisplay::Block,
        Style {
            overflow: Point {
                x: Overflow::Scroll,
                y: Overflow::Scroll,
            },
            ..positioned(20.0, 20.0, 200.0, 100.0)
        },
    )
    .with_position(moli_layout::LayoutPosition::Absolute);
    scroller.set_scrollbar_style(
        LayoutScrollbarWidth::Auto,
        LayoutScrollbarGutter::Auto,
        None,
    );
    styles.0.insert(1, scroller);
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 400.0, 300.0));
    styles.0.insert(
        3,
        resolved(LayoutDisplay::Block, positioned(200.0, 20.0, 40.0, 100.0))
            .with_position(moli_layout::LayoutPosition::Absolute)
            .with_z_index(10),
    );

    let output = build(&source, &mut styles);
    let point = LayoutPoint::new(210.0, 30.0);
    assert!(matches!(
        output.control_surface_hit_test(point),
        Some(LayoutControlSurfaceHit::Scrollbar(hit)) if hit.source == 1
    ));
    let surfaces = output.painted_surface_hits(point, false);
    assert!(matches!(
        surfaces.first(),
        Some(LayoutPaintedSurfaceHit::Dom(hit)) if hit.source == 3
    ));
    let overlay_order = surfaces[0].paint_order();
    let scrollbar_order = surfaces
        .iter()
        .copied()
        .find_map(|surface| match surface {
            LayoutPaintedSurfaceHit::Control(LayoutControlSurfaceHit::Scrollbar(hit))
                if hit.source == 1 =>
            {
                Some(hit.paint_order)
            }
            _ => None,
        })
        .expect("covered scrollbar remains in the sampled paint stack");
    assert!(overlay_order > scrollbar_order);
}

#[test]
fn non_viewport_captures_omit_only_the_root_scrollbars() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("oversized-content", Vec::new()),
    ]);
    let mut styles = Styles::default();
    let mut root = fixed_size(LayoutDisplay::Block, 320.0, 240.0);
    let thumb = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let track = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    root.set_scrollbar_style(
        LayoutScrollbarWidth::Auto,
        LayoutScrollbarGutter::Auto,
        Some(LayoutScrollbarColors::new(thumb, track)),
    );
    styles.0.insert(0, root);
    styles
        .0
        .insert(1, fixed_size(LayoutDisplay::Block, 640.0, 480.0));

    let scrollbar_fill_count = |snapshot: &moli_layout::PaintSnapshot| {
        snapshot
            .fragments
            .iter()
            .filter(|fragment| {
                matches!(
                    fragment,
                    PaintFragment::Fill {
                        brush: PaintBrush::Solid(color),
                        ..
                    } if *color == thumb || *color == track
                )
            })
            .count()
    };
    let viewport = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    assert!(
        scrollbar_fill_count(viewport.paint_snapshot().expect("viewport paint")) >= 4,
        "the viewport capture includes both root controls"
    );

    for capture in [
        PaintCaptureRequest::full_document(),
        PaintCaptureRequest::page_clip(LayoutRect::new(0.0, 0.0, 320.0, 240.0), 1.0),
    ] {
        let output = build_with_request(
            &source,
            &mut styles,
            LayoutPassRequest::with_capture(
                LayoutViewport::new(320, 240, 1.0),
                LayoutFlushReason::Test,
                capture,
            ),
        );
        assert_eq!(
            scrollbar_fill_count(output.paint_snapshot().expect("non-viewport paint")),
            0,
            "full-document and page-clip captures omit root viewport controls"
        );
    }
}

#[test]
fn viewport_paint_culls_offscreen_events_without_losing_transformed_or_fixed_content() {
    let source = Source(vec![
        Node::element("root", vec![1, 2, 3, 4, 5]),
        Node::element("visible", Vec::new()),
        Node::element("deep", Vec::new()),
        Node::element("transformed-into-view", Vec::new()),
        Node::element("viewport-fixed", Vec::new()),
        Node::element("offscreen-context", vec![6]),
        Node::element("offscreen-context-child", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 120.0, 800.0).with_position(LayoutPosition::Relative),
    );
    let absolute = |top: f32, color: PaintColor| {
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(10.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(top),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(30.0),
                    height: length(20.0),
                },
                ..Style::default()
            },
            color,
        )
        .with_position(LayoutPosition::Absolute)
    };
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    let green = PaintColor::new(0.0, 1.0, 0.0, 1.0);
    let magenta = PaintColor::new(1.0, 0.0, 1.0, 1.0);
    let yellow = PaintColor::new(1.0, 1.0, 0.0, 1.0);
    styles.0.insert(1, absolute(10.0, red));
    styles.0.insert(2, absolute(500.0, blue));
    styles.0.insert(
        3,
        absolute(500.0, green).with_2d_transform(LayoutTransform2D::translation(0.0, -480.0)),
    );
    styles.0.insert(
        4,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(60.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(30.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(30.0),
                    height: length(20.0),
                },
                ..Style::default()
            },
            magenta,
        )
        .with_position(LayoutPosition::Fixed),
    );
    styles.0.insert(
        5,
        absolute(700.0, PaintColor::TRANSPARENT).with_opacity(0.5),
    );
    styles.0.insert(
        6,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(30.0),
                    height: length(20.0),
                },
                ..Style::default()
            },
            yellow,
        ),
    );

    let viewport = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(120, 100, 1.0), LayoutFlushReason::Test),
    );
    let colors = viewport
        .paint_snapshot()
        .expect("viewport paint")
        .fragments
        .iter()
        .filter_map(PaintFragment::solid_fill)
        .map(|(_, color, _)| color)
        .collect::<Vec<_>>();
    assert!(colors.contains(&red));
    assert!(
        colors.contains(&green),
        "the CSS transform moves this box into view"
    );
    assert!(
        colors.contains(&magenta),
        "fixed content is tested in viewport space"
    );
    assert!(!colors.contains(&blue));
    assert!(!colors.contains(&yellow));
    assert!(viewport.metrics.paint_event_count > 0);
    assert!(viewport.metrics.paint_culled_event_count > 0);
    assert!(
        viewport.metrics.paint_culled_event_count < viewport.metrics.paint_event_count,
        "visible paint must remain after culling"
    );
    assert_balanced_paint_stack(&viewport.paint_snapshot().expect("viewport paint").fragments);

    let full_document = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_capture(
            LayoutViewport::new(120, 100, 1.0),
            LayoutFlushReason::Test,
            PaintCaptureRequest::full_document(),
        ),
    );
    let full_colors = full_document
        .paint_snapshot()
        .expect("full-document paint")
        .fragments
        .iter()
        .filter_map(PaintFragment::solid_fill)
        .map(|(_, color, _)| color)
        .collect::<Vec<_>>();
    assert!(full_colors.contains(&blue));
    assert!(full_colors.contains(&yellow));
    assert!(
        full_document.metrics.paint_culled_event_count < viewport.metrics.paint_culled_event_count,
        "the larger capture should discard fewer paint events"
    );

    let deep_clip = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_capture(
            LayoutViewport::new(120, 100, 1.0),
            LayoutFlushReason::Test,
            PaintCaptureRequest::page_clip(LayoutRect::new(0.0, 490.0, 120.0, 40.0), 1.0),
        ),
    );
    let clip_colors = deep_clip
        .paint_snapshot()
        .expect("page clip paint")
        .fragments
        .iter()
        .filter_map(PaintFragment::solid_fill)
        .map(|(_, color, _)| color)
        .collect::<Vec<_>>();
    assert!(clip_colors.contains(&blue));
    assert!(!clip_colors.contains(&red));
    assert!(!clip_colors.contains(&green));
}

#[test]
fn viewport_paint_skips_final_inline_lines_outside_the_capture() {
    const LONG_TEXT: &str = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::text("long-text", LONG_TEXT),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(80.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
        ),
    );

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(100, 50, 1.0), LayoutFlushReason::Test),
    );
    assert!(output.metrics.paint_text_line_count > 10);
    assert!(output.metrics.paint_culled_text_line_count > 0);
    assert!(
        output.metrics.paint_culled_text_line_count < output.metrics.paint_text_line_count,
        "the first viewport lines must still paint"
    );
    assert!(output.paint_snapshot().is_some());
}

#[test]
fn auto_scrollbar_feedback_reveals_the_perpendicular_axis() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::element("content", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    // Synthetic Taffy `Scroll` maps to CSS `auto` at this adapter boundary.
    let scroller = Style {
        size: Size {
            width: length(200.0),
            height: length(100.0),
        },
        overflow: Point {
            x: Overflow::Scroll,
            y: Overflow::Scroll,
        },
        ..Style::default()
    };
    styles.0.insert(1, resolved(LayoutDisplay::Block, scroller));
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 200.0, 200.0));

    let output = build(&source, &mut styles);
    assert_eq!(output.metrics.numeric_layout_pass_count, 3);
    assert_eq!(
        output.metrics.numeric_feedback_invalidated_node_count, 4,
        "each newly revealed axis invalidates only the scroller-to-root path"
    );
    let metrics = output.element_metrics_for_source(1).unwrap();
    assert_eq!(
        metrics.client_size,
        moli_layout::LayoutSize::new(185.0, 85.0)
    );
    assert_eq!(
        metrics.scroll_size,
        moli_layout::LayoutSize::new(200.0, 200.0)
    );
    let box_id = output.source_output(1).unwrap().principal_box.unwrap();
    let extent = output.scroll_extent(box_id).unwrap();
    assert!(extent.horizontal_scrollbar.is_some());
    assert!(extent.vertical_scrollbar.is_some());
}

#[test]
fn scrollbar_feedback_rebreaks_the_reused_inline_layout_at_its_final_width() {
    const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::text("text", TEXT),
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
                    height: length(40.0),
                },
                overflow: Point {
                    x: Overflow::Scroll,
                    y: Overflow::Scroll,
                },
                ..Style::default()
            },
        ),
    );

    let feedback = build(&source, &mut styles);
    assert_eq!(feedback.metrics.numeric_layout_pass_count, 2);
    assert_eq!(
        feedback.element_metrics_for_source(1).unwrap().client_size,
        moli_layout::LayoutSize::new(85.0, 25.0),
    );
    let feedback_text = feedback.client_rects_for_source(2);

    // Lay out the same paragraph directly at the converged 85px content
    // width. Its line fragments must match the scrollbar-corrected result;
    // otherwise paint retained the first pass's 100px Parley lines.
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(85.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
        ),
    );
    let direct = build(&source, &mut styles);
    assert_eq!(feedback_text, direct.client_rects_for_source(2));
}

#[test]
fn scrollbar_feedback_prepares_static_position_placeholders_once() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2, 3]),
        Node::element("content", Vec::new()),
        Node::element("absolute", Vec::new()),
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
                    width: length(200.0),
                    height: length(100.0),
                },
                overflow: Point {
                    x: Overflow::Scroll,
                    y: Overflow::Scroll,
                },
                ..Style::default()
            },
        ),
    );
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 200.0, 200.0));
    styles.0.insert(
        3,
        resolved(
            LayoutDisplay::Block,
            Style {
                position: Position::Absolute,
                size: Size {
                    width: length(20.0),
                    height: length(20.0),
                },
                ..Style::default()
            },
        ),
    );

    let output = build(&source, &mut styles);
    assert_eq!(output.metrics.numeric_layout_pass_count, 3);
    assert_eq!(
        output.metrics.box_count, 5,
        "four source boxes plus one stable static-position placeholder"
    );
    assert!(output.source_output(3).is_some());
}

#[test]
fn nested_auto_scrollbar_feedback_reprojects_each_affected_path_linearly() {
    for depth in [8usize, 64] {
        let leaf = depth + 1;
        let mut nodes = Vec::with_capacity(leaf + 1);
        nodes.push(Node::element("root", vec![1]));
        for index in 1..=depth {
            nodes.push(Node::element(
                "nested-scroller",
                vec![if index == depth { leaf } else { index + 1 }],
            ));
        }
        nodes.push(Node::element("oversized-leaf", Vec::new()));
        let source = Source(nodes);
        let mut styles = Styles::default();
        styles
            .0
            .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
        for index in 1..=depth {
            styles.0.insert(
                index,
                resolved(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: length(200.0),
                            height: length(100.0),
                        },
                        overflow: Point {
                            x: Overflow::Scroll,
                            y: Overflow::Scroll,
                        },
                        ..Style::default()
                    },
                ),
            );
        }
        styles
            .0
            .insert(leaf, fixed_size(LayoutDisplay::Block, 200.0, 200.0));

        let output = build(&source, &mut styles);
        assert_eq!(output.metrics.numeric_layout_pass_count, 3);
        assert_eq!(output.metrics.box_count, depth + 2);
        assert_eq!(
            output
                .metrics
                .numeric_feedback_overflow_recomputed_node_count,
            2 * (depth + 2),
            "each perpendicular-axis turn should visit the one affected chain once"
        );
    }
}

#[test]
fn scrollbar_feedback_worklist_skips_unaffected_wide_branches() {
    const UNRELATED_FIXED_BOXES: usize = 128;
    let mut root_children = vec![1];
    root_children.extend(3..3 + UNRELATED_FIXED_BOXES);
    let mut nodes = vec![
        Node::element("root", root_children),
        Node::element("scroller", vec![2]),
        Node::element("oversized-content", Vec::new()),
    ];
    nodes.extend((0..UNRELATED_FIXED_BOXES).map(|_| Node::element("unrelated-fixed", Vec::new())));
    let source = Source(nodes);
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
                    width: length(200.0),
                    height: length(100.0),
                },
                overflow: Point {
                    x: Overflow::Scroll,
                    y: Overflow::Scroll,
                },
                ..Style::default()
            },
        ),
    );
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 200.0, 200.0));
    for index in 3..3 + UNRELATED_FIXED_BOXES {
        styles.0.insert(
            index,
            resolved(
                LayoutDisplay::Block,
                Style {
                    position: Position::Absolute,
                    size: Size {
                        width: length(10.0),
                        height: length(10.0),
                    },
                    ..Style::default()
                },
            )
            .with_position(LayoutPosition::Fixed),
        );
    }

    let output = build(&source, &mut styles);
    assert_eq!(output.metrics.numeric_layout_pass_count, 3);
    assert!(output.metrics.box_count > UNRELATED_FIXED_BOXES);
    assert!(
        output
            .metrics
            .numeric_feedback_overflow_recomputed_node_count
            < 16,
        "unrelated viewport-fixed branches must stay out of the overflow worklist: {:?}",
        output
            .metrics
            .numeric_feedback_overflow_recomputed_node_count
    );
}

#[test]
fn thin_and_hidden_scrollbars_match_chromium_client_geometry() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::element("content", Vec::new()),
    ]);
    for (width, expected_client, has_controls) in [
        (LayoutScrollbarWidth::Auto, (185.0, 85.0), true),
        (LayoutScrollbarWidth::Thin, (190.0, 90.0), true),
        (LayoutScrollbarWidth::None, (200.0, 100.0), false),
    ] {
        let mut styles = Styles::default();
        styles
            .0
            .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
        let style = Style {
            size: Size {
                width: length(200.0),
                height: length(100.0),
            },
            overflow: Point {
                x: Overflow::Scroll,
                y: Overflow::Scroll,
            },
            ..Style::default()
        };
        let mut style = resolved(LayoutDisplay::Block, style);
        style.set_scrollbar_style(width, moli_layout::LayoutScrollbarGutter::Auto, None);
        styles.0.insert(1, style);
        styles
            .0
            .insert(2, fixed_size(LayoutDisplay::Block, 400.0, 300.0));

        let output = build(&source, &mut styles);
        let metrics = output.element_metrics_for_source(1).unwrap();
        assert_eq!(
            metrics.client_size,
            moli_layout::LayoutSize::new(expected_client.0, expected_client.1)
        );
        let box_id = output.source_output(1).unwrap().principal_box.unwrap();
        let extent = output.scroll_extent(box_id).unwrap();
        assert_eq!(extent.horizontal_scrollbar.is_some(), has_controls);
        assert_eq!(extent.vertical_scrollbar.is_some(), has_controls);
    }
}

#[test]
fn stable_both_edges_reserves_and_offsets_both_chromium_gutters() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("scroller", vec![2]),
        Node::element("auto-width-child", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    let scroller = Style {
        size: Size {
            width: length(200.0),
            height: length(100.0),
        },
        overflow: Point {
            x: Overflow::Visible,
            y: Overflow::Scroll,
        },
        ..Style::default()
    };
    let mut scroller = resolved(LayoutDisplay::Block, scroller);
    scroller.set_scrollbar_style(
        LayoutScrollbarWidth::Auto,
        LayoutScrollbarGutter::StableBothEdges,
        None,
    );
    styles.0.insert(1, scroller);
    let mut child = Style::default();
    child.size.height = length(200.0);
    styles.0.insert(2, resolved(LayoutDisplay::Block, child));

    let output = build(&source, &mut styles);
    let metrics = output.element_metrics_for_source(1).unwrap();
    assert_eq!(metrics.client_size.width, 170.0);
    assert_eq!(metrics.client_border.x, 15.0);
    let child_id = output.source_output(2).unwrap().principal_box.unwrap();
    let child_geometry = output.box_geometry(child_id).unwrap();
    assert_eq!(child_geometry.border_box.width, 170.0);
    assert_eq!(child_geometry.layout_origin_in_document.x, 15.0);

    let scroller_id = output.source_output(1).unwrap().principal_box.unwrap();
    let vertical = output
        .scroll_extent(scroller_id)
        .unwrap()
        .vertical_scrollbar
        .expect("overflowing content keeps the end-edge control");
    assert_rect(vertical.frame, LayoutRect::new(185.0, 0.0, 15.0, 100.0));
}

#[test]
fn stable_both_edges_reserves_horizontal_space_during_numeric_layout() {
    for display in [
        LayoutDisplay::Block,
        LayoutDisplay::Flex,
        LayoutDisplay::Grid,
    ] {
        let source = Source(vec![
            Node::element("root", vec![1]),
            Node::element("scroller", vec![2]),
            Node::element("percentage-child", Vec::new()),
        ]);
        let mut styles = Styles::default();
        styles
            .0
            .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
        let mut scroller = resolved(
            display,
            Style {
                display: match display {
                    LayoutDisplay::Flex => taffy::Display::Flex,
                    LayoutDisplay::Grid => taffy::Display::Grid,
                    LayoutDisplay::Block => taffy::Display::Block,
                    _ => unreachable!(),
                },
                size: Size {
                    width: length(200.0),
                    height: length(100.0),
                },
                overflow: Point {
                    x: Overflow::Scroll,
                    y: Overflow::Scroll,
                },
                ..Style::default()
            },
        );
        scroller.set_scrollbar_style(
            LayoutScrollbarWidth::Auto,
            LayoutScrollbarGutter::StableBothEdges,
            None,
        );
        styles.0.insert(1, scroller);
        styles.0.insert(
            2,
            resolved(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: length(400.0),
                        height: percent(1.0),
                    },
                    min_size: Size {
                        width: length(400.0),
                        height: Dimension::auto(),
                    },
                    ..Style::default()
                },
            ),
        );

        let output = build(&source, &mut styles);
        let scroller_metrics = output.element_metrics_for_source(1).unwrap();
        assert_eq!(
            scroller_metrics.client_size,
            moli_layout::LayoutSize::new(170.0, 85.0),
            "{display:?} scrollport"
        );
        let child_id = output.source_output(2).unwrap().principal_box.unwrap();
        let child = output.box_geometry(child_id).unwrap();
        assert_eq!(child.border_box.height, 85.0, "{display:?} child height");
        assert_eq!(child.layout_origin_in_document.x, 15.0);
    }
}

#[test]
fn html_body_overflow_is_propagated_to_the_viewport_only() {
    for (body_overflow, user_scrolls, has_scrollbar) in [
        (Overflow::Hidden, false, false),
        // Synthetic Taffy `Scroll` maps to CSS `auto` at this adapter seam.
        (Overflow::Scroll, true, true),
        // Viewport `clip` maps to hidden: script scrolling remains possible,
        // while direct user scrolling and scrollbar UI stay disabled.
        (Overflow::Clip, false, false),
    ] {
        let source = Source(vec![
            Node::html_element("html", "html", vec![1]),
            Node::html_element("body", "body", vec![2]),
            Node::element("tall", Vec::new()),
        ]);
        let mut styles = Styles::default();
        styles.0.insert(
            0,
            resolved(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::auto(),
                        height: length(100.0),
                    },
                    overflow: Point {
                        x: Overflow::Visible,
                        y: Overflow::Visible,
                    },
                    ..Style::default()
                },
            ),
        );
        styles.0.insert(
            1,
            resolved(
                LayoutDisplay::Block,
                Style {
                    overflow: Point {
                        x: body_overflow,
                        y: body_overflow,
                    },
                    ..Style::default()
                },
            ),
        );
        styles
            .0
            .insert(2, fixed_size(LayoutDisplay::Block, 320.0, 480.0));

        let output = build(&source, &mut styles);
        let root_id = output.source_output(0).unwrap().principal_box.unwrap();
        let root_extent = output.scroll_extent(root_id).unwrap();
        assert!(root_extent.is_scroll_container);
        assert_eq!(root_extent.allows_user_scroll_y, user_scrolls);
        assert_eq!(root_extent.vertical_scrollbar.is_some(), has_scrollbar);
        assert!(root_extent.maximum_offset.y > 0.0);

        let body_id = output.source_output(1).unwrap().principal_box.unwrap();
        let body_extent = output.scroll_extent(body_id).unwrap();
        assert!(!body_extent.is_scroll_container);
        assert!(!body_extent.clips_overflow);
    }
}

#[test]
fn display_contents_body_cannot_define_viewport_overflow() {
    // Mirrors css/css-overflow/overflow-body-propagation-003.html: a body
    // without a principal box cannot transfer its overflow to the viewport.
    for (display, expected_user_scroll, expected_scrollbar) in [
        (LayoutDisplay::Contents, true, true),
        (LayoutDisplay::Block, false, false),
    ] {
        let source = Source(vec![
            Node::html_element("html", "html", vec![1]),
            Node::html_element("body", "body", vec![2]),
            Node::element("tall", Vec::new()),
        ]);
        let mut styles = Styles::default();
        styles
            .0
            .insert(0, resolved(LayoutDisplay::Block, Style::default()));
        styles.0.insert(
            1,
            resolved(
                display,
                Style {
                    overflow: Point {
                        x: Overflow::Clip,
                        y: Overflow::Clip,
                    },
                    ..Style::default()
                },
            ),
        );
        styles
            .0
            .insert(2, fixed_size(LayoutDisplay::Block, 320.0, 480.0));

        let output = build(&source, &mut styles);
        let root_id = output.source_output(0).unwrap().principal_box.unwrap();
        let root_extent = output.scroll_extent(root_id).unwrap();
        assert_eq!(
            root_extent.allows_user_scroll_y, expected_user_scroll,
            "{display:?} body viewport user-scroll policy"
        );
        assert_eq!(
            root_extent.vertical_scrollbar.is_some(),
            expected_scrollbar,
            "{display:?} body viewport scrollbar policy"
        );
        if display == LayoutDisplay::Block {
            let body_id = output.source_output(1).unwrap().principal_box.unwrap();
            let body_extent = output.scroll_extent(body_id).unwrap();
            assert!(!body_extent.is_scroll_container);
            assert!(!body_extent.clips_overflow);
        }
    }
}

#[test]
fn default_visible_root_reserves_stable_viewport_gutters() {
    for (gutter, expected_x, expected_width) in [
        (LayoutScrollbarGutter::Stable, 0.0, 305.0),
        (LayoutScrollbarGutter::StableBothEdges, 15.0, 290.0),
    ] {
        let source = Source(vec![Node::html_element("html", "html", Vec::new())]);
        let mut styles = Styles::default();
        let mut root = resolved(LayoutDisplay::Block, Style::default());
        root.set_scrollbar_style(LayoutScrollbarWidth::Auto, gutter, None);
        styles.0.insert(0, root);

        let output = build(&source, &mut styles);
        let metrics = output.element_metrics_for_source(0).unwrap();
        assert_eq!(metrics.client_size.width, 320.0);
        let root_id = output.source_output(0).unwrap().principal_box.unwrap();
        let geometry = output.box_geometry(root_id).unwrap();
        assert_eq!(geometry.layout_origin_in_document.x, expected_x);
        assert_eq!(geometry.border_box.width, expected_width);
    }
}

#[test]
fn root_stable_gutters_are_reserved_by_the_initial_containing_block_once() {
    for (content_height, expected_client_width) in [(20.0, 320.0), (480.0, 305.0)] {
        let source = Source(vec![
            Node::element("root", vec![1]),
            Node::element("auto-width-child", Vec::new()),
        ]);
        let mut styles = Styles::default();
        let mut root = Style {
            overflow: Point {
                x: Overflow::Visible,
                // Synthetic Taffy `Scroll` maps to CSS `auto` at this adapter boundary.
                y: Overflow::Scroll,
            },
            ..Style::default()
        };
        root.size.height = length(240.0);
        let mut root = resolved(LayoutDisplay::Block, root);
        root.set_scrollbar_style(
            LayoutScrollbarWidth::Auto,
            LayoutScrollbarGutter::StableBothEdges,
            None,
        );
        styles.0.insert(0, root);
        let mut child = Style::default();
        child.size.height = length(content_height);
        styles.0.insert(1, resolved(LayoutDisplay::Block, child));

        let output = build(&source, &mut styles);
        let root_metrics = output.element_metrics_for_source(0).unwrap();
        assert_eq!(root_metrics.client_size.width, expected_client_width);
        assert_eq!(root_metrics.scroll_size.width, 290.0);
        let root_id = output.source_output(0).unwrap().principal_box.unwrap();
        let root_geometry = output.box_geometry(root_id).unwrap();
        assert_rect(
            root_geometry.border_box,
            LayoutRect::new(0.0, 0.0, 290.0, 240.0),
        );
        assert_eq!(root_geometry.layout_origin_in_document.x, 15.0);
        let child_id = output.source_output(1).unwrap().principal_box.unwrap();
        let child_geometry = output.box_geometry(child_id).unwrap();
        assert_eq!(child_geometry.border_box.width, 290.0);
        assert_eq!(child_geometry.layout_origin_in_document.x, 15.0);
    }
}

#[test]
fn scrollbar_hit_test_honors_the_same_ancestor_clip_as_paint() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("clipper", vec![2]),
        Node::element("scroller", vec![3]),
        Node::element("oversized-content", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    let clipper = Style {
        size: Size {
            width: length(50.0),
            height: length(50.0),
        },
        overflow: Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        },
        ..Style::default()
    };
    styles.0.insert(1, resolved(LayoutDisplay::Block, clipper));
    let scroller = Style {
        size: Size {
            width: length(200.0),
            height: length(100.0),
        },
        overflow: Point {
            x: Overflow::Scroll,
            y: Overflow::Scroll,
        },
        ..Style::default()
    };
    styles.0.insert(2, resolved(LayoutDisplay::Block, scroller));
    styles
        .0
        .insert(3, fixed_size(LayoutDisplay::Block, 400.0, 300.0));

    let output = build(&source, &mut styles);
    let scroller_id = output.source_output(2).unwrap().principal_box.unwrap();
    assert!(
        output
            .scroll_extent(scroller_id)
            .unwrap()
            .vertical_scrollbar
            .is_some()
    );
    assert_eq!(
        output.scrollbar_hit_test(LayoutPoint::new(190.0, 30.0)),
        None,
        "a control clipped out of paint must not intercept input"
    );
}

#[test]
fn adjacent_paint_units_share_their_common_overflow_clip_chain() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("clipper", vec![2, 3]),
        Node::element("first", Vec::new()),
        Node::element("second", Vec::new()),
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
                    width: length(200.0),
                    height: length(100.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        ),
    );
    for (node, color) in [
        (2, PaintColor::new(1.0, 0.0, 0.0, 1.0)),
        (3, PaintColor::new(0.0, 0.0, 1.0, 1.0)),
    ] {
        styles.0.insert(
            node,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: length(200.0),
                        height: length(40.0),
                    },
                    ..Style::default()
                },
                color,
            ),
        );
    }

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    let snapshot = output.paint_snapshot().expect("paint snapshot");
    let push_clip_count = snapshot
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment, PaintFragment::PushClip { .. }))
        .count();
    let pop_count = snapshot
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment, PaintFragment::PopLayer))
        .count();

    assert_eq!(push_clip_count, 2);
    assert_eq!(pop_count, 2);
    assert_eq!(
        snapshot
            .fragments
            .iter()
            .filter_map(PaintFragment::solid_fill)
            .map(|(_, color, _)| color)
            .collect::<Vec<_>>(),
        vec![
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
            PaintColor::new(0.0, 0.0, 1.0, 1.0),
        ]
    );
}

#[test]
fn equal_sibling_clips_replace_only_the_divergent_stack_suffix() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("outer-clip", vec![2, 4]),
        Node::element("first-clip", vec![3]),
        Node::element("red-content", Vec::new()),
        Node::element("second-clip", vec![5]),
        Node::element("blue-content", Vec::new()),
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
                    width: length(200.0),
                    height: length(120.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        )
        .with_position(LayoutPosition::Relative),
    );
    let overlapping_clip = || {
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
                    height: length(60.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        )
        .with_position(LayoutPosition::Absolute)
    };
    styles.0.insert(2, overlapping_clip());
    styles.0.insert(4, overlapping_clip());
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    for (node, color) in [(3, red), (5, blue)] {
        styles.0.insert(
            node,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: length(140.0),
                        height: length(90.0),
                    },
                    ..Style::default()
                },
                color,
            ),
        );
    }

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    let snapshot = output.paint_snapshot().expect("paint snapshot");
    assert_balanced_paint_stack(&snapshot.fragments);
    let red = solid_fill_index(&snapshot.fragments, red);
    let blue = solid_fill_index(&snapshot.fragments, blue);
    assert!(red < blue, "DOM-order siblings must retain paint order");

    let mut clip_depth = snapshot.fragments[..=red]
        .iter()
        .fold(0usize, |depth, fragment| match fragment {
            PaintFragment::PushClip { .. } => depth + 1,
            PaintFragment::PopLayer => depth - 1,
            _ => depth,
        });
    assert_eq!(clip_depth, 2, "red uses the outer and first sibling clips");
    let mut minimum_depth = clip_depth;
    let mut replacement_pushes = 0usize;
    for fragment in &snapshot.fragments[red + 1..blue] {
        match fragment {
            PaintFragment::PushClip { .. } => {
                clip_depth += 1;
                replacement_pushes += 1;
            }
            PaintFragment::PopLayer => {
                clip_depth -= 1;
                minimum_depth = minimum_depth.min(clip_depth);
            }
            _ => {}
        }
    }
    assert_eq!(minimum_depth, 1, "the common outer clip must stay active");
    assert_eq!(replacement_pushes, 1, "only the sibling clip is replaced");
    assert_eq!(
        clip_depth, 2,
        "blue uses the outer and second sibling clips"
    );
}

#[test]
fn stacking_context_effects_bracket_the_active_overflow_clip_stack() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("clipper", vec![2, 3, 4, 5]),
        Node::element("negative-blue", Vec::new()),
        Node::element("red", Vec::new()),
        Node::element("green", Vec::new()),
        Node::element("positive-yellow", Vec::new()),
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
                    width: length(200.0),
                    height: length(120.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        )
        .with_position(LayoutPosition::Relative),
    );
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    let green = PaintColor::new(0.0, 1.0, 0.0, 1.0);
    let yellow = PaintColor::new(1.0, 1.0, 0.0, 1.0);
    for (node, color) in [(2, blue), (3, red), (4, green), (5, yellow)] {
        let style = ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(200.0),
                    height: length(40.0),
                },
                ..Style::default()
            },
            color,
        );
        let style = match node {
            2 => style
                .with_position(LayoutPosition::Absolute)
                .with_z_index(-1)
                .with_opacity(0.4),
            5 => style
                .with_position(LayoutPosition::Absolute)
                .with_z_index(1)
                .with_opacity(0.6),
            _ => style,
        };
        styles.0.insert(node, style);
    }

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    let snapshot = output.paint_snapshot().expect("paint snapshot");
    assert_balanced_paint_stack(&snapshot.fragments);
    let red = solid_fill_index(&snapshot.fragments, red);
    let blue = solid_fill_index(&snapshot.fragments, blue);
    let green = solid_fill_index(&snapshot.fragments, green);
    let yellow = solid_fill_index(&snapshot.fragments, yellow);
    let negative_effect_push = snapshot
        .fragments
        .iter()
        .position(|fragment| {
            matches!(
                fragment,
                PaintFragment::PushLayer { opacity, .. }
                    if (*opacity - 0.4).abs() < f32::EPSILON
            )
        })
        .expect("negative opacity stacking-context layer");
    let negative_effect_pop = matching_paint_stack_pop(&snapshot.fragments, negative_effect_push);
    let positive_effect_push = snapshot
        .fragments
        .iter()
        .position(|fragment| {
            matches!(
                fragment,
                PaintFragment::PushLayer { opacity, .. }
                    if (*opacity - 0.6).abs() < f32::EPSILON
            )
        })
        .expect("positive opacity stacking-context layer");
    let positive_effect_pop = matching_paint_stack_pop(&snapshot.fragments, positive_effect_push);

    assert!(
        negative_effect_push < blue
            && blue < negative_effect_pop
            && negative_effect_pop < red
            && red < green
            && green < positive_effect_push
            && positive_effect_push < yellow
            && yellow < positive_effect_pop,
        "negative, in-flow, and positive stacking levels must retain CSS paint order"
    );
    assert!(
        snapshot.fragments[negative_effect_push + 1..blue]
            .iter()
            .any(|fragment| matches!(fragment, PaintFragment::PushClip { .. })),
        "the overflow clip must open inside the negative effect layer"
    );
    assert!(
        snapshot.fragments[negative_effect_pop + 1..red]
            .iter()
            .any(|fragment| matches!(fragment, PaintFragment::PushClip { .. })),
        "the overflow clip must reopen after the negative effect layer"
    );
    assert!(
        snapshot.fragments[green + 1..positive_effect_push]
            .iter()
            .any(|fragment| matches!(fragment, PaintFragment::PopLayer)),
        "the overflow clip must close before the positive effect layer"
    );
    assert!(
        snapshot.fragments[positive_effect_push + 1..yellow]
            .iter()
            .any(|fragment| matches!(fragment, PaintFragment::PushClip { .. })),
        "the overflow clip must reopen inside the positive effect layer"
    );
}

#[test]
fn equal_local_clips_in_distinct_transform_spaces_are_not_reused() {
    let source = Source(vec![
        Node::element("root", vec![1, 3]),
        Node::element("first-transformed-clip", vec![2]),
        Node::element("red", Vec::new()),
        Node::element("second-transformed-clip", vec![4]),
        Node::element("blue", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 320.0, 240.0).with_position(LayoutPosition::Relative),
    );
    let transformed_clip = |transform| {
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
                    height: length(60.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
        )
        .with_position(LayoutPosition::Absolute)
        .with_2d_transform(transform)
    };
    styles.0.insert(
        1,
        transformed_clip(LayoutTransform2D::translation(10.0, 5.0)),
    );
    styles.0.insert(
        3,
        transformed_clip(LayoutTransform2D::translation(70.0, 15.0)),
    );
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    for (node, color) in [(2, red), (4, blue)] {
        styles.0.insert(
            node,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: length(140.0),
                        height: length(90.0),
                    },
                    ..Style::default()
                },
                color,
            ),
        );
    }

    let output = build_with_request(
        &source,
        &mut styles,
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
    );
    let snapshot = output.paint_snapshot().expect("paint snapshot");
    assert_balanced_paint_stack(&snapshot.fragments);
    let red_clip = innermost_active_clip_at(
        &snapshot.fragments,
        solid_fill_index(&snapshot.fragments, red),
    );
    let blue_clip = innermost_active_clip_at(
        &snapshot.fragments,
        solid_fill_index(&snapshot.fragments, blue),
    );

    assert_eq!(
        red_clip.0, blue_clip.0,
        "the fixture deliberately gives both clips identical local geometry"
    );
    assert_ne!(
        red_clip.1, blue_clip.1,
        "equal local shapes in different coordinate spaces need distinct clip commands"
    );
    assert_ne!(
        red_clip.1.map_rect(red_clip.0.bounds()).bounding_rect(),
        blue_clip.1.map_rect(blue_clip.0.bounds()).bounding_rect(),
        "the two clips must land at their own transformed surface positions"
    );
}

#[test]
fn non_invertible_descendant_does_not_hide_a_valid_root_scrollbar_hit() {
    let source = Source(vec![
        Node::element("root", vec![1, 3]),
        Node::element("singular-scroller", vec![2]),
        Node::element("nested-content", Vec::new()),
        Node::element("root-overflow", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 320.0, 240.0));
    let scroller = Style {
        size: Size {
            width: length(200.0),
            height: length(100.0),
        },
        overflow: Point {
            x: Overflow::Scroll,
            y: Overflow::Scroll,
        },
        ..Style::default()
    };
    styles.0.insert(
        1,
        resolved(LayoutDisplay::Block, scroller)
            .with_2d_transform(LayoutTransform2D::scale(0.0, 0.0)),
    );
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 400.0, 300.0));
    styles
        .0
        .insert(3, fixed_size(LayoutDisplay::Block, 500.0, 500.0));

    let output = build(&source, &mut styles);
    let hit = output
        .scrollbar_hit_test(LayoutPoint::new(310.0, 100.0))
        .expect("root scrollbar remains hittable past a singular candidate");
    assert_eq!(hit.source, 0);
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
fn frozen_used_size_keeps_its_sizing_basis_after_style_inputs_are_discarded() {
    for box_sizing in [BoxSizing::ContentBox, BoxSizing::BorderBox] {
        let source = Source(vec![
            Node::element("root", vec![1]),
            Node::element("sized", Vec::new()),
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
                    box_sizing,
                    size: Size {
                        width: length(100.25),
                        height: length(50.5),
                    },
                    padding: Rect {
                        left: length(10.0),
                        right: length(10.0),
                        top: length(10.0),
                        bottom: length(10.0),
                    },
                    border: Rect {
                        left: length(5.0),
                        right: length(5.0),
                        top: length(5.0),
                        bottom: length(5.0),
                    },
                    ..Style::default()
                },
            ),
        );
        let output = build(&source, &mut styles);
        drop(styles);
        drop(source);
        assert_eq!(
            output.used_size_for_source(1),
            Some(moli_layout::LayoutUsedSize {
                width: 100.25,
                height: 50.5,
                inline_size: 100.25,
                block_size: 50.5
            })
        );
        let retention = output.retention_metrics();
        let used_sizes = output.used_sizes().collect::<HashMap<_, _>>();
        assert_eq!(used_sizes.len(), 2);
        assert_eq!(used_sizes.get(&1).copied(), output.used_size_for_source(1));
        assert_eq!(
            output.retention_metrics(),
            retention,
            "the batch lookup must stay outside the frozen tree"
        );
        let model = output.box_model_for_source(1).expect("sized box");
        let expected_border_width = if box_sizing == BoxSizing::ContentBox {
            130.25
        } else {
            100.25
        };
        assert_close(model.border.bounding_rect().width, expected_border_width);
        assert!(output.paint_snapshot().is_none());
    }
}

#[test]
fn frozen_used_size_excludes_non_replaced_inlines_and_suppressed_boxes() {
    let source = Source(vec![
        Node::element("root", vec![1, 2, 3]),
        Node::element("inline", Vec::new()),
        Node::element("none", Vec::new()),
        Node::element("contents", vec![4]),
        Node::element("child", Vec::new()),
    ]);
    let mut styles = Styles::default();
    for (index, display) in [
        LayoutDisplay::Block,
        LayoutDisplay::Inline,
        LayoutDisplay::None,
        LayoutDisplay::Contents,
        LayoutDisplay::Block,
    ]
    .into_iter()
    .enumerate()
    {
        styles.0.insert(index, fixed_size(display, 60.0, 25.0));
    }
    let output = build(&source, &mut styles);
    let used_sizes = output.used_sizes().collect::<HashMap<_, _>>();
    assert_eq!(
        used_sizes.len(),
        2,
        "only the root and the displayed child have sizing boxes"
    );
    for source in [1, 2, 3, usize::MAX] {
        assert!(!used_sizes.contains_key(&source));
        assert_eq!(
            output.used_size_for_source(source),
            None,
            "source {source} must not have a CSS sizing box"
        );
    }
    assert_eq!(
        output.used_size_for_source(4),
        Some(moli_layout::LayoutUsedSize {
            width: 60.0,
            height: 25.0,
            inline_size: 60.0,
            block_size: 25.0
        })
    );
    assert_eq!(used_sizes.get(&4).copied(), output.used_size_for_source(4));
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
        LayoutRect::new(0.0, 0.0, 305.0, 225.0),
    );
    assert_eq!(
        root_metrics.maximum_scroll_offset,
        LayoutPoint::new(245.0, 225.0)
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
fn inline_offset_metrics_use_fragment_bounds_and_keep_empty_sibling_anchors() {
    let source = Source(vec![
        Node::element("root", vec![1, 3]),
        Node::element("reference", vec![2]),
        Node::text("reference-text", "ref"),
        Node::element("empty-target", vec![4]),
        Node::text("collapsed-trailing-space", " "),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 80.0, 80.0).with_text_metrics(10.0, 10.0),
    );
    for node in 1..=4 {
        styles.0.insert(
            node,
            resolved(LayoutDisplay::Inline, Style::default()).with_text_metrics(10.0, 10.0),
        );
    }

    let output = build(&source, &mut styles);
    let reference = output
        .element_metrics_for_source(1)
        .expect("reference inline metrics");
    let target = output
        .element_metrics_for_source(3)
        .expect("empty target inline metrics");

    assert!(reference.offset_size.width > 0.0);
    assert_close(
        target.offset_position.x,
        reference.offset_position.x + reference.offset_size.width,
    );
    assert_close(target.offset_position.y, reference.offset_position.y);
}

#[test]
fn multiline_inline_offset_size_is_the_nonempty_fragment_union() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("multiline-inline", vec![2]),
        Node::text("wrapped-text", "ref ref ref"),
    ]);
    let mut styles = Styles::default();
    styles.0.insert(
        0,
        fixed_size(LayoutDisplay::Block, 14.0, 100.0).with_text_metrics(10.0, 10.0),
    );
    for node in 1..=2 {
        styles.0.insert(
            node,
            resolved(LayoutDisplay::Inline, Style::default()).with_text_metrics(10.0, 10.0),
        );
    }

    let output = build(&source, &mut styles);
    let fragments = output
        .client_rects_for_source(1)
        .into_iter()
        .map(|quad| quad.bounding_rect())
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
        .collect::<Vec<_>>();
    assert!(
        fragments.len() > 1,
        "the fixture must generate multiple inline fragments: {fragments:?}"
    );
    let union = fragments[1..]
        .iter()
        .copied()
        .fold(fragments[0], LayoutRect::union);
    let metrics = output
        .element_metrics_for_source(1)
        .expect("multiline inline metrics");

    assert_close(metrics.offset_position.x, fragments[0].x);
    assert_close(metrics.offset_position.y, fragments[0].y);
    assert_close(metrics.offset_size.width, union.width);
    assert_close(metrics.offset_size.height, union.height);
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
fn block_in_inline_offset_parent_uses_the_structural_inline_ancestor() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("positioned-inline", vec![2]),
        Node::element("promoted-block", Vec::new()),
    ]);
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, fixed_size(LayoutDisplay::Block, 200.0, 120.0));
    styles.0.insert(
        1,
        resolved(LayoutDisplay::Inline, Style::default()).with_position(LayoutPosition::Relative),
    );
    styles
        .0
        .insert(2, fixed_size(LayoutDisplay::Block, 50.0, 20.0));

    let output = build(&source, &mut styles);
    let metrics = output
        .element_metrics_for_source(2)
        .expect("promoted block metrics");
    assert_eq!(metrics.offset_parent, Some(1));
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
    let first_child_metrics = first
        .element_metrics_for_source(2)
        .expect("the scrolled child has element metrics");
    assert_eq!(
        first_child_metrics.border_origin_in_viewport_ignoring_css_transforms,
        LayoutPoint::new(-40.0, -30.0)
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
            .element_metrics_for_source(2)
            .expect("the transformed box has element metrics")
            .border_origin_in_viewport_ignoring_css_transforms,
        LayoutPoint::new(20.0, 20.0),
        "the untransformed mapping must retain layout placement while skipping CSS transforms"
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
fn transformed_hit_retains_exact_local_content_box_and_inverse_mapping() {
    let source = Source(vec![
        Node::element("root", vec![1]),
        Node::element("embedded-content", Vec::new()),
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
                box_sizing: BoxSizing::ContentBox,
                position: Position::Absolute,
                inset: Rect {
                    left: length(20.0),
                    right: LengthPercentageAuto::auto(),
                    top: length(30.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size {
                    width: length(100.0),
                    height: length(60.0),
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
        )
        .with_2d_transform(LayoutTransform2D::scale(0.5, 0.5)),
    );

    let output = build(&source, &mut styles);
    let viewport_point = LayoutPoint::new(35.0, 45.0);
    let hit = output
        .hit_test(viewport_point, false)
        .expect("scaled embedded content should be hit");
    assert_eq!(hit.source, 1);
    let mapped = hit.viewport_to_local.map_point(viewport_point);
    assert_close(mapped.x, hit.local_point.x);
    assert_close(mapped.y, hit.local_point.y);

    let local_content = hit
        .local_content_box
        .expect("box hit should retain its unprojected content box");
    assert_rect(local_content, LayoutRect::new(7.0, 7.0, 100.0, 60.0));
    let projected_content = hit
        .box_model
        .expect("box hit should retain its projected protocol model")
        .content
        .bounding_rect();
    assert_close(projected_content.width, 50.0);
    assert_close(projected_content.height, 30.0);
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
    assert_eq!(
        output
            .element_metrics_for_source(2)
            .expect("the fixed box has element metrics")
            .border_origin_in_viewport_ignoring_css_transforms,
        LayoutPoint::new(10.0, 15.0),
        "the transform-free viewport mapping must preserve fixed positioning"
    );
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
