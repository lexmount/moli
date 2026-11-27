use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

use moli_layout::{LayoutQuery, LayoutQueryAnswer, LayoutRect};
use moli_webapi::events::subclasses::{
    construct_native_clipboard_event, construct_native_composition_event,
};

use super::{
    DragDataStoreMode, create_drag_data_transfer_view, create_native_drag_data_store,
    finish_drag_data_transfer_view, native_data_transfer_get_data, native_data_transfer_has_type,
    populate_native_drag_data_store, range::new_range_for_document,
    range_algorithms::range_selection_string_contents, selection::*, selection_value_for_window,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Element, Node},
    native_bridge::{
        PageScriptHost, callback_value_dom_handle,
        element::{
            TextEditInputType, contenteditable_editing_host, dispatch_beforeinput,
            dispatch_public_event, is_text_control, is_text_control_user_editable,
            replace_contenteditable_selection_after_beforeinput,
            replace_text_control_selection_after_beforeinput,
            text_control_set_selection_range_internal, text_control_value,
        },
        node_owner_document_relevant_context,
    },
    runtime::{
        RendererDragData, RendererDragDataItem, RendererViewportEdit, RendererViewportEditAction,
        RendererViewportEditor,
    },
    util::node_wrapper_from_handle,
};

#[derive(Default)]
pub(crate) struct ViewportEditing {
    composition: Option<Composition>,
    last_edit: Option<(u64, u32, u64)>,
}

struct Composition {
    target: DomHandle,
    original: String,
    marked: Range<usize>,
    revision: u64,
}

struct Editor {
    target: DomHandle,
    text: String,
    selection: Range<usize>,
    reversed: bool,
    password: bool,
    // Contenteditable text uses a single UTF-16 coordinate space across its nodes.
    map: Option<EditableTextMap>,
}

impl Editor {
    fn revision(&self, host: &PageScriptHost) -> u64 {
        let mut hash = DefaultHasher::new();
        self.target.hash(&mut hash);
        self.selection.hash(&mut hash);
        self.reversed.hash(&mut hash);
        if self.password {
            host.dom_host().dom_version().hash(&mut hash);
        } else {
            self.text.hash(&mut hash);
        }
        hash.finish()
    }

    fn offset(&self, offset: usize) -> (DomHandle, usize) {
        self.map.as_ref().map_or((self.target, offset), |map| {
            map.boundary(self.target, offset)
        })
    }
}

struct EditableTextMap {
    text: String,
    segments: Vec<(DomHandle, Range<usize>)>,
    nodes: HashMap<DomHandle, Range<usize>>,
}

impl EditableTextMap {
    fn read(dom: &DomHost, root: DomHandle) -> Self {
        let mut map = Self {
            text: String::new(),
            segments: Vec::new(),
            nodes: HashMap::new(),
        };
        let mut position = 0;
        let mut stack = vec![(root, false)];
        while let Some((handle, exiting)) = stack.pop() {
            if exiting {
                if let Some(range) = map.nodes.get_mut(&handle) {
                    range.end = position;
                }
            } else if let Some(text) = dom.node(handle).and_then(|node| node.as_text()) {
                let start = position;
                position += text.data().encode_utf16().count();
                map.segments.push((handle, start..position));
                map.nodes.insert(handle, start..position);
                map.text.push_str(text.data());
            } else {
                map.nodes.insert(handle, position..position);
                stack.push((handle, true));
                if let Some(children) = dom.child_nodes(handle) {
                    stack.extend(children.into_iter().rev().map(|child| (child, false)));
                }
            }
        }
        map
    }

    fn absolute(&self, dom: &DomHost, node: DomHandle, offset: usize) -> Option<usize> {
        let range = self.nodes.get(&node)?;
        if dom.node(node)?.as_text().is_some() {
            return Some(range.start + offset.min(range.len()));
        }
        Some(
            dom.child_nodes(node)
                .and_then(|children| {
                    self.nodes
                        .get(children.get(offset)?)
                        .map(|range| range.start)
                })
                .unwrap_or(range.end),
        )
    }

