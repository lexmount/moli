use moli_webapi::events::subclasses::construct_native_clipboard_event;

use super::{
    ViewportEditing, composition_event,
    editor::{active_target, is_password_control, read_editor, replace_range, text_at},
};
use crate::{
    context_bootstrap::{
        DragDataStoreMode, create_drag_data_transfer_view, create_native_drag_data_store,
        finish_drag_data_transfer_view, native_data_transfer_get_data,
        native_data_transfer_has_type, populate_native_drag_data_store,
        range_algorithms::range_selection_string_contents, selection::selection_range,
        selection_value_for_window,
    },
    document_runtime::DomHandle,
    dom::native::Node,
    native_bridge::{
        PageScriptHost,
        element::{
            TextEditInputType, contenteditable_editing_host, dispatch_public_event,
            is_text_control, text_control_value,
        },
        node_owner_document_relevant_context,
    },
    runtime::{RendererDragData, RendererDragDataItem},
};

fn clipboard_selection_target(
    host: &PageScriptHost,
    focus_target: DomHandle,
    document: DomHandle,
) -> Option<DomHandle> {
    let dom = host.dom_host();
    let (start, end) = host
        .document_selection_snapshot(document)?
        .ordered_in_dom(dom)?;
    let container = start.container;
    // An unfocused caret is hidden, while a document range can remain visible.
    if start == end {
        let editor = contenteditable_editing_host(host, focus_target)?;
        if !dom.dom().contains(editor, container) {
            return None;
        }
    }
    let node = dom.node(container)?;
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
    document: DomHandle,
    event_target: DomHandle,
}

fn active_clipboard_document(host: &PageScriptHost, target: DomHandle) -> Option<DomHandle> {
    let document = host.dom_host().owner_document_handle(target)?;
    host.document_is_fully_active(document).then_some(document)
}

pub(crate) fn clipboard_document(host: &PageScriptHost) -> Option<DomHandle> {
    active_clipboard_document(
        host,
        host.active_element_handle()
            .unwrap_or_else(|| host.document_handle()),
    )
}

impl ClipboardTarget {
    fn resolve(host: &PageScriptHost) -> Option<Self> {
        let focus_target = host
            .active_element_handle()
            .or_else(|| host.document_focus_fallback_handle())?;
        let document = active_clipboard_document(host, focus_target)?;
        let event_target = if is_text_control(host, focus_target) {
            focus_target
        } else {
            clipboard_selection_target(host, focus_target, document).unwrap_or(focus_target)
        };
        Some(Self {
            document,
            event_target,
        })
    }

    fn is_current_document(&self, host: &PageScriptHost) -> bool {
        clipboard_document(host) == Some(self.document)
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
    let initial_focus = runtime.active_element_handle();
    if runtime
        .dom_host()
        .node(clipboard.event_target)
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
    if !clipboard.is_current_document(runtime)
        || (cut && runtime.active_element_handle() != initial_focus)
    {
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

impl ViewportEditing {
    pub(crate) fn paste_from_clipboard(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        text: Option<&str>,
    ) -> bool {
        let Some(clipboard) = ClipboardTarget::resolve(unsafe { &*host }) else {
            return false;
        };
        let Some(realm) = node_owner_document_relevant_context(scope, host, clipboard.document)
        else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, realm);
        if matches!(
            dispatch_clipboard_event(
                scope,
                host,
                clipboard.event_target,
                ClipboardEvent::Paste(text),
            ),
            Some(ClipboardDispatch::Default)
        ) && clipboard.is_current_document(unsafe { &*host })
            && let Some(text) = text.filter(|text| !text.is_empty())
        {
            self.insert_clipboard_text(scope, host, text)
                .unwrap_or(false)
        } else {
            false
        }
    }

    fn insert_clipboard_text(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        text: &str,
    ) -> Option<bool> {
        let runtime = unsafe { &*host };
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
        Some(applied)
    }
}
