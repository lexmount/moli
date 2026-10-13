use std::sync::Arc;

use super::*;
use crate::{
    DocumentLayoutServices, LayoutBoxKind, LayoutDisplay, LayoutElementCategory,
    LayoutElementSemantics, LayoutFormControlData, LayoutImageResource, LayoutNamespace,
    PaintColor, ResolvedLayoutStyle,
};

pub(super) fn world(text: Option<Arc<str>>) -> LayoutWorld<usize> {
    LayoutWorld::new(
        LayoutWorld::new_box(
            Some(0),
            Some(0),
            None,
            "root".into(),
            Some("root".into()),
            None,
            None,
            LayoutBoxKind::PrincipalBlock,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                taffy::Style::default(),
                PaintColor::TRANSPARENT,
            ),
            text,
        ),
        true,
    )
}

#[test]
fn retained_work_drops_pass_image_resources_but_keeps_reusable_source_inputs() {
    let pixels = Arc::new(moli_image::RgbaImage::try_new(1, 1, vec![0; 4]).unwrap());
    let image_weak = Arc::downgrade(&pixels);
    let resource = LayoutImageResource {
        intrinsic_width: 1.0,
        intrinsic_height: 1.0,
        pixels: Some(pixels),
        svg: None,
    };
    let text: Arc<str> = Arc::from("retained input");
    let text_weak = Arc::downgrade(&text);
    let mut world = world(Some(text));
    world.boxes[0].replaced_image = Some(resource.clone());
    world.boxes[0].css_images.background = vec![Some(resource.clone())];
    world.boxes[0].css_images.mask = vec![Some(resource)];
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut work = LayoutWorkingSet::default();
    work.publish(world, fonts, 0, 0, 1);

    assert_eq!(work.stats().retained_boxes, 1);
    assert!(
        image_weak.upgrade().is_none(),
        "working state must not pin paint images"
    );
    assert!(
        text_weak.upgrade().is_some(),
        "source comparison still owns its input"
    );
    work.clear();
    assert!(
        text_weak.upgrade().is_none(),
        "eviction must release retained source inputs"
    );
    assert_eq!(work.stats().estimated_retained_bytes, 0);
}

#[test]
fn retained_control_payload_capacity_is_reported_and_released_on_clear() {
    fn control_world(label: Arc<str>) -> LayoutWorld<usize> {
        let mut world = world(None);
        let mut semantics = LayoutElementSemantics::new(
            LayoutNamespace::Html,
            "select",
            LayoutElementCategory::Generic,
            None,
        );
        semantics.metadata.form_control = Some(LayoutFormControlData {
            option_labels: vec![label],
            ..LayoutFormControlData::default()
        });
        world.boxes[0].element_semantics = Some(semantics);
        world
    }

    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(control_world(Arc::from("")), fonts.clone(), 0, 0, 1);
    let mut work = LayoutWorkingSet::default();
    let label: Arc<str> = Arc::from("a".repeat(64 * 1024));
    let label_weak = Arc::downgrade(&label);
    work.publish(control_world(label), fonts, 0, 0, 1);
    let stats = work.stats();
    assert!(stats.estimated_retained_bytes > baseline.stats().estimated_retained_bytes);
    assert_eq!(stats.retained_boxes, 1);
    assert_eq!(stats.retained_text_bytes, 0);
    assert_eq!(
        stats.estimated_retained_bytes,
        stats.estimated_graph_bytes
            + stats.estimated_inline_bytes
            + stats.estimated_font_metadata_bytes
    );
    assert!(label_weak.upgrade().is_some());
    work.clear();
    assert!(label_weak.upgrade().is_none());
}

#[test]
fn numeric_placeholders_do_not_survive_as_construction_graph_members() {
    let mut previous = world(None);
    let placeholder = world(None).boxes.remove(0);
    let placeholder_id = previous.allocate(placeholder);
    previous.boxes[0].layout_children.push(placeholder_id);
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut work = LayoutWorkingSet::default();
    work.publish(previous, fonts.clone(), 0, 0, 1);

    assert_eq!(work.stats().retained_boxes, 1);
    let (fresh, reused) = update_working_tree(work.take(), world(None), &fonts);
    assert_eq!(
        reused, 1,
        "numeric-only boxes must not invalidate a stable graph"
    );
    assert_eq!(fresh.boxes.len(), 1);
    assert!(fresh.boxes[0].layout_children.is_empty());
}

#[test]
fn compact_retention_discards_pass_only_allocation_capacity() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(world(None), fonts.clone(), 0, 0, 1);
    let baseline_bytes = baseline.stats().estimated_retained_bytes;
    let mut work = LayoutWorkingSet::default();
    let mut pass = world(None);
    // Pass capacity can outlive its numeric placeholders until publication.
    // Retaining that full LayoutBox allocation defeats the compact graph.
    pass.boxes.reserve(1024);
    work.publish(pass, fonts, 0, 0, 1);

    assert_eq!(work.stats().retained_boxes, 1);
    assert_eq!(work.stats().estimated_retained_bytes, baseline_bytes);
}

#[test]
fn retained_text_above_the_former_document_limit_survives_until_clear() {
    let text: Arc<str> = Arc::from("x".repeat(512 * 1024));
    let weak = Arc::downgrade(&text);
    let mut work = LayoutWorkingSet::default();
    work.publish(
        world(Some(text)),
        DocumentLayoutServices::new().font_inputs(),
        0,
        0,
        1,
    );
    assert_eq!(work.stats().retained_text_bytes, 512 * 1024);
    assert!(weak.upgrade().is_some());
    work.clear();
    assert!(weak.upgrade().is_none());
}
