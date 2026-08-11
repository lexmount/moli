use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutAnonymousReason, LayoutBoxKind, LayoutCapabilityDiagnostic,
    LayoutDisplay, LayoutElementCategory, LayoutElementSemantics, LayoutError,
    LayoutFormControlKind, LayoutInlineAlignment, LayoutInputControlKind, LayoutListRole,
    LayoutNamespace, LayoutPosition, LayoutPseudo, LayoutReplacedKind, LayoutSource,
    LayoutSourceKind, LayoutStyleResolver, LayoutTableRole, PaintColor, PaintFragment, PaintRect,
    PaintViewport, ReplacedMetrics, ResolvedLayoutStyle, ScreenshotLayoutRequest,
    build_layout_world, build_screenshot_snapshot, normalize_layout_source,
};
use style::Atom;
use taffy::{Dimension, Display, FlexDirection, Rect, Size, Style, style_helpers::length};

#[derive(Clone, Debug)]
struct TestNode {
    label: &'static str,
    kind: LayoutSourceKind,
    text: Option<&'static str>,
    children: Vec<usize>,
    element_semantics: Option<LayoutElementSemantics>,
    replaced_metrics: Option<ReplacedMetrics>,
}

impl TestNode {
    fn element(label: &'static str, children: Vec<usize>) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Element,
            text: None,
            children,
            element_semantics: Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "div",
                LayoutElementCategory::Generic,
                None,
            )),
            replaced_metrics: None,
        }
    }

    fn semantic_element(
        label: &'static str,
        local_name: &'static str,
        category: LayoutElementCategory,
        replaced: Option<LayoutReplacedKind>,
        children: Vec<usize>,
    ) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Element,
            text: None,
            children,
            element_semantics: Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                local_name,
                category,
                replaced,
            )),
            replaced_metrics: None,
        }
    }

    fn text(label: &'static str, text: &'static str) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Text,
            text: Some(text),
            children: Vec::new(),
            element_semantics: None,
            replaced_metrics: None,
        }
    }

    fn comment(label: &'static str) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Comment,
            text: None,
            children: Vec::new(),
            element_semantics: None,
            replaced_metrics: None,
        }
    }

    fn replaced(label: &'static str, width: f32, height: f32) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Element,
            text: None,
            children: Vec::new(),
            element_semantics: Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "img",
                LayoutElementCategory::Generic,
                Some(LayoutReplacedKind::Image),
            )),
            replaced_metrics: Some(ReplacedMetrics {
                intrinsic_width: Some(width),
                intrinsic_height: Some(height),
                attribute_width: None,
                attribute_height: None,
                intrinsic_ratio: (height > 0.0).then_some(width / height),
            }),
        }
    }
}

struct TestSource {
    root: usize,
    nodes: Vec<TestNode>,
}

impl LayoutSource for TestSource {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> Self::NodeId {
        self.root
    }

    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        if node == self.root {
            return None;
        }
        self.nodes
            .iter()
            .position(|candidate| candidate.children.contains(&node))
    }

    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        self.nodes[node].children.iter().copied()
    }

    fn node_kind(&self, node: Self::NodeId) -> LayoutSourceKind {
        self.nodes[node].kind
    }

    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics> {
        self.nodes[node].element_semantics.clone()
    }

    fn text(&self, node: Self::NodeId) -> Option<&str> {
        self.nodes[node].text
    }

    fn label(&self, node: Self::NodeId) -> String {
        self.nodes[node].label.to_owned()
    }

    fn replaced_metrics(&self, node: Self::NodeId) -> Option<ReplacedMetrics> {
        self.nodes[node].replaced_metrics
    }
}

#[derive(Default)]
struct TestStyles {
    primary: HashMap<usize, ResolvedLayoutStyle>,
    pseudo: HashMap<(usize, LayoutPseudo), ResolvedLayoutStyle>,
    primary_queries: Vec<usize>,
    pseudo_queries: Vec<(usize, LayoutPseudo)>,
    anonymous_queries: Vec<(usize, LayoutDisplay)>,
}

impl LayoutStyleResolver<usize> for TestStyles {
    fn primary_style(&mut self, node: usize) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        self.primary_queries.push(node);
        Ok(self.primary.get(&node).cloned())
    }

    fn pseudo_style(
        &mut self,
        node: usize,
        pseudo: LayoutPseudo,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        self.pseudo_queries.push((node, pseudo));
        Ok(self.pseudo.get(&(node, pseudo)).cloned())
    }

    fn anonymous_style(
        &mut self,
        owner: usize,
        parent: &ResolvedLayoutStyle,
        display: LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        self.anonymous_queries.push((owner, display));
        Ok(ResolvedLayoutStyle::anonymous_from(parent, display))
    }
}

fn style(display: LayoutDisplay) -> ResolvedLayoutStyle {
    ResolvedLayoutStyle::synthetic(display, Style::<Atom>::default(), PaintColor::TRANSPARENT)
}

fn colored_box(
    display: LayoutDisplay,
    width: f32,
    height: f32,
    color: PaintColor,
) -> ResolvedLayoutStyle {
    let taffy = Style::<Atom> {
        size: Size {
            width: Dimension::length(width),
            height: Dimension::length(height),
        },
        ..Style::default()
    };
    ResolvedLayoutStyle::synthetic(display, taffy, color)
}

#[test]
fn simple_block_ignores_comment_and_is_stable() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2]),
            TestNode::comment("comment"),
            TestNode::element("child", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(2, style(LayoutDisplay::Block));

    let first = build_layout_world(&source, &mut styles).unwrap();
    first.validate_invariants().unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first.source_box(1), None);
    let first_normalized = first.normalized_tree().to_string();

    let second = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(first_normalized, second.normalized_tree().to_string());
    assert_eq!(styles.primary_queries, vec![0, 2, 0, 2]);
}

#[test]
fn display_none_prunes_the_whole_subtree() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("hidden", vec![2]),
            TestNode::element("hidden-child", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::None));
    styles.primary.insert(2, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(world.len(), 1);
    assert_eq!(world.source_box(1), None);
    assert_eq!(world.source_box(2), None);
    assert_eq!(styles.primary_queries, vec![0, 1]);
}

#[test]
fn a_missing_non_root_style_prunes_that_source_subtree() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("unstyled", vec![2]),
            TestNode::element("unstyled-child", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(2, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(world.len(), 1);
    assert_eq!(world.source_box(1), None);
    assert_eq!(world.source_box(2), None);
    assert_eq!(styles.primary_queries, vec![0, 1]);
}

#[test]
fn display_contents_hoists_children_without_a_principal_box() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("contents", vec![2]),
            TestNode::element("leaf", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::Contents));
    styles.primary.insert(2, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(world.source_box(1), None);
    let leaf = world.source_box(2).unwrap();
    assert_eq!(world.box_by_id(world.root()).unwrap().children(), &[leaf]);
}

#[test]
fn display_contents_on_unusual_html_elements_suppresses_the_element_and_subtree() {
    let unusual_elements = [
        "br", "wbr", "meter", "progress", "canvas", "embed", "object", "audio", "iframe", "img",
        "video", "frame", "frameset", "input", "textarea", "select",
    ];

    for local_name in unusual_elements {
        let source = TestSource {
            root: 0,
            nodes: vec![
                TestNode::element("root", vec![1]),
                TestNode::semantic_element(
                    "unusual",
                    local_name,
                    LayoutElementCategory::Generic,
                    None,
                    vec![2],
                ),
                TestNode::text("suppressed-child", "must-not-layout"),
            ],
        };
        let mut styles = TestStyles::default();
        styles.primary.insert(0, style(LayoutDisplay::Block));
        styles.primary.insert(1, style(LayoutDisplay::Contents));

        let world = build_layout_world(&source, &mut styles).unwrap();
        assert_eq!(world.len(), 1, "element={local_name}");
        assert_eq!(world.source_box(1), None, "element={local_name}");
        assert_eq!(world.source_box(2), None, "element={local_name}");
        assert_eq!(styles.primary_queries, vec![0, 1]);
    }
}

