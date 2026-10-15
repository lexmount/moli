use super::*;
use crate::LayoutTableRole;

struct TableStyles {
    base: Styles,
    displays: HashMap<usize, LayoutDisplay>,
    aspect_ratio: Option<f32>,
}

impl LayoutStyleResolver<usize> for TableStyles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        let Some(styles) = self.base.element_styles(node)? else {
            return Ok(None);
        };
        let (mut style, _, _) = styles.into_parts();
        let display = self
            .displays
            .get(&node)
            .copied()
            .unwrap_or(LayoutDisplay::Block);
        style.force_layout_display(display);
        style.taffy.size = taffy::Size {
            width: taffy::Dimension::auto(),
            height: taffy::Dimension::auto(),
        };
        if node == 0 {
            style.taffy.size.width = taffy::Dimension::length(320.0);
        }
        if node == 4 {
            style.taffy.size.width = taffy::Dimension::length(self.base.first_width);
        }
        style.taffy.aspect_ratio = self.aspect_ratio;
        Ok(Some(ResolvedLayoutElementStyles::from_primary(style)))
    }
}

fn fixture() -> (Source, TableStyles) {
    let mut source = Source::new();
    source.nodes = vec![
        Node::element(vec![1]),
        Node::element(vec![3]),
        Node::text("first paragraph"),
        Node::element(vec![4, 5]),
        Node::element(vec![2]),
        Node::element(vec![6]),
        Node::text("second paragraph"),
    ];
    for (node, tag, role) in [
        (0, "table", LayoutTableRole::Table),
        (1, "tbody", LayoutTableRole::BodyGroup),
        (3, "tr", LayoutTableRole::Row),
        (4, "td", LayoutTableRole::Cell),
        (5, "td", LayoutTableRole::Cell),
    ] {
        source.semantics.insert(
            node,
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                tag,
                LayoutElementCategory::Table(role),
                None,
            ),
        );
    }
    let mut base = Styles::new();
    base.hidden = false;
    let styles = TableStyles {
        base,
        displays: HashMap::from([
            (0, LayoutDisplay::Table),
            (1, LayoutDisplay::TableRowGroup),
            (3, LayoutDisplay::TableRow),
            (4, LayoutDisplay::TableCell),
            (5, LayoutDisplay::TableCell),
        ]),
        aspect_ratio: None,
    };
    (source, styles)
}

#[test]
fn table_owned_graph_refreshes_styles_and_preserves_cold_paint_and_queries() {
    let (mut source, mut styles) = fixture();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    let first = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    let frozen = first.tree.boxes.clone();
    let pointer = owned_parts(&work).0.as_ptr();
    let children = owned_parts(&work).0[0].children.as_ptr();
    for step in 0..3 {
        styles.base.first_width = 180.0 + step as f32 * 10.0;
        styles.base.color = PaintColor::new(0.25, 0.5, 0.75, 1.0);
        styles.aspect_ratio = Some(1.0 + step as f32);
        source.selection = Some(LayoutTextSelection::new(1, 5));
        source.label_revision = step + 1;
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, work.stats().retained_boxes);
        assert_eq!(owned_parts(&work).0.as_ptr(), pointer);
        assert_eq!(owned_parts(&work).0[0].children.as_ptr(), children);
        for layout_box in owned_parts(&work).0 {
            if layout_box.kind.is_internal_table_box() {
                assert!(layout_box.style.taffy.aspect_ratio.is_none());
            }
        }
        assert_eq!(first.tree.boxes, frozen);
    }
}

#[test]
fn table_semantic_spans_and_source_order_rebuild_before_warming_again() {
    for change in 0..3 {
        let (mut source, mut styles) = fixture();
        let mut services = DocumentLayoutServices::new();
        let mut work = LayoutWorkingSet::default();
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        owned_parts(&work);
        match change {
            0 => {
                source
                    .semantics
                    .get_mut(&4)
                    .unwrap()
                    .metadata
                    .table
                    .as_mut()
                    .unwrap()
                    .column_span = 2
            }
            1 => source.nodes[3].children.swap(0, 1),
            2 => {
                styles.displays.insert(1, LayoutDisplay::TableHeaderGroup);
            }
            _ => unreachable!(),
        }
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, 0);
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_owned_boxes > 0);
    }
}

#[test]
fn table_anonymous_repair_keeps_using_the_shared_cold_constructor() {
    for malformed in [false, true] {
        let (mut source, mut styles) = fixture();
        let mut services = DocumentLayoutServices::new();
        let mut work = LayoutWorkingSet::default();
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        if malformed {
            styles.displays.insert(4, LayoutDisplay::Block);
        } else {
            source.nodes.push(Node::text("needs an anonymous cell"));
            source.nodes[3].children.insert(0, 7);
        }
        for _ in 0..2 {
            compare_with_cold(&source, &mut styles, &mut services, &mut work);
            assert_eq!(work.stats().retained_owned_boxes, 0);
            assert_eq!(work.stats().reused_construction_boxes, 0);
        }
    }
}

#[test]
fn table_compact_admission_and_eviction_preserve_complete_output() {
    let (source, mut styles) = fixture();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    owned_parts(&work);
    let owned = work.stats().estimated_retained_bytes;
    let mut retained = work.take().unwrap();
    retained.compact_graph();
    let compact = retained.estimated_retention().total();
    drop(retained);
    let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: compact + (owned - compact) / 2,
        ..Default::default()
    });
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().retained_owned_boxes, 0);
    styles.base.first_width = 170.0;
    let warm = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_construction_boxes > 0);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    work.clear();
    let cold = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    assert_eq!(warm.tree.boxes, cold.tree.boxes);
    assert_eq!(warm.paint_snapshot(), cold.paint_snapshot());
}

