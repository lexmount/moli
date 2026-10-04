use super::*;

pub(super) fn hover_boundary_paths(
    runtime: &PageScriptHost,
    previous: Option<DomHandle>,
    current: Option<DomHandle>,
) -> (Vec<DomHandle>, Vec<DomHandle>) {
    let chain = |target| {
        let dom = runtime.dom_host();
        std::iter::successors(target, |node| dom.flat_tree_parent(*node))
            .filter(|node| dom.node(*node).is_some_and(|node| node.is_element()))
            .collect::<Vec<_>>()
    };
    let mut previous_chain = chain(previous);
    let mut current_chain = chain(current);
    let shared = previous_chain
        .iter()
        .rev()
        .zip(current_chain.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    previous_chain.truncate(previous_chain.len() - shared);
    current_chain.truncate(current_chain.len() - shared);
    (previous_chain, current_chain)
}

/// Mouse enter/leave describe branches of the flat tree, not just two targets.
/// The same transition is used by physical mouse input and touch compatibility.
pub(super) fn dispatch_mouse_boundary_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::PageScriptHost,
    previous: Option<DomHandle>,
    current: Option<DomHandle>,
    previous_coordinates: InputEventCoordinates,
    coordinates: InputEventCoordinates,
    button: i32,
    buttons: i32,
    modifiers: u8,
) {
    if previous == current {
        return;
    }
    let (leaving, entering) =
        hover_boundary_paths(unsafe { &*runtime_ptr }, previous, current);
    let mut dispatch = |name, target, related, coordinates| {
        let related = related_target_value(scope, related);
        if let Some(event) = construct_mouse_event_with_related_target_for_target(
            scope,
            runtime_ptr,
            target,
            name,
            coordinates,
            button,
            buttons,
            related,
            modifiers,
        ) {
            let _ = dispatch_public_event(scope, runtime_ptr, target, event);
        }
    };
    if let Some(previous) = previous {
        dispatch("mouseout", previous, current, previous_coordinates);
    }
    for target in &leaving {
        dispatch("mouseleave", *target, current, previous_coordinates);
    }
    if let Some(current) = current {
        dispatch("mouseover", current, previous, coordinates);
    }
    for target in entering.iter().rev() {
        dispatch("mouseenter", *target, previous, coordinates);
    }
}