#[test]
fn display_contents_remains_transparent_for_button_fallback_content() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::semantic_element(
                "button",
                "button",
                LayoutElementCategory::FormControl(LayoutFormControlKind::Button),
                None,
                vec![2],
            ),
            TestNode::text("button-text", "button"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::Contents));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(world.source_box(1), None);
    assert_eq!(
        world.box_by_id(world.root()).unwrap().children(),
        &[world.source_box(2).unwrap()]
    );
}

#[test]
fn hidden_input_never_constructs_a_box_even_when_css_requests_block() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::semantic_element(
                "hidden-input",
                "input",
                LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                    LayoutInputControlKind::Hidden,
                )),
                Some(LayoutReplacedKind::FormControl),
                vec![2],
            ),
            TestNode::text("hidden-child", "must-not-layout"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(world.len(), 1);
    assert_eq!(world.source_box(1), None);
    assert_eq!(world.source_box(2), None);
    assert_eq!(styles.primary_queries, vec![0, 1]);
}

#[test]
fn mixed_flow_wraps_each_inline_run_in_one_anonymous_block() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3]),
            TestNode::text("text-before", "before"),
            TestNode::element("block", Vec::new()),
            TestNode::text("text-after", "after"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(2, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(
        world.normalized_tree().to_string(),
        concat!(
            "box-tree-schema=3\n",
            "principal-block path=0 display=block source=root element=html:div category=generic fc=block\n",
            "  anonymous-block path=0/0 display=block owner=root anonymous=mixed-flow-inline-run fc=inline\n",
            "    text path=0/0/0 display=inline source=text-before text=\"before\"\n",
            "  principal-block path=0/1 display=block source=block element=html:div category=generic fc=block\n",
            "  anonymous-block path=0/2 display=block owner=root anonymous=mixed-flow-inline-run fc=inline\n",
            "    text path=0/2/0 display=inline source=text-after text=\"after\"\n",
        )
    );
}

#[test]
fn all_inline_children_make_the_principal_box_an_inline_context() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2]),
            TestNode::text("text", "hello"),
            TestNode::element("span", vec![3]),
            TestNode::text("span-text", "world"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(2, style(LayoutDisplay::Inline));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let root = world.box_by_id(world.root()).unwrap();
    assert!(root.establishes_inline_formatting_context());
    let span = world.box_by_id(world.source_box(2).unwrap()).unwrap();
    assert!(span.establishes_inline_formatting_context());
    assert_eq!(root.children().len(), 2);
}

#[test]
fn computed_display_matrix_keeps_every_phase_one_box_role() {
    let cases = [
        (LayoutDisplay::Block, LayoutBoxKind::PrincipalBlock),
        (LayoutDisplay::FlowRoot, LayoutBoxKind::PrincipalFlowRoot),
        (LayoutDisplay::Inline, LayoutBoxKind::PrincipalInline),
        (
            LayoutDisplay::InlineBlock,
            LayoutBoxKind::PrincipalInlineBlock,
        ),
        (LayoutDisplay::Flex, LayoutBoxKind::PrincipalFlex),
        (
            LayoutDisplay::InlineFlex,
            LayoutBoxKind::PrincipalInlineFlex,
        ),
        (LayoutDisplay::Grid, LayoutBoxKind::PrincipalGrid),
        (
            LayoutDisplay::InlineGrid,
            LayoutBoxKind::PrincipalInlineGrid,
        ),
        (LayoutDisplay::BlockListItem, LayoutBoxKind::ListItem),
        (LayoutDisplay::InlineListItem, LayoutBoxKind::InlineListItem),
        (LayoutDisplay::Table, LayoutBoxKind::TableWrapper),
        (
            LayoutDisplay::InlineTable,
            LayoutBoxKind::InlineTableWrapper,
        ),
        (LayoutDisplay::TableCaption, LayoutBoxKind::TableCaption),
        (LayoutDisplay::TableRowGroup, LayoutBoxKind::TableRowGroup),
        (
            LayoutDisplay::TableHeaderGroup,
            LayoutBoxKind::TableHeaderGroup,
        ),
        (
            LayoutDisplay::TableFooterGroup,
            LayoutBoxKind::TableFooterGroup,
        ),
        (
            LayoutDisplay::TableColumnGroup,
            LayoutBoxKind::TableColumnGroup,
        ),
        (LayoutDisplay::TableColumn, LayoutBoxKind::TableColumn),
        (LayoutDisplay::TableRow, LayoutBoxKind::TableRow),
        (LayoutDisplay::TableCell, LayoutBoxKind::TableCell),
    ];

    for (display, expected_kind) in cases {
        let source = TestSource {
            root: 0,
            nodes: vec![TestNode::element("root", Vec::new())],
        };
        let mut styles = TestStyles::default();
        styles.primary.insert(0, style(display));
        let world = build_layout_world(&source, &mut styles).unwrap();
        let root = world.box_by_id(world.root()).unwrap();
        assert_eq!(root.kind(), expected_kind, "display={display:?}");
        assert_eq!(root.style().display(), display);
    }
}

#[test]
fn flex_direct_text_uses_an_anonymous_flex_item_and_preserves_run_whitespace() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3]),
            TestNode::text("text", "card"),
            TestNode::text("whitespace", "  \n  "),
            TestNode::element("span", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Flex));
    styles.primary.insert(3, style(LayoutDisplay::Inline));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let root = world.box_by_id(world.root()).unwrap();
    assert_eq!(root.children().len(), 2);
    assert_eq!(
        world.box_by_id(root.children()[0]).unwrap().kind(),
        LayoutBoxKind::AnonymousFlexItem
    );
    assert_eq!(
        world
            .box_by_id(root.children()[0])
            .unwrap()
            .anonymous_reason(),
        Some(LayoutAnonymousReason::FlexTextRun)
    );
    let flex_text_item = world.box_by_id(root.children()[0]).unwrap();
    assert_eq!(flex_text_item.children().len(), 2);
    assert_eq!(world.source_box(2), Some(flex_text_item.children()[1]));
    assert_eq!(
        world.box_by_id(root.children()[1]).unwrap().kind(),
        LayoutBoxKind::PrincipalInline
    );
}

#[test]
fn grid_direct_text_uses_one_anonymous_grid_item_and_keeps_text_boundaries() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3, 4]),
            TestNode::text("left", "left"),
            TestNode::text("space", " "),
            TestNode::text("right", "right"),
            TestNode::element("item", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Grid));
    styles.primary.insert(4, style(LayoutDisplay::Inline));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let root = world.box_by_id(world.root()).unwrap();
    assert_eq!(root.kind(), LayoutBoxKind::PrincipalGrid);
    assert_eq!(root.children().len(), 2);
    let text_item = world.box_by_id(root.children()[0]).unwrap();
    assert_eq!(text_item.kind(), LayoutBoxKind::AnonymousGridItem);
    assert_eq!(
        text_item.anonymous_reason(),
        Some(LayoutAnonymousReason::GridTextRun)
    );
    assert_eq!(text_item.children().len(), 3);
    assert_eq!(world.source_box(2), Some(text_item.children()[1]));
    assert!(root.capability_diagnostics().is_empty());
}