    fn boundary(&self, root: DomHandle, offset: usize) -> (DomHandle, usize) {
        self.segments
            .iter()
            .find(|(_, range)| offset <= range.end)
            .map_or((root, 0), |(handle, range)| {
                (*handle, offset.saturating_sub(range.start))
            })
    }
}

fn active_target(host: &PageScriptHost) -> Option<DomHandle> {
    let active = host.active_element_handle()?;
    if is_text_control(host, active) {
        return is_text_control_user_editable(host, active).then_some(active);
    }
    contenteditable_editing_host(host, active)
}

fn contenteditable_clipboard_target(
    host: &PageScriptHost,
    focus_target: DomHandle,
    document: DomHandle,
) -> Option<DomHandle> {
    if is_text_control(host, focus_target)
        || contenteditable_editing_host(host, focus_target).is_none()
    {
        return None;
    }
    let selection = host.document_selection_snapshot(document)?;
    let dom = host.dom_host();
    let (start, _) = selection.ordered_in_dom(dom)?;
    let container = start.container;
    let node = dom.node(container)?;
    // The selection can start in a nested editable while its outer editor has focus.
    if !node.is_connected() || dom.owner_document_handle(container) != Some(document) {
        return None;
    }
    if node.is_element() {
        Some(container)
    } else {
        node.parent_node()
            .filter(|parent| dom.node(*parent).is_some_and(Node::is_element))
    }
}

fn is_password_control(element: &Element) -> bool {
    element.is_html_input() && element.input_type() == moli_dom::forms::InputType::Password
}

pub(crate) fn selected_viewport_text(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
) -> Option<String> {
    if let Some(target) = unsafe { &*host }.active_element_handle() {
        let realm = node_owner_document_relevant_context(scope, host, target)?;
        let scope = &mut v8::ContextScope::new(scope, realm);
        let host = unsafe { &*host };
        if is_text_control(host, target) {
            let element = host.dom_host().node(target)?.as_element()?;
            if is_password_control(element) {
                return None;
            }
            let range = element.selection_start() as usize..element.selection_end() as usize;
            return (!range.is_empty()).then(|| text_at(&text_control_value(host, target), &range));
        }
        return document_selection_text(scope);
    }
    document_selection_text(scope)
}

fn document_selection_text(scope: &mut v8::PinScope<'_, '_>) -> Option<String> {
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let range = selection_range(scope, selection)?;
    range_selection_string_contents(scope, range).filter(|text| !text.is_empty())
}

struct ClipboardTarget {
    focused: Option<DomHandle>,
    focus_target: DomHandle,
    document: DomHandle,
    event_target: DomHandle,
}

impl ClipboardTarget {
    fn resolve(host: &PageScriptHost) -> Option<Self> {
        let focused = host.active_element_handle();
        let focus_target = focused.or_else(|| host.document_focus_fallback_handle())?;
        let document = host.dom_host().owner_document_handle(focus_target)?;
        let event_target =
            contenteditable_clipboard_target(host, focus_target, document).unwrap_or(focus_target);
        Some(Self {
            focused,
            focus_target,
            document,
            event_target,
        })
    }

    fn still_focused(&self, host: &PageScriptHost) -> bool {
        host.active_element_handle() == self.focused
            && host.dom_host().owner_document_handle(self.focus_target) == Some(self.document)
    }
}

enum ClipboardEvent<'a> {
    Copy,
    Cut,
    Paste(Option<&'a str>),
}

enum ClipboardDispatch {
    Default,
    Canceled(Option<String>),
}

