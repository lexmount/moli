mod clipboard;
mod editor;

use std::ops::Range;

use moli_webapi::events::subclasses::construct_native_composition_event;

pub(crate) use self::clipboard::{
    clipboard_document, copy_or_cut_viewport_selection, selected_viewport_text,
};
use self::editor::{active_target, read_editor, replace_range, set_range, text_at};
use crate::{
    document_runtime::DomHandle,
    native_bridge::{
        PageScriptHost,
        element::{TextEditInputType, dispatch_public_event},
        node_owner_document_relevant_context,
    },
    runtime::{RendererViewportEdit, RendererViewportEditAction, RendererViewportEditor},
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

impl ViewportEditing {
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

    pub(crate) fn complete_clipboard_edit(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        document: DomHandle,
        sequence: u64,
    ) {
        let runtime = unsafe { &*host };
        if clipboard_document(runtime) == Some(document)
            && let Some(target) = active_target(runtime)
            && let Some(realm) = node_owner_document_relevant_context(scope, host, target)
        {
            let scope = &mut v8::ContextScope::new(scope, realm);
            self.remember_edit(scope, unsafe { &*host }, target, sequence);
        }
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
        Some(editor.into_snapshot(host, revision, marked))
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
            RendererViewportEditAction::Paste { text } => {
                let document = clipboard_document(unsafe { &*host_ptr });
                let applied = self.paste_from_clipboard(scope, host_ptr, Some(&text));
                if let Some(document) = document {
                    self.complete_clipboard_edit(scope, host_ptr, document, edit.sequence);
                }
                return applied;
            }
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
