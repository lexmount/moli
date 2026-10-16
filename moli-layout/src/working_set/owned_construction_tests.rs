use std::collections::HashMap;

mod grid;
mod table;

use super::*;
use crate::{
    DocumentLayoutServices, EmbeddedFrameRenderer, EmbeddedFrameSnapshot, LayoutElementCategory,
    LayoutError, LayoutFlushReason, LayoutImageResource, LayoutNamespace, LayoutPassRequest,
    LayoutPassResult, LayoutPoint, LayoutQuery, LayoutSource, LayoutSourceKind,
    LayoutStyleResolver, LayoutViewport, PaintColor, ResolvedLayoutElementStyles,
    build_layout_pass, build_layout_pass_with_working_set,
    builder::{build_layout_world, build_layout_world_with_retained_inputs},
};

#[derive(Clone)]
struct Node {
    children: Vec<usize>,
    text: Option<String>,
}

impl Node {
    fn element(children: Vec<usize>) -> Self {
        Self {
            children,
            text: None,
        }
    }

    fn text(text: &str) -> Self {
        Self {
            children: Vec::new(),
            text: Some(text.into()),
        }
    }
}

#[derive(Clone)]
struct Source {
    nodes: Vec<Node>,
    semantics: HashMap<usize, LayoutElementSemantics>,
    hidden_name: Arc<str>,
    scroll: LayoutPoint,
    selection: Option<LayoutTextSelection>,
    label_revision: usize,
}

impl Source {
    fn new() -> Self {
        Self {
            nodes: vec![
                Node::element(vec![1, 3, 5]),
                Node::element(vec![2]),
                Node::text("first paragraph"),
                Node::element(vec![4]),
                Node::text("second paragraph"),
                Node::element(Vec::new()),
            ],
            semantics: HashMap::new(),
            hidden_name: Arc::from("owned-hidden-dependency"),
            scroll: LayoutPoint::ZERO,
            selection: None,
            label_revision: 0,
        }
    }
}

impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> usize {
        0
    }

    fn flat_parent(&self, node: usize) -> Option<usize> {
        self.nodes
            .iter()
            .position(|parent| parent.children.contains(&node))
    }

    fn flat_children(&self, node: usize) -> Self::ChildIter<'_> {
        self.nodes[node].children.iter().copied()
    }

    fn node_kind(&self, node: usize) -> LayoutSourceKind {
        if self.nodes[node].text.is_some() {
            LayoutSourceKind::Text
        } else {
            LayoutSourceKind::Element
        }
    }

    fn element_semantics(&self, node: usize) -> Option<LayoutElementSemantics> {
        (self.node_kind(node) == LayoutSourceKind::Element).then(|| {
            if let Some(semantics) = self.semantics.get(&node) {
                return semantics.clone();
            }
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                if node == 5 {
                    self.hidden_name.clone()
                } else {
                    Arc::from("div")
                },
                LayoutElementCategory::Generic,
                None,
            )
        })
    }

    fn text(&self, node: usize) -> Option<&str> {
        self.nodes[node].text.as_deref()
    }

    fn label(&self, node: usize) -> String {
        format!("source-{node}-revision-{}", self.label_revision)
    }

    fn text_selection(&self, node: usize) -> Option<LayoutTextSelection> {
        (node == 2).then_some(self.selection).flatten()
    }

    fn scroll_offset(&self, node: usize) -> LayoutPoint {
        if node == 1 {
            self.scroll
        } else {
            LayoutPoint::ZERO
        }
    }
}

struct Styles {
    root_display: LayoutDisplay,
    first_width: f32,
    color: PaintColor,
    hidden: bool,
    fail_at: Option<usize>,
    visited: Vec<usize>,
    font_style: Option<ResolvedLayoutStyle>,
    positioned: Option<(usize, crate::LayoutPosition)>,
}

impl Styles {
    fn new() -> Self {
        Self {
            root_display: LayoutDisplay::Block,
            first_width: 100.0,
            color: PaintColor::WHITE,
            hidden: true,
            fail_at: None,
            visited: Vec::new(),
            font_style: None,
            positioned: None,
        }
    }
}

