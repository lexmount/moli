use std::{collections::HashMap, sync::Arc};

use moli_layout::{
    DocumentLayoutServices, LayoutDisplay, LayoutElementCategory, LayoutElementMetadata,
    LayoutElementSemantics, LayoutError, LayoutFormControlData, LayoutFormControlKind,
    LayoutImageResource, LayoutInputControlKind, LayoutListData, LayoutListMarkerPosition,
    LayoutListMarkerType, LayoutListRole, LayoutNamespace, LayoutPosition, LayoutPseudo,
    LayoutReplacedKind, LayoutSource, LayoutSourceKind, LayoutStyleResolver, LayoutTableData,
    LayoutTableRole, LayoutTextSelection, PaintBrush, PaintColor, PaintFragment, PaintRect,
    PaintShape, PaintSnapshot, PaintTransform2D, PaintViewport, ReplacedMetrics,
    ResolvedLayoutStyle, ScreenshotLayoutRequest, build_screenshot_snapshot,
};
use style::Atom;
use taffy::{
    BoxSizing, Clear, Dimension, FlexDirection, Float, Overflow, Point, Rect, Size, Style,
};

const RED: PaintColor = PaintColor::new(0.9, 0.1, 0.1, 1.0);
const GREEN: PaintColor = PaintColor::new(0.1, 0.7, 0.2, 1.0);
const BLUE: PaintColor = PaintColor::new(0.1, 0.25, 0.8, 1.0);
const YELLOW: PaintColor = PaintColor::new(0.95, 0.8, 0.1, 1.0);

#[derive(Clone)]
struct Node {
    label: &'static str,
    kind: LayoutSourceKind,
    semantics: Option<LayoutElementSemantics>,
    text: Option<&'static str>,
    children: Vec<usize>,
    metrics: Option<ReplacedMetrics>,
    image: Option<LayoutImageResource>,
    selection: Option<LayoutTextSelection>,
}

impl Node {
    fn element(
        label: &'static str,
        local_name: &'static str,
        category: LayoutElementCategory,
        replaced: Option<LayoutReplacedKind>,
        children: Vec<usize>,
    ) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Element,
            semantics: Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                local_name,
                category,
                replaced,
            )),
            text: None,
            children,
            metrics: None,
            image: None,
            selection: None,
        }
    }

    fn text(label: &'static str, text: &'static str) -> Self {
        Self {
            label,
            kind: LayoutSourceKind::Text,
            semantics: None,
            text: Some(text),
            children: Vec::new(),
            metrics: None,
            image: None,
            selection: None,
        }
    }

    fn with_metadata(mut self, metadata: LayoutElementMetadata) -> Self {
        self.semantics.as_mut().unwrap().metadata = metadata;
        self
    }

    fn with_selection(mut self, selection: LayoutTextSelection) -> Self {
        self.selection = Some(selection);
        self
    }

    fn with_metrics(mut self, metrics: ReplacedMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    fn with_image(mut self, image: LayoutImageResource) -> Self {
        self.image = Some(image);
        self
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
        if node == 0 {
            return None;
        }
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
        self.0[node].semantics.clone()
    }

    fn text(&self, node: Self::NodeId) -> Option<&str> {
        self.0[node].text
    }

    fn label(&self, node: Self::NodeId) -> String {
        self.0[node].label.to_owned()
    }

    fn text_selection(&self, node: Self::NodeId) -> Option<LayoutTextSelection> {
        self.0[node].selection
    }

    fn replaced_metrics(&self, node: Self::NodeId) -> Option<ReplacedMetrics> {
        self.0[node].metrics
    }

    fn replaced_image(
        &self,
        node: Self::NodeId,
        _style: &ResolvedLayoutStyle,
    ) -> Option<LayoutImageResource> {
        self.0[node].image.clone()
    }
}

#[derive(Default)]
struct Styles {
    primary: HashMap<usize, ResolvedLayoutStyle>,
    pseudo: HashMap<(usize, LayoutPseudo), ResolvedLayoutStyle>,
}

impl LayoutStyleResolver<usize> for Styles {
    fn primary_style(&mut self, node: usize) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        Ok(self.primary.get(&node).cloned())
    }

    fn pseudo_style(
        &mut self,
        node: usize,
        pseudo: LayoutPseudo,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        Ok(self.pseudo.get(&(node, pseudo)).cloned())
    }
}

fn style(display: LayoutDisplay, color: PaintColor) -> ResolvedLayoutStyle {
    ResolvedLayoutStyle::synthetic(display, Style::default(), color)
}

fn sized(
    display: LayoutDisplay,
    width: f32,
    height: f32,
    color: PaintColor,
) -> ResolvedLayoutStyle {
    style(display, color)
        .with_position(LayoutPosition::Static)
        .tap_taffy(|taffy| {
            taffy.size = Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            };
        })
}

trait StyleTestExt {
    fn tap_taffy(self, update: impl FnOnce(&mut Style<Atom>)) -> Self;
}

impl StyleTestExt for ResolvedLayoutStyle {
    fn tap_taffy(mut self, update: impl FnOnce(&mut Style<Atom>)) -> Self {
        // Synthetic tests deliberately need direct numeric inputs. Recreate
        // through the public constructor because the retained Taffy style is
        // otherwise correctly encapsulated.
        let display = self.display();
        let color = self.background_color();
        let mut taffy = Style::default();
        update(&mut taffy);
        self = ResolvedLayoutStyle::synthetic(display, taffy, color);
        self
    }
}

fn render(source: &Source, styles: &mut Styles, width: u32, height: u32) -> PaintSnapshot {
    build_screenshot_snapshot(
        source,
        styles,
        &mut DocumentLayoutServices::new(),
        ScreenshotLayoutRequest::new(PaintViewport::new(width, height, 1.0)),
    )
    .unwrap()
}

fn rect(snapshot: &PaintSnapshot, color: PaintColor) -> PaintRect {
    snapshot
        .fragments
        .iter()
        .find_map(|fragment| {
            fragment
                .solid_fill_in_surface()
                .filter(|(_, actual)| *actual == color)
                .map(|(rect, _)| rect)
        })
        .unwrap_or_else(|| panic!("missing {color:?} in {:?}", snapshot.fragments))
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.05, "{actual} != {expected}");
}

