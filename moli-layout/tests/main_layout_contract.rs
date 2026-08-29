use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutFlushReason, LayoutNamespace, LayoutPassRequest, LayoutPosition,
    LayoutReplacedKind, LayoutSource, LayoutSourceKind, LayoutStyleResolver, PaintBlendMode,
    PaintCaptureRequest, PaintColor, PaintFragment, PaintRect, PaintSnapshot, PaintViewport,
    ReplacedMetrics, ResolvedLayoutElementStyles, ResolvedLayoutStyle, ScreenshotLayoutRequest,
    build_layout_pass, build_screenshot_snapshot,
};
use style::Atom;
use taffy::{
    AlignContent, BoxSizing, Dimension, Display, FlexDirection, FlexWrap, Line, Rect, Size, Style,
    style_helpers::{fr, length, line, percent},
};

const RED: PaintColor = PaintColor::new(1.0, 0.0, 0.0, 1.0);
const GREEN: PaintColor = PaintColor::new(0.0, 1.0, 0.0, 1.0);
const BLUE: PaintColor = PaintColor::new(0.0, 0.0, 1.0, 1.0);
const YELLOW: PaintColor = PaintColor::new(1.0, 1.0, 0.0, 1.0);

#[derive(Clone, Debug)]
struct FixtureNode {
    label: &'static str,
    semantics: LayoutElementSemantics,
    children: Vec<usize>,
    replaced_metrics: Option<ReplacedMetrics>,
}

impl FixtureNode {
    fn div(label: &'static str, children: Vec<usize>) -> Self {
        Self::html(label, "div", children)
    }

    fn html(label: &'static str, local_name: &'static str, children: Vec<usize>) -> Self {
        Self {
            label,
            semantics: LayoutElementSemantics::new(
                LayoutNamespace::Html,
                local_name,
                LayoutElementCategory::Generic,
                None,
            ),
            children,
            replaced_metrics: None,
        }
    }

    fn svg(label: &'static str, width: f32, height: f32) -> Self {
        Self {
            label,
            semantics: LayoutElementSemantics::new(
                LayoutNamespace::Svg,
                "svg",
                LayoutElementCategory::Generic,
                Some(LayoutReplacedKind::Svg),
            ),
            children: Vec::new(),
            replaced_metrics: Some(ReplacedMetrics {
                intrinsic_width: Some(width),
                intrinsic_height: Some(height),
                intrinsic_ratio: Some(width / height),
                ..ReplacedMetrics::default()
            }),
        }
    }

    fn iframe(label: &'static str) -> Self {
        Self {
            label,
            semantics: LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "iframe",
                LayoutElementCategory::Generic,
                Some(LayoutReplacedKind::Frame),
            ),
            children: Vec::new(),
            replaced_metrics: None,
        }
    }
}

#[derive(Debug)]
struct FixtureSource {
    nodes: Vec<FixtureNode>,
}

impl LayoutSource for FixtureSource {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.nodes
            .iter()
            .position(|candidate| candidate.children.contains(&node))
    }

    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        self.nodes[node].children.iter().copied()
    }

    fn node_kind(&self, _node: Self::NodeId) -> LayoutSourceKind {
        LayoutSourceKind::Element
    }

    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics> {
        Some(self.nodes[node].semantics.clone())
    }

    fn text(&self, _node: Self::NodeId) -> Option<&str> {
        None
    }

    fn label(&self, node: Self::NodeId) -> String {
        self.nodes[node].label.to_owned()
    }

    fn replaced_metrics(&self, node: Self::NodeId) -> Option<ReplacedMetrics> {
        self.nodes[node].replaced_metrics
    }
}

#[derive(Default)]
struct FixtureStyles(HashMap<usize, ResolvedLayoutStyle>);

impl LayoutStyleResolver<usize> for FixtureStyles {
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

    fn anonymous_style(
        &mut self,
        _owner: usize,
        parent: &ResolvedLayoutStyle,
        display: LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        Ok(ResolvedLayoutStyle::anonymous_from(parent, display))
    }
}

fn resolved(display: LayoutDisplay, taffy: Style<Atom>, color: PaintColor) -> ResolvedLayoutStyle {
    ResolvedLayoutStyle::synthetic(display, taffy, color)
}