impl LayoutStyleResolver<usize> for Styles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        self.visited.push(node);
        if self.fail_at == Some(node) {
            return Err(LayoutError::style_resolution(
                node.to_string(),
                "injected late source validation failure",
            ));
        }
        let display = match node {
            0 => self.root_display,
            5 if self.hidden => LayoutDisplay::None,
            _ => LayoutDisplay::Block,
        };
        let width = match node {
            0 => 320.0,
            1 => self.first_width,
            _ => 140.0,
        };
        let size = taffy::Size {
            width: taffy::Dimension::length(width),
            height: taffy::Dimension::length(if node == 0 { 120.0 } else { 40.0 }),
        };
        let style = if let Some(base) = &self.font_style {
            let mut style = base.clone();
            style.force_layout_display(display);
            style.taffy.size = size;
            style
        } else {
            ResolvedLayoutStyle::synthetic(
                display,
                taffy::Style {
                    size,
                    ..taffy::Style::default()
                },
                self.color,
            )
        };
        let style = if let Some((target, position)) = self.positioned {
            if target == node {
                style.with_position(position)
            } else {
                style
            }
        } else {
            style
        };
        Ok(Some(ResolvedLayoutElementStyles::from_primary(style)))
    }
}

fn owned_parts(
    work: &LayoutWorkingSet<usize>,
) -> (&Vec<LayoutBox<usize>>, &HashMap<usize, LayoutBoxId>) {
    match &work.retained.as_ref().expect("retained work").graph {
        RetainedGraph::Owned {
            boxes,
            source_mapping,
        } => (boxes, source_mapping),
        RetainedGraph::Compact(_) => panic!("fixture must admit its owned construction"),
    }
}

fn publish(world: LayoutWorld<usize>, work: &mut LayoutWorkingSet<usize>) {
    let count = world.len();
    work.publish(
        world,
        DocumentLayoutServices::new().font_inputs(),
        0,
        0,
        count,
    );
}

#[test]
fn owned_construction_moves_the_box_and_child_allocations_across_passes() {
    let mut source = Source::new();
    let mut styles = Styles::new();
    let mut pass = build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    pass.boxes.reserve_exact(17);
    pass.boxes[0].children.reserve_exact(11);
    pass.source_mapping.reserve(13);
    let box_pointer = pass.boxes.as_ptr();
    let box_capacity = pass.boxes.capacity();
    let child_pointer = pass.boxes[0].children.as_ptr();
    let child_capacity = pass.boxes[0].children.capacity();
    let mapping_capacity = pass.source_mapping.capacity();
    let mapping = pass.source_mapping.clone();
    let mut work = LayoutWorkingSet::default();
    publish(pass, &mut work);
    let (boxes, source_mapping) = owned_parts(&work);
    assert_eq!(boxes.as_ptr(), box_pointer);
    assert_eq!(boxes.capacity(), box_capacity);
    assert_eq!(boxes[0].children.as_ptr(), child_pointer);
    assert_eq!(boxes[0].children.capacity(), child_capacity);
    assert_eq!(source_mapping.capacity(), mapping_capacity);
    assert_eq!(source_mapping, &mapping);

    // These observations span one continuously owned allocation, through
    // take/restore/publish. No address is compared after cache eviction.
    source.scroll = LayoutPoint::new(3.0, 7.0);
    source.selection = Some(LayoutTextSelection::new(1, 4));
    source.label_revision = 1;
    styles.first_width = 200.0;
    styles.color = PaintColor::new(0.25, 0.5, 0.75, 1.0);
    let mut previous = work.take();
    let pass =
        build_layout_world_with_retained_inputs(&source, &mut styles, previous.as_mut()).unwrap();
    assert_eq!(pass.boxes.as_ptr(), box_pointer);
    assert_eq!(pass.boxes.capacity(), box_capacity);
    assert_eq!(pass.boxes[0].children.as_ptr(), child_pointer);
    assert_eq!(pass.boxes[0].children.capacity(), child_capacity);
    assert_eq!(pass.source_mapping, mapping);
    assert_eq!(pass.source_mapping.capacity(), mapping_capacity);
    assert_eq!(
        pass.construction_inputs
            .as_ref()
            .unwrap()
            .reused_owned_boxes(),
        pass.len()
    );
    let first = &pass.boxes[pass.source_box(1).unwrap().index()];
    assert_eq!(
        first.style.taffy.size.width,
        taffy::Dimension::length(200.0)
    );
    assert_eq!(first.style.background_color(), styles.color);
    assert_eq!(first.scroll_offset, source.scroll);
    assert_eq!(first.source_label, source.label(1));
    let text = &pass.boxes[pass.source_box(2).unwrap().index()];
    assert_eq!(text.text_selection, source.selection);
    assert_eq!(text.source_label, source.label(2));

    let (pass, _) =
        update_working_tree(previous, pass, &DocumentLayoutServices::new().font_inputs());
    publish(pass, &mut work);
    assert_eq!(owned_parts(&work).0.as_ptr(), box_pointer);
    assert_eq!(owned_parts(&work).0[0].children.as_ptr(), child_pointer);
    assert_eq!(work.stats().reused_owned_boxes, work.stats().retained_boxes);
}