#[test]
fn table_caption_tracks_rows_cells_and_common_spans_share_one_wrapper_geometry() {
    use LayoutElementCategory::Table;
    use LayoutTableRole::{BodyGroup, Caption, Cell, Row, Table as Root};

    let cell = |label, metadata| {
        Node::element(label, "td", Table(Cell), None, Vec::new()).with_metadata(
            LayoutElementMetadata {
                table: Some(metadata),
                ..LayoutElementMetadata::default()
            },
        )
    };
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element("table", "table", Table(Root), None, vec![2, 3]),
        Node::element("caption", "caption", Table(Caption), None, Vec::new()),
        Node::element("tbody", "tbody", Table(BodyGroup), None, vec![4, 7]),
        Node::element("row-one", "tr", Table(Row), None, vec![5, 6]),
        cell("cell-a", LayoutTableData::default()),
        cell("cell-b", LayoutTableData::default()),
        Node::element("row-two", "tr", Table(Row), None, vec![8]),
        cell(
            "cell-span",
            LayoutTableData {
                column_span: 2,
                ..LayoutTableData::default()
            },
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 400.0, 200.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        sized(
            LayoutDisplay::Table,
            300.0,
            70.0,
            PaintColor::new(0.7, 0.7, 0.7, 1.0),
        ),
    );
    styles
        .primary
        .insert(2, sized(LayoutDisplay::TableCaption, 300.0, 20.0, YELLOW));
    styles.primary.insert(
        3,
        style(LayoutDisplay::TableRowGroup, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(4, style(LayoutDisplay::TableRow, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(5, sized(LayoutDisplay::TableCell, 100.0, 30.0, RED));
    styles
        .primary
        .insert(6, sized(LayoutDisplay::TableCell, 200.0, 30.0, GREEN));
    styles
        .primary
        .insert(7, style(LayoutDisplay::TableRow, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(8, sized(LayoutDisplay::TableCell, 300.0, 40.0, BLUE));

    let snapshot = render(&source, &mut styles, 400, 200);
    let caption = rect(&snapshot, YELLOW);
    let first = rect(&snapshot, RED);
    let second = rect(&snapshot, GREEN);
    let span = rect(&snapshot, BLUE);
    assert_close(caption.width, 300.0);
    assert_close(caption.y, 0.0);
    assert_close(first.x, 0.0);
    assert_close(first.y, 20.0);
    assert_close(first.width, 100.0);
    assert_close(second.x, 100.0);
    assert_close(second.width, 200.0);
    assert_close(span.x, 0.0);
    assert_close(span.y, 50.0);
    assert_close(span.width, 300.0);
}

#[test]
fn table_sections_use_visual_header_body_footer_order() {
    use LayoutElementCategory::Table;
    use LayoutTableRole::{BodyGroup, Cell, FooterGroup, HeaderGroup, Row, Table as Root};

    let cell = |label| {
        Node::element(label, "td", Table(Cell), None, Vec::new()).with_metadata(
            LayoutElementMetadata {
                table: Some(LayoutTableData::default()),
                ..LayoutElementMetadata::default()
            },
        )
    };
    // Deliberately place tbody and tfoot before thead in tree order. CSS table
    // layout promotes the first header group before all bodies and the first
    // footer group after them.
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element("table", "table", Table(Root), None, vec![2, 6, 10]),
        Node::element("body", "tbody", Table(BodyGroup), None, vec![3]),
        Node::element("body-row", "tr", Table(Row), None, vec![4, 5]),
        cell("body-a"),
        cell("body-b"),
        Node::element("footer", "tfoot", Table(FooterGroup), None, vec![7]),
        Node::element("footer-row", "tr", Table(Row), None, vec![8, 9]),
        cell("footer-a"),
        cell("footer-b"),
        Node::element("header", "thead", Table(HeaderGroup), None, vec![11]),
        Node::element("header-row", "tr", Table(Row), None, vec![12, 13]),
        cell("header-a"),
        cell("header-b"),
    ]);
    let body_a = PaintColor::new(0.11, 0.12, 0.13, 1.0);
    let body_b = PaintColor::new(0.21, 0.22, 0.23, 1.0);
    let footer_a = PaintColor::new(0.31, 0.32, 0.33, 1.0);
    let footer_b = PaintColor::new(0.41, 0.42, 0.43, 1.0);
    let header_a = PaintColor::new(0.51, 0.52, 0.53, 1.0);
    let header_b = PaintColor::new(0.61, 0.62, 0.63, 1.0);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        sized(LayoutDisplay::Table, 100.0, 30.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        2,
        style(LayoutDisplay::TableRowGroup, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(3, style(LayoutDisplay::TableRow, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(4, sized(LayoutDisplay::TableCell, 20.0, 10.0, body_a));
    styles
        .primary
        .insert(5, sized(LayoutDisplay::TableCell, 80.0, 10.0, body_b));
    styles.primary.insert(
        6,
        style(LayoutDisplay::TableFooterGroup, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(7, style(LayoutDisplay::TableRow, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(8, sized(LayoutDisplay::TableCell, 50.0, 10.0, footer_a));
    styles
        .primary
        .insert(9, sized(LayoutDisplay::TableCell, 50.0, 10.0, footer_b));
    styles.primary.insert(
        10,
        style(LayoutDisplay::TableHeaderGroup, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(11, style(LayoutDisplay::TableRow, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(12, sized(LayoutDisplay::TableCell, 75.0, 10.0, header_a));
    styles
        .primary
        .insert(13, sized(LayoutDisplay::TableCell, 25.0, 10.0, header_b));

    let snapshot = render(&source, &mut styles, 200, 100);
    for (color, expected) in [
        (header_a, (0.0, 0.0, 75.0)),
        (header_b, (75.0, 0.0, 25.0)),
        (body_a, (0.0, 10.0, 75.0)),
        (body_b, (75.0, 10.0, 25.0)),
        (footer_a, (0.0, 20.0, 75.0)),
        (footer_b, (75.0, 20.0, 25.0)),
    ] {
        let actual = rect(&snapshot, color);
        assert_close(actual.x, expected.0);
        assert_close(actual.y, expected.1);
        assert_close(actual.width, expected.2);
        assert_close(actual.height, 10.0);
    }
}

#[test]
fn left_float_restricts_inline_slots_and_clear_moves_the_next_block_below_it() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2],
        ),
        Node::element(
            "float",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "atom",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        sized(LayoutDisplay::Block, 60.0, 40.0, BLUE).with_float(Float::Left, Clear::None),
    );
    styles
        .primary
        .insert(2, sized(LayoutDisplay::InlineBlock, 100.0, 20.0, RED));
    let snapshot = render(&source, &mut styles, 200, 100);
    assert_close(rect(&snapshot, BLUE).x, 0.0);
    assert_close(rect(&snapshot, RED).x, 60.0);

    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2],
        ),
        Node::element(
            "float",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "clear",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        sized(LayoutDisplay::Block, 60.0, 40.0, BLUE).with_float(Float::Left, Clear::None),
    );
    styles.primary.insert(
        2,
        sized(LayoutDisplay::Block, 100.0, 10.0, GREEN).with_float(Float::None, Clear::Both),
    );
    let snapshot = render(&source, &mut styles, 200, 100);
    assert_close(rect(&snapshot, GREEN).y, 40.0);
}

#[test]
fn float_descendant_of_structural_inline_rounds_with_its_ifc_owner() {
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element("link", "a", LayoutElementCategory::Generic, None, vec![2]),
        Node::element(
            "float",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![3],
        ),
        Node::element(
            "image",
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            intrinsic_width: Some(166.0),
            intrinsic_height: Some(42.0),
            intrinsic_ratio: Some(166.0 / 42.0),
            ..ReplacedMetrics::default()
        }),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 40.0, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(1, style(LayoutDisplay::Inline, PaintColor::TRANSPARENT));
    styles.primary.insert(
        2,
        sized(LayoutDisplay::Block, 60.0, 20.0, BLUE).with_float(Float::Left, Clear::None),
    );
    styles.primary.insert(
        3,
        sized(LayoutDisplay::Inline, 55.0, 14.0, GREEN)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Top),
    );

    let snapshot = render(&source, &mut styles, 200, 40);
    let floated = rect(&snapshot, BLUE);
    let image = rect(&snapshot, GREEN);
    assert_close(floated.x, 0.0);
    assert_close(floated.y, 0.0);
    assert_close(floated.width, 60.0);
    assert_close(floated.height, 20.0);
    assert_close(image.x, 0.0);
    assert_close(image.y, 0.0);
    assert_close(image.width, 55.0);
    assert_close(image.height, 14.0);
}

#[test]
fn block_content_alignment_moves_the_inline_float_fragment_group() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 3],
        ),
        Node::element(
            "float",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "float-child",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "atom",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        style(LayoutDisplay::Block, PaintColor::TRANSPARENT).tap_taffy(|taffy| {
            taffy.size = Size {
                width: Dimension::length(200.0),
                height: Dimension::length(100.0),
            };
            taffy.align_content = Some(taffy::AlignContent::SAFE_CENTER);
        }),
    );
    styles.primary.insert(
        1,
        sized(LayoutDisplay::Block, 60.0, 20.0, BLUE).with_float(Float::Left, Clear::None),
    );
    styles
        .primary
        .insert(2, sized(LayoutDisplay::Block, 10.0, 5.0, RED));
    styles.primary.insert(
        3,
        sized(LayoutDisplay::InlineBlock, 20.0, 10.0, GREEN)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Top),
    );

    let snapshot = render(&source, &mut styles, 200, 100);
    let floated = rect(&snapshot, BLUE);
    let floated_child = rect(&snapshot, RED);
    let atom = rect(&snapshot, GREEN);
    // The float remains outside normal-flow auto height, but Chromium includes
    // its 20px block-end extent in the single align-content subject.
    assert_close(floated.y, 40.0);
    assert_close(floated_child.y, 40.0);
    assert_close(atom.y, 40.0);
    assert_close(floated_child.y - floated.y, 0.0);
}

#[test]
fn shrink_to_fit_inline_context_accounts_for_left_and_right_float_bands() {
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element(
            "float-root",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2, 3, 4],
        ),
        Node::element(
            "left",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "right",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "zero-width-atom",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(1, style(LayoutDisplay::InlineBlock, YELLOW));
    styles.primary.insert(
        2,
        style(LayoutDisplay::Block, BLUE)
            .tap_taffy(|taffy| {
                taffy.size = Size {
                    width: Dimension::length(50.0),
                    height: Dimension::length(40.0),
                };
                taffy.margin.right = taffy::LengthPercentageAuto::length(10.0);
            })
            .with_float(Float::Left, Clear::None),
    );
    styles.primary.insert(
        3,
        style(LayoutDisplay::Block, GREEN)
            .tap_taffy(|taffy| {
                taffy.size = Size {
                    width: Dimension::length(40.0),
                    height: Dimension::length(30.0),
                };
                taffy.margin.left = taffy::LengthPercentageAuto::length(10.0);
            })
            .with_float(Float::Right, Clear::None),
    );
    styles.primary.insert(
        4,
        sized(
            LayoutDisplay::InlineBlock,
            0.0,
            0.0,
            PaintColor::TRANSPARENT,
        ),
    );

    let snapshot = render(&source, &mut styles, 300, 100);
    let float_root = rect(&snapshot, YELLOW);
    assert_close(float_root.width, 110.0);
    assert_close(float_root.height, 40.0);
    assert_close(rect(&snapshot, BLUE).x, 0.0);
    assert_close(rect(&snapshot, GREEN).x, 70.0);
}

#[test]
fn html_list_metadata_and_marker_style_produce_inside_and_outside_glyph_geometry() {
    let source = Source(vec![
        Node::element(
            "list",
            "ol",
            LayoutElementCategory::List(LayoutListRole::Container),
            None,
            vec![1, 3, 5],
        )
        .with_metadata(LayoutElementMetadata {
            list: Some(LayoutListData {
                ordered: true,
                start: Some(3),
                reversed: false,
                value: None,
            }),
            ..LayoutElementMetadata::default()
        }),
        Node::element(
            "first",
            "li",
            LayoutElementCategory::List(LayoutListRole::Item),
            None,
            vec![2],
        ),
        Node::text("first-text", "first"),
        Node::element(
            "second",
            "li",
            LayoutElementCategory::List(LayoutListRole::Item),
            None,
            vec![4],
        )
        .with_metadata(LayoutElementMetadata {
            list: Some(LayoutListData {
                value: Some(9),
                ..LayoutListData::default()
            }),
            ..LayoutElementMetadata::default()
        }),
        Node::text("second-text", "second"),
        Node::element(
            "custom",
            "li",
            LayoutElementCategory::List(LayoutListRole::Item),
            None,
            vec![6],
        ),
        Node::text("custom-text", "custom"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 240.0, 100.0, PaintColor::TRANSPARENT),
    );
    for (item, color) in [(1, RED), (3, GREEN)] {
        styles.primary.insert(
            item,
            style(LayoutDisplay::BlockListItem, color).with_list_marker(
                LayoutListMarkerType::Decimal,
                LayoutListMarkerPosition::Outside,
            ),
        );
        styles.pseudo.insert(
            (item, LayoutPseudo::Marker),
            style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_normal_generated_content(),
        );
    }
    styles.primary.insert(
        5,
        style(LayoutDisplay::BlockListItem, BLUE).with_list_marker(
            LayoutListMarkerType::None,
            LayoutListMarkerPosition::Outside,
        ),
    );
    styles.pseudo.insert(
        (5, LayoutPseudo::Marker),
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_generated_text("X "),
    );
    let snapshot = render(&source, &mut styles, 240, 100);
    let glyph_count = snapshot
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment, PaintFragment::GlyphRun(_)))
        .count();
    assert!(
        glyph_count >= 6,
        "expected item and marker glyph runs: {:?}",
        snapshot.fragments
    );
    let first = rect(&snapshot, RED);
    let second = rect(&snapshot, GREEN);
    let custom = rect(&snapshot, BLUE);
    assert_close(second.y, first.y + first.height);
    assert_close(custom.y, second.y + second.height);
    assert!(
        snapshot.fragments.iter().any(|fragment| {
            matches!(fragment, PaintFragment::GlyphRun(run) if run.glyphs_in_surface().iter().any(|glyph| glyph.x < first.x))
        }),
        "outside markers must own geometry in the marker gutter: {:?}",
        snapshot.fragments
    );
    assert!(
        snapshot
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "list-marker-layout-deferred")
    );
}

#[test]
fn degenerate_css_ratio_falls_back_to_the_replaced_intrinsic_ratio() {
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element(
            "image",
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            intrinsic_width: Some(80.0),
            intrinsic_height: Some(40.0),
            attribute_width: None,
            attribute_height: None,
            intrinsic_ratio: Some(2.0),
        }),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 200.0, PaintColor::TRANSPARENT),
    );
    let image_style = ResolvedLayoutStyle::synthetic(
        LayoutDisplay::Block,
        Style {
            size: Size {
                width: Dimension::length(120.0),
                height: Dimension::auto(),
            },
            aspect_ratio: Some(0.0),
            ..Style::default()
        },
        BLUE,
    );
    styles.primary.insert(1, image_style);
    let snapshot = render(&source, &mut styles, 300, 200);
    let image = rect(&snapshot, BLUE);
    assert_close(image.width, 120.0);
    assert_close(image.height, 60.0);
    assert!(
        snapshot
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "replaced-content-placeholder")
    );
}

