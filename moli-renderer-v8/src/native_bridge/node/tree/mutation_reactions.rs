use super::*;
use crate::custom_elements;

pub(in crate::native_bridge) fn append_child_in_reaction_scope(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        append_child_to_current_reaction_queue(scope, runtime_ptr, parent, child)
    })
}

pub(in crate::native_bridge) fn append_child_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    unsafe { &mut *runtime_ptr }.append_child_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        parent,
        child,
    )
}

pub(in crate::native_bridge) fn insert_before_in_reaction_scope(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
) -> bool {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        insert_before_to_current_reaction_queue(scope, runtime_ptr, parent, child, reference_child)
    })
}

pub(in crate::native_bridge) fn insert_before_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
) -> bool {
    unsafe { &mut *runtime_ptr }.insert_before_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        parent,
        child,
        reference_child,
    )
}

pub(in crate::native_bridge) fn remove_child_in_reaction_scope(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        remove_child_to_current_reaction_queue(scope, runtime_ptr, parent, child)
    })
}

pub(in crate::native_bridge) fn remove_child_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    unsafe { &mut *runtime_ptr }.remove_child_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        parent,
        child,
    )
}