fn fixed_box(
    display: LayoutDisplay,
    width: f32,
    height: f32,
    color: PaintColor,
) -> ResolvedLayoutStyle {
    resolved(
        display,
        Style {
            size: Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            },
            ..Style::default()
        },
        color,
    )
}

fn render(
    source: &FixtureSource,
    styles: &mut FixtureStyles,
    viewport: PaintViewport,
) -> PaintSnapshot {
    build_screenshot_snapshot(
        source,
        styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(viewport),
    )
    .unwrap()
}

fn solid_rect(snapshot: &PaintSnapshot, color: PaintColor) -> PaintRect {
    snapshot
        .fragments
        .iter()
        .find_map(|fragment| {
            fragment
                .solid_fill_in_surface()
                .filter(|(_, actual)| *actual == color)
                .map(|(rect, _)| rect)
        })
        .unwrap_or_else(|| panic!("missing {color:?} rect in {:?}", snapshot.fragments))
}

fn assert_rect(actual: PaintRect, expected: PaintRect) {
    for (name, actual, expected) in [
        ("x", actual.x, expected.x),
        ("y", actual.y, expected.y),
        ("width", actual.width, expected.width),
        ("height", actual.height, expected.height),
    ] {
        assert!(
            (actual - expected).abs() <= 0.01,
            "{name}: expected {expected}, got {actual}; actual rect={actual:?}"
        );
    }
}

#[test]
fn explicit_and_implicit_grid_tracks_use_real_grid_layout() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("grid", vec![1, 2, 3]),
            FixtureNode::div("first", Vec::new()),
            FixtureNode::div("second", Vec::new()),
            FixtureNode::div("implicit", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Grid,
            Style {
                display: Display::Grid,
                size: Size {
                    width: length(400.0),
                    height: length(200.0),
                },
                gap: Size {
                    width: length(10.0),
                    height: length(20.0),
                },
                grid_template_columns: vec![length(100.0), fr(1.0)],
                grid_template_rows: vec![length(50.0), fr(1.0)],
                grid_auto_rows: vec![length(30.0)],
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                grid_column: line(1),
                grid_row: line(1),
                ..Style::default()
            },
            RED,
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                grid_column: line(2),
                grid_row: Line {
                    start: line(1),
                    end: line(3),
                },
                ..Style::default()
            },
            GREEN,
        ),
    );
    styles.0.insert(
        3,
        resolved(
            LayoutDisplay::Block,
            Style {
                grid_column: line(1),
                grid_row: line(3),
                ..Style::default()
            },
            BLUE,
        ),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    assert!(snapshot.diagnostics.is_empty());
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(0.0, 0.0, 100.0, 50.0),
    );
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(110.0, 0.0, 290.0, 150.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(0.0, 170.0, 100.0, 30.0),
    );
}

#[test]
fn flex_wrap_gap_and_cross_axis_distribution_are_numeric() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("flex", vec![1, 2, 3]),
            FixtureNode::div("first", Vec::new()),
            FixtureNode::div("second", Vec::new()),
            FixtureNode::div("third", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Flex,
            Style {
                display: Display::Flex,
                size: Size {
                    width: length(250.0),
                    height: length(160.0),
                },
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                gap: Size {
                    width: length(10.0),
                    height: length(20.0),
                },
                align_content: Some(AlignContent::SPACE_BETWEEN),
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    for (node, color) in [(1, RED), (2, GREEN), (3, BLUE)] {
        styles
            .0
            .insert(node, fixed_box(LayoutDisplay::Block, 100.0, 40.0, color));
    }

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(0.0, 0.0, 100.0, 40.0),
    );
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(110.0, 0.0, 100.0, 40.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(0.0, 120.0, 100.0, 40.0),
    );
}

