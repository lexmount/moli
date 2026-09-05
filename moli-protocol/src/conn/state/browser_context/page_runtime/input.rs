use super::BrowserContext;
use moli_core::page::{
    PageInputExt, PendingPageCommand, RendererDragData, RendererPointerEventProperties,
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
    pub(crate) fn start_target_autofill_trigger(
        &self,
        target_id: &str,
        request: moli_core::page::RendererAutofillTriggerRequest,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_autofill_trigger(request)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_autofill_trigger(
        &mut self,
        target_id: &str,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<moli_core::page::RendererAutofillTriggerOutcome, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_autofill_trigger(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_target_input_command(
        &self,
        target_id: &str,
        command: PageInputCommand<'_>,
    ) -> Result<PendingPageCommand, String> {
        let page = self
            .loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?;
        match command {
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
        .map_err(|error| error.to_string())
    }
}