fn dispatch_clipboard_event(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    target: DomHandle,
    event: ClipboardEvent<'_>,
) -> Option<ClipboardDispatch> {
    let store = create_native_drag_data_store(scope)?;
    let (event_type, mode) = match event {
        ClipboardEvent::Copy => ("copy", DragDataStoreMode::ReadWrite),
        ClipboardEvent::Cut => ("cut", DragDataStoreMode::ReadWrite),
        ClipboardEvent::Paste(text) => {
            if let Some(text) = text {
                populate_native_drag_data_store(
                    scope,
                    &store,
                    &RendererDragData {
                        items: vec![RendererDragDataItem {
                            mime_type: "text/plain".into(),
                            data: text.to_owned(),
                            title: None,
                            base_url: None,
                        }],
                        files: Vec::new(),
                        directories: Vec::new(),
                        drag_operations_mask: 0,
                    },
                )?;
            }
            ("paste", DragDataStoreMode::ReadOnly)
        }
    };
    let transfer = create_drag_data_transfer_view(scope, &store, mode, "none")?;
    let event = construct_native_clipboard_event(scope, event_type, transfer)?;
    let allowed = dispatch_public_event(scope, host, target, event).allows_default();
    let replacement = (!allowed
        && mode == DragDataStoreMode::ReadWrite
        && native_data_transfer_has_type(scope, transfer, "text/plain"))
    .then(|| native_data_transfer_get_data(scope, transfer, "text/plain"));
    finish_drag_data_transfer_view(scope, transfer);
    Some(if allowed {
        ClipboardDispatch::Default
    } else {
        ClipboardDispatch::Canceled(replacement)
    })
}

pub(crate) fn copy_or_cut_viewport_selection(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    cut: bool,
) -> Option<String> {
    let runtime = unsafe { &*host };
    let clipboard = ClipboardTarget::resolve(runtime)?;
    if runtime
        .dom_host()
        .node(clipboard.focus_target)
        .and_then(Node::as_element)
        .is_some_and(is_password_control)
    {
        return None;
    }
    let realm = node_owner_document_relevant_context(scope, host, clipboard.event_target)?;
    let scope = &mut v8::ContextScope::new(scope, realm);
    let event = if cut {
        ClipboardEvent::Cut
    } else {
        ClipboardEvent::Copy
    };
    if let ClipboardDispatch::Canceled(text) =
        dispatch_clipboard_event(scope, host, clipboard.event_target, event)?
    {
        return text;
    }

    let runtime = unsafe { &*host };
    if !clipboard.still_focused(runtime) {
        return None;
    }
    if !cut {
        return selected_viewport_text(scope, host);
    }
    // Use the current selection, including changes made by the cut listener.
    let target = active_target(runtime)?;
    let editor = read_editor(scope, runtime, target)?;
    if editor.password || editor.selection.is_empty() {
        return None;
    }
    let text = text_at(&editor.text, &editor.selection);
    // The default cut copies the selection even if beforeinput vetoes the
    // deletion. Selection, focus, and value are revalidated before editing.
    replace_range(
        scope,
        host,
        &editor,
        editor.revision(runtime),
        &editor.selection,
        "",
        TextEditInputType::DeleteByCut,
    );
    Some(text)
}

fn read_editor(
    scope: &mut v8::PinScope<'_, '_>,
    host: &PageScriptHost,
    target: DomHandle,
) -> Option<Editor> {
    let dom = host.dom_host();
    let element = dom.node(target)?.as_element()?;
    if is_text_control_user_editable(host, target) {
        return Some(Editor {
            target,
            text: text_control_value(host, target),
            selection: element.selection_start() as usize..element.selection_end() as usize,
            reversed: element.selection_direction() == "backward",
            password: is_password_control(element),
            map: None,
        });
    }
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let map = EditableTextMap::read(dom, target);
    let anchor = selection_anchor_node(scope, selection)
        .and_then(|node| callback_value_dom_handle(scope, node.into()))
        .and_then(|node| {
            map.absolute(
                dom,
                node,
                selection_anchor_offset(scope, selection) as usize,
            )
        })
        .unwrap_or_default();
    let focus = selection_focus_node(scope, selection)
        .and_then(|node| callback_value_dom_handle(scope, node.into()))
        .and_then(|node| map.absolute(dom, node, selection_focus_offset(scope, selection) as usize))
        .unwrap_or(anchor);
    Some(Editor {
        target,
        text: map.text.clone(),
        selection: anchor.min(focus)..anchor.max(focus),
        reversed: anchor > focus,
        password: false,
        map: Some(map),
    })
}