#[test]
fn table_owned_refresh_normalizes_internal_styles_before_numeric_layout() {
    let (source, mut styles) = fixture();
    styles.aspect_ratio = Some(1.0);
    let world = build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    for layout_box in &world.boxes {
        if layout_box.kind.is_internal_table_box() {
            assert_eq!(layout_box.style.taffy.aspect_ratio, None);
        }
    }
    let mut work = LayoutWorkingSet::default();
    publish(world, &mut work);
    owned_parts(&work);
    styles.aspect_ratio = Some(2.0);
    let mut previous = work.take();
    let world =
        build_layout_world_with_retained_inputs(&source, &mut styles, previous.as_mut()).unwrap();
    assert_eq!(
        world
            .construction_inputs
            .as_ref()
            .unwrap()
            .reused_owned_boxes(),
        world.len()
    );
    assert_eq!(
        world.boxes[world.root.index()].style.taffy.aspect_ratio,
        Some(2.0)
    );
    for layout_box in &world.boxes {
        if layout_box.kind.is_internal_table_box() {
            assert_eq!(
                layout_box.style.taffy.aspect_ratio, None,
                "{:?}",
                layout_box.kind
            );
        }
    }
}

#[test]
fn table_caption_columns_and_visual_sections_match_the_shared_cold_layout() {
    let (mut source, mut styles) = fixture();
    source.nodes.extend([
        Node::element(vec![8]),
        Node::text("caption"),
        Node::element(vec![10]),
        Node::element(vec![19]),
        Node::element(vec![12]),
        Node::element(vec![13]),
        Node::element(vec![14]),
        Node::text("footer"),
        Node::element(vec![16]),
        Node::element(vec![17]),
        Node::element(vec![18]),
        Node::text("header"),
        Node::text("ignored column descendant"),
    ]);
    source.nodes[0].children = vec![7, 9, 11, 1, 15];
    for (node, tag, role, display) in [
        (
            7,
            "caption",
            LayoutTableRole::Caption,
            LayoutDisplay::TableCaption,
        ),
        (
            9,
            "colgroup",
            LayoutTableRole::ColumnGroup,
            LayoutDisplay::TableColumnGroup,
        ),
        (
            10,
            "col",
            LayoutTableRole::Column,
            LayoutDisplay::TableColumn,
        ),
        (
            11,
            "tfoot",
            LayoutTableRole::FooterGroup,
            LayoutDisplay::TableFooterGroup,
        ),
        (12, "tr", LayoutTableRole::Row, LayoutDisplay::TableRow),
        (13, "td", LayoutTableRole::Cell, LayoutDisplay::TableCell),
        (
            15,
            "thead",
            LayoutTableRole::HeaderGroup,
            LayoutDisplay::TableHeaderGroup,
        ),
        (16, "tr", LayoutTableRole::Row, LayoutDisplay::TableRow),
        (17, "td", LayoutTableRole::Cell, LayoutDisplay::TableCell),
    ] {
        source.semantics.insert(
            node,
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                tag,
                LayoutElementCategory::Table(role),
                None,
            ),
        );
        styles.displays.insert(node, display);
    }
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    owned_parts(&work);
    // The shared constructor never consumes column descendants.
    source.nodes[19].text = Some("still ignored".into());
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, work.stats().retained_boxes);
    styles.displays.insert(0, LayoutDisplay::InlineTable);
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_owned_boxes > 0);
    source
        .semantics
        .get_mut(&10)
        .unwrap()
        .metadata
        .table
        .as_mut()
        .unwrap()
        .span = 2;
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, 0);
}

#[test]
fn table_discarded_column_group_elements_do_not_enter_the_retained_style_sequence() {
    let (mut source, mut styles) = fixture();
    source.nodes.extend([
        Node::element(vec![8]),
        Node::element(vec![9]),
        Node::text("discarded"),
    ]);
    source.nodes[0].children.insert(0, 7);
    source.semantics.insert(
        7,
        LayoutElementSemantics::new(
            LayoutNamespace::Html,
            "colgroup",
            LayoutElementCategory::Table(LayoutTableRole::ColumnGroup),
            None,
        ),
    );
    styles.displays.insert(7, LayoutDisplay::TableColumnGroup);
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    for _ in 0..2 {
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_construction_boxes, 0);
        assert_eq!(work.stats().retained_owned_boxes, 0);
    }
}

#[test]
fn table_inline_wrapper_inside_an_atomic_parent_keeps_current_geometry() {
    let (mut source, mut styles) = fixture();
    source.nodes[0].children = vec![7];
    source.nodes.push(Node::element(vec![1]));
    source.semantics.remove(&0);
    source.semantics.insert(
        7,
        LayoutElementSemantics::new(
            LayoutNamespace::Html,
            "table",
            LayoutElementCategory::Table(LayoutTableRole::Table),
            None,
        ),
    );
    styles.displays.insert(0, LayoutDisplay::Block);
    styles.displays.insert(7, LayoutDisplay::InlineTable);
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    owned_parts(&work);
    styles.base.first_width = 200.0;
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, work.stats().retained_boxes);
}
