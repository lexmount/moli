//! Undo records keep changed native nodes, not serialized markup. V8 owns the
//! per-Document stacks and node references, so retiring a realm retires history.
use super::super::contenteditable_editing_host;
use crate::native_bridge::element::forms::{
    is_text_control_user_editable, text_control_set_selection_range_with_direction_internal,
};
use crate::native_bridge::element::{
    TextEditInputType, construct_input_event, dispatch_beforeinput, dispatch_public_event,
    is_text_control, text_control_value,
};
use crate::native_bridge::node::{
    insert_before_in_reaction_scope, remove_child_in_reaction_scope,
    set_text_content_in_reaction_scope,
};
use crate::util::{get_private_value, node_wrapper_from_handle, set_private_value};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const UNDO: &str = "__moliEditingUndo";
const REDO: &str = "__moliEditingRedo";
const RECORD: &str = "__moliEditingRecord";
const NODES: &str = "__moliEditingNodes";
const LIMIT: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Selection {
    Control(u32, u32, String),
    Dom(usize, u32, usize, u32),
    None,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum State {
    Text(String),
    Children(Vec<usize>),
    Value(String),
}
#[derive(Clone, Serialize, Deserialize)]
struct Change {
    node: usize,
    before: State,
    after: State,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    target: usize,
    typing: bool,
    before: Selection,
    after: Selection,
    changes: Vec<Change>,
}
pub(crate) struct EditSnapshot {
    target: usize,
    selection: Selection,
    nodes: BTreeMap<usize, State>,
}

fn children(host: &PageScriptHost, handle: DomHandle) -> Vec<usize> {
    let mut result = Vec::new();
    let mut child = host
        .dom_host()
        .node(handle)
        .and_then(|node| node.first_child());
    while let Some(node) = child {
        result.push(node.index());
        child = host
            .dom_host()
            .node(node)
            .and_then(|node| node.next_sibling());
    }
    result
}
fn state(host: &PageScriptHost, node: usize, target: usize) -> Option<State> {
    let handle = DomHandle::new(node);
    if node == target && is_text_control(host, handle) {
        return Some(State::Value(text_control_value(host, handle)));
    }
    let node = host.dom_host().node(handle)?;
    Some(if let Some(text) = node.as_text() {
        State::Text(text.data().to_owned())
    } else {
        State::Children(children(host, handle))
    })
}
fn selection(scope: &mut v8::PinScope<'_, '_>, host: &PageScriptHost, target: usize) -> Selection {
    let handle = DomHandle::new(target);
    if is_text_control(host, handle) {
        return host
            .dom_host()
            .node(handle)
            .and_then(|node| node.as_element())
            .map_or(Selection::None, |element| {
                Selection::Control(
                    element.selection_start(),
                    element.selection_end(),
                    element.selection_direction().to_owned(),
                )
            });
    }
    crate::context_bootstrap::native_editing_selection(scope)
        .map_or(Selection::None, |(a, ao, f, fo)| {
            Selection::Dom(a.index(), ao, f.index(), fo)
        })
}
fn restore_selection(
    scope: &mut v8::PinScope<'_, '_>,
    ptr: *mut PageScriptHost,
    target: usize,
    selected: &Selection,
) {
    match selected {
        Selection::Control(start, end, direction) => {
            text_control_set_selection_range_with_direction_internal(
                scope,
                ptr,
                DomHandle::new(target),
                *start,
                *end,
                direction,
            );
        }
        Selection::Dom(anchor, ao, focus, fo) => {
            crate::context_bootstrap::restore_native_editing_selection(
                scope,
                DomHandle::new(*anchor),
                *ao,
                DomHandle::new(*focus),
                *fo,
            );
        }
        Selection::None => {}
    }
}
pub(crate) fn capture_edit(
    scope: &mut v8::PinScope<'_, '_>,
    ptr: *mut PageScriptHost,
    target: DomHandle,
) -> Option<EditSnapshot> {
    let host = unsafe { &*ptr };
    let selected = selection(scope, host, target.index());
    // The common ancestor of the selected boundaries is the smallest region
    // an editing replacement can change. A caret in text visits one node.
    let mut root = target;
    if let Selection::Dom(anchor, _, focus, _) = &selected {
        let dom = host.dom_host().dom();
        if dom.node(target)?.contains(dom, DomHandle::new(*anchor))
            && dom.node(target)?.contains(dom, DomHandle::new(*focus))
        {
            let mut ancestors = BTreeSet::new();
            let mut current = Some(DomHandle::new(*anchor));
            while let Some(node) = current {
                ancestors.insert(node.index());
                if node == target {
                    break;
                }
                current = dom.node(node).and_then(|node| node.parent_node());
            }
            let mut current = Some(DomHandle::new(*focus));
            while let Some(node) = current {
                if ancestors.contains(&node.index()) {
                    root = node;
                    break;
                }
                current = dom.node(node).and_then(|node| node.parent_node());
            }
        }
    }
    let mut nodes = BTreeMap::new();
    let mut pending = vec![root.index()];
    while let Some(node) = pending.pop() {
        let value = state(host, node, target.index())?;
        if let State::Children(list) = &value {
            pending.extend(list.iter().copied());
        }
        nodes.insert(node, value);
    }
    Some(EditSnapshot {
        target: target.index(),
        selection: selected,
        nodes,
    })
}
fn stack<'s>(scope: &mut v8::PinScope<'s, '_>, key: &'static str) -> Vec<v8::Local<'s, v8::Value>> {
    let Some(owner) = crate::context_bootstrap::native_editing_history_owner(scope) else {
        return Vec::new();
    };
    let Some(value) = get_private_value(scope, owner, key)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    (0..value.length())
        .filter_map(|i| value.get_index(scope, i))
        .collect()
}
fn store_stack<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &'static str,
    values: &[v8::Local<'s, v8::Value>],
) {
    let array = v8::Array::new_with_elements(scope, values);
    if let Some(owner) = crate::context_bootstrap::native_editing_history_owner(scope) {
        set_private_value(scope, owner, key, array.into());
    }
}
fn read_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Entry> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let value = get_private_value(scope, object, RECORD)?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    serde_json::from_str(&value).ok()
}
fn write_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: &Entry,
) -> Option<v8::Local<'s, v8::Value>> {
    let object = v8::Object::new(scope);
    let text = serde_json::to_string(entry).ok()?;
    let value = v8::String::new(scope, &text)?;
    set_private_value(scope, object, RECORD, value.into());
    let mut nodes = BTreeSet::from([entry.target]);
    for selected in [&entry.before, &entry.after] {
        if let Selection::Dom(a, _, f, _) = selected {
            nodes.insert(*a);
            nodes.insert(*f);
        }
    }
    for change in &entry.changes {
        nodes.insert(change.node);
        for value in [&change.before, &change.after] {
            if let State::Children(children) = value {
                nodes.extend(children.iter().copied());
            }
        }
    }
    let refs = nodes
        .into_iter()
        .filter_map(|node| node_wrapper_from_handle(scope, DomHandle::new(node)).map(Into::into))
        .collect::<Vec<_>>();
    let refs = v8::Array::new_with_elements(scope, &refs);
    set_private_value(scope, object, NODES, refs.into());
    Some(object.into())
}
pub(crate) fn commit_edit(
    scope: &mut v8::PinScope<'_, '_>,
    ptr: *mut PageScriptHost,
    before: Option<EditSnapshot>,
    input_type: TextEditInputType,
) {
    let Some(before) = before else {
        return;
    };
    let host = unsafe { &*ptr };
    let mut changes = Vec::new();
    for (node, old) in &before.nodes {
        if let Some(new) = state(host, *node, before.target)
            && *old != new
        {
            changes.push(Change {
                node: *node,
                before: old.clone(),
                after: new,
            });
        }
    }
    if changes.is_empty() {
        return;
    }
    let after = selection(scope, host, before.target);
    let typing = input_type == TextEditInputType::InsertText;
    let mut entry = Entry {
        target: before.target,
        typing,
        before: before.selection,
        after,
        changes,
    };
    let mut undo = stack(scope, UNDO);
    if typing
        && let Some(last) = undo.last().and_then(|value| read_entry(scope, *value))
        && last.typing
        && last.target == entry.target
        && last.after == entry.before
        && last.changes.iter().all(|change| {
            before
                .nodes
                .get(&change.node)
                .cloned()
                .or_else(|| state(host, change.node, before.target))
                .as_ref()
                == Some(&change.after)
        })
    {
        entry.before = last.before;
        let mut merged = last.changes;
        for change in entry.changes {
            if let Some(existing) = merged.iter_mut().find(|old| old.node == change.node) {
                existing.after = change.after;
            } else {
                merged.push(change);
            }
        }
        entry.changes = merged;
        undo.pop();
    }
    if let Some(value) = write_entry(scope, &entry) {
        undo.push(value);
        if undo.len() > LIMIT {
            undo.remove(0);
        }
        store_stack(scope, UNDO, &undo);
        store_stack(scope, REDO, &[]);
    }
}
// A removed subtree may still be retained by history. It must stay detached;
// moving it elsewhere must not let undo pull it out of an author's new tree.
fn belongs_to_edit(
    host: &PageScriptHost,
    target: DomHandle,
    node: usize,
    detached_roots: &BTreeSet<usize>,
    require_editable: bool,
) -> bool {
    let mut current = DomHandle::new(node);
    loop {
        let Some(node) = host.dom_host().node(current) else {
            return false;
        };
        if require_editable
            && let Some(element) = node.as_element()
            && (element.has_attribute("inert")
                || element
                    .attribute("contenteditable")
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("false")))
        {
            return false;
        }
        if current == target {
            return true;
        }
        let Some(parent) = node.parent_node() else {
            return detached_roots.contains(&current.index());
        };
        current = parent;
    }
}