#[test]
fn owned_construction_shape_changes_preserve_current_source_mappings() {
    for change in 0..4 {
        let mut source = Source::new();
        let mut styles = Styles::new();
        let mut work = LayoutWorkingSet::default();
        publish(
            build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap(),
            &mut work,
        );
        owned_parts(&work);
        match change {
            0 => {
                source.nodes[0].children.push(6);
                source.nodes.push(Node::element(vec![7]));
                source.nodes.push(Node::text("appended paragraph"));
            }
            1 => source.nodes[0].children.swap(0, 1),
            2 => styles.hidden = false,
            3 => styles.root_display = LayoutDisplay::Flex,
            _ => unreachable!(),
        }
        let mut previous = work.take();
        let pass = build_layout_world_with_retained_inputs(&source, &mut styles, previous.as_mut())
            .unwrap();
        assert_eq!(
            pass.construction_inputs
                .as_ref()
                .map_or(0, |inputs| inputs.reused_owned_boxes()),
            if change == 0 { 5 } else { 0 }
        );
        let cold = build_layout_world(&source, &mut styles).unwrap();
        assert_eq!(pass.source_mapping, cold.source_mapping);
        assert_eq!(pass.display_contents_mapping, cold.display_contents_mapping);
        assert_eq!(pass.len(), cold.len());
        for (actual, expected) in pass.boxes.iter().zip(&cold.boxes) {
            assert_eq!(actual.source, expected.source);
            assert_eq!(actual.kind, expected.kind);
            assert_eq!(actual.parent, expected.parent);
            assert_eq!(actual.structural_parent, expected.structural_parent);
            assert_eq!(actual.children, expected.children);
            assert_eq!(actual.text, expected.text);
        }
        let (pass, _) =
            update_working_tree(previous, pass, &DocumentLayoutServices::new().font_inputs());
        publish(pass, &mut work);
        assert_eq!(
            work.stats().reused_owned_boxes,
            if change == 0 { 5 } else { 0 }
        );
        if change == 3 {
            assert!(matches!(
                work.retained.as_ref().unwrap().graph,
                RetainedGraph::Compact(_)
            ));
            assert!(
                work.retained
                    .as_ref()
                    .unwrap()
                    .construction_inputs()
                    .is_none()
            );
        } else {
            owned_parts(&work);
        }
    }
}

#[test]
fn terminal_block_list_edits_keep_survivor_ids_and_match_full_cold_output() {
    let mut source = Source::new();
    let mut styles = Styles::new();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    let frozen = compare_with_cold(&source, &mut styles, &mut services, &mut work)
        .tree
        .boxes;
    let survivor_children = owned_parts(&work).0[1].children.as_ptr();
    for cycle in 0..3 {
        let old_boxes = work.stats().retained_boxes;
        let node = source.nodes.len();
        source.nodes.push(Node::element(vec![node + 1]));
        source.nodes.push(Node::text(&format!("new tail {cycle}")));
        source.nodes[0].children.push(node);
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, old_boxes);
        assert_eq!(work.stats().retained_boxes, old_boxes + 2);
        assert_eq!(owned_parts(&work).0[1].children.as_ptr(), survivor_children);
        assert_eq!(owned_parts(&work).1[&1].index(), 1);

        source.nodes[0].children.pop();
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, old_boxes);
        assert_eq!(work.stats().retained_boxes, old_boxes);
        assert!(!owned_parts(&work).1.contains_key(&node));
        assert_eq!(owned_parts(&work).0[1].children.as_ptr(), survivor_children);
    }
    let current = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(current.tree.boxes, frozen);
}