/// Regression for
/// <https://wpt.live/css/css-sizing/image-fractional-height-with-wide-aspect-ratio.html>.
#[test]
fn fractional_replaced_images_project_contiguous_pre_transform_destinations() {
    let svg = Arc::new(
        moli_image::decode_svg_image(
            br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 12"><rect width="96" height="12" fill="green"/></svg>"##,
        )
        .expect("fixture SVG should parse"),
    );
    let image = LayoutImageResource {
        intrinsic_width: 96.0,
        intrinsic_height: 12.0,
        pixels: None,
        svg: Some(svg),
    };
    let labels = [
        "row-1", "row-2", "row-3", "row-4", "row-5", "row-6", "row-7", "row-8",
    ];
    let mut nodes = vec![Node::element(
        "root",
        "div",
        LayoutElementCategory::Generic,
        None,
        (1..=8).collect(),
    )];
    nodes.extend(labels.into_iter().map(|label| {
        Node::element(
            label,
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            intrinsic_width: Some(96.0),
            intrinsic_height: Some(12.0),
            intrinsic_ratio: Some(8.0),
            ..ReplacedMetrics::default()
        })
        .with_image(image.clone())
    }));
    let source = Source(nodes);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 100.0, 100.0, PaintColor::TRANSPARENT),
    );
    for node in 1..=8 {
        styles.primary.insert(
            node,
            sized(LayoutDisplay::Block, 100.0, 12.5, PaintColor::TRANSPARENT),
        );
    }

    let snapshot = render(&source, &mut styles, 100, 101);
    let images = snapshot
        .fragments
        .iter()
        .filter_map(|fragment| match fragment {
            PaintFragment::SvgImage(image) => Some(image),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(images.len(), 8);
    for (row, image) in images.into_iter().enumerate() {
        assert_eq!(
            image.destination,
            PaintRect::new(0.0, row as f32 * 12.5, 100.0, 12.5)
        );
        assert_eq!(image.transform, PaintTransform2D::IDENTITY);
    }
}

#[test]
fn replaced_attributes_canvas_defaults_and_image_button_share_resource_free_sizing() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2, 3],
        ),
        Node::element(
            "image",
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            attribute_width: Some(80.0),
            attribute_height: Some(40.0),
            ..ReplacedMetrics::default()
        }),
        Node::element(
            "canvas",
            "canvas",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Canvas),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            attribute_width: Some(600.0),
            ..ReplacedMetrics::default()
        }),
        Node::element(
            "image-button",
            "input",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                LayoutInputControlKind::Image,
            )),
            Some(LayoutReplacedKind::FormControl),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            attribute_width: Some(90.0),
            attribute_height: Some(45.0),
            ..ReplacedMetrics::default()
        }),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 700.0, 400.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::length(120.0),
                    height: Dimension::auto(),
                },
                ..Style::default()
            },
            RED,
        ),
    );
    styles.primary.insert(2, style(LayoutDisplay::Block, GREEN));
    styles.primary.insert(3, style(LayoutDisplay::Block, BLUE));

    let snapshot = render(&source, &mut styles, 700, 400);
    assert_eq!(rect(&snapshot, RED), PaintRect::new(0.0, 0.0, 120.0, 60.0));
    assert_eq!(
        rect(&snapshot, GREEN),
        PaintRect::new(0.0, 60.0, 600.0, 150.0)
    );
    assert_eq!(
        rect(&snapshot, BLUE),
        PaintRect::new(0.0, 210.0, 90.0, 45.0)
    );
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "replaced-content-placeholder")
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "canvas-content-unavailable")
            .count(),
        1
    );
}

