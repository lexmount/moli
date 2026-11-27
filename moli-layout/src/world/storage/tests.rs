use super::*;
use crate::{LayoutBoxKind, LayoutDisplay, LayoutWorld, PaintColor, ResolvedLayoutStyle};

fn block(source: usize) -> LayoutBox<usize> {
    LayoutWorld::new_box(
        Some(source),
        None,
        None,
        format!("box-{source}"),
        None,
        None,
        None,
        LayoutBoxKind::PrincipalBlock,
        ResolvedLayoutStyle::synthetic(
            LayoutDisplay::Block,
            taffy::Style::default(),
            PaintColor::TRANSPARENT,
        ),
        None,
    )
}

#[test]
fn order_and_storage_changes_preserve_handles_and_reject_retired_generations() {
    let mut boxes = LayoutBoxes::default();
    let first = boxes.insert(block(1));
    let removed = boxes.insert(block(2));
    let last = boxes.insert(block(3));
    let original_order = boxes.order_snapshot();
    assert!(boxes.retain_in_order(vec![last, first]));
    assert_eq!(boxes.get(first).unwrap().source(), Some(1));
    assert_eq!(boxes.get(last).unwrap().source(), Some(3));
    assert_eq!(
        boxes
            .ordered()
            .map(|value| value.source())
            .collect::<Vec<_>>(),
        vec![Some(3), Some(1)]
    );
    assert_eq!(boxes.index_of(last), 0);
    assert_eq!(boxes.index_of(first), 1);
    assert!(boxes.get(removed).is_none());
    // A published order is immutable even when the working order changes.
    assert_eq!(original_order.id_at(0), first);
    assert_eq!(original_order.id_at(1), removed);

    let mut current = boxes.insert(block(4));
    let capacity = boxes.capacity();
    let mut retired = vec![removed];
    for source in 5..105 {
        boxes.retain_in_order(vec![last, first]);
        retired.push(current);
        current = boxes.insert(block(source));
        assert_eq!(boxes.capacity(), capacity);
        assert_eq!(boxes.get(current).unwrap().source(), Some(source));
        assert_eq!(boxes.get(first).unwrap().source(), Some(1));
        assert_eq!(boxes.get(last).unwrap().source(), Some(3));
        for old in &retired {
            assert!(
                boxes.get(*old).is_none(),
                "a retired handle must never name a new box"
            );
            assert_ne!(*old, current);
        }
    }
}

#[test]
fn viewport_node_ids_do_not_alias_growing_or_recycled_box_handles() {
    let mut world = LayoutWorld::new(block(0), false);
    let viewport = world.viewport_taffy_node();
    let fixed = world.fixed_viewport_taffy_node();
    assert_ne!(viewport, fixed);
    let mut old = Vec::new();
    for source in 1..65 {
        let handle = world.allocate(block(source));
        assert_ne!(handle.to_taffy(), viewport);
        assert_ne!(handle.to_taffy(), fixed);
        assert_eq!(LayoutBoxId::from_taffy(handle.to_taffy()), handle);
        assert_eq!(world.viewport_taffy_node(), viewport);
        assert_eq!(world.fixed_viewport_taffy_node(), fixed);
        old.push(handle);
    }
    let root = world.root();
    world.compact_for_reuse(vec![root]);
    for source in 65..129 {
        let handle = world.allocate(block(source));
        assert!(old.iter().all(|key| *key != handle));
        assert_eq!(LayoutBoxId::from_taffy(handle.to_taffy()), handle);
        assert_ne!(handle.to_taffy(), viewport);
        assert_ne!(handle.to_taffy(), fixed);
    }
    assert!(old.into_iter().all(|key| world.box_by_id(key).is_none()));
}

#[test]
fn retiring_a_construction_wrapper_cleans_links_without_renaming_children() {
    let mut world = LayoutWorld::new(block(0), false);
    let root = world.root();
    world.map_source(0, root);
    let wrapper = world.allocate(block(1));
    let child = world.allocate(block(2));
    world.map_source(1, wrapper);
    world.map_source(2, child);
    world.replace_children(root, vec![child]).unwrap();
    world.boxes[child].structural_parent = Some(wrapper);
    world.compact_reachable();
    assert!(world.box_by_id(wrapper).is_none());
    assert_eq!(world.source_box(1), None);
    assert_eq!(world.source_box(2), Some(child));
    assert_eq!(world.boxes[child].structural_parent, None);
    assert_eq!(world.boxes[child].parent, Some(root));
    assert_eq!(
        world.validate_invariants(),
        Err(crate::LayoutError::InvalidBoxHandle { id: child })
    );
    // Construction must supply the replacement source ancestry before the
    // completed graph can be validated or published.
    world.record_structural_children(root, &[child]).unwrap();
    world.validate_invariants().unwrap();
}

#[test]
fn physical_geometry_snapshot_detects_same_length_slot_replacement() {
    let mut boxes = LayoutBoxes::default();
    let first = boxes.insert(block(1));
    let removed = boxes.insert(block(2));
    let last = boxes.insert(block(3));
    boxes.retain_in_order(vec![last, first]);
    let current = boxes.insert(block(4));
    boxes.retain_in_order(vec![current, first, last]);
    let read_order = boxes.values().map(|b| b.source()).collect::<Vec<_>>();
    let write_order = boxes.values_mut().map(|b| b.source()).collect::<Vec<_>>();
    assert_eq!(
        read_order, write_order,
        "a physical snapshot must restore in physical order"
    );
    assert!(boxes.get(removed).is_none());
    let revision = boxes.storage_revision();
    boxes.pop();
    boxes.insert(block(5));
    assert_eq!(boxes.len(), 3);
    assert_ne!(
        boxes.storage_revision(),
        revision,
        "same length does not imply the same storage identities"
    );
}