impl ViewportEditing {
    pub(crate) fn paste_from_clipboard(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        text: Option<&str>,
        sequence: Option<u64>,
    ) -> Option<()> {
        let clipboard = ClipboardTarget::resolve(unsafe { &*host })?;
        let realm = node_owner_document_relevant_context(scope, host, clipboard.event_target)?;
        let scope = &mut v8::ContextScope::new(scope, realm);
        let result = dispatch_clipboard_event(
            scope,
            host,
            clipboard.event_target,
            ClipboardEvent::Paste(text),
        )?;
        let runtime = unsafe { &*host };
        if !matches!(result, ClipboardDispatch::Default) || !clipboard.still_focused(runtime) {
            return None;
        }
        let text = text.filter(|text| !text.is_empty())?;
        let target = active_target(runtime)?;
        let editor = read_editor(scope, runtime, target)?;
        let revision = editor.revision(runtime);
        let existing = self.take_composition(target, revision);
        let range = existing
            .as_ref()
            .map_or(&editor.selection, |state| &state.marked);
        let applied = replace_range(
            scope,
            host,
            &editor,
            revision,
            range,
            text,
            TextEditInputType::InsertFromPaste,
        );
        if existing.is_some() {
            composition_event(scope, host, target, "compositionend", "");
        }
        if applied && let Some(sequence) = sequence {
            self.remember_edit(scope, unsafe { &*host }, target, sequence);
        }
        applied.then_some(())
    }

    fn remember_edit(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: &PageScriptHost,
        target: DomHandle,
        sequence: u64,
    ) {
        self.last_edit = read_editor(scope, host, target)
            .map(|editor| (sequence, target.index_u32(), editor.revision(host)));
    }

    fn take_composition(&mut self, target: DomHandle, revision: u64) -> Option<Composition> {
        self.composition
            .take()
            .filter(|state| state.target == target && state.revision == revision)
    }

    pub(crate) fn snapshot(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
    ) -> Option<RendererViewportEditor> {
        let host = unsafe { &*host_ptr };
        let target = active_target(host)?;
        let realm = node_owner_document_relevant_context(scope, host_ptr, target)?;
        let scope = &mut v8::ContextScope::new(scope, realm);
        let editor = read_editor(scope, unsafe { &*host_ptr }, target)?;
        let host = unsafe { &*host_ptr };
        let revision = editor.revision(host);
        let marked = self
            .composition
            .as_ref()
            .filter(|state| state.target == target && state.revision == revision)
            .map(|state| state.marked.clone());
        let extent = editor.text.encode_utf16().count();
        let start = editor.selection.start.saturating_sub(1024).min(extent);
        let end = (start + 4096).min(extent);
        let text = if editor.password {
            String::new()
        } else {
            String::from_utf16_lossy(
                &editor
                    .text
                    .encode_utf16()
                    .skip(start)
                    .take(end - start)
                    .collect::<Vec<_>>(),
            )
        };
        let caret = if editor.reversed {
            editor.selection.start
        } else {
            editor.selection.end
        };
        let caret_rect = editor_rect(host, &editor, caret..caret);
        let mark_rects = marked.as_ref().map_or_else(Vec::new, |range| {
            let length = range.len().min(128);
            (range.start..range.start + length)
                .filter_map(|index| {
                    editor_rect(host, &editor, index..index + 1).map(|rect| (index, rect))
                })
                .collect()
        });
        Some(RendererViewportEditor {
            target: target.index_u32(),
            revision,
            text,
            password: editor.password,
            text_start: start,
            selection: editor.selection,
            reversed: editor.reversed,
            marked,
            caret_rect,
            mark_rects,
        })
    }

