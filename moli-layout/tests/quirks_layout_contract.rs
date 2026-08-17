use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutDisplay, LayoutDocumentMode, LayoutElementCategory,
    LayoutElementContent, LayoutElementSemantics, LayoutError, LayoutFlushReason, LayoutNamespace,
    LayoutPassRequest, LayoutPassResult, LayoutPosition, LayoutPseudo, LayoutSource,
    LayoutSourceKind, LayoutStyleResolver, LayoutViewport, PaintColor, ResolvedLayoutStyle,
    build_layout_pass,
};
use style::Atom;
use taffy::{Dimension, Overflow, Point, Rect, Size, Style, style_helpers::length};

#[derive(Clone)]
struct Node {
    local_name: &'static str,
    children: Vec<usize>,
}

struct Source {
    mode: LayoutDocumentMode,
    document_body: Option<usize>,
    nodes: Vec<Node>,
}

impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn root_is_document_element(&self) -> bool {
        true
    }

    fn document_mode(&self) -> LayoutDocumentMode {
        self.mode
    }

    fn document_body(&self) -> Option<Self::NodeId> {
        self.document_body
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
            LayoutElementContent::Normal,
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
    fn primary_style(&mut self, node: usize) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        Ok(self.0.get(&node).cloned())
    }

    fn pseudo_style(
        &mut self,
        _node: usize,
        _pseudo: LayoutPseudo,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        Ok(None)
    }
}

fn resolved(style: Style<Atom>) -> ResolvedLayoutStyle {
    ResolvedLayoutStyle::synthetic(LayoutDisplay::Block, style, PaintColor::TRANSPARENT)
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
    resolved(body_taffy_style())
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
            mode,
            document_body: Some(1),
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

fn empty_body_style(writing_mode: taffy::WritingMode) -> ResolvedLayoutStyle {
    resolved(Style {
        margin: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(8.0),
            bottom: length(8.0),
        },
        ..Style::default()
    })
    .with_writing_mode(writing_mode)
}

#[test]
fn document_viewport_uses_root_or_propagated_body_writing_mode() {
    for (name, root_writing_mode) in [
        ("root", taffy::WritingMode::VerticalLr),
        ("body", taffy::WritingMode::HorizontalTb),
    ] {
        let output = document(
            LayoutDocumentMode::NoQuirks,
            resolved(Style::default()).with_writing_mode(root_writing_mode),
            empty_body_style(taffy::WritingMode::VerticalLr),
            None,
        );
        let html = output
            .box_model_for_source(0)
            .unwrap()
            .border
            .bounding_rect();
        let body = output
            .box_model_for_source(1)
            .unwrap()
            .border
            .bounding_rect();

        assert_close(html.x, 0.0);
        assert_close(html.y, 0.0);
        assert_close(html.height, 600.0);
        assert_close(body.y, 8.0);
        assert_close(body.width, 0.0);
        assert_close(body.height, 584.0);
        assert_eq!(
            output
                .element_metrics_for_source(0)
                .unwrap()
                .client_size
                .height,
            600.0,
            "{name} writing mode must establish the viewport's logical inline axis"
        );
    }
}

#[test]
fn quirks_html_and_body_fill_the_available_block_size_before_ratio_transfer() {
    let child = resolved(Style {
        size: Size {
            width: Dimension::auto(),
            height: Dimension::percent(0.5),
        },
        ..Style::default()
    });
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(Style::default()),
        body_style(),
        Some(child),
    );

    let html = output
        .box_model_for_source(0)
        .unwrap()
        .border
        .bounding_rect();
    let body = output
        .box_model_for_source(1)
        .unwrap()
        .border
        .bounding_rect();
    let child = output
        .box_model_for_source(2)
        .unwrap()
        .border
        .bounding_rect();
    assert_close(html.x, 0.0);
    assert_close(html.y, 0.0);
    assert_close(html.width, 800.0);
    assert_close(html.height, 600.0);
    assert_close(body.y, 8.0);
    assert_close(body.width, 100.0);
    assert_close(body.height, 584.0);
    assert_close(child.height, 292.0);

    let html_metrics = output.element_metrics_for_source(0).unwrap();
    let body_metrics = output.element_metrics_for_source(1).unwrap();
    assert_close(html_metrics.client_size.height, 600.0);
    assert_close(body_metrics.offset_size.height, 584.0);
    assert_close(body_metrics.client_size.height, 600.0);
}

#[test]
fn standards_and_limited_quirks_documents_keep_ratio_based_body_height() {
    for mode in [
        LayoutDocumentMode::NoQuirks,
        LayoutDocumentMode::LimitedQuirks,
    ] {
        let output = document(mode, resolved(Style::default()), body_style(), None);
        let html = output
            .box_model_for_source(0)
            .unwrap()
            .border
            .bounding_rect();
        let body = output
            .box_model_for_source(1)
            .unwrap()
            .border
            .bounding_rect();
        let metrics = output.element_metrics_for_source(1).unwrap();
        assert_close(html.x, 0.0);
        assert_close(html.y, 0.0);
        assert_close(html.width, 800.0);
        assert_close(html.height, 116.0);
        assert_close(body.height, 100.0);
        assert_close(metrics.client_size.height, 100.0);
    }
}

