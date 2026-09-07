use super::BrowserContext;
use super::document_commands::{
    CompletedDocumentAutofillTrigger, CompletedDocumentInputCommand,
    PendingDocumentAutofillTrigger, PendingDocumentInputCommand,
};
use moli_core::browser::{DocumentHandle, DocumentLifetimeObserver};
use moli_core::page::{
    PageInputExt, RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
    RendererCommandTurnOutput, RendererDragData, RendererPointerEventProperties,
    RendererTouchPoint,
};

/// Native input admitted to the current document, independent of inspector sessions.
pub(crate) enum PageInputCommand<'a> {
    Key {
        event_name: &'a str,
        key: &'a str,
        code: &'a str,
        text: &'a str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    },
    Mouse {
        x: f64,
        y: f64,
        event_name: &'a str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
    },
    Touch {
        points: Vec<RendererTouchPoint>,
        event_name: &'a str,
        activate: bool,
    },
    Drag {
        x: f64,
        y: f64,
        event_name: &'a str,
        data: RendererDragData,
        modifiers: u8,
    },
    InsertText(&'a str),
}

impl BrowserContext {
    pub(crate) fn observe_document_lifetime(
        &mut self,
        document: DocumentHandle,
    ) -> Result<DocumentLifetimeObserver, String> {
        Ok(self.physical.document_mut(document)?.lifetime.observe())
    }

    pub(crate) fn start_document_autofill_trigger(
        &self,
        document: DocumentHandle,
        request: RendererAutofillTriggerRequest,
    ) -> Result<PendingDocumentAutofillTrigger, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_autofill_trigger(request)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentAutofillTrigger::new(document, pending))
    }

    pub(crate) fn finish_document_autofill_trigger(
        &mut self,
        completed: CompletedDocumentAutofillTrigger,
    ) -> Result<RendererAutofillTriggerOutcome, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_autofill_trigger(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_document_input_command(
        &self,
        document: DocumentHandle,
        command: PageInputCommand<'_>,
    ) -> Result<PendingDocumentInputCommand, String> {
        let page = &self.physical.document(document)?.page;
        let pending = match command {
            PageInputCommand::Key {
                event_name,
                key,
                code,
                text,
                modifiers,
                auto_repeat,
                should_insert_text,
            } => page.start_dispatch_key_event_with_outcome(
                event_name,
                key,
                code,
                text,
                modifiers,
                auto_repeat,
                should_insert_text,
            ),
            PageInputCommand::Mouse {
                x,
                y,
                event_name,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                pointer,
                modifiers,
            } => page.start_dispatch_mouse_event_at_point_with_pointer_outcome(
                x,
                y,
                event_name,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                pointer,
                modifiers,
            ),
            PageInputCommand::Touch {
                points,
                event_name,
                activate,
            } => {
                page.start_dispatch_touch_event_at_points_with_outcome(points, event_name, activate)
            }
            PageInputCommand::Drag {
                x,
                y,
                event_name,
                data,
                modifiers,
            } => page
                .start_dispatch_drag_event_at_point_with_outcome(x, y, event_name, data, modifiers),
            PageInputCommand::InsertText(text) => page.start_insert_text_into_active_control(text),
        }
        .map_err(|error| error.to_string())?;
        Ok(PendingDocumentInputCommand::new(document, pending))
    }

    pub(crate) fn finish_document_input_command(
        &mut self,
        completed: CompletedDocumentInputCommand,
    ) -> Result<RendererCommandTurnOutput, String> {
        let (document, completion) = completed.into_parts();
        let completion = completion?;
        match self.physical.document_mut(document) {
            Ok(document) => Ok(document.page.finish_page_command_turn(completion)),
            // InputInjector acknowledges an event consumed by the retired
            // renderer, but its frozen Page state cannot enter a replacement.
            Err(error) if matches!(error.as_str(), "Document changed" | "NoDocumentLoaded") => {
                Ok(completion.into_output())
            }
            Err(error) => Err(error),
        }
    }
}