    pub(crate) fn apply(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        edit: RendererViewportEdit,
    ) -> bool {
        let Some(target) =
            active_target(unsafe { &*host_ptr }).filter(|target| target.index_u32() == edit.target)
        else {
            return false;
        };
        let Some(realm) = node_owner_document_relevant_context(scope, host_ptr, target) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, realm);
        let Some(editor) = read_editor(scope, unsafe { &*host_ptr }, target) else {
            return false;
        };
        let revision = editor.revision(unsafe { &*host_ptr });
        let based_on_current = match edit.previous {
            Some(sequence) => self.last_edit == Some((sequence, edit.target, revision)),
            None => revision == edit.revision,
        };
        if !based_on_current {
            return false;
        }
        let existing = if matches!(edit.action, RendererViewportEditAction::Paste { .. }) {
            None
        } else {
            self.take_composition(target, revision)
        };
        let applied = match edit.action {
            RendererViewportEditAction::Compose {
                range,
                text,
                selection,
            } => {
                let range = range
                    .or_else(|| existing.as_ref().map(|state| state.marked.clone()))
                    .unwrap_or_else(|| editor.selection.clone());
                let original = existing
                    .as_ref()
                    .map(|state| state.original.clone())
                    .unwrap_or_else(|| text_at(&editor.text, &range));
                if existing.is_none()
                    && !composition_event(scope, host_ptr, target, "compositionstart", "")
                {
                    return false;
                }
                composition_event(scope, host_ptr, target, "compositionupdate", &text);
                if !replace_range(
                    scope,
                    host_ptr,
                    &editor,
                    revision,
                    &range,
                    &text,
                    TextEditInputType::InsertCompositionText,
                ) {
                    composition_event(scope, host_ptr, target, "compositionend", "");
                    return false;
                }
                let Some(after) = read_editor(scope, unsafe { &*host_ptr }, target) else {
                    return false;
                };
                let inserted = after
                    .text
                    .encode_utf16()
                    .count()
                    .saturating_add(range.len())
                    .saturating_sub(editor.text.encode_utf16().count());
                let marked = range.start..range.start + inserted;
                if let Some(selection) = selection {
                    let range = (marked.start + selection.start.min(inserted))
                        ..(marked.start + selection.end.min(inserted));
                    set_range(scope, host_ptr, &after, &range);
                }
                self.composition =
                    read_editor(scope, unsafe { &*host_ptr }, target).map(|after| Composition {
                        target,
                        original,
                        marked,
                        revision: after.revision(unsafe { &*host_ptr }),
                    });
                true
            }
            RendererViewportEditAction::Commit { range, text } => {
                let range = range
                    .or_else(|| existing.as_ref().map(|state| state.marked.clone()))
                    .unwrap_or_else(|| editor.selection.clone());
                let input_type = if existing.is_some() {
                    TextEditInputType::InsertCompositionText
                } else {
                    TextEditInputType::InsertText
                };
                let applied = replace_range(
                    scope, host_ptr, &editor, revision, &range, &text, input_type,
                );
                if existing.is_some() {
                    composition_event(
                        scope,
                        host_ptr,
                        target,
                        "compositionend",
                        if applied { &text } else { "" },
                    );
                }
                applied
            }
            RendererViewportEditAction::Paste { text } => self
                .paste_from_clipboard(scope, host_ptr, Some(&text), None)
                .is_some(),
            RendererViewportEditAction::Finish => {
                if let Some(state) = existing {
                    composition_event(
                        scope,
                        host_ptr,
                        target,
                        "compositionend",
                        &text_at(&editor.text, &state.marked),
                    );
                }
                true
            }
            RendererViewportEditAction::Cancel => {
                if let Some(state) = existing {
                    let applied = replace_range(
                        scope,
                        host_ptr,
                        &editor,
                        revision,
                        &state.marked,
                        &state.original,
                        TextEditInputType::InsertCompositionText,
                    );
                    if applied
                        && let Some(after) = read_editor(scope, unsafe { &*host_ptr }, target)
                    {
                        let restored = state.marked.start
                            ..state.marked.start + state.original.encode_utf16().count();
                        set_range(scope, host_ptr, &after, &restored);
                    }
                    composition_event(scope, host_ptr, target, "compositionend", "");
                    applied
                } else {
                    true
                }
            }
        };
        if applied {
            self.remember_edit(scope, unsafe { &*host_ptr }, target, edit.sequence);
        }
        applied
    }
}

