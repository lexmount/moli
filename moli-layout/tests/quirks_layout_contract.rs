use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutDisplay, LayoutDocumentContext, LayoutDocumentMode,
    LayoutElementCategory, LayoutElementSemantics, LayoutError, LayoutFlushReason, LayoutNamespace,
    LayoutPassRequest, LayoutPassResult, LayoutPosition, LayoutSource, LayoutSourceKind,
    LayoutStyleResolver, LayoutViewport, PaintColor, ResolvedLayoutElementStyles,
    ResolvedLayoutStyle, build_layout_pass,
};
use style::Atom;
use taffy::{Dimension, Float, Overflow, Point, Rect, Size, Style, style_helpers::length};

#[derive(Clone)]
struct Node {
    local_name: &'static str,
    children: Vec<usize>,
}

struct Source {
    context: Option<LayoutDocumentContext<usize>>,
    nodes: Vec<Node>,
}

impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn document_context(&self) -> Option<LayoutDocumentContext<Self::NodeId>> {
        self.context
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
        Some(LayoutElementSemantics::new(
            LayoutNamespace::Html,
            self.nodes[node].local_name,
            LayoutElementCategory::Generic,
            None,
        ))
    }

    fn text(&self, _node: Self::NodeId) -> Option<&str> {
        None
    }

    fn label(&self, node: Self::NodeId) -> String {
        self.nodes[node].local_name.to_owned()
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

fn resolved(display: LayoutDisplay, style: Style<Atom>) -> ResolvedLayoutStyle {
    ResolvedLayoutStyle::synthetic(display, style, PaintColor::TRANSPARENT)
}

fn body_taffy_style() -> Style<Atom> {
    Style {
        size: Size {
            width: length(100.0),
            height: Dimension::auto(),
        },
        aspect_ratio: Some(1.0),
        margin: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(8.0),
            bottom: length(8.0),
        },
        ..Style::default()
    }
}

fn body_style() -> ResolvedLayoutStyle {
    resolved(LayoutDisplay::Block, body_taffy_style())
}

fn document(
    mode: LayoutDocumentMode,
    html: ResolvedLayoutStyle,
    body: ResolvedLayoutStyle,
    child: Option<ResolvedLayoutStyle>,
) -> LayoutPassResult<usize> {
    let body_children = child.is_some().then_some(vec![2]).unwrap_or_default();
    let mut nodes = vec![
        Node {
            local_name: "html",
            children: vec![1],
        },
        Node {
            local_name: "body",
            children: body_children,
        },
    ];
    let mut styles = Styles::default();
    styles.0.insert(0, html);
    styles.0.insert(1, body);
    if let Some(child) = child {
        nodes.push(Node {
            local_name: "div",
            children: Vec::new(),
        });
        styles.0.insert(2, child);
    }
    build_layout_pass(
        &Source {
            context: Some(LayoutDocumentContext::new(0, Some(1), mode)),
            nodes,
        },
        &mut styles,
        &mut DocumentLayoutServices::new(),
        LayoutPassRequest::new(LayoutViewport::new(800, 600, 1.0), LayoutFlushReason::Test),
    )
    .unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.01,
        "expected {expected}, got {actual}"
    );
}

fn height(output: &LayoutPassResult<usize>, source: usize) -> f32 {
    output
        .box_model_for_source(source)
        .unwrap()
        .border
        .bounding_rect()
        .height
}

#[test]
fn quirks_document_boxes_floor_auto_block_size_before_ratio_transfer() {
    let child = resolved(
        LayoutDisplay::Block,
        Style {
            size: Size {
                width: Dimension::auto(),
                height: Dimension::percent(0.5),
            },
            ..Style::default()
        },
    );
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        body_style(),
        Some(child),
    );
    assert_close(height(&output, 0), 600.0);
    assert_close(height(&output, 1), 584.0);
    assert_close(height(&output, 2), 292.0);

    let body_metrics = output.element_metrics_for_source(1).unwrap();
    assert_close(body_metrics.offset_size.height, 584.0);
    assert_close(body_metrics.client_size.height, 600.0);
}

