use anyhow::Result;

use super::{
    CompletedPageCommand, Page, PendingPageCommand, RendererCommandTurnCompletion,
    RendererDragData, RendererInputDispatchOutcome, RendererPageCommand, RendererPageReply,
    RendererPointerEventProperties, RendererTouchPoint,
};

fn decode_input_dispatch_outcome_reply(
    reply: RendererPageReply,
    operation: &str,
) -> Result<RendererInputDispatchOutcome> {
    expect_page_reply!(
        reply,
        operation,
        "an input dispatch outcome reply",
        RendererPageReply::InputDispatchOutcome(value) => Ok(value),
    )
}

fn decode_insert_text_reply(reply: RendererPageReply) -> Result<bool> {
    expect_page_reply!(
        reply,
        "insert text page command",
        "a bool reply",
        RendererPageReply::Bool(value) => Ok(value),
    )
}

fn settled_input_reply(completion: RendererCommandTurnCompletion) -> Result<RendererPageReply> {
    let (reply, _page_state, post_response_continuation) = completion.into_parts();
    anyhow::ensure!(
        post_response_continuation.is_none(),
        "input completion must consume its renderer turn before reply decoding"
    );
    Ok(reply)
}

/// Decodes a completed renderer turn shared by mouse, touch, drag, and key dispatches.
#[doc(hidden)]
pub fn decode_input_dispatch_outcome_completion(
    completion: RendererCommandTurnCompletion,
    operation: &str,
) -> Result<RendererInputDispatchOutcome> {
    decode_input_dispatch_outcome_reply(settled_input_reply(completion)?, operation)
}

/// Decodes the completed renderer turn returned by an insert-text command.
#[doc(hidden)]
pub fn decode_insert_text_completion(completion: RendererCommandTurnCompletion) -> Result<bool> {
    decode_insert_text_reply(settled_input_reply(completion)?)
}