fn composition_event(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    target: DomHandle,
    event_type: &str,
    data: &str,
) -> bool {
    construct_native_composition_event(scope, event_type, data)
        .is_some_and(|event| dispatch_public_event(scope, host, target, event).allows_default())
}

fn replace_range(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    editor: &Editor,
    revision: u64,
    range: &Range<usize>,
    text: &str,
    input_type: TextEditInputType,
) -> bool {
    let target = editor.target;
    if range.start > range.end
        || range.end > editor.text.encode_utf16().count()
        || active_target(unsafe { &*host }) != Some(target)
        || !dispatch_beforeinput(scope, host, target, input_type, input_type.data(text))
    {
        return false;
    }
    let live_host = unsafe { &*host };
    let Some(live) = read_editor(scope, live_host, target) else {
        return false;
    };
    if live.revision(live_host) != revision
        || active_target(live_host) != Some(target)
        || !set_range(scope, host, &live, range)
    {
        return false;
    }
    if live.map.is_some() {
        replace_contenteditable_selection_after_beforeinput(scope, host, target, text, input_type)
    } else {
        replace_text_control_selection_after_beforeinput(
            scope, host, target, text, input_type, None,
        )
    }
}

fn set_range(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    editor: &Editor,
    range: &Range<usize>,
) -> bool {
    if editor.map.is_none() {
        text_control_set_selection_range_internal(
            scope,
            host,
            editor.target,
            range.start as u32,
            range.end as u32,
        );
        return true;
    }
    let window = scope.get_current_context().global(scope);
    let (start_node, start) = editor.offset(range.start);
    let (end_node, end) = editor.offset(range.end);
    let Some(selection) = selection_value_for_window(scope, window) else {
        return false;
    };
    let Some(document) = selection_owner_document(scope, selection) else {
        return false;
    };
    let Some(range) = new_range_for_document(scope, document) else {
        return false;
    };
    let Some(start_node) = node_wrapper_from_handle(scope, start_node) else {
        return false;
    };
    let Some(end_node) = node_wrapper_from_handle(scope, end_node) else {
        return false;
    };
    selection_store(
        scope,
        selection,
        range,
        start_node,
        start as u32,
        end_node,
        end as u32,
        "forward",
    );
    selection_dispatch_change(scope);
    true
}

fn text_at(text: &str, range: &Range<usize>) -> String {
    String::from_utf16_lossy(
        &text
            .encode_utf16()
            .skip(range.start)
            .take(range.len())
            .collect::<Vec<_>>(),
    )
}

fn editor_rect(host: &PageScriptHost, editor: &Editor, range: Range<usize>) -> Option<[f32; 4]> {
    let (source, start) = editor.offset(range.start);
    let (end_node, end) = editor.offset(range.end);
    let document = host.layout_document_for_source(source)?;
    let query = LayoutQuery::TextRangeRects {
        source,
        utf16_range: start..if end_node == source { end } else { start },
    };
    let rect = match host
        .answer_layout_query_for_document(
            document,
            &query,
            crate::native_bridge::LayoutQueryMode::Published,
        )
        .ok()?
    {
        LayoutQueryAnswer::TextRangeRects(rects) => rects.first().map(|quad| quad.bounding_rect()),
        _ => None,
    }
    .or_else(|| {
        let LayoutQueryAnswer::ClientRects(rects) = host
            .answer_layout_query_for_document(
                document,
                &LayoutQuery::ClientRects {
                    source: editor.target,
                },
                crate::native_bridge::LayoutQueryMode::Published,
            )
            .ok()?
        else {
            return None;
        };
        let rect = rects.first()?.bounding_rect();
        Some(LayoutRect::new(
            rect.x + 4.,
            rect.y + 3.,
            0.,
            rect.height.max(14.) - 6.,
        ))
    })?;
    let frame_to_root = host.input_frame_transform(document, None)?.inverse()?;
    let rect = frame_to_root.map_rect(rect).bounding_rect();
    Some([rect.x, rect.y, rect.width.max(1.), rect.height.max(1.)])
}