#[test]
fn unavailable_images_use_zero_default_size_and_a_content_box_outline() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2, 3, 4],
        ),
        Node::element(
            "canvas",
            "canvas",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Canvas),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics::default()),
        Node::element(
            "sized-image",
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics {
            attribute_width: Some(80.0),
            attribute_height: Some(40.0),
            ..ReplacedMetrics::default()
        }),
        Node::element(
            "unsized-image",
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        )
        .with_metrics(ReplacedMetrics::default()),
        Node::element(
            "following-block",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 300.0, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(1, style(LayoutDisplay::Block, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(2, style(LayoutDisplay::Block, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(3, style(LayoutDisplay::Block, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(4, sized(LayoutDisplay::Block, 50.0, 10.0, YELLOW));

    let snapshot = render(&source, &mut styles, 300, 300);
    let old_placeholder = PaintColor::new(0.82, 0.84, 0.87, 1.0);
    assert!(
        snapshot
            .fragments
            .iter()
            .filter_map(PaintFragment::solid_fill_in_surface)
            .all(|(_, color)| color != old_placeholder)
    );
    assert_eq!(
        rect(&snapshot, YELLOW),
        PaintRect::new(0.0, 190.0, 50.0, 10.0)
    );
    let light_gray = PaintColor::new(211.0 / 255.0, 211.0 / 255.0, 211.0 / 255.0, 1.0);
    assert!(
        snapshot.fragments.iter().any(|fragment| {
            matches!(
                fragment,
                PaintFragment::Border {
                    rect,
                    widths,
                    colors,
                    transform,
                    ..
                } if rect.width == 80.0
                    && rect.height == 40.0
                    && *widths == moli_layout::PaintEdgeSizes::new(1.0, 1.0, 1.0, 1.0)
                    && colors.top == light_gray
                    && colors.right == light_gray
                    && colors.bottom == light_gray
                    && colors.left == light_gray
                    && transform.map_point(moli_layout::LayoutPoint::new(rect.x, rect.y))
                        == moli_layout::LayoutPoint::new(0.0, 150.0)
            )
        }),
        "missing Chromium-style unavailable image outline: {:?}",
        snapshot.fragments
    );
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "canvas-content-unavailable")
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "replaced-content-placeholder")
            .count(),
        2
    );
}

#[test]
fn inline_blocks_use_their_internal_last_line_baseline_and_overflow_fallback() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 3, 5],
        ),
        Node::element("news", "a", LayoutElementCategory::Generic, None, vec![2]),
        Node::text("news-text", "News"),
        Node::element("hao", "a", LayoutElementCategory::Generic, None, vec![4]),
        Node::text("hao-text", "hao123"),
        Node::element("more", "div", LayoutElementCategory::Generic, None, vec![6]),
        Node::element(
            "more-link",
            "a",
            LayoutElementCategory::Generic,
            None,
            vec![7],
        ),
        Node::text("more-text", "More"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 100.0, PaintColor::TRANSPARENT)
            .with_text_metrics(12.0, 19.2),
    );
    let nav_item_style = |color, padding_bottom, overflow| {
        style(LayoutDisplay::InlineBlock, color)
            .tap_taffy(|taffy| {
                taffy.margin = Rect {
                    left: taffy::LengthPercentageAuto::length(0.0),
                    right: taffy::LengthPercentageAuto::length(24.0),
                    top: taffy::LengthPercentageAuto::length(19.0),
                    bottom: taffy::LengthPercentageAuto::length(0.0),
                };
                taffy.padding.bottom = taffy::LengthPercentage::length(padding_bottom);
                taffy.overflow = Point {
                    x: overflow,
                    y: overflow,
                };
            })
            .with_text_metrics(13.0, 23.0)
    };
    for (node, color, padding_bottom) in [(1, RED, 0.0), (3, GREEN, 0.0), (5, BLUE, 19.0)] {
        styles.primary.insert(
            node,
            nav_item_style(color, padding_bottom, Overflow::Visible),
        );
    }
    styles.primary.insert(
        6,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(13.0, 23.0),
    );

    let snapshot = render(&source, &mut styles, 300, 100);
    let news = rect(&snapshot, RED);
    let hao = rect(&snapshot, GREEN);
    let more = rect(&snapshot, BLUE);
    assert_close(news.y, 19.0);
    assert_close(hao.y, news.y);
    assert_close(more.y, news.y);
    assert_close(news.height, 23.0);
    assert_close(hao.height, 23.0);
    assert_close(more.height, 42.0);

    styles
        .primary
        .insert(5, nav_item_style(BLUE, 19.0, Overflow::Hidden));
    let fallback = render(&source, &mut styles, 300, 100);
    assert_close(rect(&fallback, RED).y, 45.0);
    assert_close(rect(&fallback, GREEN).y, 45.0);
    assert_close(rect(&fallback, BLUE).y, 19.0);
}