#[test]
fn contents_text_is_an_item_of_the_flattened_flex_or_grid_container() {
    let cases = [
        (
            LayoutDisplay::Flex,
            LayoutBoxKind::AnonymousFlexItem,
            LayoutAnonymousReason::FlexTextRun,
        ),
        (
            LayoutDisplay::Grid,
            LayoutBoxKind::AnonymousGridItem,
            LayoutAnonymousReason::GridTextRun,
        ),
    ];

    for (container_display, expected_kind, expected_reason) in cases {
        let source = TestSource {
            root: 0,
            nodes: vec![
                TestNode::element("root", vec![1]),
                TestNode::element("contents", vec![2, 3]),
                TestNode::text("left", "left"),
                TestNode::text("right", "right"),
            ],
        };
        let mut styles = TestStyles::default();
        styles.primary.insert(0, style(container_display));
        styles.primary.insert(1, style(LayoutDisplay::Contents));

        let world = build_layout_world(&source, &mut styles).unwrap();
        assert_eq!(world.source_box(1), None);
        let root = world.box_by_id(world.root()).unwrap();
        assert_eq!(root.children().len(), 1, "display={container_display:?}");
        let item = world.box_by_id(root.children()[0]).unwrap();
        assert_eq!(item.kind(), expected_kind);
        assert_eq!(item.anonymous_reason(), Some(expected_reason));
        assert_eq!(item.owner(), Some(0));
        assert_eq!(item.children().len(), 2);
        assert_eq!(styles.anonymous_queries, vec![(0, LayoutDisplay::Block)]);
    }
}

#[test]
fn whitespace_does_not_create_an_ifc_around_only_out_of_flow_content() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3]),
            TestNode::text("leading-space", " \n "),
            TestNode::element("absolute", Vec::new()),
            TestNode::text("trailing-space", "\t"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles
        .primary
        .insert(2, style(LayoutDisplay::Block).with_out_of_flow());

    let world = build_layout_world(&source, &mut styles).unwrap();
    let root = world.box_by_id(world.root()).unwrap();
    assert!(!root.establishes_inline_formatting_context());
    assert_eq!(root.children().len(), 1);
    assert_eq!(root.children()[0], world.source_box(2).unwrap());
    assert_eq!(world.source_box(1), None);
    assert_eq!(world.source_box(3), None);
}

#[test]
fn out_of_flow_box_terminates_each_mixed_flow_anonymous_run() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3, 4]),
            TestNode::text("before", "before"),
            TestNode::element("absolute", Vec::new()),
            TestNode::text("after", "after"),
            TestNode::element("block", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles
        .primary
        .insert(2, style(LayoutDisplay::Block).with_out_of_flow());
    styles.primary.insert(4, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let children = world.box_by_id(world.root()).unwrap().children();
    assert_eq!(children.len(), 4);
    assert_eq!(
        world.box_by_id(children[0]).unwrap().kind(),
        LayoutBoxKind::AnonymousBlock
    );
    assert_eq!(children[1], world.source_box(2).unwrap());
    assert_eq!(
        world.box_by_id(children[2]).unwrap().kind(),
        LayoutBoxKind::AnonymousBlock
    );
    assert_eq!(children[3], world.source_box(4).unwrap());
    assert_eq!(styles.anonymous_queries.len(), 2);
}

#[test]
fn list_marker_precedes_before_content_and_keeps_normal_marker_state() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::semantic_element(
            "item",
            "li",
            LayoutElementCategory::List(LayoutListRole::Item),
            None,
            Vec::new(),
        )],
    };
    let mut styles = TestStyles::default();
    styles
        .primary
        .insert(0, style(LayoutDisplay::BlockListItem));
    styles.pseudo.insert(
        (0, LayoutPseudo::Marker),
        style(LayoutDisplay::Inline).with_normal_generated_content(),
    );
    styles.pseudo.insert(
        (0, LayoutPseudo::Before),
        style(LayoutDisplay::Inline).with_generated_text("before"),
    );

    let world = build_layout_world(&source, &mut styles).unwrap();
    let children = world.box_by_id(world.root()).unwrap().children();
    assert_eq!(children.len(), 2);
    let marker = world.box_by_id(children[0]).unwrap();
    assert_eq!(marker.kind(), LayoutBoxKind::PseudoMarker);
    assert_eq!(marker.pseudo(), Some(LayoutPseudo::Marker));
    assert_eq!(marker.text(), None);
    assert!(marker.capability_diagnostics().is_empty());
    assert_eq!(
        world.box_by_id(children[1]).unwrap().pseudo(),
        Some(LayoutPseudo::Before)
    );
    assert_eq!(
        styles.pseudo_queries,
        vec![
            (0, LayoutPseudo::Marker),
            (0, LayoutPseudo::Before),
            (0, LayoutPseudo::After),
        ]
    );
}

#[test]
fn list_item_and_marker_construction_follow_computed_display_not_the_html_tag() {
    let generic_source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("generic", Vec::new())],
    };
    let mut generic_styles = TestStyles::default();
    generic_styles
        .primary
        .insert(0, style(LayoutDisplay::BlockListItem));
    generic_styles.pseudo.insert(
        (0, LayoutPseudo::Marker),
        style(LayoutDisplay::Inline).with_normal_generated_content(),
    );
    let generic_world = build_layout_world(&generic_source, &mut generic_styles).unwrap();
    let generic = generic_world.box_by_id(generic_world.root()).unwrap();
    assert_eq!(generic.kind(), LayoutBoxKind::ListItem);
    assert_eq!(generic.children().len(), 1);
    assert_eq!(
        generic_world
            .box_by_id(generic.children()[0])
            .unwrap()
            .kind(),
        LayoutBoxKind::PseudoMarker
    );

    let li_source = TestSource {
        root: 0,
        nodes: vec![TestNode::semantic_element(
            "li",
            "li",
            LayoutElementCategory::List(LayoutListRole::Item),
            None,
            Vec::new(),
        )],
    };
    let mut li_styles = TestStyles::default();
    li_styles.primary.insert(0, style(LayoutDisplay::Block));
    li_styles.pseudo.insert(
        (0, LayoutPseudo::Marker),
        style(LayoutDisplay::Inline).with_normal_generated_content(),
    );
    let li_world = build_layout_world(&li_source, &mut li_styles).unwrap();
    let li = li_world.box_by_id(li_world.root()).unwrap();
    assert_eq!(li.kind(), LayoutBoxKind::PrincipalBlock);
    assert!(li.children().is_empty());
    assert_eq!(
        li_styles.pseudo_queries,
        vec![(0, LayoutPseudo::Before), (0, LayoutPseudo::After)]
    );
}

#[test]
fn unsupported_generated_item_keeps_the_pseudo_and_reports_capability() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("root", Vec::new())],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.pseudo.insert(
        (0, LayoutPseudo::Before),
        style(LayoutDisplay::Inline).with_unsupported_generated_content(),
    );

    let world = build_layout_world(&source, &mut styles).unwrap();
    let pseudo = world
        .box_by_id(world.box_by_id(world.root()).unwrap().children()[0])
        .unwrap();
    assert_eq!(pseudo.kind(), LayoutBoxKind::PseudoBefore);
    assert_eq!(pseudo.text(), None);
    assert!(pseudo.children().is_empty());
    assert_eq!(
        pseudo.capability_diagnostics(),
        &[LayoutCapabilityDiagnostic::GeneratedContentUnsupported]
    );
}

