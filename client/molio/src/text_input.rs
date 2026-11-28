use std::ops::Range;

use moli_core::{
    RendererDocumentLifecycleIdentity, RendererViewportEdit, RendererViewportEditAction,
    RendererViewportEditor,
};

use crate::address::{utf8_range, utf16_range};
use crate::viewport_edit_sequence::ViewportEditSequence;

/// Synchronous state for the platform input client. Versions refer to the
/// renderer's edit position, not the most recent raster frame.
#[derive(Default)]
pub struct MoliTextInputBridge {
    document: Option<RendererDocumentLifecycleIdentity>,
    editor: Option<RendererViewportEditor>,
    edits: ViewportEditSequence,
}

impl MoliTextInputBridge {
    pub fn clear(&mut self) {
        self.document = None;
        self.editor = None;
        self.edits.clear();
    }

    pub fn refresh(
        &mut self,
        document: Option<RendererDocumentLifecycleIdentity>,
        editor: Option<RendererViewportEditor>,
        ack: Option<u64>,
    ) -> bool {
        let same_target = self.document == document
            && self.editor.as_ref().map(|editor| editor.target)
                == editor.as_ref().map(|editor| editor.target);
        if !self.edits.accept_editor(same_target, ack) {
            return false;
        }
        let changed = self.document != document || self.editor != editor;
        self.document = document;
        self.editor = editor;
        changed
    }

    pub fn editor(&self) -> Option<&RendererViewportEditor> {
        self.editor.as_ref()
    }

    pub fn text(&self, range: Range<usize>, actual: &mut Option<Range<usize>>) -> Option<String> {
        let editor = self.editor()?;
        if range.end < editor.text_start
            || range.start > editor.text_start + editor.text.encode_utf16().count()
        {
            return None;
        }
        let bytes = utf8_range(
            &editor.text,
            range.start.saturating_sub(editor.text_start)
                ..range.end.saturating_sub(editor.text_start),
        );
        let units = utf16_range(&editor.text, bytes.clone());
        *actual = Some(editor.text_start + units.start..editor.text_start + units.end);
        Some(editor.text[bytes].to_owned())
    }

    pub fn edit(&mut self, action: RendererViewportEditAction) -> Option<RendererViewportEdit> {
        let document = self.document?;
        let editor = self.editor.as_mut()?;
        let edit = self.edits.submit(document, editor, action);
        match &edit.action {
            RendererViewportEditAction::Compose {
                range,
                text,
                selection,
            } => {
                let replacement = replacement_range(editor, range);
                update_text(editor, &replacement, text);
                let length = text.encode_utf16().count();
                let start = replacement.start;
                editor.marked = Some(start..start + length);
                let selection = selection.clone().unwrap_or(length..length);
                editor.selection =
                    start + selection.start.min(length)..start + selection.end.min(length);
                editor.reversed = false;
            }
            RendererViewportEditAction::Commit { range, text } => {
                let replacement = replacement_range(editor, range);
                update_text(editor, &replacement, text);
                let caret = replacement.start + text.encode_utf16().count();
                editor.selection = caret..caret;
                editor.reversed = false;
                editor.marked = None;
            }
            RendererViewportEditAction::Paste { text } => {
                let replacement = replacement_range(editor, &None);
                update_text(editor, &replacement, text);
                let caret = replacement.start + text.encode_utf16().count();
                editor.selection = caret..caret;
                editor.reversed = false;
                editor.marked = None;
            }
            RendererViewportEditAction::Finish | RendererViewportEditAction::Cancel => {
                editor.marked = None
            }
        }
        Some(edit)
    }
}

fn replacement_range(
    editor: &RendererViewportEditor,
    range: &Option<Range<usize>>,
) -> Range<usize> {
    range
        .clone()
        .or_else(|| editor.marked.clone())
        .unwrap_or_else(|| editor.selection.clone())
}

fn update_text(editor: &mut RendererViewportEditor, range: &Range<usize>, text: &str) {
    let end = editor.text_start + editor.text.encode_utf16().count();
    if !editor.password && range.start >= editor.text_start && range.end <= end {
        let local = range.start - editor.text_start..range.end - editor.text_start;
        let bytes = utf8_range(&editor.text, local);
        editor.text.replace_range(bytes, text);
    }
    editor.mark_rects.clear();
}