#[test]
fn inline_flex_and_grid_use_their_first_container_baseline() {
    for atomic_display in [LayoutDisplay::InlineFlex, LayoutDisplay::InlineGrid] {
        let source = Source(vec![
            Node::element(
                "root",
                "div",
                LayoutElementCategory::Generic,
                None,
                vec![1, 4],
            ),
            Node::element(
                "atomic",
                "span",
                LayoutElementCategory::Generic,
                None,
                vec![2, 3],
            ),
            Node::element(
                "first-item",
                "span",
                LayoutElementCategory::Generic,
                None,
                Vec::new(),
            ),
            Node::element(
                "last-item",
                "span",
                LayoutElementCategory::Generic,
                None,
                Vec::new(),
            ),
            Node::element(
                "tail",
                "span",
                LayoutElementCategory::Generic,
                None,
                Vec::new(),
            ),
        ]);
        let mut styles = Styles::default();
        styles.primary.insert(
            0,
            style(LayoutDisplay::Block, PaintColor::TRANSPARENT)
                .tap_taffy(|taffy| taffy.size.width = Dimension::length(100.0))
                .with_text_metrics(0.0, 0.0),
        );
        styles.primary.insert(
            1,
            style(atomic_display, YELLOW).tap_taffy(|taffy| {
                taffy.size.width = Dimension::length(10.0);
                if atomic_display == LayoutDisplay::InlineFlex {
                    taffy.flex_direction = FlexDirection::Column;
                } else {
                    taffy.grid_template_columns = vec![taffy::style_helpers::length(10.0)];
                }
            }),
        );
        styles
            .primary
            .insert(2, sized(LayoutDisplay::Block, 10.0, 10.0, GREEN));
        styles
            .primary
            .insert(3, sized(LayoutDisplay::Block, 10.0, 10.0, BLUE));
        styles
            .primary
            .insert(4, sized(LayoutDisplay::InlineBlock, 10.0, 10.0, RED));

        let snapshot = render(&source, &mut styles, 100, 60);
        let atomic = rect(&snapshot, YELLOW);
        let tail = rect(&snapshot, RED);
        assert_close(atomic.width, 10.0);
        assert_close(atomic.height, 20.0);
        assert_close(tail.y, atomic.y);
    }
}

#[test]
fn negative_atomic_block_margin_uses_signed_baseline_metrics() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 4],
        ),
        Node::element(
            "atomic",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2, 3],
        ),
        Node::element(
            "first-item",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "last-item",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "tail",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        style(LayoutDisplay::Block, GREEN)
            .tap_taffy(|taffy| taffy.size.width = Dimension::length(100.0))
            .with_text_metrics(0.0, 0.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::InlineFlex, YELLOW).tap_taffy(|taffy| {
            taffy.size.width = Dimension::length(10.0);
            taffy.flex_direction = FlexDirection::Column;
            taffy.margin.top = taffy::LengthPercentageAuto::length(-5.0);
        }),
    );
    styles
        .primary
        .insert(2, sized(LayoutDisplay::Block, 10.0, 10.0, BLUE));
    styles.primary.insert(
        3,
        sized(LayoutDisplay::Block, 10.0, 10.0, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(4, sized(LayoutDisplay::InlineBlock, 10.0, 10.0, RED));

    let snapshot = render(&source, &mut styles, 100, 60);
    let root = rect(&snapshot, GREEN);
    let atomic = rect(&snapshot, YELLOW);
    let tail = rect(&snapshot, RED);
    assert_close(root.height, 20.0);
    assert_close(atomic.height, 20.0);
    assert_close(atomic.y, tail.y);
}

#[test]
fn inline_table_baseline_includes_top_caption_offset() {
    use LayoutElementCategory::Table;
    use LayoutTableRole::{BodyGroup, Caption, Cell, Row, Table as Root};

    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 6],
        ),
        Node::element("table", "table", Table(Root), None, vec![2, 3]),
        Node::element("caption", "caption", Table(Caption), None, Vec::new()),
        Node::element("tbody", "tbody", Table(BodyGroup), None, vec![4]),
        Node::element("row", "tr", Table(Row), None, vec![5]),
        Node::element("cell", "td", Table(Cell), None, Vec::new()),
        Node::element(
            "tail",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        style(LayoutDisplay::Block, PaintColor::TRANSPARENT)
            .tap_taffy(|taffy| taffy.size.width = Dimension::length(100.0))
            .with_text_metrics(0.0, 0.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::InlineTable, YELLOW)
            .tap_taffy(|taffy| taffy.size.width = Dimension::length(10.0)),
    );
    styles
        .primary
        .insert(2, sized(LayoutDisplay::TableCaption, 10.0, 10.0, GREEN));
    styles.primary.insert(
        3,
        style(LayoutDisplay::TableRowGroup, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(4, style(LayoutDisplay::TableRow, PaintColor::TRANSPARENT));
    styles
        .primary
        .insert(5, sized(LayoutDisplay::TableCell, 10.0, 10.0, BLUE));
    styles
        .primary
        .insert(6, sized(LayoutDisplay::InlineBlock, 10.0, 10.0, RED));

    let snapshot = render(&source, &mut styles, 100, 60);
    let table = rect(&snapshot, YELLOW);
    let cell = rect(&snapshot, BLUE);
    let tail = rect(&snapshot, RED);
    assert_close(table.height, 20.0);
    assert_close(cell.y, table.y + 10.0);
    assert_close(tail.y, table.y + 10.0);
}

#[test]
fn inline_block_propagates_scroll_block_end_baseline_through_block_children() {
    let render_variant = |inner_display, inner_overflow, margin_bottom| {
        let source = Source(vec![
            Node::element(
                "root",
                "div",
                LayoutElementCategory::Generic,
                None,
                vec![1, 4],
            ),
            Node::element(
                "outer",
                "span",
                LayoutElementCategory::Generic,
                None,
                vec![2],
            ),
            Node::element(
                "inner",
                "div",
                LayoutElementCategory::Generic,
                None,
                vec![3],
            ),
            Node::text("inner-text", "X"),
            Node::element(
                "tail",
                "span",
                LayoutElementCategory::Generic,
                None,
                Vec::new(),
            ),
        ]);
        let mut styles = Styles::default();
        styles.primary.insert(
            0,
            style(LayoutDisplay::Block, PaintColor::TRANSPARENT)
                .tap_taffy(|taffy| {
                    taffy.size.width = Dimension::length(200.0);
                })
                .with_text_metrics(16.0, 20.0),
        );
        styles.primary.insert(
            1,
            style(LayoutDisplay::InlineBlock, YELLOW)
                .tap_taffy(|taffy| {
                    taffy.box_sizing = BoxSizing::ContentBox;
                    if inner_display == LayoutDisplay::InlineBlock {
                        taffy.size.height = Dimension::length(30.0 + margin_bottom);
                    }
                    taffy.padding.bottom = taffy::LengthPercentage::length(20.0);
                })
                .with_text_metrics(16.0, 20.0),
        );
        styles.primary.insert(
            2,
            style(inner_display, BLUE)
                .tap_taffy(|taffy| {
                    taffy.size = Size {
                        width: Dimension::length(30.0),
                        height: Dimension::length(30.0),
                    };
                    taffy.margin.bottom = taffy::LengthPercentageAuto::length(margin_bottom);
                    taffy.overflow = Point {
                        x: inner_overflow,
                        y: inner_overflow,
                    };
                })
                .with_text_metrics(16.0, 20.0),
        );
        styles
            .primary
            .insert(4, sized(LayoutDisplay::InlineBlock, 10.0, 10.0, RED));

        render(&source, &mut styles, 200, 120)
    };

    for margin_bottom in [0.0, 30.0] {
        let test = render_variant(LayoutDisplay::Block, Overflow::Hidden, margin_bottom);
        let reference = render_variant(LayoutDisplay::InlineBlock, Overflow::Hidden, margin_bottom);

        assert_eq!(rect(&test, YELLOW), rect(&reference, YELLOW));
        assert_eq!(rect(&test, BLUE), rect(&reference, BLUE));
        assert_eq!(rect(&test, RED), rect(&reference, RED));
    }
}

#[test]
fn phantom_inline_line_does_not_supply_an_inline_block_baseline() {
    let source = Source(vec![
        Node::element(
            "wrapper",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2],
        ),
        Node::element(
            "empty-atomic",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "block-atomic",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![3, 4],
        ),
        Node::element(
            "empty-inline",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "green-block",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        style(LayoutDisplay::Block, RED)
            .tap_taffy(|taffy| {
                taffy.size.width = Dimension::length(200.0);
            })
            .with_text_metrics(0.0, 0.0),
    );
    styles
        .primary
        .insert(1, sized(LayoutDisplay::InlineBlock, 100.0, 200.0, GREEN));
    styles.primary.insert(
        2,
        style(LayoutDisplay::InlineBlock, PaintColor::TRANSPARENT).with_text_metrics(0.0, 0.0),
    );
    styles.primary.insert(
        3,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(0.0, 0.0),
    );
    styles
        .primary
        .insert(4, sized(LayoutDisplay::Block, 100.0, 200.0, GREEN));

    let snapshot = render(&source, &mut styles, 240, 240);
    let mut green_rects = snapshot
        .fragments
        .iter()
        .filter_map(PaintFragment::solid_fill_in_surface)
        .filter_map(|(rect, color)| (color == GREEN).then_some(rect))
        .collect::<Vec<_>>();
    green_rects.sort_by(|left, right| left.x.total_cmp(&right.x));

    assert_eq!(
        green_rects,
        vec![
            PaintRect::new(0.0, 0.0, 100.0, 200.0),
            PaintRect::new(100.0, 0.0, 100.0, 200.0),
        ]
    );
}

#[test]
fn top_and_middle_inline_blocks_align_against_the_parent_line_strut() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 3],
        ),
        Node::element(
            "settings",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::text("settings-text", "Settings"),
        Node::element("login", "a", LayoutElementCategory::Generic, None, vec![4]),
        Node::text("login-text", "Login"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 60.0, PaintColor::TRANSPARENT)
            .with_text_metrics(12.0, 14.4),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::InlineBlock, RED)
            .tap_taffy(|taffy| {
                taffy.margin.top = taffy::LengthPercentageAuto::length(19.0);
            })
            .with_text_metrics(13.0, 23.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Top),
    );
    styles.primary.insert(
        3,
        style(LayoutDisplay::InlineBlock, BLUE)
            .tap_taffy(|taffy| {
                taffy.size = Size {
                    width: Dimension::length(48.0),
                    height: Dimension::length(24.0),
                };
                taffy.margin.top = taffy::LengthPercentageAuto::length(18.0);
            })
            .with_text_metrics(13.0, 24.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Middle),
    );

    let snapshot = render(&source, &mut styles, 300, 60);
    let settings = rect(&snapshot, RED);
    let login = rect(&snapshot, BLUE);
    assert_close(settings.y, 19.0);
    assert_close(login.y, 18.0);
    assert_close(settings.y + settings.height, 42.0);
    assert_close(login.y + login.height, 42.0);
}