#[test]
fn pseudo_generated_content_uses_the_owning_formatting_context_rules() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("root", Vec::new())],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Flex));
    styles.pseudo.insert(
        (0, LayoutPseudo::Before),
        style(LayoutDisplay::InlineFlex).with_generated_text("generated"),
    );

    let world = build_layout_world(&source, &mut styles).unwrap();
    let pseudo = world
        .box_by_id(world.box_by_id(world.root()).unwrap().children()[0])
        .unwrap();
    assert_eq!(pseudo.kind(), LayoutBoxKind::PseudoBefore);
    assert_eq!(pseudo.style().display(), LayoutDisplay::InlineFlex);
    assert_eq!(pseudo.children().len(), 1);
    let item = world.box_by_id(pseudo.children()[0]).unwrap();
    assert_eq!(item.kind(), LayoutBoxKind::AnonymousFlexItem);
    assert_eq!(item.children().len(), 1);
    assert_eq!(
        world.box_by_id(item.children()[0]).unwrap().text(),
        Some("generated")
    );
    assert_eq!(styles.anonymous_queries, vec![(0, LayoutDisplay::Block)]);
}

#[test]
fn table_display_on_a_pseudo_participates_in_missing_parent_fixup() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("root", Vec::new())],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.pseudo.insert(
        (0, LayoutPseudo::After),
        style(LayoutDisplay::TableCell).with_generated_text("cell"),
    );

    let world = build_layout_world(&source, &mut styles).unwrap();
    let table = world
        .box_by_id(world.box_by_id(world.root()).unwrap().children()[0])
        .unwrap();
    let row_group = world.box_by_id(table.children()[0]).unwrap();
    let row = world.box_by_id(row_group.children()[0]).unwrap();
    let pseudo = world.box_by_id(row.children()[0]).unwrap();
    assert_eq!(table.kind(), LayoutBoxKind::AnonymousTableWrapper);
    assert_eq!(row_group.kind(), LayoutBoxKind::AnonymousTableRowGroup);
    assert_eq!(row.kind(), LayoutBoxKind::AnonymousTableRow);
    assert_eq!(pseudo.kind(), LayoutBoxKind::PseudoAfter);
    assert_eq!(pseudo.style().display(), LayoutDisplay::TableCell);
    assert_eq!(pseudo.pseudo(), Some(LayoutPseudo::After));
    assert_eq!(pseudo.children().len(), 1);
    assert_eq!(
        world.box_by_id(pseudo.children()[0]).unwrap().text(),
        Some("cell")
    );
}

#[test]
fn display_contents_pseudo_hoists_supported_text_and_retains_unsupported_diagnostics() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("root", Vec::new())],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.pseudo.insert(
        (0, LayoutPseudo::Before),
        style(LayoutDisplay::Contents).with_generated_text("before"),
    );
    styles.pseudo.insert(
        (0, LayoutPseudo::After),
        style(LayoutDisplay::Contents).with_unsupported_generated_content(),
    );

    let world = build_layout_world(&source, &mut styles).unwrap();
    let children = world.box_by_id(world.root()).unwrap().children();
    assert_eq!(children.len(), 2);
    let before = world.box_by_id(children[0]).unwrap();
    let after = world.box_by_id(children[1]).unwrap();
    assert_eq!(before.kind(), LayoutBoxKind::Text);
    assert_eq!(before.owner(), Some(0));
    assert_eq!(before.pseudo(), Some(LayoutPseudo::Before));
    assert_eq!(before.text(), Some("before"));
    assert_eq!(after.kind(), LayoutBoxKind::Text);
    assert_eq!(after.pseudo(), Some(LayoutPseudo::After));
    assert_eq!(
        after.capability_diagnostics(),
        &[LayoutCapabilityDiagnostic::GeneratedContentUnsupported]
    );
}

#[test]
fn inline_with_block_descendant_is_split_into_owned_continuations() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("span", vec![2, 3, 4]),
            TestNode::text("before", "before"),
            TestNode::element("block", Vec::new()),
            TestNode::text("after", "after"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::Inline));
    styles.primary.insert(3, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let root_children = world.box_by_id(world.root()).unwrap().children();
    assert_eq!(root_children.len(), 3);
    let before_wrapper = world.box_by_id(root_children[0]).unwrap();
    let block = world.box_by_id(root_children[1]).unwrap();
    let after_wrapper = world.box_by_id(root_children[2]).unwrap();
    assert_eq!(before_wrapper.kind(), LayoutBoxKind::AnonymousBlock);
    assert_eq!(block.kind(), LayoutBoxKind::PrincipalBlock);
    assert_eq!(after_wrapper.kind(), LayoutBoxKind::AnonymousBlock);
    let first_fragment = world.box_by_id(before_wrapper.children()[0]).unwrap();
    let continuation = world.box_by_id(after_wrapper.children()[0]).unwrap();
    assert_eq!(first_fragment.kind(), LayoutBoxKind::PrincipalInline);
    assert_eq!(first_fragment.source(), Some(1));
    assert_eq!(continuation.kind(), LayoutBoxKind::InlineContinuation);
    assert_eq!(continuation.source(), None);
    assert_eq!(continuation.owner(), Some(1));
    assert_eq!(
        continuation.anonymous_reason(),
        Some(LayoutAnonymousReason::InlineSplitContinuation)
    );
    assert_eq!(world.source_box(1), Some(before_wrapper.children()[0]));
}

#[test]
fn missing_table_parents_form_a_well_typed_anonymous_chain() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("cell", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::TableCell));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let table = world
        .box_by_id(world.box_by_id(world.root()).unwrap().children()[0])
        .unwrap();
    let row_group = world.box_by_id(table.children()[0]).unwrap();
    let row = world.box_by_id(row_group.children()[0]).unwrap();
    let cell = world.box_by_id(row.children()[0]).unwrap();
    assert_eq!(table.kind(), LayoutBoxKind::AnonymousTableWrapper);
    assert_eq!(row_group.kind(), LayoutBoxKind::AnonymousTableRowGroup);
    assert_eq!(row.kind(), LayoutBoxKind::AnonymousTableRow);
    assert_eq!(cell.kind(), LayoutBoxKind::TableCell);
    assert_eq!(cell.source(), Some(1));
    assert_eq!(table.owner(), Some(0));
    assert_eq!(
        styles.anonymous_queries,
        vec![
            (0, LayoutDisplay::Table),
            (0, LayoutDisplay::TableRowGroup),
            (0, LayoutDisplay::TableRow),
        ]
    );
}

#[test]
fn every_internal_table_role_gets_the_minimal_missing_parent_chain() {
    let cases: &[(LayoutDisplay, &[LayoutDisplay])] = &[
        (
            LayoutDisplay::TableCaption,
            &[LayoutDisplay::Table, LayoutDisplay::TableCaption],
        ),
        (
            LayoutDisplay::TableRowGroup,
            &[LayoutDisplay::Table, LayoutDisplay::TableRowGroup],
        ),
        (
            LayoutDisplay::TableColumnGroup,
            &[LayoutDisplay::Table, LayoutDisplay::TableColumnGroup],
        ),
        (
            LayoutDisplay::TableColumn,
            &[LayoutDisplay::Table, LayoutDisplay::TableColumn],
        ),
        (
            LayoutDisplay::TableRow,
            &[
                LayoutDisplay::Table,
                LayoutDisplay::TableRowGroup,
                LayoutDisplay::TableRow,
            ],
        ),
        (
            LayoutDisplay::TableCell,
            &[
                LayoutDisplay::Table,
                LayoutDisplay::TableRowGroup,
                LayoutDisplay::TableRow,
                LayoutDisplay::TableCell,
            ],
        ),
    ];

    for (source_display, expected_chain) in cases {
        let source = TestSource {
            root: 0,
            nodes: vec![
                TestNode::element("root", vec![1]),
                TestNode::element("table-part", Vec::new()),
            ],
        };
        let mut styles = TestStyles::default();
        styles.primary.insert(0, style(LayoutDisplay::Block));
        styles.primary.insert(1, style(*source_display));

        let world = build_layout_world(&source, &mut styles).unwrap();
        let mut actual = Vec::new();
        let mut current = Some(world.box_by_id(world.root()).unwrap().children()[0]);
        while let Some(id) = current {
            let layout_box = world.box_by_id(id).unwrap();
            actual.push(layout_box.style().display());
            current = layout_box.children().first().copied();
        }
        assert_eq!(&actual, expected_chain, "source display={source_display:?}");
        assert_eq!(
            world
                .box_by_id(world.source_box(1).unwrap())
                .unwrap()
                .style()
                .display(),
            *source_display
        );
    }
}

