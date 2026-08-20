// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Marker formatting follows the common counter styles handled by Blitz's
// `layout/list.rs`, while HTML start/reversed/value inputs are supplied by
// Moli's DOM-neutral source adapter.

use std::{fmt::Debug, hash::Hash, sync::Arc};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutCapabilityDiagnostic, LayoutElementCategory, LayoutError,
    LayoutListMarkerPosition, LayoutListMarkerType, LayoutListRole, LayoutPseudo, LayoutWorld,
    ResolvedLayoutStyle,
};

pub(crate) fn prepare_list_markers<N>(world: &mut LayoutWorld<N>) -> Result<(), LayoutError>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut handled = vec![false; world.boxes.len()];
    let containers = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| {
            matches!(
                world.boxes[id.index()]
                    .element_semantics
                    .as_ref()
                    .map(|semantics| semantics.category),
                Some(LayoutElementCategory::List(LayoutListRole::Container))
            )
        })
        .collect::<Vec<_>>();

    for container in containers {
        let mut items = Vec::new();
        collect_container_items(world, container, container, &mut items);
        let data = world.boxes[container.index()]
            .element_semantics
            .as_ref()
            .and_then(|semantics| semantics.metadata.list)
            .unwrap_or_default();
        let mut counter = data.start.unwrap_or_else(|| {
            if data.reversed {
                i32::try_from(items.len()).unwrap_or(i32::MAX)
            } else {
                1
            }
        });
        let step = if data.reversed { -1 } else { 1 };
        for item in items {
            let value = world.boxes[item.index()]
                .element_semantics
                .as_ref()
                .and_then(|semantics| semantics.metadata.list)
                .and_then(|item| item.value)
                .unwrap_or(counter);
            prepare_item_marker(world, item, value)?;
            handled[item.index()] = true;
            counter = value.saturating_add(step);
        }
    }

    // CSS can create a list item outside an HTML list. It still gets a
    // deterministic marker; full CSS counter scopes remain an explicit local
    // fallback rather than requiring a live DOM callback.
    let standalone = (0..handled.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| !handled[id.index()] && world.boxes[id.index()].style.display().is_list_item())
        .collect::<Vec<_>>();
    for item in standalone {
        prepare_item_marker(world, item, 1)?;
    }
    Ok(())
}

fn collect_container_items<N>(
    world: &LayoutWorld<N>,
    container: LayoutBoxId,
    current: LayoutBoxId,
    output: &mut Vec<LayoutBoxId>,
) where
    N: Copy + Debug + Eq + Hash,
{
    for child in world.boxes[current.index()].children.iter().copied() {
        let is_nested_container = child != container
            && matches!(
                world.boxes[child.index()]
                    .element_semantics
                    .as_ref()
                    .map(|semantics| semantics.category),
                Some(LayoutElementCategory::List(LayoutListRole::Container))
            );
        if is_nested_container {
            continue;
        }
        if world.boxes[child.index()].style.display().is_list_item() {
            output.push(child);
            continue;
        }
        collect_container_items(world, container, child, output);
    }
}