#[test]
fn middle_inline_block_uses_its_nearest_inline_parents_x_height() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 3],
        ),
        Node::element(
            "large-parent",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "large-icon",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "small-parent",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![4],
        ),
        Node::element(
            "small-icon",
            "span",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 80.0, PaintColor::TRANSPARENT)
            .with_text_metrics(10.0, 60.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(40.0, 60.0),
    );
    styles.primary.insert(
        2,
        sized(LayoutDisplay::InlineBlock, 18.0, 18.0, RED)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Middle),
    );
    styles.primary.insert(
        3,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(10.0, 60.0),
    );
    styles.primary.insert(
        4,
        sized(LayoutDisplay::InlineBlock, 18.0, 18.0, BLUE)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Middle),
    );

    let snapshot = render(&source, &mut styles, 300, 80);
    let large_parent_icon = rect(&snapshot, RED);
    let small_parent_icon = rect(&snapshot, BLUE);
    assert!(
        large_parent_icon.y + 5.0 < small_parent_icon.y,
        "a larger nearest-parent x-height must raise the middle-aligned icon: large={large_parent_icon:?}, small={small_parent_icon:?}"
    );
}

#[test]
fn middle_aligned_text_uses_its_nearest_inline_parents_x_height() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 4],
        ),
        Node::element(
            "large-parent",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "large-middle",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![3],
        ),
        Node::text("large-text", "A"),
        Node::element(
            "small-parent",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![5],
        ),
        Node::element(
            "small-middle",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![6],
        ),
        Node::text("small-text", "A"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 80.0, PaintColor::TRANSPARENT)
            .with_text_metrics(10.0, 60.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(40.0, 60.0),
    );
    styles.primary.insert(
        2,
        style(LayoutDisplay::Inline, RED)
            .with_text_metrics(10.0, 20.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Middle),
    );
    styles.primary.insert(
        4,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(10.0, 60.0),
    );
    styles.primary.insert(
        5,
        style(LayoutDisplay::Inline, BLUE)
            .with_text_metrics(10.0, 20.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Middle),
    );

    let snapshot = render(&source, &mut styles, 300, 80);
    let large_parent_text = rect(&snapshot, RED);
    let small_parent_text = rect(&snapshot, BLUE);
    assert!(
        large_parent_text.y + 5.0 < small_parent_text.y,
        "a larger nearest-parent x-height must raise middle-aligned text: large={large_parent_text:?}, small={small_parent_text:?}"
    );
}

#[test]
fn structural_inline_own_strut_contributes_without_direct_text() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 5],
        ),
        Node::element("line", "div", LayoutElementCategory::Generic, None, vec![2]),
        Node::element(
            "large-structural-inline",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![3],
        ),
        Node::element(
            "small-child",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![4],
        ),
        Node::text("text", "A"),
        Node::element(
            "following-block",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![],
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Block, BLUE).with_text_metrics(10.0, 10.0),
    );
    styles.primary.insert(
        2,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(40.0, 60.0),
    );
    styles.primary.insert(
        3,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(10.0, 10.0),
    );
    styles
        .primary
        .insert(5, sized(LayoutDisplay::Block, 10.0, 5.0, RED));

    let snapshot = render(&source, &mut styles, 200, 100);
    let line = rect(&snapshot, BLUE);
    let following = rect(&snapshot, RED);
    assert!(
        line.height >= 59.0,
        "a structural inline's own 60px strut must size the line even when all direct text is in a 10px child: {line:?}"
    );
    assert!(
        following.y >= line.y + 59.0,
        "following flow content must start after the structural strut: line={line:?}, following={following:?}"
    );
}

#[test]
fn inline_fragment_uses_its_own_font_box_instead_of_descendant_union() {
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element(
            "small-parent",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "large-child",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![3],
        ),
        Node::text("text", "A"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT)
            .with_text_metrics(10.0, 60.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Inline, RED).with_text_metrics(10.0, 10.0),
    );
    styles.primary.insert(
        2,
        style(LayoutDisplay::Inline, BLUE).with_text_metrics(40.0, 40.0),
    );

    let snapshot = render(&source, &mut styles, 200, 100);
    let parent = rect(&snapshot, RED);
    let child = rect(&snapshot, BLUE);
    assert!(
        parent.height + 10.0 < child.height,
        "an inline fragment's block size comes from its own primary font, not the union of a larger descendant: parent={parent:?}, child={child:?}"
    );
}

#[test]
fn empty_decorated_inline_fragment_does_not_fill_the_line_height() {
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element(
            "empty-inline",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![],
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 200.0, 100.0, PaintColor::TRANSPARENT)
            .with_text_metrics(10.0, 60.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Inline, RED)
            .tap_taffy(|taffy| {
                taffy.padding.left = taffy::LengthPercentage::length(4.0);
                taffy.padding.right = taffy::LengthPercentage::length(4.0);
            })
            .with_text_metrics(10.0, 10.0),
    );

    let snapshot = render(&source, &mut styles, 200, 100);
    let empty = rect(&snapshot, RED);
    assert!(empty.width >= 7.9, "missing inline-axis padding: {empty:?}");
    assert!(
        empty.height < 30.0,
        "an empty decorated inline uses its own font box rather than the root's 60px line box: {empty:?}"
    );
}

