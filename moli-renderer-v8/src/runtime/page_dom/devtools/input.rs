use super::super::*;
use super::child_frame_nodes::child_frame_document_contains_live_handle;

impl PageVm {
    pub(crate) fn dispatch_mouse_event_at_point_with_pointer(
        &mut self,
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
        intercept_drags: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self
            .vm_mut()
            .dispatch_mouse_event_at_point_with_pointer_and_modifiers(
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
                intercept_drags,
            )?;
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(crate) fn dispatch_touch_event_input(
        &mut self,
        input: &RendererTouchEventInput,
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = match input {
            RendererTouchEventInput::ChangedPoints(points) => self
                .vm_mut()
                .dispatch_touch_event_at_points(points, event_name, activate)?,
            RendererTouchEventInput::ContactUpdates(points) => self
                .vm_mut()
                .dispatch_touch_contact_updates(points, event_name)?,
        };
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(crate) fn cancel_dragging(&mut self) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self.vm_mut().cancel_dragging()?;
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(crate) fn dispatch_drag_event_at_point(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self
            .vm_mut()
            .dispatch_drag_event_at_point(x, y, event_name, data, modifiers)?;
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(in crate::runtime) fn bind_input_dispatch_file_chooser_backend_node_id(
        &mut self,
        outcome: &mut RendererInputDispatchOutcome,
    ) {
        let Some(file_chooser) = outcome.pending_file_chooser.as_mut() else {
            return;
        };
        if !self.bind_file_chooser_activation_backend_node_id(file_chooser) {
            outcome.pending_file_chooser = None;
        }
    }

    pub(crate) fn clear_active_drag_data_transfer(&mut self) -> Result<()> {
        self.vm_mut().clear_active_drag_data_transfer()
    }

    pub(crate) fn set_file_input_files_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Result<Option<bool>> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(None);
        };
        self.vm_mut()
            .set_file_input_files(handle, files, append)
            .map(Some)
    }

    pub(crate) fn set_file_input_files_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Result<Option<bool>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        self.vm_mut()
            .set_file_input_files(handle, files, append)
            .map(Some)
    }

    pub(crate) fn insert_text_into_active_control(&mut self, text: &str) -> Result<bool> {
        self.vm_mut().insert_text_into_active_control(text)
    }

    pub(crate) fn dispatch_key_event(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
        location: u32,
    ) -> Result<RendererInputDispatchOutcome> {
        self.vm_mut().dispatch_key_event_with_location(
            event_name,
            key,
            code,
            text,
            modifiers,
            auto_repeat,
            should_insert_text,
            location,
        )
    }

    pub(crate) fn trigger_autofill(
        &mut self,
        request: RendererAutofillTriggerRequest,
    ) -> Result<RendererAutofillTriggerOutcome> {
        let expected_document = if let Some(frame_id) = request.frame_id.as_deref() {
            let Some(document) = self
                .vm()
                .child_browsing_context_document_handle_by_frame_id(frame_id)
            else {
                return Ok(RendererAutofillTriggerOutcome::FrameNotFound);
            };
            document
        } else {
            self.vm().document_runtime.dom_host().document_handle()
        };
        let Some(handle) = self.live_handle_for_backend_node_id(request.field_id) else {
            return Ok(RendererAutofillTriggerOutcome::FieldNotFound);
        };
        let belongs_to_expected_document = if request.frame_id.is_some() {
            child_frame_document_contains_live_handle(
                self.vm().document_runtime.dom_host(),
                expected_document,
                handle,
            )
        } else {
            handle == expected_document
                || self
                    .vm()
                    .document_runtime
                    .dom_host()
                    .owner_document_handle(handle)
                    == Some(expected_document)
        };
        if !belongs_to_expected_document {
            return Ok(RendererAutofillTriggerOutcome::FieldNotFound);
        }
        self.vm_mut().trigger_autofill(handle, request)
    }
}
