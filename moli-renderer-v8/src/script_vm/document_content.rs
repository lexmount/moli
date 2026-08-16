use super::ScriptVm;
use crate::runtime::RendererSetDocumentContentResult;
use anyhow::Result;

impl ScriptVm {
    pub(crate) fn set_script_execution_disabled(&mut self, disabled: bool) {
        self.document_runtime
            .set_script_execution_disabled(disabled);
    }

    pub(crate) fn script_execution_disabled(&self) -> bool {
        self.document_runtime.script_execution_disabled()
    }

    pub(crate) fn script_execution_control(
        &self,
    ) -> crate::script_execution_control::RendererScriptExecutionControl {
        self.document_runtime.script_execution_control()
    }

    pub(crate) fn bind_script_execution_control(
        &mut self,
        control: crate::script_execution_control::RendererScriptExecutionControl,
    ) {
        self.document_runtime.bind_script_execution_control(control);
    }

    pub(crate) fn set_document_content_for_frame(
        &mut self,
        frame_id: &str,
        html: &str,
    ) -> Result<RendererSetDocumentContentResult> {
        let result = if self.root_frame_id() == Some(frame_id) {
            self.with_default_context_scope(|scope, host_ptr| {
                unsafe { &mut *host_ptr }.set_root_document_content(scope, host_ptr, html);
                Ok(())
            })?;
            RendererSetDocumentContentResult::Updated
        } else {
            let child_handle = self
                ._context_host
                .borrow()
                .child_browsing_context_handle_by_frame_id(frame_id);
            let Some(child_handle) = child_handle else {
                return Ok(RendererSetDocumentContentResult::FrameNotFound);
            };
            let updated = self.with_default_context_scope(|scope, host_ptr| {
                Ok(
                    unsafe { &mut *host_ptr }.set_child_browsing_context_document_content(
                        scope,
                        host_ptr,
                        child_handle,
                        html,
                    ),
                )
            })?;
            if !updated {
                return Ok(RendererSetDocumentContentResult::DocumentNotFound);
            }
            RendererSetDocumentContentResult::Updated
        };

        // Chromium runs InspectorPageAgent::setDocumentContent inside one
        // renderer task, and the main-thread scheduler performs the microtask
        // checkpoint when that task completes. Our Page command is the
        // equivalent task boundary, so MutationObserver callbacks and custom
        // element reactions produced by Document::SetContent must settle before
        // the command response may cross its output cursor.
        self.perform_owner_lane_task_microtask_checkpoints()?;
        Ok(result)
    }
    pub(crate) fn resume_document_write_stylesheet_blocked_script(&mut self) -> Result<bool> {
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *host_ptr }
                .resume_document_write_stylesheet_blocked_script(scope, host_ptr))
        })
    }
}
