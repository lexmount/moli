use super::*;

struct GridStyles {
    base: Styles,
    first_track: f32,
    order: i32,
    placement: i16,
    auto_flow: taffy::GridAutoFlow,
    child_display: Option<LayoutDisplay>,
}

impl GridStyles {
    fn new() -> Self {
        let mut base = Styles::new();
        base.root_display = LayoutDisplay::Grid;
        Self {
            base,
            first_track: 160.0,
            order: 0,
            placement: 0,
            auto_flow: taffy::GridAutoFlow::Row,
            child_display: None,
        }
    }
}

impl LayoutStyleResolver<usize> for GridStyles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        let Some(styles) = self.base.element_styles(node)? else {
            return Ok(None);
        };
        let (mut style, _, _) = styles.into_parts();
        if node == 0 {
            let fixed: taffy::TrackSizingFunction = taffy::style_helpers::length(self.first_track);
            let flexible: taffy::TrackSizingFunction = taffy::style_helpers::fr(1.0);
            style.taffy.grid_template_columns = vec![fixed.into(), flexible.into()];
            style.taffy.grid_template_column_names = vec![Vec::new(); 3];
            style.taffy.grid_auto_flow = self.auto_flow;
        }
        if node == 1 {
            style = style.with_order(self.order);
            if self.placement != 0 {
                style.taffy.grid_column = taffy::Line {
                    start: taffy::style_helpers::line(self.placement),
                    end: taffy::style_helpers::span(1),
                };
            }
            if let Some(display) = self.child_display {
                style.force_layout_display(display);
            }
        }
        Ok(Some(ResolvedLayoutElementStyles::from_primary(style)))
    }
}

fn assert_owned(work: &LayoutWorkingSet<usize>) {
    assert!(work.stats().retained_owned_boxes > 0, "{:#?}", work.stats());
    owned_parts(work);
}

#[test]
fn grid_owned_objects_refresh_tracks_order_placement_and_auto_flow() {
    let source = Source::new();
    let mut styles = GridStyles::new();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    let first = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_owned(&work);
    let pointer = owned_parts(&work).0.as_ptr();
    let capacity = owned_parts(&work).0.capacity();
    let children = owned_parts(&work).0[0].children.as_ptr();
    let frozen = first.tree.boxes.clone();
    let first_tracks = first.answer_query(&LayoutQuery::UsedGridTracks { source: 0 });
    for change in 0..4 {
        match change {
            0 => styles.first_track = 96.0,
            1 => styles.order = 2,
            2 => styles.placement = 2,
            3 => styles.auto_flow = taffy::GridAutoFlow::ColumnDense,
            _ => unreachable!(),
        }
        let current = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, work.stats().retained_boxes);
        assert_eq!(owned_parts(&work).0.as_ptr(), pointer);
        assert_eq!(owned_parts(&work).0.capacity(), capacity);
        assert_eq!(owned_parts(&work).0[0].children.as_ptr(), children);
        assert_eq!(first.tree.boxes, frozen);
        assert_ne!(
            current.answer_query(&LayoutQuery::UsedGridTracks { source: 0 }),
            first_tracks
        );
    }
}

#[test]
fn grid_source_and_display_changes_rebuild_before_warming_again() {
    for change in 0..4 {
        let mut source = Source::new();
        let mut styles = GridStyles::new();
        let mut services = DocumentLayoutServices::new();
        let mut work = LayoutWorkingSet::default();
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_owned(&work);
        match change {
            0 => source.nodes[0].children.swap(0, 1),
            1 => source.nodes[2].text = Some("changed grid item".into()),
            2 => styles.base.hidden = false,
            3 => styles.base.root_display = LayoutDisplay::Block,
            _ => unreachable!(),
        }
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, 0);
        assert_owned(&work);
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert!(work.stats().reused_owned_boxes > 0);
    }
}

#[test]
fn grid_anonymous_text_and_contents_keep_using_the_shared_cold_builder() {
    for contents in [false, true] {
        let mut source = Source::new();
        let mut styles = GridStyles::new();
        let mut services = DocumentLayoutServices::new();
        let mut work = LayoutWorkingSet::default();
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        if contents {
            styles.child_display = Some(LayoutDisplay::Contents);
        } else {
            source.nodes.push(Node::text("anonymous grid item"));
            source.nodes[0].children.insert(0, 6);
        }
        for _ in 0..2 {
            compare_with_cold(&source, &mut styles, &mut services, &mut work);
            assert_eq!(work.stats().reused_construction_boxes, 0);
            assert_eq!(work.stats().retained_owned_boxes, 0);
            assert!(
                work.retained
                    .as_ref()
                    .unwrap()
                    .construction_inputs()
                    .is_none()
            );
        }
    }
}

#[test]
fn grid_ignored_whitespace_and_positioned_items_match_cold_after_style_updates() {
    let mut source = Source::new();
    source.nodes.push(Node::text(" \n\t "));
    source.nodes[0].children.insert(0, 6);
    let mut styles = GridStyles::new();
    styles.base.positioned = Some((3, crate::LayoutPosition::Absolute));
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_owned(&work);
    styles.first_track = 80.0;
    styles.base.first_width = 180.0;
    source.scroll = LayoutPoint::new(0.0, 9.0);
    source.selection = Some(LayoutTextSelection::new(1, 5));
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_owned_boxes > 0);
    source.nodes[6].text = Some("now a meaningful grid text item".into());
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().retained_owned_boxes, 0);
}

#[test]
fn grid_compact_budget_fallback_and_eviction_preserve_current_answers() {
    let source = Source::new();
    let mut styles = GridStyles::new();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_owned(&work);
    let owned_charge = work.stats().estimated_retained_bytes;
    let mut compact = work.take().unwrap();
    compact.compact_graph();
    let compact_charge = compact.estimated_retention().total();
    drop(compact);
    assert!(compact_charge < owned_charge);
    let limit = compact_charge + (owned_charge - compact_charge) / 2;
    let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: limit,
        ..LayoutWorkingSetLimits::default()
    });
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().retained_owned_boxes, 0);
    assert!(
        work.retained
            .as_ref()
            .unwrap()
            .construction_inputs()
            .is_some()
    );
    styles.first_track = 80.0;
    let warm = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_construction_boxes > 0);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    work.clear();
    let evicted = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_construction_boxes, 0);
    assert_eq!(warm.tree.boxes, evicted.tree.boxes);
    assert_eq!(warm.paint_snapshot(), evicted.paint_snapshot());
}