#[test]
fn nonterminal_and_unsupported_tail_edits_fall_back_to_the_shared_constructor() {
    for unsupported in [false, true] {
        let mut source = Source::new();
        let mut styles = Styles::new();
        let mut services = DocumentLayoutServices::new();
        let mut work = LayoutWorkingSet::default();
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        let node = source.nodes.len();
        source.nodes.push(Node::element(vec![node + 1]));
        source.nodes.push(Node::text("new subtree"));
        if unsupported {
            source.nodes[0].children.push(node);
            styles.positioned = Some((node, crate::LayoutPosition::Absolute));
        } else {
            // This parent has a following constructed sibling. Inserting
            // below it cannot preserve both preorder and surviving IDs.
            source.nodes[1].children.clear();
            source.nodes[1].children.push(node);
        }
        compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(work.stats().reused_owned_boxes, 0);
    }
}

#[test]
fn terminal_batch_removal_retires_all_deleted_closures_without_remapping_survivors() {
    let mut source = Source::new();
    let mut styles = Styles::new();
    styles.hidden = false;
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    source.nodes[0].children.clear();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().retained_boxes, 1);
    assert_eq!(work.stats().reused_owned_boxes, 1);
    assert_eq!(owned_parts(&work).1.len(), 1);
    assert_eq!(owned_parts(&work).1[&0].index(), 0);
}

#[test]
fn terminal_subtrees_with_discarded_whitespace_use_the_shared_compaction_path() {
    let mut source = Source::new();
    let mut styles = Styles::new();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    let node = source.nodes.len();
    source.nodes.push(Node::element(vec![node + 1, node + 2]));
    source.nodes.push(Node::text(" \t\n"));
    source.nodes.push(Node::element(vec![node + 3]));
    source.nodes.push(Node::text("nested block"));
    source.nodes[0].children.push(node);
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    assert!(!owned_parts(&work).1.contains_key(&(node + 1)));
}

#[test]
fn owned_construction_capacity_is_reported_even_when_slots_are_unused() {
    let source = Source::new();
    let mut styles = Styles::new();
    let baseline_pass =
        build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    let base_box_capacity = baseline_pass.boxes.capacity();
    let base_child_capacity = baseline_pass.boxes[0].children.capacity();
    let mut baseline = LayoutWorkingSet::default();
    publish(baseline_pass, &mut baseline);
    owned_parts(&baseline);

    let mut pass = build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    pass.boxes.reserve_exact(31);
    pass.boxes[0].children.reserve_exact(23);
    let additional = (pass.boxes.capacity() - base_box_capacity)
        * std::mem::size_of::<LayoutBox<usize>>()
        + (pass.boxes[0].children.capacity() - base_child_capacity)
            * std::mem::size_of::<LayoutBoxId>();
    let mut grown = LayoutWorkingSet::default();
    publish(pass, &mut grown);
    owned_parts(&grown);
    assert_eq!(
        grown.stats().retained_boxes,
        baseline.stats().retained_boxes
    );
    assert_eq!(
        grown.stats().estimated_retained_bytes,
        baseline.stats().estimated_retained_bytes + additional
    );
}

#[test]
fn owned_construction_releases_paint_resources_and_owned_dependencies() {
    for explicit_clear in [false, true] {
        let source = Source::new();
        let dependency = Arc::downgrade(&source.hidden_name);
        let mut pass =
            build_layout_world_with_retained_inputs(&source, &mut Styles::new(), None).unwrap();
        let pixels = Arc::new(moli_image::RgbaImage::try_new(1, 1, vec![0; 4]).unwrap());
        let pixels_weak = Arc::downgrade(&pixels);
        let image = LayoutImageResource {
            intrinsic_width: 1.0,
            intrinsic_height: 1.0,
            pixels: Some(pixels),
            svg: None,
        };
        pass.boxes[0].replaced_image = Some(image.clone());
        pass.boxes[0].css_images.background = vec![Some(image.clone())];
        pass.boxes[0].css_images.mask = vec![Some(image)];
        drop(source);
        let mut work = LayoutWorkingSet::default();
        publish(pass, &mut work);
        let (boxes, _) = owned_parts(&work);
        assert!(boxes[0].replaced_image.is_none());
        assert_eq!(boxes[0].css_images.background.capacity(), 0);
        assert_eq!(boxes[0].css_images.mask.capacity(), 0);
        assert!(pixels_weak.upgrade().is_none());
        assert!(dependency.upgrade().is_some());
        if explicit_clear {
            work.clear();
            assert!(dependency.upgrade().is_none());
            assert_eq!(work.stats(), LayoutWorkingSetStats::default());
        }
        drop(work);
        assert!(dependency.upgrade().is_none());
    }
}

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