#[test]
fn collapsible_whitespace_reuses_one_anonymous_table_but_text_breaks_it() {
    let whitespace_source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3]),
            TestNode::element("left-cell", Vec::new()),
            TestNode::text("space", " \n "),
            TestNode::element("right-cell", Vec::new()),
        ],
    };
    let mut whitespace_styles = TestStyles::default();
    whitespace_styles
        .primary
        .insert(0, style(LayoutDisplay::Block));
    whitespace_styles
        .primary
        .insert(1, style(LayoutDisplay::TableCell));
    whitespace_styles
        .primary
        .insert(3, style(LayoutDisplay::TableCell));
    let whitespace_world = build_layout_world(&whitespace_source, &mut whitespace_styles).unwrap();
    let root = whitespace_world.box_by_id(whitespace_world.root()).unwrap();
    assert_eq!(root.children().len(), 1);
    let table = whitespace_world.box_by_id(root.children()[0]).unwrap();
    let row_group = whitespace_world.box_by_id(table.children()[0]).unwrap();
    let row = whitespace_world.box_by_id(row_group.children()[0]).unwrap();
    assert_eq!(row.children().len(), 2);
    assert_eq!(whitespace_world.source_box(2), None);

    let text_source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3]),
            TestNode::element("left-cell", Vec::new()),
            TestNode::text("text", "X"),
            TestNode::element("right-cell", Vec::new()),
        ],
    };
    let mut text_styles = TestStyles::default();
    text_styles.primary.insert(0, style(LayoutDisplay::Block));
    text_styles
        .primary
        .insert(1, style(LayoutDisplay::TableCell));
    text_styles
        .primary
        .insert(3, style(LayoutDisplay::TableCell));
    let text_world = build_layout_world(&text_source, &mut text_styles).unwrap();
    let children = text_world.box_by_id(text_world.root()).unwrap().children();
    assert_eq!(children.len(), 3);
    assert_eq!(
        text_world.box_by_id(children[0]).unwrap().kind(),
        LayoutBoxKind::AnonymousTableWrapper
    );
    assert_eq!(
        text_world.box_by_id(children[1]).unwrap().kind(),
        LayoutBoxKind::AnonymousBlock
    );
    assert_eq!(
        text_world.box_by_id(children[2]).unwrap().kind(),
        LayoutBoxKind::AnonymousTableWrapper
    );
}

#[test]
fn anonymous_table_is_inline_only_for_a_true_inline_flow_parent() {
    for (parent_display, expected_table_display) in [
        (LayoutDisplay::Inline, LayoutDisplay::InlineTable),
        (LayoutDisplay::InlineBlock, LayoutDisplay::Table),
        (LayoutDisplay::InlineFlex, LayoutDisplay::Table),
        (LayoutDisplay::InlineGrid, LayoutDisplay::Table),
    ] {
        let source = TestSource {
            root: 0,
            nodes: vec![
                TestNode::element("root", vec![1]),
                TestNode::element("parent", vec![2]),
                TestNode::element("cell", Vec::new()),
            ],
        };
        let mut styles = TestStyles::default();
        styles.primary.insert(0, style(LayoutDisplay::Block));
        styles.primary.insert(1, style(parent_display));
        styles.primary.insert(2, style(LayoutDisplay::TableCell));

        let world = build_layout_world(&source, &mut styles).unwrap();
        let parent = world.box_by_id(world.source_box(1).unwrap()).unwrap();
        let table = world.box_by_id(parent.children()[0]).unwrap();
        assert_eq!(table.kind(), LayoutBoxKind::AnonymousTableWrapper);
        assert_eq!(table.style().display(), expected_table_display);
    }
}

#[test]
fn column_group_keeps_only_columns_and_columns_are_leaf_boxes() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("table", vec![1]),
            TestNode::element("column-group", vec![2, 3]),
            TestNode::element("column", vec![4]),
            TestNode::element("invalid-child", Vec::new()),
            TestNode::text("column-text", "must-not-layout"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Table));
    styles
        .primary
        .insert(1, style(LayoutDisplay::TableColumnGroup));
    styles.primary.insert(2, style(LayoutDisplay::TableColumn));
    styles.primary.insert(3, style(LayoutDisplay::Block));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let table = world.box_by_id(world.root()).unwrap();
    let column_group = world.box_by_id(table.children()[0]).unwrap();
    assert_eq!(column_group.children().len(), 1);
    let column = world.box_by_id(column_group.children()[0]).unwrap();
    assert_eq!(column.kind(), LayoutBoxKind::TableColumn);
    assert!(column.children().is_empty());
    assert_eq!(world.source_box(3), None);
    assert_eq!(world.source_box(4), None);
}

#[test]
fn display_contents_is_transparent_to_structural_table_fixup() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("table", vec![1]),
            TestNode::element("contents", vec![2]),
            TestNode::element("cell", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Table));
    styles.primary.insert(1, style(LayoutDisplay::Contents));
    styles.primary.insert(2, style(LayoutDisplay::TableCell));

    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(world.source_box(1), None);
    let table = world.box_by_id(world.root()).unwrap();
    let row_group = world.box_by_id(table.children()[0]).unwrap();
    let row = world.box_by_id(row_group.children()[0]).unwrap();
    assert_eq!(row.children(), &[world.source_box(2).unwrap()]);
}

#[test]
fn table_non_structural_content_is_wrapped_through_cell_and_keeps_order() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("table", vec![1, 2]),
            TestNode::text("left", "left"),
            TestNode::text("right", "right"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Table));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let table = world.box_by_id(world.root()).unwrap();
    let row_group = world.box_by_id(table.children()[0]).unwrap();
    let row = world.box_by_id(row_group.children()[0]).unwrap();
    let cell = world.box_by_id(row.children()[0]).unwrap();
    assert_eq!(cell.kind(), LayoutBoxKind::AnonymousTableCell);
    assert!(cell.establishes_inline_formatting_context());
    assert_eq!(cell.children().len(), 2);
    assert_eq!(
        world.box_by_id(cell.children()[0]).unwrap().text(),
        Some("left")
    );
    assert_eq!(
        world.box_by_id(cell.children()[1]).unwrap().text(),
        Some("right")
    );
}

#[test]
fn computed_display_overrides_html_table_semantics() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::semantic_element(
            "table",
            "table",
            LayoutElementCategory::Table(LayoutTableRole::Table),
            None,
            Vec::new(),
        )],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    let world = build_layout_world(&source, &mut styles).unwrap();
    assert_eq!(
        world.box_by_id(world.root()).unwrap().kind(),
        LayoutBoxKind::PrincipalBlock
    );
}

#[test]
fn form_control_kind_and_replaced_state_are_both_retained() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::semantic_element(
            "checkbox",
            "input",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                LayoutInputControlKind::Checkbox,
            )),
            Some(LayoutReplacedKind::FormControl),
            Vec::new(),
        )],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::InlineBlock));
    let world = build_layout_world(&source, &mut styles).unwrap();
    let control = world.box_by_id(world.root()).unwrap();
    assert_eq!(control.kind(), LayoutBoxKind::FormControl);
    assert!(control.capability_diagnostics().is_empty());
}