#[test]
fn flex_order_changes_layout_and_paint_order_stably() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("flex", vec![1, 2, 3]),
            FixtureNode::div("late", Vec::new()),
            FixtureNode::div("early", Vec::new()),
            FixtureNode::div("middle", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Flex,
            Style {
                display: Display::Flex,
                size: Size {
                    width: length(300.0),
                    height: length(40.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        fixed_box(LayoutDisplay::Block, 50.0, 40.0, RED).with_order(2),
    );
    styles.0.insert(
        2,
        fixed_box(LayoutDisplay::Block, 50.0, 40.0, GREEN).with_order(-1),
    );
    styles.0.insert(
        3,
        fixed_box(LayoutDisplay::Block, 50.0, 40.0, BLUE).with_order(0),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(300, 100, 1.0));
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(0.0, 0.0, 50.0, 40.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(50.0, 0.0, 50.0, 40.0),
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(100.0, 0.0, 50.0, 40.0),
    );
    let paint_order = snapshot
        .fragments
        .iter()
        .filter_map(|fragment| fragment.solid_fill().map(|(_, color, _)| color))
        .collect::<Vec<_>>();
    assert_eq!(paint_order, vec![GREEN, BLUE, RED]);
}

#[test]
fn stacking_context_levels_and_effect_layer_are_atomic() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1, 2, 3, 4]),
            FixtureNode::div("positive", Vec::new()),
            FixtureNode::div("auto", Vec::new()),
            FixtureNode::div("zero", Vec::new()),
            FixtureNode::div("negative", Vec::new()),
        ],
    };
    let positioned = |color| {
        fixed_box(LayoutDisplay::Block, 20.0, 20.0, color).with_position(LayoutPosition::Absolute)
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        fixed_box(LayoutDisplay::Block, 40.0, 40.0, PaintColor::TRANSPARENT)
            .with_position(LayoutPosition::Relative),
    );
    styles.0.insert(1, positioned(YELLOW).with_z_index(2));
    styles.0.insert(2, positioned(RED));
    styles.0.insert(
        3,
        positioned(BLUE)
            .with_z_index(0)
            .with_opacity(0.5)
            .with_blend_mode(PaintBlendMode::Multiply),
    );
    styles.0.insert(4, positioned(GREEN).with_z_index(-1));

    let snapshot = render(&source, &mut styles, PaintViewport::new(40, 40, 1.0));
    let paint_order = snapshot
        .fragments
        .iter()
        .filter_map(|fragment| fragment.solid_fill().map(|(_, color, _)| color))
        .collect::<Vec<_>>();
    assert_eq!(paint_order, vec![GREEN, RED, BLUE, YELLOW]);

    let blue = snapshot
        .fragments
        .iter()
        .position(|fragment| {
            fragment
                .solid_fill()
                .is_some_and(|(_, color, _)| color == BLUE)
        })
        .unwrap();
    assert!(matches!(
        snapshot.fragments.get(blue - 1),
        Some(PaintFragment::PushLayer {
            opacity,
            blend_mode: PaintBlendMode::Multiply,
            ..
        }) if (*opacity - 0.5).abs() < f32::EPSILON
    ));
    assert!(matches!(
        snapshot.fragments.get(blue + 1),
        Some(PaintFragment::PopLayer)
    ));
}

#[test]
fn flex_grow_shrink_and_cross_axis_alignment_are_numeric() {
    let grow_source = FixtureSource {
        nodes: vec![
            FixtureNode::div("grow", vec![1, 2]),
            FixtureNode::div("one", Vec::new()),
            FixtureNode::div("three", Vec::new()),
        ],
    };
    let mut grow_styles = FixtureStyles::default();
    grow_styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Flex,
            Style {
                display: Display::Flex,
                size: Size {
                    width: length(300.0),
                    height: length(100.0),
                },
                align_items: Some(taffy::AlignItems::CENTER),
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    grow_styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(20.0),
                },
                flex_basis: length(100.0),
                flex_grow: 1.0,
                ..Style::default()
            },
            RED,
        ),
    );
    grow_styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(40.0),
                },
                flex_basis: length(100.0),
                flex_grow: 3.0,
                ..Style::default()
            },
            BLUE,
        ),
    );
    let grown = render(
        &grow_source,
        &mut grow_styles,
        PaintViewport::new(300, 100, 1.0),
    );
    assert_rect(
        solid_rect(&grown, RED),
        PaintRect::new(0.0, 40.0, 125.0, 20.0),
    );
    assert_rect(
        solid_rect(&grown, BLUE),
        PaintRect::new(125.0, 30.0, 175.0, 40.0),
    );

    let mut shrink_styles = FixtureStyles::default();
    shrink_styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Flex,
            Style {
                display: Display::Flex,
                size: Size {
                    width: length(140.0),
                    height: length(30.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    for (node, color, shrink) in [(1, RED, 1.0), (2, BLUE, 3.0)] {
        shrink_styles.0.insert(
            node,
            resolved(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::auto(),
                        height: length(20.0),
                    },
                    flex_basis: length(100.0),
                    flex_shrink: shrink,
                    ..Style::default()
                },
                color,
            ),
        );
    }
    let shrunk = render(
        &grow_source,
        &mut shrink_styles,
        PaintViewport::new(300, 100, 1.0),
    );
    assert_rect(
        solid_rect(&shrunk, RED),
        PaintRect::new(0.0, 0.0, 85.0, 20.0),
    );
    assert_rect(
        solid_rect(&shrunk, BLUE),
        PaintRect::new(85.0, 0.0, 55.0, 20.0),
    );
}