#[test]
fn quirks_floor_is_shared_by_block_flex_grid_and_flow_root_containers() {
    for display in [
        LayoutDisplay::Block,
        LayoutDisplay::Flex,
        LayoutDisplay::Grid,
        LayoutDisplay::FlowRoot,
    ] {
        let output = document(
            LayoutDocumentMode::Quirks,
            resolved(LayoutDisplay::Block, Style::default()),
            resolved(display, body_taffy_style()),
            None,
        );
        assert_close(height(&output, 1), 584.0);
    }
}

#[test]
fn quirks_floor_remains_content_based_and_follows_the_logical_block_axis() {
    let tall_child = resolved(
        LayoutDisplay::Block,
        Style {
            size: Size {
                width: length(10.0),
                height: length(700.0),
            },
            ..Style::default()
        },
    );
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        resolved(
            LayoutDisplay::Block,
            Style {
                margin: body_taffy_style().margin,
                ..Style::default()
            },
        ),
        Some(tall_child),
    );
    assert_close(height(&output, 1), 700.0);

    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        resolved(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(100.0),
                },
                aspect_ratio: Some(1.0),
                ..Style::default()
            },
        )
        .with_writing_mode(taffy::WritingMode::VerticalRl),
        None,
    );
    let body = output
        .box_model_for_source(1)
        .unwrap()
        .border
        .bounding_rect();
    assert_close(body.width, 800.0);
    assert_close(body.height, 100.0);
}

#[test]
fn standards_modes_keep_ratio_sizing_and_root_viewport_client_metrics() {
    for mode in [
        LayoutDocumentMode::NoQuirks,
        LayoutDocumentMode::LimitedQuirks,
    ] {
        let output = document(
            mode,
            resolved(LayoutDisplay::Block, Style::default()),
            body_style(),
            None,
        );
        assert_close(height(&output, 1), 100.0);
        assert_close(
            output
                .element_metrics_for_source(0)
                .unwrap()
                .client_size
                .height,
            600.0,
        );
        assert_close(
            output
                .element_metrics_for_source(1)
                .unwrap()
                .client_size
                .height,
            100.0,
        );
        assert_eq!(output.document_scrolling_element(), Some(0));
    }
}

#[test]
fn quirks_floor_obeys_normal_height_min_max_and_scroll_container_sizing() {
    let mut explicit = body_taffy_style();
    explicit.size.height = length(120.0);
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        resolved(LayoutDisplay::Block, explicit),
        None,
    );
    assert_close(height(&output, 1), 120.0);
    assert_close(
        output
            .element_metrics_for_source(1)
            .unwrap()
            .client_size
            .height,
        600.0,
    );
    let mut capped = body_taffy_style();
    capped.max_size.height = length(200.0);
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        resolved(LayoutDisplay::Block, capped),
        None,
    );
    assert_close(height(&output, 1), 200.0);

    let mut explicit_minimum = body_taffy_style();
    explicit_minimum.min_size.height = length(0.0);
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        resolved(LayoutDisplay::Block, explicit_minimum),
        None,
    );
    assert_close(height(&output, 1), 100.0);

    let mut scrolling = body_taffy_style();
    scrolling.overflow = Point {
        x: Overflow::Hidden,
        y: Overflow::Hidden,
    };
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(LayoutDisplay::Block, Style::default()),
        resolved(LayoutDisplay::Block, scrolling),
        None,
    );
    assert_close(height(&output, 1), 100.0);
}

#[test]
fn quirks_floor_excludes_out_of_flow_float_and_inline_document_boxes() {
    let cases = [
        (
            body_style().with_position(LayoutPosition::Absolute),
            Some(100.0),
        ),
        (
            body_style().with_float(Float::Left, taffy::Clear::None),
            Some(100.0),
        ),
        (resolved(LayoutDisplay::Inline, body_taffy_style()), None),
    ];
    for (body, expected_height) in cases {
        let output = document(
            LayoutDocumentMode::Quirks,
            resolved(LayoutDisplay::Block, Style::default()),
            body,
            None,
        );
        let actual_height = height(&output, 1);
        if let Some(expected_height) = expected_height {
            assert_close(actual_height, expected_height);
        } else {
            assert!(
                actual_height < 584.0,
                "inline body must not receive the viewport floor, got {actual_height}"
            );
        }
        assert_close(
            output
                .element_metrics_for_source(1)
                .unwrap()
                .client_size
                .height,
            600.0,
        );
    }
}

