use std::collections::HashMap;

mod grid;

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
fn owned_construction_shape_changes_rebuild_current_source_mappings() {
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
            0
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
        assert_eq!(work.stats().reused_owned_boxes, 0);
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
fn owned_construction_capacity_is_charged_even_when_slots_are_unused() {
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
fn owned_construction_reclaims_spare_capacity_before_compact_fallback_and_rejection() {
    let source = Source::new();
    let mut styles = Styles::new();
    let mut live_pass =
        build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    let construction_count = live_pass.len();
    live_pass.boxes.shrink_to_fit();
    let live_capacity = live_pass.boxes.capacity();
    let mut baseline = LayoutWorkingSet::default();
    publish(live_pass, &mut baseline);
    assert_eq!(owned_parts(&baseline).0.capacity(), live_capacity);
    let live_owned_charge = baseline.stats().estimated_retained_bytes;
    let mut compact_probe = baseline.take().unwrap();
    compact_probe.compact_graph();
    let compact_charge = compact_probe.estimated_retention().total();
    drop(compact_probe);
    assert!(compact_charge < live_owned_charge);

    let mut pass = build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    let extra_slots =
        DEFAULT_DOCUMENT_WORKING_SET_BYTES / std::mem::size_of::<LayoutBox<usize>>() + 1;
    pass.boxes.reserve_exact(extra_slots);
    let oversized_capacity = pass.boxes.capacity();
    assert!(
        oversized_capacity * std::mem::size_of::<LayoutBox<usize>>()
            > DEFAULT_DOCUMENT_WORKING_SET_BYTES
    );
    let mut work = LayoutWorkingSet::default();
    publish(pass, &mut work);
    assert_eq!(
        work.limits().max_estimated_retained_bytes,
        DEFAULT_DOCUMENT_WORKING_SET_BYTES
    );
    assert_eq!(work.stats().admission, LayoutWorkingSetAdmission::Retained);
    assert_eq!(work.stats().retained_owned_boxes, construction_count);
    assert_eq!(owned_parts(&work).0.capacity(), live_capacity);
    assert_eq!(
        work.stats().owned_box_capacity_trimmed,
        oversized_capacity - live_capacity
    );
    assert!(work.stats().owned_box_capacity_trimmed > 0);
    assert_eq!(work.stats().candidate_owned_bytes, live_owned_charge);
    assert_eq!(work.stats().estimated_retained_bytes, live_owned_charge);

    // Use the measured compact charge of this same graph and dependencies,
    // not a source-count proxy. Even after real spare-capacity reclamation,
    // the lower limit admits compact records but cannot admit live boxes.
    let fallback_limit = compact_charge + (live_owned_charge - compact_charge) / 2;
    assert!(compact_charge <= fallback_limit);
    assert!(fallback_limit < live_owned_charge);
    let pass = build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    let mut fallback = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: fallback_limit,
        ..LayoutWorkingSetLimits::default()
    });
    publish(pass, &mut fallback);
    assert_eq!(
        fallback.stats().admission,
        LayoutWorkingSetAdmission::Retained
    );
    assert!(matches!(
        fallback.retained.as_ref().unwrap().graph,
        RetainedGraph::Compact(_)
    ));
    assert_eq!(fallback.stats().retained_owned_boxes, 0);
    assert_eq!(fallback.stats().candidate_owned_bytes, live_owned_charge);
    assert_eq!(fallback.stats().estimated_retained_bytes, compact_charge);
    assert_eq!(
        fallback.limits().max_estimated_retained_bytes,
        fallback_limit
    );

    // The same source dependencies also obey rejection when even compact
    // storage cannot fit. A rejected candidate cannot keep its hidden source.
    let dependency = Arc::downgrade(&source.hidden_name);
    let pass = build_layout_world_with_retained_inputs(&source, &mut styles, None).unwrap();
    work.clear();
    fallback.clear();
    drop(source);
    let mut rejected = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: 1,
        ..LayoutWorkingSetLimits::default()
    });
    publish(pass, &mut rejected);
    assert_eq!(
        rejected.stats().admission,
        LayoutWorkingSetAdmission::EstimatedByteLimit
    );
    assert_eq!(rejected.stats().estimated_retained_bytes, 0);
    assert!(rejected.retained.is_none());
    assert!(dependency.upgrade().is_none());
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
    assert_eq!(work.stats().candidate_boxes, construction_count);
    assert_eq!(work.stats().retained_boxes, construction_count);
    assert_eq!(work.stats().retained_owned_boxes, construction_count);
    assert_eq!(work.stats().reused_owned_boxes, 0);
    assert_eq!(
        work.limits().max_estimated_retained_bytes,
        DEFAULT_DOCUMENT_WORKING_SET_BYTES
    );
    assert!(work.stats().candidate_owned_bytes <= DEFAULT_DOCUMENT_WORKING_SET_BYTES);
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
fn owned_construction_1024_text_blocks_fit_the_default_budget_and_reuse_their_allocation() {
    assert_medium_owned_construction(1024);
}

#[test]
fn owned_construction_1152_text_blocks_fit_the_default_budget_and_reuse_their_allocation() {
    assert_medium_owned_construction(1152);
}