#[test]
fn grid_order_controls_auto_placement_order() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("grid", vec![1, 2, 3]),
            FixtureNode::div("late", Vec::new()),
            FixtureNode::div("early", Vec::new()),
            FixtureNode::div("middle", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Grid,
            Style {
                display: Display::Grid,
                size: Size {
                    width: length(300.0),
                    height: length(50.0),
                },
                grid_template_columns: vec![length(100.0), length(100.0), length(100.0)],
                grid_template_rows: vec![length(50.0)],
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(LayoutDisplay::Block, Style::default(), RED).with_order(2),
    );
    styles.0.insert(
        2,
        resolved(LayoutDisplay::Block, Style::default(), GREEN).with_order(-1),
    );
    styles.0.insert(
        3,
        resolved(LayoutDisplay::Block, Style::default(), BLUE).with_order(0),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(300, 100, 1.0));
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(0.0, 0.0, 100.0, 50.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(100.0, 0.0, 100.0, 50.0),
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(200.0, 0.0, 100.0, 50.0),
    );
}

#[test]
fn grid_item_placement_and_self_alignment_use_the_selected_grid_area() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("grid", vec![1]),
            FixtureNode::div("aligned", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Grid,
            Style {
                display: Display::Grid,
                size: Size {
                    width: length(300.0),
                    height: length(200.0),
                },
                grid_template_columns: vec![length(100.0), length(200.0)],
                grid_template_rows: vec![length(50.0), length(150.0)],
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(10.0),
                },
                grid_column: line(2),
                grid_row: line(2),
                justify_self: Some(taffy::AlignSelf::END),
                align_self: Some(taffy::AlignSelf::CENTER),
                ..Style::default()
            },
            RED,
        ),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(300, 200, 1.0));
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(280.0, 120.0, 20.0, 10.0),
    );
}

#[test]
fn grid_container_justify_items_center_shrink_wraps_an_auto_sized_item() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("grid", vec![1]),
            FixtureNode::div("item", vec![2]),
            FixtureNode::div("intrinsic-child", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Grid,
            Style {
                display: Display::Grid,
                size: Size {
                    width: length(300.0),
                    height: length(200.0),
                },
                justify_items: Some(taffy::AlignItems::CENTER),
                align_items: Some(taffy::AlignItems::CENTER),
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles
        .0
        .insert(1, resolved(LayoutDisplay::Block, Style::default(), GREEN));
    styles
        .0
        .insert(2, fixed_box(LayoutDisplay::Block, 20.0, 10.0, RED));

    let snapshot = render(&source, &mut styles, PaintViewport::new(300, 200, 1.0));
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(140.0, 95.0, 20.0, 10.0),
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(140.0, 95.0, 20.0, 10.0),
    );
}

#[test]
fn centered_grid_item_with_percent_height_and_auto_block_margin_keeps_intrinsic_inline_size() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("grid", vec![1]),
            FixtureNode::div("item", vec![2]),
            FixtureNode::svg("logo", 272.0, 92.0),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Grid,
            Style {
                display: Display::Grid,
                size: Size {
                    width: length(1280.0),
                    height: length(160.0),
                },
                padding: Rect {
                    left: length(96.0),
                    right: length(96.0),
                    top: length(0.0),
                    bottom: length(0.0),
                },
                grid_template_rows: vec![fr(1.0)],
                justify_items: Some(taffy::AlignItems::CENTER),
                align_items: Some(taffy::AlignItems::CENTER),
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: percent(1.0),
                },
                max_size: Size {
                    width: Dimension::auto(),
                    height: length(92.0),
                },
                margin: Rect {
                    left: length(0.0),
                    right: length(0.0),
                    top: taffy::LengthPercentageAuto::auto(),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
            GREEN,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Inline,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(92.0),
                },
                max_size: Size {
                    width: percent(1.0),
                    height: percent(1.0),
                },
                ..Style::default()
            },
            RED,
        ),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(1280, 720, 1.0));
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(504.0, 68.0, 272.0, 92.0),
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(504.0, 68.0, 272.0, 92.0),
    );
}