#[test]
fn nested_inline_alignments_move_each_structural_subtree_in_order() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 4],
        ),
        Node::element(
            "middle-outer",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "text-top-inner",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![3],
        ),
        Node::text("middle-text", "A"),
        Node::element(
            "baseline-outer",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![5],
        ),
        Node::element(
            "reference-text-top-inner",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![6],
        ),
        Node::text("reference-text", "A"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 100.0, PaintColor::TRANSPARENT)
            .with_text_metrics(10.0, 20.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT)
            .with_text_metrics(40.0, 50.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Middle),
    );
    styles.primary.insert(
        2,
        style(LayoutDisplay::Inline, RED)
            .with_text_metrics(10.0, 10.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::TextTop),
    );
    styles.primary.insert(
        4,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT).with_text_metrics(40.0, 50.0),
    );
    styles.primary.insert(
        5,
        style(LayoutDisplay::Inline, BLUE)
            .with_text_metrics(10.0, 10.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::TextTop),
    );

    let snapshot = render(&source, &mut styles, 300, 100);
    let nested = rect(&snapshot, RED);
    let reference = rect(&snapshot, BLUE);
    assert!(
        (nested.y - reference.y).abs() > 2.0,
        "the outer middle alignment must move the already text-top-aligned inner subtree: nested={nested:?}, reference={reference:?}"
    );
}

#[test]
fn nested_bottom_alignment_targets_the_nearest_top_or_bottom_subtree() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 4],
        ),
        Node::element(
            "top-outer",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "nested-bottom",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![3],
        ),
        Node::text("nested-text", "A"),
        Node::element(
            "root-bottom",
            "span",
            LayoutElementCategory::Generic,
            None,
            vec![5],
        ),
        Node::text("root-text", "A"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 120.0, PaintColor::TRANSPARENT)
            .with_text_metrics(10.0, 100.0),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Inline, PaintColor::TRANSPARENT)
            .with_text_metrics(20.0, 40.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Top),
    );
    styles.primary.insert(
        2,
        style(LayoutDisplay::Inline, RED)
            .with_text_metrics(10.0, 10.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Bottom),
    );
    styles.primary.insert(
        4,
        style(LayoutDisplay::Inline, BLUE)
            .with_text_metrics(10.0, 10.0)
            .with_inline_alignment(moli_layout::LayoutInlineAlignment::Bottom),
    );

    let snapshot = render(&source, &mut styles, 300, 120);
    let nested = rect(&snapshot, RED);
    let root_aligned = rect(&snapshot, BLUE);
    assert!(
        nested.y + 30.0 < root_aligned.y,
        "the nested bottom must align to its 40px top-aligned ancestor, while the direct bottom aligns to the 100px root line: nested={nested:?}, root={root_aligned:?}"
    );
}

#[test]
fn replaced_box_model_is_applied_exactly_once() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2],
        ),
        Node::element(
            "image",
            "img",
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Image),
            Vec::new(),
        ),
        Node::element(
            "following",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 200.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                box_sizing: taffy::BoxSizing::ContentBox,
                size: Size {
                    width: Dimension::length(80.0),
                    height: Dimension::length(40.0),
                },
                padding: Rect {
                    left: taffy::LengthPercentage::length(5.0),
                    right: taffy::LengthPercentage::length(7.0),
                    top: taffy::LengthPercentage::length(11.0),
                    bottom: taffy::LengthPercentage::length(13.0),
                },
                border: Rect {
                    left: taffy::LengthPercentage::length(2.0),
                    right: taffy::LengthPercentage::length(2.0),
                    top: taffy::LengthPercentage::length(2.0),
                    bottom: taffy::LengthPercentage::length(2.0),
                },
                ..Style::default()
            },
            BLUE,
        ),
    );
    styles
        .primary
        .insert(2, sized(LayoutDisplay::Block, 10.0, 10.0, GREEN));

    let snapshot = render(&source, &mut styles, 300, 200);
    assert_eq!(rect(&snapshot, BLUE), PaintRect::new(0.0, 0.0, 96.0, 68.0));
    assert_eq!(
        rect(&snapshot, GREEN),
        PaintRect::new(0.0, 68.0, 10.0, 10.0)
    );
}

#[test]
fn single_row_textarea_does_not_expand_its_flex_search_bar() {
    let source = Source(vec![
        Node::element(
            "search",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1],
        ),
        Node::element(
            "query",
            "textarea",
            LayoutElementCategory::FormControl(LayoutFormControlKind::TextArea),
            Some(LayoutReplacedKind::FormControl),
            Vec::new(),
        )
        .with_metadata(LayoutElementMetadata {
            form_control: Some(LayoutFormControlData {
                rows: 1,
                ..LayoutFormControlData::default()
            }),
            ..LayoutElementMetadata::default()
        }),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        style(LayoutDisplay::Flex, PaintColor::TRANSPARENT).tap_taffy(|taffy| {
            taffy.display = taffy::Display::Flex;
            taffy.size.width = Dimension::length(300.0);
            taffy.min_size.height = Dimension::length(50.0);
        }),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Flex, YELLOW)
            .tap_taffy(|taffy| {
                taffy.display = taffy::Display::Flex;
                taffy.flex_grow = 1.0;
                taffy.overflow = Point {
                    x: Overflow::Hidden,
                    y: Overflow::Scroll,
                };
                taffy.padding.top = taffy::LengthPercentage::length(14.0);
                taffy.border.bottom = taffy::LengthPercentage::length(8.0);
            })
            .with_text_metrics(16.0, 22.0),
    );

    let snapshot = render(&source, &mut styles, 300, 100);
    assert_eq!(
        rect(&snapshot, YELLOW),
        PaintRect::new(0.0, 0.0, 300.0, 50.0)
    );
}

#[test]
fn form_controls_have_deterministic_intrinsic_geometry_content_and_basic_appearance() {
    let form_data = LayoutFormControlData {
        placeholder: "Search".into(),
        size: Some(4),
        ..LayoutFormControlData::default()
    };
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 2, 3],
        ),
        Node::element(
            "text",
            "input",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                LayoutInputControlKind::Text,
            )),
            Some(LayoutReplacedKind::FormControl),
            Vec::new(),
        )
        .with_metadata(LayoutElementMetadata {
            form_control: Some(form_data),
            ..LayoutElementMetadata::default()
        }),
        Node::element(
            "checked",
            "input",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                LayoutInputControlKind::Checkbox,
            )),
            Some(LayoutReplacedKind::FormControl),
            Vec::new(),
        )
        .with_metadata(LayoutElementMetadata {
            form_control: Some(LayoutFormControlData {
                checked: true,
                ..LayoutFormControlData::default()
            }),
            ..LayoutElementMetadata::default()
        }),
        Node::element(
            "button",
            "button",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Button),
            None,
            vec![4],
        ),
        Node::text("button-text", "Button"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        style(LayoutDisplay::Block, YELLOW).with_text_metrics(20.0, 20.0),
    );
    styles
        .primary
        .insert(2, style(LayoutDisplay::Block, PaintColor::WHITE));
    styles
        .primary
        .insert(3, style(LayoutDisplay::InlineBlock, GREEN));
    let snapshot = render(&source, &mut styles, 300, 100);
    let input = rect(&snapshot, YELLOW);
    assert_close(input.width, 48.0);
    assert_close(input.height, 20.0);
    let button = rect(&snapshot, GREEN);
    assert!(
        button.width > 20.0 && button.width < 100.0,
        "button must shrink to its real DOM content instead of stretching: {button:?}"
    );
    assert!(
        snapshot
            .fragments
            .iter()
            .any(|fragment| matches!(fragment, PaintFragment::GlyphRun(_)))
    );
    assert!(snapshot.fragments.iter().any(|fragment| {
        matches!(
            fragment,
            PaintFragment::Stroke(stroke)
                if stroke.color == PaintColor::WHITE
                    && stroke.width > 0.0
                    && stroke.start_cap == moli_layout::PaintLineCap::Round
                    && stroke.end_cap == moli_layout::PaintLineCap::Round
        )
    }));
    assert!(
        snapshot
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "replaced-content-placeholder")
    );
}

