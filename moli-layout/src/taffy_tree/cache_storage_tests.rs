use super::*;
use crate::{LayoutDisplay, PaintColor, ResolvedLayoutStyle};
use taffy::WritingMode;

fn block(source: usize) -> crate::LayoutBox<usize> {
    LayoutWorld::new_box(
        Some(source),
        None,
        None,
        format!("cache-block-{source}"),
        None,
        None,
        None,
        LayoutBoxKind::PrincipalBlock,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            Style::default(),
            PaintColor::TRANSPARENT,
        ),
        None,
    )
}

fn input() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        sizing_purpose: SizingPurpose::Layout,
        axis: taffy::RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(60.0),
            height: Some(20.0),
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size {
            width: Some(240.0),
            height: None,
        },
        parent_writing_mode: WritingMode::HorizontalTb,
        available_space: Size {
            width: AvailableSpace::Definite(60.0),
            height: AvailableSpace::MaxContent,
        },
        block_auto_behavior: AutoSizeBehavior::FitContent,
        vertical_margins_are_collapsible: taffy::Line::FALSE,
    }
}

fn output(width: f32) -> LayoutOutput {
    let size = Size {
        width,
        height: 20.0,
    };
    LayoutOutput::from_sizes(size, size)
}

#[test]
fn numeric_cache_growth_keeps_box_viewport_and_invalidation_owners_distinct() {
    let mut world = LayoutWorld::new(block(0), false);
    let input = input();
    let root = world.root().to_taffy();
    assert_eq!(world.cache_get(root, &input), None);
    world.cache_store(root, &input, output(60.0));
    assert_eq!(world.cache_get(root, &input), Some(output(60.0)));
    assert_eq!(
        world.cache_get(
            root,
            &LayoutInput {
                sizing_purpose: SizingPurpose::IntrinsicContribution,
                ..input
            }
        ),
        None
    );

    // Numeric box growth reuses the old viewport ID for a real box. Its
    // cache starts empty, while the viewport's own cache survives the growth.
    let sibling = world.viewport_taffy_node();
    world.cache_store(sibling, &input, output(70.0));
    world.boxes.push(block(1));
    assert_eq!(world.cache_get(sibling, &input), None);
    let viewport = world.viewport_taffy_node();
    assert_eq!(world.cache_get(viewport, &input), Some(output(70.0)));

    world.cache_store(sibling, &input, output(80.0));
    world.cache_clear(root);
    assert_eq!(world.cache_get(root, &input), None);
    assert_eq!(world.cache_get(sibling, &input), Some(output(80.0)));
    assert_eq!(world.cache_get(viewport, &input), Some(output(70.0)));
    world.cache_clear(viewport);
    assert_eq!(world.cache_get(viewport, &input), None);
    assert_eq!(world.cache_get(sibling, &input), Some(output(80.0)));

    let allocated = world.box_caches.len();
    world.cache_store(root, &input, output(90.0));
    assert_eq!(world.cache_get(root, &input), Some(output(90.0)));
    assert_eq!(world.box_caches.len(), allocated);
}

#[test]
fn numeric_cache_allocation_follows_stores_in_large_flattened_and_block_trees() {
    for flattened in [false, true] {
        let mut world = LayoutWorld::new(block(0), false);
        for source in 1..=4096 {
            let mut child = block(source);
            if flattened {
                child.inline_flattened = true;
                child.inline_context_owner = Some(world.root());
            }
            world.boxes.push(child);
        }
        let sibling = LayoutBoxId::from_index(world.len());
        world.boxes.push(block(4097));
        let input = input();
        let root = world.root().to_taffy();
        world.cache_store(root, &input, output(60.0));
        world.cache_store(sibling.to_taffy(), &input, output(80.0));
        assert_eq!(world.cache_get(root, &input), Some(output(60.0)));
        assert_eq!(
            world.cache_get(sibling.to_taffy(), &input),
            Some(output(80.0))
        );
        let allocated_bytes = world.box_caches.capacity() * std::mem::size_of::<taffy::Cache>();
        assert!(
            allocated_bytes < 64 * 1024,
            "4098 construction boxes with two stores should not allocate dense caches: {allocated_bytes} bytes"
        );
        assert_eq!(world.box_caches.len(), 2);
        // Unvisited boxes, including flattened boxes, can still store later.
        let previously_unvisited = LayoutBoxId::from_index(4096).to_taffy();
        world.cache_store(previously_unvisited, &input, output(90.0));
        assert_eq!(
            world.cache_get(previously_unvisited, &input),
            Some(output(90.0))
        );
    }
}

#[test]
fn numeric_cache_bindings_follow_boxes_through_compaction_and_growth() {
    let mut world = LayoutWorld::new(block(0), false);
    let root = world.root();
    let discarded = world.allocate(block(1));
    let kept = world.allocate(block(2));
    world.replace_children(root, vec![kept]).unwrap();
    world.map_source(2, kept);
    let input = input();
    world.cache_store(root.to_taffy(), &input, output(60.0));
    world.cache_store(discarded.to_taffy(), &input, output(70.0));
    world.cache_store(kept.to_taffy(), &input, output(80.0));

    world.compact_reachable();
    let kept = world.source_box(2).unwrap();
    assert_eq!(kept.index(), 1);
    assert_eq!(world.cache_get(kept.to_taffy(), &input), Some(output(80.0)));

    // Grow both vectors past their initial allocations. A recycled box ID
    // must not inherit a discarded box's cache or replace a surviving cache.
    let mut appended = Vec::new();
    for source in 3..35 {
        let id = world.allocate(block(source));
        assert_eq!(world.cache_get(id.to_taffy(), &input), None);
        world.cache_store(id.to_taffy(), &input, output(source as f32));
        appended.push((id, source));
    }
    for (id, source) in appended {
        assert_eq!(
            world.cache_get(id.to_taffy(), &input),
            Some(output(source as f32))
        );
    }
    world.cache_clear(kept.to_taffy());
    assert_eq!(world.cache_get(kept.to_taffy(), &input), None);
    assert_eq!(world.cache_get(root.to_taffy(), &input), Some(output(60.0)));
}

#[test]
fn hidden_numeric_store_does_not_materialize_empty_cache_storage() {
    let mut world = LayoutWorld::new(block(0), false);
    let input = LayoutInput {
        run_mode: RunMode::PerformHiddenLayout,
        ..input()
    };
    world.cache_store(world.root().to_taffy(), &input, output(60.0));
    world.cache_store(world.viewport_taffy_node(), &input, output(70.0));
    assert!(world.box_caches.is_empty());
    assert_eq!(world.cache_get(world.root().to_taffy(), &input), None);
}