#[test]
fn oversized_percentage_inline_iframe_is_not_clamped_to_the_line_width() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1]),
            FixtureNode::iframe("iframe"),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        fixed_box(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Inline,
            Style {
                size: Size {
                    width: percent(1.25),
                    height: length(40.0),
                },
                ..Style::default()
            },
            BLUE,
        ),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(300, 200, 1.0));
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(0.0, 0.0, 250.0, 40.0),
    );
}

#[test]
fn static_insets_are_ignored_but_relative_insets_shift_the_box() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1, 2]),
            FixtureNode::div("static", Vec::new()),
            FixtureNode::div("relative", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(10.0),
                },
                inset: Rect {
                    left: length(80.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(40.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            RED,
        )
        .with_position(LayoutPosition::Static),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(10.0),
                },
                inset: Rect {
                    left: length(30.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(5.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            BLUE,
        )
        .with_position(LayoutPosition::Relative),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(200, 100, 1.0));
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(0.0, 0.0, 20.0, 10.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(30.0, 15.0, 20.0, 10.0),
    );
}

#[test]
fn root_percentages_and_root_level_absolute_boxes_resolve_against_viewport() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::html("html", "html", vec![1]),
            FixtureNode::div("static", vec![2]),
            FixtureNode::div("absolute", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: percent(0.5),
                    height: percent(1.0),
                },
                margin: Rect {
                    left: length(20.0),
                    right: length(0.0),
                    top: length(10.0),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
            BLUE,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: length(50.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: percent(0.5),
                    height: length(20.0),
                },
                inset: Rect {
                    left: percent(0.1),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: percent(0.25),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            RED,
        )
        .with_position(LayoutPosition::Absolute),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    // The HTML background propagates to the canvas, so its box is intentionally
    // not emitted as a local blue fragment.
    assert_eq!(snapshot.canvas_color, BLUE);
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(80.0, 150.0, 400.0, 20.0),
    );
}

#[test]
fn horizontal_auto_margins_center_a_definite_block() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1]),
            FixtureNode::div("centered", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(400.0),
                    height: length(100.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: length(20.0),
                },
                margin: Rect {
                    left: taffy::LengthPercentageAuto::auto(),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(0.0),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
            RED,
        ),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(150.0, 0.0, 100.0, 20.0),
    );
}

#[test]
fn block_percentage_box_sizing_and_min_max_resolve_against_containing_block() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1, 2]),
            FixtureNode::div("content-box", Vec::new()),
            FixtureNode::div("clamped", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style {
                box_sizing: BoxSizing::ContentBox,
                size: Size {
                    width: length(400.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                box_sizing: BoxSizing::ContentBox,
                size: Size {
                    width: percent(0.5),
                    height: length(20.0),
                },
                padding: Rect {
                    left: percent(0.1),
                    right: percent(0.1),
                    top: length(5.0),
                    bottom: length(5.0),
                },
                border: Rect {
                    left: length(5.0),
                    right: length(5.0),
                    top: length(2.0),
                    bottom: length(2.0),
                },
                ..Style::default()
            },
            RED,
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: percent(0.9),
                    height: length(10.0),
                },
                max_size: Size {
                    width: length(240.0),
                    height: Dimension::auto(),
                },
                margin: Rect {
                    left: length(30.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(10.0),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
            GREEN,
        ),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(0.0, 0.0, 290.0, 34.0),
    );
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(30.0, 44.0, 240.0, 10.0),
    );
}