#[test]
fn replaced_and_line_break_boxes_are_leaves_but_object_fallback_content_is_not() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 3, 5]),
            TestNode::semantic_element(
                "image",
                "img",
                LayoutElementCategory::Generic,
                Some(LayoutReplacedKind::Image),
                vec![2],
            ),
            TestNode::text("image-fallback", "must-not-layout"),
            TestNode::semantic_element(
                "break",
                "br",
                LayoutElementCategory::LineBreak,
                None,
                vec![4],
            ),
            TestNode::text("break-child", "must-not-layout"),
            TestNode::semantic_element(
                "object",
                "object",
                LayoutElementCategory::Generic,
                None,
                vec![6],
            ),
            TestNode::text("object-fallback", "fallback"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::Inline));
    styles.primary.insert(3, style(LayoutDisplay::Inline));
    styles.primary.insert(5, style(LayoutDisplay::InlineBlock));

    let world = build_layout_world(&source, &mut styles).unwrap();
    let image = world.box_by_id(world.source_box(1).unwrap()).unwrap();
    let line_break = world.box_by_id(world.source_box(3).unwrap()).unwrap();
    let object = world.box_by_id(world.source_box(5).unwrap()).unwrap();
    assert_eq!(image.kind(), LayoutBoxKind::Replaced);
    assert!(image.children().is_empty());
    assert_eq!(line_break.kind(), LayoutBoxKind::LineBreak);
    assert!(line_break.children().is_empty());
    assert_eq!(object.kind(), LayoutBoxKind::PrincipalInlineBlock);
    assert_eq!(object.children(), &[world.source_box(6).unwrap()]);
    assert_eq!(world.source_box(2), None);
    assert_eq!(world.source_box(4), None);
    assert_eq!(styles.primary_queries, vec![0, 1, 3, 5]);
    assert!(line_break.capability_diagnostics().is_empty());
}

#[test]
fn pseudo_boxes_have_an_owner_not_a_fake_source_and_keep_empty_content() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("root", Vec::new())],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.pseudo.insert(
        (0, LayoutPseudo::Before),
        style(LayoutDisplay::Inline).with_generated_text(""),
    );
    styles.pseudo.insert(
        (0, LayoutPseudo::After),
        style(LayoutDisplay::Inline).with_generated_text("tail"),
    );

    let world = build_layout_world(&source, &mut styles).unwrap();
    let children = world.box_by_id(world.root()).unwrap().children();
    assert_eq!(children.len(), 2);
    let before = world.box_by_id(children[0]).unwrap();
    let after = world.box_by_id(children[1]).unwrap();
    assert_eq!(before.source(), None);
    assert_eq!(before.owner(), Some(0));
    assert_eq!(before.pseudo(), Some(LayoutPseudo::Before));
    assert_eq!(before.text(), None);
    assert!(before.children().is_empty());
    assert_eq!(after.pseudo(), Some(LayoutPseudo::After));
    assert_eq!(after.text(), None);
    assert_eq!(after.children().len(), 1);
    let after_text = world.box_by_id(after.children()[0]).unwrap();
    assert_eq!(after_text.kind(), LayoutBoxKind::Text);
    assert_eq!(after_text.source(), None);
    assert_eq!(after_text.owner(), Some(0));
    assert_eq!(after_text.pseudo(), Some(LayoutPseudo::After));
    assert_eq!(after_text.text(), Some("tail"));
}

#[test]
fn source_cycle_is_a_structured_error() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("child", vec![0]),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(1, style(LayoutDisplay::Block));

    let error = build_layout_world(&source, &mut styles).unwrap_err();
    assert_eq!(
        error,
        LayoutError::SourceCycle {
            source_label: "root".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "layout source flat tree contains a cycle at root"
    );
}

#[test]
fn normalized_source_dump_locks_element_semantics_before_box_construction() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3, 4, 5]),
            TestNode::semantic_element(
                "break",
                "br",
                LayoutElementCategory::LineBreak,
                None,
                Vec::new(),
            ),
            TestNode::semantic_element(
                "body-group",
                "tbody",
                LayoutElementCategory::Table(LayoutTableRole::BodyGroup),
                None,
                Vec::new(),
            ),
            TestNode::semantic_element(
                "item",
                "li",
                LayoutElementCategory::List(LayoutListRole::Item),
                None,
                Vec::new(),
            ),
            TestNode::semantic_element(
                "control",
                "input",
                LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                    LayoutInputControlKind::Text,
                )),
                Some(LayoutReplacedKind::FormControl),
                Vec::new(),
            ),
            TestNode::text("text", "hello"),
        ],
    };

    assert_eq!(
        normalize_layout_source(&source).unwrap().to_string(),
        concat!(
            "source-tree-schema=1\n",
            "Element path=0 source=root element=html:div category=generic\n",
            "  Element path=0/0 source=break element=html:br category=line-break\n",
            "  Element path=0/1 source=body-group element=html:tbody category=table-row-group\n",
            "  Element path=0/2 source=item element=html:li category=list-item\n",
            "  Element path=0/3 source=control element=html:input category=form-input-text replaced=form-control\n",
            "  Text path=0/4 source=text text=\"hello\"\n",
        )
    );
}

#[test]
fn missing_element_semantics_is_a_structured_source_contract_error() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode {
            label: "root",
            kind: LayoutSourceKind::Element,
            text: None,
            children: Vec::new(),
            element_semantics: None,
            replaced_metrics: None,
        }],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));

    assert_eq!(
        build_layout_world(&source, &mut styles).unwrap_err(),
        LayoutError::SourceContract {
            source_label: "root".to_owned(),
            detail: "element source has no element semantics".to_owned(),
        }
    );
}

#[test]
fn duplicate_flat_tree_child_is_rejected_before_box_identity_can_diverge() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2]),
            TestNode::element("first-parent", vec![3]),
            TestNode::element("second-parent", vec![3]),
            TestNode::element("shared-child", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    for node in 0..4 {
        styles.primary.insert(node, style(LayoutDisplay::Block));
    }

    let error = build_layout_world(&source, &mut styles).unwrap_err();
    assert!(matches!(
        error,
        LayoutError::SourceContract {
            source_label,
            detail,
        } if source_label == "shared-child" && detail.contains("flat_parent")
    ));
}

#[test]
fn taffy_flex_geometry_enters_the_owned_paint_snapshot() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3]),
            TestNode::element("red", Vec::new()),
            TestNode::element("green", Vec::new()),
            TestNode::element("blue", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    let mut root_taffy = Style::<Atom> {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        ..Style::default()
    };
    root_taffy.size.height = Dimension::length(100.0);
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(LayoutDisplay::Flex, root_taffy, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        colored_box(
            LayoutDisplay::Block,
            100.0,
            50.0,
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ),
    );
    styles.primary.insert(
        2,
        colored_box(
            LayoutDisplay::Block,
            100.0,
            50.0,
            PaintColor::new(0.0, 1.0, 0.0, 1.0),
        ),
    );
    styles.primary.insert(
        3,
        colored_box(
            LayoutDisplay::Block,
            100.0,
            50.0,
            PaintColor::new(0.0, 0.0, 1.0, 1.0),
        ),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(400, 200, 1.0)),
    )
    .unwrap();
    assert_eq!(snapshot.canvas_color, PaintColor::WHITE);
    assert_eq!(snapshot.fragments.len(), 3);
    assert_eq!(
        snapshot.fragments[0].solid_fill_in_surface(),
        Some((
            PaintRect::new(0.0, 0.0, 100.0, 50.0),
            PaintColor::new(1.0, 0.0, 0.0, 1.0)
        ))
    );
    assert_eq!(
        snapshot.fragments[1].solid_fill_in_surface(),
        Some((
            PaintRect::new(100.0, 0.0, 100.0, 50.0),
            PaintColor::new(0.0, 1.0, 0.0, 1.0)
        ))
    );
    assert_eq!(
        snapshot.fragments[2].solid_fill_in_surface(),
        Some((
            PaintRect::new(200.0, 0.0, 100.0, 50.0),
            PaintColor::new(0.0, 0.0, 1.0, 1.0)
        ))
    );
}