#[allow(async_fn_in_trait)]
pub trait PageInputExt {
    async fn dispatch_mouse_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn dispatch_mouse_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<bool>;
    fn start_dispatch_mouse_event_at_point_with_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<PendingPageCommand>;
    fn start_dispatch_mouse_event_at_point_with_pointer_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
    ) -> Result<PendingPageCommand>;
    fn finish_dispatch_mouse_event_at_point_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn dispatch_touch_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn dispatch_touch_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<bool>;
    fn start_dispatch_touch_event_at_point_with_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<PendingPageCommand>;
    fn start_dispatch_touch_event_at_points_with_outcome(
        &self,
        points: Vec<RendererTouchPoint>,
        event_name: &str,
        activate: bool,
    ) -> Result<PendingPageCommand>;
    fn finish_dispatch_touch_event_at_point_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn dispatch_drag_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn dispatch_drag_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<bool>;
    fn start_dispatch_drag_event_at_point_with_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<PendingPageCommand>;
    fn finish_dispatch_drag_event_at_point_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn clear_active_drag_data_transfer_async(&mut self) -> Result<()>;
    fn start_clear_active_drag_data_transfer(&self) -> Result<PendingPageCommand>;
    fn finish_clear_active_drag_data_transfer(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()>;
    async fn insert_text_into_active_control_async(&mut self, text: &str) -> Result<bool>;
    fn start_insert_text_into_active_control(&self, text: &str) -> Result<PendingPageCommand>;
    fn finish_insert_text_into_active_control(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool>;
    async fn dispatch_key_event_with_outcome_async(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<RendererInputDispatchOutcome>;
    fn start_dispatch_key_event_with_outcome(
        &self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<PendingPageCommand>;
    fn finish_dispatch_key_event_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome>;
    async fn dispatch_key_event_async(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<bool>;
}

impl PageInputExt for Page {
    async fn dispatch_mouse_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<RendererInputDispatchOutcome> {
        Page::dispatch_mouse_event_at_point_with_outcome_async(
            self, x, y, event_name, button, buttons, delta_x, delta_y,
        )
        .await
    }

    async fn dispatch_mouse_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<bool> {
        Page::dispatch_mouse_event_at_point_async(
            self, x, y, event_name, button, buttons, delta_x, delta_y,
        )
        .await
    }

    fn start_dispatch_mouse_event_at_point_with_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<PendingPageCommand> {
        self.start_dispatch_mouse_event_at_point_with_pointer_outcome(
            x,
            y,
            event_name,
            button,
            buttons,
            0,
            delta_x,
            delta_y,
            RendererPointerEventProperties::default(),
            0,
        )
    }

    fn start_dispatch_mouse_event_at_point_with_pointer_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DispatchMouseEventAtPoint {
            x,
            y,
            event_name: event_name.to_owned(),
            button,
            buttons,
            click_count,
            delta_x,
            delta_y,
            pointer,
            modifiers,
        })
    }

    fn finish_dispatch_mouse_event_at_point_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome> {
        let reply = self.finish_page_command(completion);
        decode_input_dispatch_outcome_reply(reply, "mouse event page command")
    }

    async fn dispatch_touch_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        Page::dispatch_touch_event_at_point_with_outcome_async(self, x, y, event_name, activate)
            .await
    }

    async fn dispatch_touch_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<bool> {
        Page::dispatch_touch_event_at_point_async(self, x, y, event_name, activate).await
    }

    fn start_dispatch_touch_event_at_point_with_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<PendingPageCommand> {
        self.start_dispatch_touch_event_at_points_with_outcome(
            vec![RendererTouchPoint { id: 0, x, y }],
            event_name,
            activate,
        )
    }

    fn start_dispatch_touch_event_at_points_with_outcome(
        &self,
        points: Vec<RendererTouchPoint>,
        event_name: &str,
        activate: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DispatchTouchEvent {
            points,
            event_name: event_name.to_owned(),
            activate,
        })
    }

    fn finish_dispatch_touch_event_at_point_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome> {
        let reply = self.finish_page_command(completion);
        decode_input_dispatch_outcome_reply(reply, "touch event page command")
    }

    async fn dispatch_drag_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        Page::dispatch_drag_event_at_point_with_outcome_async(
            self, x, y, event_name, data, modifiers,
        )
        .await
    }

    async fn dispatch_drag_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<bool> {
        Page::dispatch_drag_event_at_point_async(self, x, y, event_name, data, modifiers).await
    }

    fn start_dispatch_drag_event_at_point_with_outcome(
        &self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DispatchDragEventAtPoint {
            x,
            y,
            event_name: event_name.to_owned(),
            data,
            modifiers,
        })
    }

    fn finish_dispatch_drag_event_at_point_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome> {
        let reply = self.finish_page_command(completion);
        decode_input_dispatch_outcome_reply(reply, "drag event page command")
    }

    async fn clear_active_drag_data_transfer_async(&mut self) -> Result<()> {
        Page::clear_active_drag_data_transfer_async(self).await
    }

    fn start_clear_active_drag_data_transfer(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ClearActiveDragDataTransfer)
    }

    fn finish_clear_active_drag_data_transfer(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "clear active drag data transfer page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    async fn insert_text_into_active_control_async(&mut self, text: &str) -> Result<bool> {
        Page::insert_text_into_active_control_async(self, text).await
    }

    fn start_insert_text_into_active_control(&self, text: &str) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::InsertTextIntoActiveControl(
            text.to_owned(),
        ))
    }

    fn finish_insert_text_into_active_control(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<bool> {
        let reply = self.finish_page_command(completion);
        decode_insert_text_reply(reply)
    }

    async fn dispatch_key_event_with_outcome_async(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        Page::dispatch_key_event_with_outcome_async(
            self,
            event_name,
            key,
            code,
            text,
            modifiers,
            auto_repeat,
            should_insert_text,
        )
        .await
    }

    fn start_dispatch_key_event_with_outcome(
        &self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DispatchKeyEvent {
            event_name: event_name.to_owned(),
            key: key.to_owned(),
            code: code.to_owned(),
            text: text.to_owned(),
            modifiers,
            auto_repeat,
            should_insert_text,
        })
    }

    fn finish_dispatch_key_event_with_outcome(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererInputDispatchOutcome> {
        let reply = self.finish_page_command(completion);
        decode_input_dispatch_outcome_reply(reply, "key event page command")
    }

    async fn dispatch_key_event_async(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<bool> {
        Page::dispatch_key_event_async(
            self,
            event_name,
            key,
            code,
            text,
            modifiers,
            auto_repeat,
            should_insert_text,
        )
        .await
    }
}
