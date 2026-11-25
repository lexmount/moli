use super::super::*;
use super::child_frame_nodes::child_frame_document_contains_live_handle;
use crate::script_vm::{MouseDispatchCompletion, WheelDispatchOptions};

impl PageVm {
    pub(in crate::runtime) fn bind_wheel_input_controller(
        &mut self,
        controller: std::rc::Rc<std::cell::RefCell<crate::wheel_input::WheelInputController>>,
    ) {
        if !std::rc::Rc::ptr_eq(&self.vm().wheel_input, &controller) {
            controller.borrow_mut().retire_document();
            self.vm_mut().wheel_input = controller;
        }
    }

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
        #[cfg(feature = "molio")]
        let tracks_viewport_cursor = pointer.pointer_type == "mouse"
            && matches!(event_name, "mousemove" | "mousedown" | "mouseup" | "wheel");
        let mut outcome = self.vm_mut().dispatch_mouse_input(
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
            MouseDispatchCompletion::FinishTurn,
        )?;
        #[cfg(feature = "molio")]
        if tracks_viewport_cursor {
            self.update_viewport_pointer_position(Some((x, y)));
        }
        self.bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
        Ok(outcome)
    }

    pub(in crate::runtime) fn wheel_position_in_bounds(&self, x: f64, y: f64) -> bool {
        let (width, height) = self
            .vm()
            .document_runtime
            .viewport_surface()
            .unwrap_or_default()
            .visible_size();
        x >= 0.0 && y >= 0.0 && x < f64::from(width) && y < f64::from(height)
    }

    pub(crate) fn validate_synthetic_wheel_anchor(
        &self,
        x: f64,
        y: f64,
        gesture: &crate::runtime::RendererSyntheticWheelGesture,
    ) -> Result<RendererInputDispatchOutcome> {
        anyhow::ensure!(
            self.wheel_position_in_bounds(x, y),
            "Position out of bounds"
        );
        gesture.validate_anchor(true);
        Ok(RendererInputDispatchOutcome::default())
    }

    pub(crate) fn dispatch_synthetic_wheel(
        &mut self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        end: bool,
        intercept_drags: bool,
        gesture: crate::runtime::RendererSyntheticWheelGesture,
    ) -> Result<RendererInputDispatchOutcome> {
        use crate::wheel_input::{WheelInput, WheelPhase, WheelSource};
        let Some(owner) = self.vm().wheel_document_owner() else {
            return Ok(RendererInputDispatchOutcome::default());
        };
        // Aura filters producer samples (including End) before platform
        // phase inference. A clipped sample does not bind the round.
        let in_bounds = self.wheel_position_in_bounds(x, y);
        if !end {
            anyhow::ensure!(gesture.validate_anchor(in_bounds), "Position out of bounds");
        }
        if !in_bounds || !gesture.accepts_document(owner, !end) {
            return Ok(RendererInputDispatchOutcome::default());
        }
        self.vm_mut().dispatch_wheel_input(
            WheelInput {
                position: (x, y),
                delta: (delta_x, delta_y),
                modifiers: 0,
                timestamp: std::time::Instant::now(),
                phase: if end {
                    WheelPhase::Ended
                } else {
                    WheelPhase::Unspecified
                },
                source: WheelSource::Synthetic(gesture.round_id()),
            },
            WheelDispatchOptions {
                intercept_drags,
                ..Default::default()
            },
        )
    }

    pub(crate) fn cancel_synthetic_wheel(
        &self,
        gesture: &crate::runtime::RendererSyntheticWheelGesture,
    ) -> RendererInputDispatchOutcome {
        if let Some(document) = self.vm().wheel_document_owner()
            && gesture.accepts_document(document, false)
        {
            self.vm()
                .wheel_input
                .borrow_mut()
                .cancel_synthetic_round(document, gesture.round_id());
        }
        RendererInputDispatchOutcome::default()
    }

    pub(crate) fn dispatch_touch_event_at_points(
        &mut self,
        points: &[RendererTouchPoint],
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let mut outcome = self
            .vm_mut()
            .dispatch_touch_event_at_points(points, event_name, activate)?;
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