#[test]
fn block_margin_collapse_matches_parent_child_and_sibling_rules() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::html("body", "body", vec![1, 4]),
            FixtureNode::div("collapsing-parent", vec![2, 3]),
            FixtureNode::div("first", Vec::new()),
            FixtureNode::div("second", Vec::new()),
            FixtureNode::div("after", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(400.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(20.0),
                },
                margin: Rect {
                    left: length(0.0),
                    right: length(0.0),
                    top: length(30.0),
                    bottom: length(40.0),
                },
                ..Style::default()
            },
            RED,
        ),
    );
    styles.0.insert(
        3,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(10.0),
                },
                margin: Rect {
                    left: length(0.0),
                    right: length(0.0),
                    top: length(20.0),
                    bottom: length(50.0),
                },
                ..Style::default()
            },
            BLUE,
        ),
    );
    styles
        .0
        .insert(4, fixed_box(LayoutDisplay::Block, 800.0, 5.0, GREEN));

    // This test inspects all three painted boxes to validate margin collapse,
    // so request the document surface instead of relying on offscreen viewport
    // paint that capture culling now intentionally omits.
    let snapshot = build_layout_pass(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        LayoutPassRequest::with_capture(
            PaintViewport::new(800, 100, 1.0),
            LayoutFlushReason::Test,
            PaintCaptureRequest::full_document(),
        ),
    )
    .unwrap()
    .into_paint_snapshot()
    .expect("full-document paint");
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(0.0, 30.0, 400.0, 20.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(0.0, 90.0, 400.0, 10.0),
    );
    assert_rect(
        solid_rect(&snapshot, GREEN),
        PaintRect::new(0.0, 150.0, 800.0, 5.0),
    );
    assert_eq!(snapshot.content_size.height, 155.0);
}

#[test]
fn aspect_ratio_and_absolute_auto_size_use_the_containing_block() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("positioned", vec![1, 2]),
            FixtureNode::div("ratio", Vec::new()),
            FixtureNode::div("absolute", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style {
                box_sizing: BoxSizing::ContentBox,
                size: Size {
                    width: length(400.0),
                    height: length(300.0),
                },
                padding: Rect {
                    left: length(20.0),
                    right: length(20.0),
                    top: length(20.0),
                    bottom: length(20.0),
                },
                border: Rect {
                    left: length(5.0),
                    right: length(5.0),
                    top: length(5.0),
                    bottom: length(5.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(120.0),
                    height: Dimension::auto(),
                },
                aspect_ratio: Some(2.0),
                ..Style::default()
            },
            BLUE,
        ),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                inset: Rect {
                    left: length(10.0),
                    right: length(30.0),
                    top: length(20.0),
                    bottom: length(40.0),
                },
                ..Style::default()
            },
            RED,
        )
        .with_position(LayoutPosition::Absolute),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(25.0, 25.0, 120.0, 60.0),
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(15.0, 25.0, 400.0, 280.0),
    );
}

#[test]
fn absolute_uses_nearest_positioned_ancestor_and_fixed_uses_viewport() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1]),
            FixtureNode::div("positioned", vec![2, 4]),
            FixtureNode::div("static", vec![3]),
            FixtureNode::div("absolute", Vec::new()),
            FixtureNode::div("fixed", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        1,
        resolved(
            LayoutDisplay::Block,
            Style {
                box_sizing: BoxSizing::ContentBox,
                size: Size {
                    width: length(400.0),
                    height: length(300.0),
                },
                margin: Rect {
                    left: length(30.0),
                    right: length(0.0),
                    top: length(30.0),
                    bottom: length(30.0),
                },
                padding: Rect {
                    left: length(20.0),
                    right: length(20.0),
                    top: length(20.0),
                    bottom: length(20.0),
                },
                border: Rect {
                    left: length(5.0),
                    right: length(5.0),
                    top: length(5.0),
                    bottom: length(5.0),
                },
                ..Style::default()
            },
            YELLOW,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(200.0),
                    height: length(100.0),
                },
                margin: Rect {
                    left: length(10.0),
                    right: length(0.0),
                    top: length(10.0),
                    bottom: length(0.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.0.insert(
        3,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: percent(0.5),
                    height: length(10.0),
                },
                inset: Rect {
                    left: percent(0.1),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: percent(0.25),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            RED,
        )
        .with_position(LayoutPosition::Absolute),
    );
    styles.0.insert(
        4,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(30.0),
                    height: length(40.0),
                },
                inset: Rect {
                    left: taffy::LengthPercentageAuto::auto(),
                    right: length(10.0),
                    top: taffy::LengthPercentageAuto::auto(),
                    bottom: length(20.0),
                },
                ..Style::default()
            },
            BLUE,
        )
        .with_position(LayoutPosition::Fixed),
    );

    let snapshot = render(&source, &mut styles, PaintViewport::new(800, 600, 1.0));
    assert_rect(
        solid_rect(&snapshot, YELLOW),
        PaintRect::new(30.0, 30.0, 450.0, 350.0),
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(79.0, 120.0, 220.0, 10.0),
    );
    assert_rect(
        solid_rect(&snapshot, BLUE),
        PaintRect::new(760.0, 540.0, 30.0, 40.0),
    );
    assert!(
        !snapshot
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "fixed-position-layout-deferred")
    );
}