#[test]
fn owned_construction_pressure_retains_real_numeric_suffix_headroom() {
    for position in [
        crate::LayoutPosition::Absolute,
        crate::LayoutPosition::Fixed,
    ] {
        // The positioned child follows an ordinary block in a static container.
        // Its containing block differs from its construction parent, and auto
        // insets require the numeric backend to append a static-position box.
        let mut source = Source::new();
        source.nodes = vec![
            Node::element(vec![1]),
            Node::element(vec![2, 3]),
            Node::element(Vec::new()),
            Node::element(Vec::new()),
        ];
        let mut styles = Styles::new();
        styles.positioned = Some((3, position));
        let mut services = DocumentLayoutServices::new();
        let mut probe = LayoutWorkingSet::default();
        let reference = compare_with_cold(&source, &mut styles, &mut services, &mut probe);
        let construction_count = probe.stats().candidate_boxes;
        let pass_count = reference.metrics.box_count;
        assert_eq!(construction_count, 4);
        assert_eq!(pass_count, construction_count + 1, "{position:?}");
        assert_eq!(probe.stats().retained_owned_boxes, construction_count);
        assert_eq!(probe.stats().owned_box_capacity_trimmed, 0);
        let spare_capacity = owned_parts(&probe).0.capacity();
        assert!(
            spare_capacity > pass_count,
            "fixture needs real spare slots"
        );
        let spare_charge = probe.stats().estimated_retained_bytes;

        // Measure this actual retained candidate with and without spare outer
        // slots. No byte constant or construction-count proxy chooses the cap.
        let mut trimmed_probe = probe.take().unwrap();
        let removed = trimmed_probe.trim_owned_box_capacity();
        let needed_capacity = match &trimmed_probe.graph {
            RetainedGraph::Owned { boxes, .. } => boxes.capacity(),
            RetainedGraph::Compact(_) => panic!("small fixture must retain owned boxes"),
        };
        let needed_charge = trimmed_probe.estimated_retention().total();
        assert_eq!(needed_capacity, pass_count);
        assert_eq!(removed, spare_capacity - needed_capacity);
        assert!(removed > 0);
        assert_eq!(
            spare_charge - needed_charge,
            removed * std::mem::size_of::<LayoutBox<usize>>()
        );
        drop(trimmed_probe);
        let limit = needed_charge + (spare_charge - needed_charge) / 2;
        assert!(needed_charge < limit && limit < spare_charge);
        assert!(limit < DEFAULT_DOCUMENT_WORKING_SET_BYTES);
        let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
            max_estimated_retained_bytes: limit,
            ..LayoutWorkingSetLimits::default()
        });

        // The public full pass, not the measuring helper, must now reclaim
        // spare capacity while preserving room for its actual numeric suffix.
        let cold = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(cold.metrics.box_count, pass_count);
        assert_eq!(work.stats().candidate_boxes, construction_count);
        assert_eq!(work.stats().admission, LayoutWorkingSetAdmission::Retained);
        assert_eq!(work.stats().retained_owned_boxes, construction_count);
        assert_eq!(work.stats().reused_owned_boxes, 0);
        assert_eq!(work.stats().owned_box_capacity_trimmed, removed);
        assert_eq!(work.stats().candidate_owned_bytes, needed_charge);
        assert_eq!(work.stats().estimated_retained_bytes, needed_charge);
        let (boxes, mapping) = owned_parts(&work);
        assert_eq!(boxes.len(), construction_count);
        assert_eq!(boxes.capacity(), pass_count);
        assert!(mapping.values().all(|id| id.index() < construction_count));
        let pointer = boxes.as_ptr();
        let capacity = boxes.capacity();

        // Numeric preparation appends again. Trimming to construction length
        // would force regrowth here even though the source graph is unchanged.
        let warm = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(warm.metrics.box_count, pass_count);
        assert_eq!(work.stats().retained_owned_boxes, construction_count);
        assert_eq!(work.stats().reused_owned_boxes, construction_count);
        assert_eq!(work.stats().owned_box_capacity_trimmed, 0);
        assert_eq!(work.stats().estimated_retained_bytes, needed_charge);
        assert_eq!(owned_parts(&work).0.as_ptr(), pointer);
        assert_eq!(owned_parts(&work).0.capacity(), capacity);
        assert_eq!(reference.tree.boxes, warm.tree.boxes);
        assert_eq!(reference.tree.fragments, warm.tree.fragments);
        assert_eq!(reference.paint_snapshot(), warm.paint_snapshot());

        // Each pass above also compares all fixture queries and a hit grid to
        // a fresh one-shot pass. Explicit eviction must preserve the same data.
        work.clear();
        let evicted = compare_with_cold(&source, &mut styles, &mut services, &mut work);
        assert_eq!(evicted.metrics.box_count, pass_count);
        assert_eq!(work.stats().reused_owned_boxes, 0);
        assert_eq!(work.stats().retained_owned_boxes, construction_count);
        assert_eq!(work.stats().owned_box_capacity_trimmed, removed);
        assert_eq!(owned_parts(&work).0.capacity(), pass_count);
        assert_eq!(warm.tree.boxes, evicted.tree.boxes);
        assert_eq!(warm.tree.fragments, evicted.tree.fragments);
        assert_eq!(warm.paint_snapshot(), evicted.paint_snapshot());
    }
}