#[test]
fn reparented_absolute_child_is_not_measured_twice_by_an_inline_context() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("inline-context", vec![2, 3]),
            TestNode::text("text", "in flow"),
            TestNode::element("absolute", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(200.0),
                    height: length(100.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles.primary.insert(1, style(LayoutDisplay::Block));
    styles.primary.insert(
        3,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(30.0),
                    height: length(10.0),
                },
                inset: Rect {
                    left: length(10.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(20.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        )
        .with_position(LayoutPosition::Absolute),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(200, 100, 1.0)),
    )
    .unwrap();
    assert!(
        snapshot
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "positioned-inline-context-deferred")
    );
    assert!(snapshot.fragments.iter().any(|fragment| {
        fragment
            .solid_fill_in_surface()
            .is_some_and(|(rect, color)| {
                rect == PaintRect::new(10.0, 20.0, 30.0, 10.0)
                    && color == PaintColor::new(1.0, 0.0, 0.0, 1.0)
            })
    }));
}

#[test]
fn positioned_child_of_inline_context_uses_its_parley_static_position() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("inline", vec![2, 3]),
            TestNode::text("text", "in flow"),
            TestNode::element("absolute", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(200.0),
                    height: length(100.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles.primary.insert(1, style(LayoutDisplay::Inline));
    styles.primary.insert(
        3,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(30.0),
                    height: length(10.0),
                },
                inset: Rect {
                    left: length(10.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(20.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        )
        .with_position(LayoutPosition::Absolute),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(200, 100, 1.0)),
    )
    .unwrap();
    assert!(snapshot.diagnostics.iter().all(|diagnostic| {
        diagnostic.code != "positioned-inline-context-deferred"
            && diagnostic.code != "positioned-static-position-deferred"
    }));
    assert!(snapshot.fragments.iter().any(|fragment| {
        fragment
            .solid_fill_in_surface()
            .is_some_and(|(rect, color)| {
                rect == PaintRect::new(10.0, 20.0, 30.0, 10.0)
                    && color == PaintColor::new(1.0, 0.0, 0.0, 1.0)
            })
    }));
}

#[test]
fn positioned_inline_containing_block_lays_out_its_absolute_descendant() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2]),
            TestNode::text("text", "in flow"),
            TestNode::element("positioned-inline", vec![3]),
            TestNode::element("absolute", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(
        2,
        style(LayoutDisplay::Inline).with_position(LayoutPosition::Relative),
    );
    styles.primary.insert(
        3,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(30.0),
                    height: length(10.0),
                },
                inset: Rect {
                    left: length(10.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    top: length(20.0),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        )
        .with_position(LayoutPosition::Absolute),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(200, 100, 1.0)),
    )
    .unwrap();
    assert!(snapshot.diagnostics.iter().all(|diagnostic| {
        diagnostic.code != "positioned-inline-context-deferred"
            && diagnostic.code != "positioned-static-position-deferred"
    }));
    assert!(snapshot.fragments.iter().any(|fragment| {
        fragment
            .solid_fill_in_surface()
            .is_some_and(|(rect, color)| {
                color == PaintColor::new(1.0, 0.0, 0.0, 1.0)
                    && rect.width == 30.0
                    && rect.height == 10.0
                    && rect.x >= 10.0
                    && rect.y >= 20.0
            })
    }));
}

#[test]
fn relative_atomic_inline_resolves_insets_against_the_ifc_content_box() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::element("relative-atomic", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(200.0),
                    height: length(40.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    styles.primary.insert(
        1,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::InlineBlock,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(10.0),
                },
                inset: Rect {
                    left: taffy::style_helpers::percent(0.1),
                    right: length(40.0),
                    top: taffy::style_helpers::percent(0.25),
                    bottom: length(20.0),
                },
                ..Style::default()
            },
            red,
        )
        .with_position(LayoutPosition::Relative)
        .with_inline_alignment(LayoutInlineAlignment::Top),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(200, 100, 1.0)),
    )
    .unwrap();
    assert!(snapshot.fragments.iter().any(|fragment| {
        fragment
            .solid_fill_in_surface()
            .is_some_and(|(rect, color)| {
                rect == PaintRect::new(20.0, 10.0, 20.0, 10.0) && color == red
            })
    }));
}

#[test]
fn all_auto_absolute_inline_uses_the_zero_width_placeholder_after_atomic_content() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2]),
            TestNode::element("atomic", Vec::new()),
            TestNode::element("absolute-inline", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(200.0),
                    height: length(100.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_position(LayoutPosition::Relative),
    );
    styles.primary.insert(
        1,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::InlineBlock,
            Style {
                size: Size {
                    width: length(50.0),
                    height: length(10.0),
                },
                ..Style::default()
            },
            PaintColor::new(0.0, 0.0, 1.0, 1.0),
        ),
    );
    styles.primary.insert(
        2,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::InlineBlock,
            Style {
                size: Size {
                    width: length(20.0),
                    height: length(10.0),
                },
                ..Style::default()
            },
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        )
        .with_position(LayoutPosition::Absolute),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(200, 100, 1.0)),
    )
    .unwrap();
    assert!(snapshot.diagnostics.iter().all(|diagnostic| {
        diagnostic.code != "positioned-inline-context-deferred"
            && diagnostic.code != "positioned-static-position-deferred"
    }));
    assert!(snapshot.fragments.iter().any(|fragment| {
        fragment
            .solid_fill_in_surface()
            .is_some_and(|(rect, color)| {
                color == PaintColor::new(1.0, 0.0, 0.0, 1.0)
                    && rect.x == 50.0
                    && rect.width == 20.0
                    && rect.height == 10.0
            })
    }));
}

#[test]
fn replaced_metrics_are_used_without_a_resource_backend() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::replaced("image", 64.0, 32.0),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    styles.primary.insert(
        1,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style::<Atom>::default(),
            PaintColor::BLACK,
        ),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(200, 100, 1.0)),
    )
    .unwrap();
    assert_eq!(
        snapshot.fragments.first(),
        Some(&PaintFragment::solid_rect(
            PaintRect::new(0.0, 0.0, 64.0, 32.0),
            PaintColor::BLACK
        ))
    );
    let light_gray = PaintColor::new(211.0 / 255.0, 211.0 / 255.0, 211.0 / 255.0, 1.0);
    assert!(snapshot.fragments.iter().any(|fragment| {
        matches!(
            fragment,
            PaintFragment::Border { rect, colors, .. }
                if *rect == PaintRect::new(0.0, 0.0, 64.0, 32.0)
                    && colors.top == light_gray
                    && colors.right == light_gray
                    && colors.bottom == light_gray
                    && colors.left == light_gray
        )
    }));
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec!["replaced-content-placeholder"]
    );
}

#[test]
fn parley_contexts_are_lazy_reused_and_project_owned_glyph_runs() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1]),
            TestNode::text("text", "Moli text"),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    let mut services = DocumentLayoutServices::new();
    assert!(!services.is_initialized());

    let first = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut services,
        ScreenshotLayoutRequest::new(PaintViewport::new(240, 80, 1.0)),
    )
    .unwrap();
    assert!(services.is_initialized());
    assert_eq!(services.text_layout_passes(), 1);
    assert!(!first.fonts.is_empty());
    assert!(first.fragments.iter().any(
        |fragment| matches!(fragment, PaintFragment::GlyphRun(run) if !run.glyphs.is_empty())
    ));

    let second = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut services,
        ScreenshotLayoutRequest::new(PaintViewport::new(240, 80, 1.0)),
    )
    .unwrap();
    assert_eq!(services.text_layout_passes(), 2);
    assert_eq!(first, second);
}