#[test]
fn html_body_canvas_background_propagation_has_one_paint_owner() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::html("html", "html", vec![1]),
            FixtureNode::html("body", "body", vec![2]),
            FixtureNode::div("content", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        ),
    );
    styles
        .0
        .insert(1, resolved(LayoutDisplay::Block, Style::default(), BLUE));
    styles
        .0
        .insert(2, fixed_box(LayoutDisplay::Block, 40.0, 30.0, RED));

    let snapshot = render(&source, &mut styles, PaintViewport::new(200, 100, 1.0));
    assert_eq!(snapshot.canvas_color, BLUE);
    assert!(
        snapshot.fragments.iter().all(|fragment| {
            !fragment
                .solid_fill()
                .is_some_and(|(_, color, _)| color == BLUE)
        }),
        "the propagated body background must not also paint at the body box"
    );
    assert_rect(
        solid_rect(&snapshot, RED),
        PaintRect::new(0.0, 0.0, 40.0, 30.0),
    );

    styles
        .0
        .insert(0, resolved(LayoutDisplay::Block, Style::default(), GREEN));
    let root_wins = render(&source, &mut styles, PaintViewport::new(200, 100, 1.0));
    assert_eq!(root_wins.canvas_color, GREEN);
    assert!(root_wins.fragments.iter().any(|fragment| {
        fragment
            .solid_fill()
            .is_some_and(|(_, color, _)| color == BLUE)
    }));
}

#[test]
fn document_extent_includes_flow_and_absolute_overflow_but_excludes_fixed_subtrees() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1, 2, 3]),
            FixtureNode::div("flow", Vec::new()),
            FixtureNode::div("absolute", Vec::new()),
            FixtureNode::div("fixed", vec![4]),
            FixtureNode::div("fixed-child", Vec::new()),
        ],
    };
    let mut styles = FixtureStyles::default();
    styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles
        .0
        .insert(1, fixed_box(LayoutDisplay::Block, 20.0, 500.0, RED));
    styles.0.insert(
        2,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(50.0),
                },
                inset: Rect {
                    left: length(0.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(700.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            GREEN,
        )
        .with_position(LayoutPosition::Absolute),
    );
    styles.0.insert(
        3,
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(100.0),
                },
                inset: Rect {
                    left: length(0.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(900.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            BLUE,
        )
        .with_position(LayoutPosition::Fixed),
    );
    styles
        .0
        .insert(4, fixed_box(LayoutDisplay::Block, 600.0, 300.0, YELLOW));

    let snapshot = render(&source, &mut styles, PaintViewport::new(300, 200, 1.0));
    assert_eq!(snapshot.content_size.width, 300.0);
    assert_eq!(snapshot.content_size.height, 750.0);
}

#[test]
fn device_pixel_ratio_does_not_change_css_layout_coordinates() {
    let source = FixtureSource {
        nodes: vec![
            FixtureNode::div("root", vec![1]),
            FixtureNode::div("box", Vec::new()),
        ],
    };
    let mut first_styles = FixtureStyles::default();
    first_styles.0.insert(
        0,
        resolved(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        ),
    );
    first_styles
        .0
        .insert(1, fixed_box(LayoutDisplay::Block, 75.0, 25.0, RED));
    let mut second_styles = FixtureStyles(first_styles.0.clone());

    let one_x = render(
        &source,
        &mut first_styles,
        PaintViewport::new(300, 200, 1.0),
    );
    let two_x = render(
        &source,
        &mut second_styles,
        PaintViewport::new(300, 200, 2.0),
    );
    assert_eq!(solid_rect(&one_x, RED), solid_rect(&two_x, RED));
}