fn matches_entry(host: &PageScriptHost, entry: &Entry, redo: bool) -> bool {
    let target = DomHandle::new(entry.target);
    let editable = if is_text_control(host, target) {
        is_text_control_user_editable(host, target)
    } else {
        contenteditable_editing_host(host, target) == Some(target)
    };
    if !editable || !host.dom_host().is_connected(target) {
        return false;
    }
    let mut detached_roots = BTreeSet::new();
    for change in &entry.changes {
        let (State::Children(before), State::Children(after)) = (&change.before, &change.after)
        else {
            continue;
        };
        let (removed, retained) = if redo {
            (after, before)
        } else {
            (before, after)
        };
        detached_roots.extend(
            removed
                .iter()
                .filter(|child| !retained.contains(child))
                .copied(),
        );
    }
    entry.changes.iter().all(|change| {
        belongs_to_edit(host, target, change.node, &detached_roots, true)
            && state(host, change.node, entry.target).as_ref()
                == Some(if redo { &change.before } else { &change.after })
            && [&change.before, &change.after].iter().all(|state| {
                let State::Children(children) = state else {
                    return true;
                };
                children
                    .iter()
                    .all(|child| belongs_to_edit(host, target, *child, &detached_roots, false))
            })
    })
}

fn restore_children(
    scope: &mut v8::PinScope<'_, '_>,
    ptr: *mut PageScriptHost,
    parent: DomHandle,
    wanted: &[usize],
) -> bool {
    let retained = wanted.iter().copied().collect::<BTreeSet<_>>();
    for child in children(unsafe { &*ptr }, parent) {
        if !retained.contains(&child)
            && !remove_child_in_reaction_scope(scope, ptr, parent, DomHandle::new(child))
        {
            return false;
        }
    }
    let mut cursor = unsafe { &*ptr }
        .dom_host()
        .node(parent)
        .and_then(|node| node.first_child());
    for child in wanted {
        let child = DomHandle::new(*child);
        if cursor == Some(child) {
            cursor = unsafe { &*ptr }
                .dom_host()
                .node(child)
                .and_then(|node| node.next_sibling());
        } else if !insert_before_in_reaction_scope(scope, ptr, parent, child, cursor) {
            return false;
        }
    }
    true
}