#[test]
fn flow_button_centers_real_dom_content_in_its_content_box() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 3],
        ),
        Node::element(
            "button",
            "button",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Button),
            None,
            vec![2],
        ),
        Node::text("button-text", "B"),
        Node::element(
            "tight-button",
            "button",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Button),
            None,
            vec![4],
        ),
        Node::text("tight-button-text", "B"),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        sized(LayoutDisplay::Block, 108.0, 44.0, GREEN).with_text_metrics(20.0, 20.0),
    );
    styles.primary.insert(
        3,
        sized(LayoutDisplay::Block, 108.0, 20.0, BLUE).with_text_metrics(20.0, 20.0),
    );

    let snapshot = render(&source, &mut styles, 300, 100);
    let centered = rect(&snapshot, GREEN);
    let tight = rect(&snapshot, BLUE);
    assert_eq!(centered, PaintRect::new(0.0, 0.0, 108.0, 44.0));
    assert_eq!(tight, PaintRect::new(0.0, 44.0, 108.0, 20.0));
    let mut glyphs = snapshot
        .fragments
        .iter()
        .filter_map(|fragment| match fragment {
            PaintFragment::GlyphRun(run) => Some(run.glyphs_in_surface()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    glyphs.sort_by(|left, right| left.y.total_cmp(&right.y));
    assert_eq!(
        glyphs.len(),
        2,
        "button fixtures must each produce one glyph"
    );
    let centered_local_baseline = glyphs[0].y - centered.y;
    let tight_local_baseline = glyphs[1].y - tight.y;
    assert_close(centered_local_baseline - tight_local_baseline, 12.0);
}

#[test]
fn flow_button_delegates_block_child_centering_to_taffy() {
    let source = Source(vec![
        Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
        Node::element(
            "button",
            "button",
            LayoutElementCategory::FormControl(LayoutFormControlKind::Button),
            None,
            vec![2],
        ),
        Node::element(
            "button-child",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 100.0, PaintColor::TRANSPARENT),
    );
    styles
        .primary
        .insert(1, sized(LayoutDisplay::Block, 108.0, 44.0, GREEN));
    styles
        .primary
        .insert(2, sized(LayoutDisplay::Block, 20.0, 10.0, RED));

    let snapshot = render(&source, &mut styles, 300, 100);
    assert_eq!(
        rect(&snapshot, GREEN),
        PaintRect::new(0.0, 0.0, 108.0, 44.0)
    );
    assert_eq!(rect(&snapshot, RED), PaintRect::new(0.0, 17.0, 20.0, 10.0));
}

#[test]
fn text_selection_and_caret_use_parley_geometry_in_the_owned_snapshot() {
    let selection_color = PaintColor::new(180.0 / 255.0, 213.0 / 255.0, 1.0, 1.0);
    let render_text = |selection| {
        let source = Source(vec![
            Node::element("root", "div", LayoutElementCategory::Generic, None, vec![1]),
            Node::text("text", "hello world").with_selection(selection),
        ]);
        let mut styles = Styles::default();
        styles.primary.insert(
            0,
            sized(LayoutDisplay::Block, 200.0, 40.0, PaintColor::TRANSPARENT)
                .with_text_metrics(20.0, 24.0),
        );
        render(&source, &mut styles, 200, 40)
    };

    let selected = render_text(LayoutTextSelection::new(1, 5));
    assert!(selected.fragments.iter().any(|fragment| {
        matches!(
            fragment,
            PaintFragment::Fill {
                shape: PaintShape::Rect(rect),
                brush: PaintBrush::Solid(color),
                ..
            } if *color == selection_color && rect.width > 0.0 && rect.height > 0.0
        )
    }));

    let caret = render_text(LayoutTextSelection::new(5, 5));
    assert!(caret.fragments.iter().any(|fragment| {
        matches!(
            fragment,
            PaintFragment::Fill {
                shape: PaintShape::Rect(rect),
                brush: PaintBrush::Solid(PaintColor::BLACK),
                ..
            } if (rect.width - 1.5).abs() < 0.05 && rect.height > 0.0
        )
    }));
}

#[test]
fn sticky_uses_the_scrollport_and_transformed_ancestor_contains_fixed_descendants() {
    let source = Source(vec![
        Node::element(
            "root",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![1, 3],
        ),
        Node::element(
            "scroll",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![2],
        ),
        Node::element(
            "sticky",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
        Node::element(
            "transform",
            "div",
            LayoutElementCategory::Generic,
            None,
            vec![4],
        ),
        Node::element(
            "fixed",
            "div",
            LayoutElementCategory::Generic,
            None,
            Vec::new(),
        ),
    ]);
    let mut styles = Styles::default();
    styles.primary.insert(
        0,
        sized(LayoutDisplay::Block, 300.0, 200.0, PaintColor::TRANSPARENT),
    );
    styles.primary.insert(
        1,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::length(100.0),
                    height: Dimension::length(50.0),
                },
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Hidden,
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        ),
    );
    styles.primary.insert(
        2,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::length(50.0),
                    height: Dimension::length(20.0),
                },
                inset: Rect {
                    top: taffy::LengthPercentageAuto::length(10.0),
                    right: taffy::LengthPercentageAuto::auto(),
                    bottom: taffy::LengthPercentageAuto::auto(),
                    left: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            RED,
        )
        .with_position(LayoutPosition::Sticky),
    );
    styles.primary.insert(
        3,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::length(100.0),
                    height: Dimension::length(100.0),
                },
                margin: Rect {
                    left: taffy::LengthPercentageAuto::length(50.0),
                    right: taffy::LengthPercentageAuto::length(0.0),
                    top: taffy::LengthPercentageAuto::length(0.0),
                    bottom: taffy::LengthPercentageAuto::length(0.0),
                },
                ..Style::default()
            },
            PaintColor::TRANSPARENT,
        )
        .with_transform_containing_block(),
    );
    styles.primary.insert(
        4,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style {
                size: Size {
                    width: Dimension::length(20.0),
                    height: Dimension::length(20.0),
                },
                inset: Rect {
                    right: taffy::LengthPercentageAuto::length(0.0),
                    top: taffy::LengthPercentageAuto::length(0.0),
                    left: taffy::LengthPercentageAuto::auto(),
                    bottom: taffy::LengthPercentageAuto::auto(),
                },
                ..Style::default()
            },
            BLUE,
        )
        .with_position(LayoutPosition::Fixed),
    );
    let snapshot = render(&source, &mut styles, 300, 200);
    assert_close(rect(&snapshot, RED).y, 10.0);
    assert_close(rect(&snapshot, BLUE).x, 130.0);
    assert!(snapshot.fragments.iter().any(|fragment| matches!(
        fragment,
        PaintFragment::PushClip {
            shape: moli_layout::PaintShape::Rect(rect),
            ..
        } if rect.width == 100.0 && rect.height == 50.0
    )));
}