#[test]
fn document_identity_prevents_named_nested_and_subtree_boxes_from_getting_quirks() {
    let nodes = vec![
        Node {
            local_name: "html",
            children: vec![1],
        },
        Node {
            local_name: "body",
            children: vec![2],
        },
        Node {
            local_name: "body",
            children: Vec::new(),
        },
    ];
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, resolved(LayoutDisplay::Block, Style::default()));
    styles
        .0
        .insert(1, resolved(LayoutDisplay::Block, Style::default()));
    styles.0.insert(2, body_style());
    let output = build_layout_pass(
        &Source {
            context: Some(LayoutDocumentContext::new(
                0,
                Some(1),
                LayoutDocumentMode::Quirks,
            )),
            nodes,
        },
        &mut styles,
        &mut DocumentLayoutServices::new(),
        LayoutPassRequest::new(LayoutViewport::new(800, 600, 1.0), LayoutFlushReason::Test),
    )
    .unwrap();
    assert_close(height(&output, 2), 100.0);
    assert_close(
        output
            .element_metrics_for_source(2)
            .unwrap()
            .client_size
            .height,
        100.0,
    );

    let subtree = Source {
        context: None,
        nodes: vec![
            Node {
                local_name: "html",
                children: vec![1],
            },
            Node {
                local_name: "body",
                children: Vec::new(),
            },
        ],
    };
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, resolved(LayoutDisplay::Block, Style::default()));
    styles.0.insert(1, body_style());
    let output = build_layout_pass(
        &subtree,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        LayoutPassRequest::new(LayoutViewport::new(800, 600, 1.0), LayoutFlushReason::Test),
    )
    .unwrap();
    assert_close(height(&output, 1), 100.0);
    assert_close(
        output
            .element_metrics_for_source(1)
            .unwrap()
            .client_size
            .height,
        100.0,
    );
    assert_eq!(output.document_scrolling_element(), None);
}

#[test]
fn quirks_scrolling_element_uses_both_root_and_body_overflow() {
    let visible = resolved(LayoutDisplay::Block, Style::default());
    let output = document(
        LayoutDocumentMode::Quirks,
        visible.clone(),
        body_style(),
        None,
    );
    assert_eq!(output.document_scrolling_element(), Some(1));

    let hidden = Point {
        x: Overflow::Hidden,
        y: Overflow::Hidden,
    };
    let root_hidden = resolved(
        LayoutDisplay::Block,
        Style {
            overflow: hidden,
            ..Style::default()
        },
    );
    let mut body_hidden = body_taffy_style();
    body_hidden.overflow = hidden;
    let output = document(
        LayoutDocumentMode::Quirks,
        root_hidden.clone(),
        resolved(LayoutDisplay::Block, body_hidden.clone()),
        None,
    );
    assert_eq!(output.document_scrolling_element(), None);

    body_hidden.overflow = Point {
        x: Overflow::Clip,
        y: Overflow::Clip,
    };
    let output = document(
        LayoutDocumentMode::Quirks,
        root_hidden,
        resolved(LayoutDisplay::Block, body_hidden),
        None,
    );
    assert_eq!(output.document_scrolling_element(), Some(1));
}

#[test]
fn mismatched_document_context_is_rejected_at_the_source_boundary() {
    let source = Source {
        context: Some(LayoutDocumentContext::new(
            1,
            None,
            LayoutDocumentMode::Quirks,
        )),
        nodes: vec![
            Node {
                local_name: "html",
                children: vec![1],
            },
            Node {
                local_name: "body",
                children: Vec::new(),
            },
        ],
    };
    let mut styles = Styles::default();
    styles
        .0
        .insert(0, resolved(LayoutDisplay::Block, Style::default()));
    let error = match build_layout_pass(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        LayoutPassRequest::new(LayoutViewport::new(800, 600, 1.0), LayoutFlushReason::Test),
    ) {
        Ok(_) => panic!("a document context cannot identify a different source root"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("document context must identify the source root")
    );
}