pub(crate) fn undo_redo(
    scope: &mut v8::PinScope<'_, '_>,
    ptr: *mut PageScriptHost,
    target: DomHandle,
    redo: bool,
) -> bool {
    let Some(context) =
        crate::native_bridge::node_owner_document_relevant_context(scope, ptr, target)
    else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let (source, destination) = if redo { (REDO, UNDO) } else { (UNDO, REDO) };
    let document = unsafe { &*ptr }.dom_host().owner_document_handle(target);
    let mut from = stack(scope, source);
    let (value, entry) = loop {
        let Some(value) = from.last().copied() else {
            return false;
        };
        if let Some(entry) = read_entry(scope, value)
            && unsafe { &*ptr }
                .dom_host()
                .owner_document_handle(DomHandle::new(entry.target))
                == document
            && matches_entry(unsafe { &*ptr }, &entry, redo)
        {
            break (value, entry);
        }
        // History is per Document, not per focused control. An unavailable
        // target must not permanently block earlier, still-valid edits.
        from.pop();
        store_stack(scope, source, &from);
    };
    let target = DomHandle::new(entry.target);
    let input_type = if redo {
        TextEditInputType::HistoryRedo
    } else {
        TextEditInputType::HistoryUndo
    };
    if !dispatch_beforeinput(scope, ptr, target, input_type, None)
        || unsafe { &*ptr }.dom_host().owner_document_handle(target) != document
        || !matches_entry(unsafe { &*ptr }, &entry, redo)
    {
        return false;
    }
    super::super::update_focus_preserving_selection(scope, ptr, Some(target));
    if unsafe { &*ptr }.dom_host().owner_document_handle(target) != document
        || !matches_entry(unsafe { &*ptr }, &entry, redo)
    {
        return false;
    }
    // Restore character data and child relationships on the existing nodes.
    // Their wrappers, listeners and author properties retain identity.
    let restored =
        crate::custom_elements::with_custom_element_reaction_scope(scope, ptr, |scope| {
            for change in if redo {
                entry.changes.iter().collect::<Vec<_>>()
            } else {
                entry.changes.iter().rev().collect()
            } {
                let handle = DomHandle::new(change.node);
                match if redo { &change.after } else { &change.before } {
                    State::Value(value) => {
                        let old = text_control_value(unsafe { &*ptr }, handle);
                        let host = unsafe { &mut *ptr };
                        host.set_input_value_from_user_edit(handle, value);
                        host.mark_text_control_change_pending(handle, &old);
                    }
                    State::Text(text) => {
                        if !set_text_content_in_reaction_scope(scope, ptr, handle, text) {
                            return false;
                        }
                    }
                    State::Children(wanted) => {
                        if !restore_children(scope, ptr, handle, wanted) {
                            return false;
                        }
                    }
                }
            }
            true
        });
    if !restored {
        return false;
    }
    restore_selection(
        scope,
        ptr,
        entry.target,
        if redo { &entry.after } else { &entry.before },
    );
    from.pop();
    store_stack(scope, source, &from);
    let mut to = stack(scope, destination);
    to.push(value);
    store_stack(scope, destination, &to);
    if let Some(event) = construct_input_event(scope, "input", input_type, None) {
        dispatch_public_event(scope, ptr, target, event);
    }
    true
}