#[test]
fn nested_inline_baseline_shift_expands_the_line_once_for_text_and_atomic_content() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 4]),
            TestNode::element("shifted-inline", vec![2, 3]),
            TestNode::text("text", "A"),
            TestNode::element("atomic", Vec::new()),
            TestNode::element("after", Vec::new()),
        ],
    };
    let render = |baseline_shift| {
        let mut styles = TestStyles::default();
        styles.primary.insert(
            0,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: length(100.0),
                        height: Dimension::auto(),
                    },
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            )
            .with_text_metrics(20.0, 20.0),
        );
        styles.primary.insert(
            1,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Inline,
                Style::default(),
                PaintColor::new(0.0, 1.0, 0.0, 1.0),
            )
            // Synthetic styles do not run CSS inheritance; mirror the
            // structural inline's inherited parent font metrics explicitly.
            .with_text_metrics(20.0, 20.0)
            .with_inline_baseline_shift(baseline_shift),
        );
        styles.primary.insert(
            3,
            colored_box(
                LayoutDisplay::InlineBlock,
                10.0,
                10.0,
                PaintColor::new(0.0, 0.0, 1.0, 1.0),
            ),
        );
        styles.primary.insert(
            4,
            colored_box(
                LayoutDisplay::Block,
                20.0,
                5.0,
                PaintColor::new(1.0, 0.0, 0.0, 1.0),
            ),
        );
        build_screenshot_snapshot(
            &source,
            &mut styles,
            &mut DocumentLayoutServices::new(),
            ScreenshotLayoutRequest::new(PaintViewport::new(100, 100, 1.0)),
        )
        .unwrap()
    };
    let baseline = render(0.0);
    let raised = render(10.0);
    let solid_rect = |snapshot: &moli_layout::PaintSnapshot, color| {
        snapshot
            .fragments
            .iter()
            .find_map(|fragment| {
                fragment
                    .solid_fill_in_surface()
                    .filter(|(_, actual)| *actual == color)
                    .map(|(rect, _)| rect)
            })
            .expect("colored fixture box")
    };
    let blue = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    let green = PaintColor::new(0.0, 1.0, 0.0, 1.0);
    let red = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let baseline_atomic = solid_rect(&baseline, blue);
    let raised_atomic = solid_rect(&raised, blue);
    let baseline_inline = solid_rect(&baseline, green);
    let raised_inline = solid_rect(&raised, green);
    let baseline_after = solid_rect(&baseline, red);
    let raised_after = solid_rect(&raised, red);

    assert!(
        (raised_atomic.y - baseline_atomic.y).abs() < 0.01,
        "baseline={baseline_atomic:?}, raised={raised_atomic:?}"
    );
    assert_eq!(raised_inline, baseline_inline);
    assert!(
        (raised_after.y - baseline_after.y - 10.0).abs() < 0.01,
        "baseline={baseline_after:?}, raised={raised_after:?}"
    );
}

#[test]
fn baseline_atomic_inline_keeps_the_parent_strut_descent_in_the_line_box() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2]),
            TestNode::element("atomic", Vec::new()),
            TestNode::element("after", Vec::new()),
        ],
    };
    let atomic_color = PaintColor::new(0.0, 0.0, 1.0, 1.0);
    let after_color = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    let mut styles = TestStyles::default();
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_text_metrics(14.0, 16.0),
    );
    styles.primary.insert(
        1,
        colored_box(LayoutDisplay::InlineBlock, 48.0, 48.0, atomic_color),
    );
    styles
        .primary
        .insert(2, colored_box(LayoutDisplay::Block, 10.0, 5.0, after_color));

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(100, 100, 1.0)),
    )
    .unwrap();
    let rect_for = |color| {
        snapshot
            .fragments
            .iter()
            .find_map(|fragment| {
                fragment
                    .solid_fill_in_surface()
                    .filter(|(_, actual)| *actual == color)
                    .map(|(rect, _)| rect)
            })
            .expect("colored fixture box")
    };
    let atomic = rect_for(atomic_color);
    let after = rect_for(after_color);

    assert!(
        after.y > atomic.y + atomic.height,
        "the parent font descent must remain below a baseline-aligned atomic inline: \
         atomic={atomic:?}, after={after:?}"
    );
}

#[test]
fn top_and_bottom_atomic_inlines_share_the_final_line_edges() {
    let source = TestSource {
        root: 0,
        nodes: vec![
            TestNode::element("root", vec![1, 2, 3, 4]),
            TestNode::text("strut", "A"),
            TestNode::element("top", Vec::new()),
            TestNode::element("bottom", Vec::new()),
            TestNode::element("after", Vec::new()),
        ],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(
        0,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: length(100.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_text_metrics(20.0, 20.0),
    );
    styles.primary.insert(
        2,
        colored_box(
            LayoutDisplay::InlineBlock,
            10.0,
            30.0,
            PaintColor::new(0.0, 0.0, 1.0, 1.0),
        )
        .with_inline_alignment(LayoutInlineAlignment::Top),
    );
    styles.primary.insert(
        3,
        colored_box(
            LayoutDisplay::InlineBlock,
            10.0,
            10.0,
            PaintColor::new(0.0, 1.0, 0.0, 1.0),
        )
        .with_inline_alignment(LayoutInlineAlignment::Bottom),
    );
    styles.primary.insert(
        4,
        colored_box(
            LayoutDisplay::Block,
            10.0,
            5.0,
            PaintColor::new(1.0, 0.0, 0.0, 1.0),
        ),
    );

    let snapshot = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(100, 100, 1.0)),
    )
    .unwrap();
    let rect_for = |color| {
        snapshot
            .fragments
            .iter()
            .find_map(|fragment| {
                fragment
                    .solid_fill_in_surface()
                    .filter(|(_, actual)| *actual == color)
                    .map(|(rect, _)| rect)
            })
            .expect("colored fixture box")
    };
    let top = rect_for(PaintColor::new(0.0, 0.0, 1.0, 1.0));
    let bottom = rect_for(PaintColor::new(0.0, 1.0, 0.0, 1.0));
    let after = rect_for(PaintColor::new(1.0, 0.0, 0.0, 1.0));
    assert!((top.y - 0.0).abs() < 0.01, "top={top:?}, after={after:?}");
    assert!(
        (after.y - 30.0).abs() < 0.01,
        "top={top:?}, bottom={bottom:?}, after={after:?}"
    );
    assert!(
        (bottom.y - 20.0).abs() < 0.01,
        "bottom={bottom:?}, after={after:?}"
    );
    assert!(
        (bottom.y + bottom.height - after.y).abs() < 0.01,
        "bottom={bottom:?}, after={after:?}"
    );
}

#[test]
fn textless_layout_does_not_initialize_parley() {
    let source = TestSource {
        root: 0,
        nodes: vec![TestNode::element("root", Vec::new())],
    };
    let mut styles = TestStyles::default();
    styles.primary.insert(0, style(LayoutDisplay::Block));
    let mut services = DocumentLayoutServices::new();
    let _ = build_screenshot_snapshot(
        &source,
        &mut styles,
        &mut services,
        ScreenshotLayoutRequest::new(PaintViewport::new(20, 20, 1.0)),
    )
    .unwrap();
    assert!(!services.is_initialized());
    assert_eq!(services.text_layout_passes(), 0);
}