fn compare_with_cold<R: LayoutStyleResolver<usize>>(
    source: &Source,
    styles: &mut R,
    services: &mut DocumentLayoutServices,
    work: &mut LayoutWorkingSet<usize>,
) -> LayoutPassResult<usize> {
    let request =
        LayoutPassRequest::with_paint(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test);
    let retained = build_layout_pass_with_working_set(
        source,
        styles,
        services,
        request,
        &mut NoFrames,
        Some(work),
    )
    .unwrap();
    let cold = build_layout_pass(source, styles, services, request).unwrap();
    assert_eq!(retained.tree.boxes, cold.tree.boxes);
    assert_eq!(retained.tree.fragments, cold.tree.fragments);
    assert_eq!(retained.paint_snapshot(), cold.paint_snapshot());
    for source in 0..source.nodes.len() {
        for query in [
            LayoutQuery::ElementMetrics { source },
            LayoutQuery::ClientRects { source },
            LayoutQuery::TextRangeRects {
                source,
                utf16_range: 0..usize::MAX,
            },
            LayoutQuery::IntersectionGeometry {
                target: source,
                root: None,
            },
            LayoutQuery::UsedGridTracks { source },
        ] {
            assert_eq!(retained.answer_query(&query), cold.answer_query(&query));
        }
    }
    for y in (0..240).step_by(23) {
        for x in (0..320).step_by(29) {
            let point = LayoutPoint::new(x as f32, y as f32);
            assert_eq!(retained.hit_test(point, false), cold.hit_test(point, false));
        }
    }
    retained
}

#[test]
fn owned_construction_updates_match_cold_and_evicted_output_without_mutating_a_frozen_sample() {
    let mut source = Source::new();
    let mut styles = Styles::new();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    let first = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    owned_parts(&work);
    let frozen_boxes = first.tree.boxes.clone();
    let frozen_fragments = first.tree.fragments.clone();
    styles.first_width = 200.0;
    styles.color = PaintColor::new(1.0, 0.25, 0.0, 1.0);
    source.scroll = LayoutPoint::new(0.0, 9.0);
    source.selection = Some(LayoutTextSelection::new(0, 5));
    let warm = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert!(work.stats().reused_owned_boxes > 0);
    assert_eq!(first.tree.boxes, frozen_boxes);
    assert_eq!(first.tree.fragments, frozen_fragments);
    work.clear();
    let evicted = compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    assert_eq!(warm.tree.boxes, evicted.tree.boxes);
    assert_eq!(warm.tree.fragments, evicted.tree.fragments);
    assert_eq!(warm.paint_snapshot(), evicted.paint_snapshot());
}

#[test]
fn owned_construction_late_validation_error_discards_checkout_and_recovers_cold() {
    let source = Source::new();
    let dependency = Arc::downgrade(&source.hidden_name);
    let mut styles = Styles::new();
    let mut services = DocumentLayoutServices::new();
    let mut work = LayoutWorkingSet::default();
    drop(compare_with_cold(
        &source,
        &mut styles,
        &mut services,
        &mut work,
    ));
    owned_parts(&work);
    drop(source);
    assert!(dependency.upgrade().is_some());
    let source = Source::new();
    styles.first_width = 200.0;
    styles.fail_at = Some(3);
    styles.visited.clear();
    let error = build_layout_pass_with_working_set(
        &source,
        &mut styles,
        &mut services,
        LayoutPassRequest::new(LayoutViewport::new(320, 240, 1.0), LayoutFlushReason::Test),
        &mut NoFrames,
        Some(&mut work),
    )
    .err()
    .expect("later principal style must fail");
    assert!(matches!(error, LayoutError::StyleResolution { .. }));
    assert_eq!(styles.visited, [0, 1, 3]);
    assert!(work.retained.is_none());
    assert_eq!(work.stats(), LayoutWorkingSetStats::default());
    assert!(dependency.upgrade().is_none());
    styles.fail_at = None;
    compare_with_cold(&source, &mut styles, &mut services, &mut work);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    owned_parts(&work);
}