#[test]
fn quirks_fill_respects_explicit_and_max_block_sizes() {
    let explicit = document(
        LayoutDocumentMode::Quirks,
        resolved(Style::default()),
        resolved(Style {
            size: Size {
                width: length(100.0),
                height: length(120.0),
            },
            aspect_ratio: Some(1.0),
            margin: body_taffy_style().margin,
            ..Style::default()
        }),
        None,
    );
    assert_close(
        explicit
            .box_model_for_source(1)
            .unwrap()
            .border
            .bounding_rect()
            .height,
        120.0,
    );

    let mut constrained = body_taffy_style();
    constrained.max_size.height = length(200.0);
    let constrained = document(
        LayoutDocumentMode::Quirks,
        resolved(Style::default()),
        resolved(constrained),
        None,
    );
    assert_close(
        constrained
            .box_model_for_source(1)
            .unwrap()
            .border
            .bounding_rect()
            .height,
        200.0,
    );
    assert_close(
        constrained
            .element_metrics_for_source(1)
            .unwrap()
            .client_size
            .height,
        600.0,
    );
}

#[test]
fn out_of_flow_body_does_not_participate_in_the_layout_quirk() {
    let body = body_style().with_position(LayoutPosition::Absolute);
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(Style::default()),
        body,
        None,
    );
    assert_close(
        output
            .box_model_for_source(1)
            .unwrap()
            .border
            .bounding_rect()
            .height,
        100.0,
    );
}

#[test]
fn nested_body_named_element_is_not_the_document_body() {
    let nodes = vec![
        Node {
            local_name: "html",
            children: vec![1],
        },
        Node {
            local_name: "div",
            children: vec![2],
        },
        Node {
            local_name: "body",
            children: Vec::new(),
        },
    ];
    let mut styles = Styles::default();
    styles.0.insert(0, resolved(Style::default()));
    styles.0.insert(1, resolved(Style::default()));
    styles.0.insert(2, body_style());
    let output = build_layout_pass(
        &Source {
            mode: LayoutDocumentMode::Quirks,
            document_body: None,
            nodes,
        },
        &mut styles,
        &mut DocumentLayoutServices::new(),
        LayoutPassRequest::new(LayoutViewport::new(800, 600, 1.0), LayoutFlushReason::Test),
    )
    .unwrap();
    let body = output
        .box_model_for_source(2)
        .unwrap()
        .border
        .bounding_rect();
    let metrics = output.element_metrics_for_source(2).unwrap();
    assert_close(body.height, 100.0);
    assert_close(metrics.client_size.height, 100.0);
}

#[test]
fn cssom_view_selects_the_root_or_body_viewport_client_box_by_document_mode() {
    let html = resolved(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(200.0),
        },
        ..Style::default()
    });
    let quirks = document(LayoutDocumentMode::Quirks, html.clone(), body_style(), None);
    assert_close(
        quirks
            .element_metrics_for_source(0)
            .unwrap()
            .client_size
            .height,
        200.0,
    );
    assert_close(
        quirks
            .element_metrics_for_source(1)
            .unwrap()
            .client_size
            .height,
        600.0,
    );

    let standards = document(LayoutDocumentMode::NoQuirks, html, body_style(), None);
    assert_close(
        standards
            .element_metrics_for_source(0)
            .unwrap()
            .client_size
            .height,
        600.0,
    );
    assert_close(
        standards
            .element_metrics_for_source(1)
            .unwrap()
            .client_size
            .height,
        100.0,
    );
}

#[test]
fn potentially_scrollable_quirks_body_keeps_its_physical_scroll_box() {
    let scroll_overflow = Point {
        x: Overflow::Hidden,
        y: Overflow::Hidden,
    };
    let html = resolved(Style {
        overflow: scroll_overflow,
        ..Style::default()
    });
    let mut body = body_taffy_style();
    body.overflow = scroll_overflow;
    let output = document(LayoutDocumentMode::Quirks, html, resolved(body), None);
    let metrics = output.element_metrics_for_source(1).unwrap();
    assert_close(metrics.offset_size.height, 584.0);
    assert_close(metrics.client_size.height, 600.0);
    assert_close(metrics.scroll_size.height, 584.0);
}

#[test]
fn root_clip_keeps_body_overflow_physical_and_removes_the_quirks_scrolling_element() {
    let root = resolved(Style {
        overflow: Point {
            x: Overflow::Clip,
            y: Overflow::Clip,
        },
        ..Style::default()
    });
    let mut body = body_taffy_style();
    body.overflow = Point {
        x: Overflow::Scroll,
        y: Overflow::Scroll,
    };
    let output = document(LayoutDocumentMode::Quirks, root, resolved(body), None);

    assert_eq!(output.document_scrolling_element, None);
    let body_metrics = output.element_metrics_for_source(1).unwrap();
    assert!(body_metrics.is_scroll_container);
    assert_close(body_metrics.scroll_size.height, 584.0);
}

#[test]
fn body_clip_propagates_to_the_viewport_and_remains_the_quirks_scrolling_element() {
    let mut body = body_taffy_style();
    body.overflow = Point {
        x: Overflow::Clip,
        y: Overflow::Clip,
    };
    let output = document(
        LayoutDocumentMode::Quirks,
        resolved(Style::default()),
        resolved(body),
        None,
    );

    assert_eq!(output.document_scrolling_element, Some(1));
    let body_box = output.source_output(1).unwrap().principal_box.unwrap();
    let physical_extent = output.scroll_extent(body_box).unwrap();
    assert!(!physical_extent.is_scroll_container);
    assert!(!physical_extent.clips_overflow);
    let body_metrics = output.element_metrics_for_source(1).unwrap();
    assert!(body_metrics.is_scroll_container);
    assert_close(body_metrics.scroll_size.height, 600.0);
}