fn prepare_item_marker<N>(
    world: &mut LayoutWorld<N>,
    item: LayoutBoxId,
    counter: i32,
) -> Result<(), LayoutError>
where
    N: Copy + Debug + Eq + Hash,
{
    let marker_type = world.boxes[item.index()].style.list_marker_type().clone();
    let Some(marker) = find_marker(world, item) else {
        return Ok(());
    };
    world.boxes[marker.index()].outside_list_marker =
        world.boxes[item.index()].style.list_marker_position() == LayoutListMarkerPosition::Outside;

    if marker_type == LayoutListMarkerType::Fallback {
        push_diagnostic(
            &mut world.boxes[marker.index()].capability_diagnostics,
            LayoutCapabilityDiagnostic::ListMarkerStyleFallback,
        );
    }
    if !world.boxes[marker.index()].children.is_empty() {
        return Ok(());
    }
    let Some(text) = marker_text(&marker_type, counter) else {
        return Ok(());
    };

    let owner = world.boxes[marker.index()].owner;
    let owner_label = world.boxes[marker.index()].owner_label.clone();
    let marker_label = world.boxes[marker.index()].source_label.clone();
    let text_style = ResolvedLayoutStyle::text_leaf_from(&world.boxes[marker.index()].style);
    let text_box = LayoutWorld::new_box(
        None,
        owner,
        Some(LayoutPseudo::Marker),
        format!("{marker_label}::default-text"),
        owner_label,
        None,
        None,
        LayoutBoxKind::Text,
        text_style,
        Some(Arc::from(text)),
    );
    let text_id = world.allocate(text_box);
    world.append_synthesized_child(marker, text_id)?;
    world.boxes[marker.index()].inline_formatting_context = true;
    Ok(())
}

fn find_marker<N>(world: &LayoutWorld<N>, item: LayoutBoxId) -> Option<LayoutBoxId>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut stack = world.boxes[item.index()]
        .children
        .iter()
        .rev()
        .copied()
        .collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        if world.boxes[id.index()].kind == LayoutBoxKind::PseudoMarker {
            return Some(id);
        }
        if world.boxes[id.index()].style.display().is_list_item() {
            continue;
        }
        stack.extend(world.boxes[id.index()].children.iter().rev().copied());
    }
    None
}

fn marker_text(marker_type: &LayoutListMarkerType, counter: i32) -> Option<String> {
    let output = match marker_type {
        LayoutListMarkerType::None => return None,
        LayoutListMarkerType::Decimal => format!("{counter}. "),
        LayoutListMarkerType::LowerAlpha => format!("{}. ", alpha_marker(counter, false)),
        LayoutListMarkerType::UpperAlpha => format!("{}. ", alpha_marker(counter, true)),
        LayoutListMarkerType::Disc => "• ".to_owned(),
        LayoutListMarkerType::Circle => "◦ ".to_owned(),
        LayoutListMarkerType::Square => "▪ ".to_owned(),
        LayoutListMarkerType::DisclosureOpen => "▾ ".to_owned(),
        LayoutListMarkerType::DisclosureClosed => "▸ ".to_owned(),
        LayoutListMarkerType::String(value) => value.to_string(),
        LayoutListMarkerType::Symbols(symbols) if symbols.is_empty() => "• ".to_owned(),
        LayoutListMarkerType::Symbols(symbols) => {
            let index = counter.saturating_sub(1).unsigned_abs() as usize % symbols.len();
            format!("{} ", symbols[index])
        }
        LayoutListMarkerType::Fallback => "□ ".to_owned(),
    };
    Some(output)
}

fn alpha_marker(counter: i32, uppercase: bool) -> String {
    if counter <= 0 {
        return counter.to_string();
    }
    let mut value = counter as u32;
    let mut output = String::new();
    while value > 0 {
        value -= 1;
        let base = if uppercase { b'A' } else { b'a' };
        output.insert(0, char::from(base + (value % 26) as u8));
        value /= 26;
    }
    output
}

fn push_diagnostic(
    diagnostics: &mut Vec<LayoutCapabilityDiagnostic>,
    diagnostic: LayoutCapabilityDiagnostic,
) {
    if !diagnostics.contains(&diagnostic) {
        diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::{alpha_marker, marker_text};
    use crate::LayoutListMarkerType;

    #[test]
    fn common_marker_families_include_html_counter_values() {
        assert_eq!(
            marker_text(&LayoutListMarkerType::Decimal, -2).as_deref(),
            Some("-2. ")
        );
        assert_eq!(alpha_marker(1, false), "a");
        assert_eq!(alpha_marker(26, false), "z");
        assert_eq!(alpha_marker(27, false), "aa");
        assert_eq!(alpha_marker(28, true), "AB");
    }
}