fn assert_medium_owned_construction(paragraphs: usize) {
    use style::values::computed::font::{
        FamilyName, FontFamily, FontFamilyList, FontFamilyNameSyntax, SingleFontFamily,
    };

    let mut source = Source::new();
    source.nodes = vec![Node::element(Vec::with_capacity(paragraphs))];
    for _ in 0..paragraphs {
        let owner = source.nodes.len();
        source.nodes[0].children.push(owner);
        source.nodes.push(Node::element(vec![owner + 1]));
        source.nodes.push(Node::text("ABCD"));
    }
    let mut font = style::properties::style_structs::Font::initial_values();
    font.set_font_family(FontFamily {
        families: FontFamilyList {
            list: style::ArcSlice::from_iter(std::iter::once(SingleFontFamily::FamilyName(
                FamilyName {
                    name: style::Atom::from("Moli Ahem"),
                    syntax: FontFamilyNameSyntax::Quoted,
                },
            ))),
        },
        is_system_font: false,
        is_initial: false,
    });
    let mut styles = Styles::new();
    styles.hidden = false;
    styles.font_style = Some(
        ResolvedLayoutStyle::from_stylo(
            style::properties::ComputedValues::initial_values_with_font_override(font),
        )
        .with_text_metrics(16.0, 20.0),
    );
    let mut services =
        DocumentLayoutServices::with_system_font_policy(crate::SystemFontPolicy::Disabled);
    services
        .register_web_font(crate::WebFontRegistration::new(
            "owned-medium-ahem",
            crate::WebFontFace::new("Moli Ahem"),
            include_bytes!("../../tests/fixtures/moli-ahem.ttf").to_vec(),
        ))
        .unwrap();
    let mut work = LayoutWorkingSet::default();
    let request =
        LayoutPassRequest::with_paint(LayoutViewport::new(1280, 720, 1.0), LayoutFlushReason::Test);
    let cold = build_layout_pass_with_working_set(
        &source,
        &mut styles,
        &mut services,
        request,
        &mut NoFrames,
        Some(&mut work),
    )
    .unwrap();
    let retained_capacity = match &work.retained.as_ref().unwrap().graph {
        RetainedGraph::Owned { boxes, .. } => (Some(boxes.capacity()), None),
        RetainedGraph::Compact(boxes) => (None, Some(boxes.capacity())),
    };
    eprintln!(
        "owned-medium paragraphs={paragraphs} sizeof_layout_box={} owned_capacity={:?} \
         compact_capacity={:?} stats={:?}",
        std::mem::size_of::<LayoutBox<usize>>(),
        retained_capacity.0,
        retained_capacity.1,
        work.stats(),
    );
    let construction_count = 1 + 2 * paragraphs;
    assert_eq!(work.stats().retained_boxes, construction_count);
    assert_eq!(work.stats().retained_boxes, construction_count);
    assert_eq!(work.stats().retained_owned_boxes, construction_count);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    let box_pointer = owned_parts(&work).0.as_ptr();
    let box_capacity = owned_parts(&work).0.capacity();
    let warm = build_layout_pass_with_working_set(
        &source,
        &mut styles,
        &mut services,
        request,
        &mut NoFrames,
        Some(&mut work),
    )
    .unwrap();
    eprintln!(
        "owned-medium-warm paragraphs={paragraphs} owned_capacity={} stats={:?}",
        owned_parts(&work).0.capacity(),
        work.stats(),
    );
    assert_eq!(work.stats().retained_owned_boxes, construction_count);
    assert_eq!(work.stats().reused_owned_boxes, construction_count);
    assert_eq!(owned_parts(&work).0.as_ptr(), box_pointer);
    assert_eq!(owned_parts(&work).0.capacity(), box_capacity);
    let one_shot = build_layout_pass(&source, &mut styles, &mut services, request).unwrap();
    for actual in [&cold, &warm] {
        assert_eq!(actual.tree.boxes, one_shot.tree.boxes);
        assert_eq!(actual.tree.fragments, one_shot.tree.fragments);
        assert_eq!(actual.paint_snapshot(), one_shot.paint_snapshot());
        for source in 0..construction_count {
            let query = LayoutQuery::ElementMetrics { source };
            assert_eq!(actual.answer_query(&query), one_shot.answer_query(&query));
        }
    }
}

#[test]
fn owned_construction_1024_text_blocks_retain_and_reuse_their_allocation() {
    assert_medium_owned_construction(1024);
}

#[test]
fn owned_construction_1152_text_blocks_retain_and_reuse_their_allocation() {
    assert_medium_owned_construction(1152);
}

#[test]
fn owned_construction_4096_text_blocks_retain_and_reuse_their_allocation() {
    assert_medium_owned_construction(4096);
}
