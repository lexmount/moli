use super::PageVm;
use crate::ace_page_task::*;
use crate::dom::native::DomHost;
use crate::runtime::*;
use anyhow::Result;
use moli_dom::NodeId as DomHandle;

#[cfg(test)]
mod tests;

impl PageVm {
    pub(in crate::runtime) fn execute_ace_page_task(
        &mut self,
        task: Box<dyn AcePageTask>,
    ) -> Result<RendererPageReply> {
        let mut host = AcePageTaskHost {
            page: self,
            kind: task.kind(),
            input_outcomes: Vec::new(),
        };
        let result = task.run(&mut host).map_err(|error| format!("{error:#}"));
        Ok(RendererPageReply::AcePageTask(Box::new(
            AcePageTaskExecution {
                result,
                input_outcomes: host.input_outcomes,
            },
        )))
    }

    fn inspection_device_pixel_ratio(&self) -> f64 {
        self.document_settings.viewport_surface.map_or(
            moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.device_pixel_ratio,
            |surface| surface.device_pixel_ratio,
        )
    }

    fn inspection_viewport(&self) -> moli_layout::LayoutViewport {
        let surface = self.document_settings.viewport_surface;
        let defaults = moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
        moli_layout::LayoutViewport::new(
            surface.map_or(defaults.inner_width as u32, |surface| surface.inner_width),
            surface.map_or(defaults.inner_height as u32, |surface| surface.inner_height),
            self.inspection_device_pixel_ratio() as f32,
        )
    }
}

struct AcePageTaskHost<'a> {
    page: &'a mut PageVm,
    kind: AcePageTaskKind,
    input_outcomes: Vec<RendererInputDispatchOutcome>,
}

impl AcePageTaskHost<'_> {
    fn require_input(&self) -> Result<()> {
        anyhow::ensure!(
            self.kind == AcePageTaskKind::Input,
            "Read-only ACE task cannot dispatch input"
        );
        Ok(())
    }

    // Even a failed input method may have entered page JavaScript. Keep its
    // navigation publication with the task's effects rather than its result.
    fn retain_input<T>(&mut self, result: Result<T>) -> Result<T> {
        self.input_outcomes
            .push(RendererInputDispatchOutcome::default());
        result
    }

    fn retain_dispatch(&mut self, result: Result<RendererInputDispatchOutcome>) -> Result<bool> {
        match result {
            Ok(mut outcome) => {
                self.page
                    .bind_input_dispatch_file_chooser_backend_node_id(&mut outcome);
                let handled = outcome.handled;
                self.input_outcomes.push(outcome);
                Ok(handled)
            }
            Err(error) => self.retain_input(Err(error)),
        }
    }
}

impl PageExtensionHost for AcePageTaskHost<'_> {
    fn layout_enabled(&self) -> bool {
        self.page.layout_configuration.policy.uses_real_layout()
    }
    fn document_identity(&self) -> RendererDocumentLifecycleIdentity {
        self.page.document_lifecycle.identity()
    }
    fn dom(&self) -> &DomHost {
        self.page.vm().document_runtime.dom_host()
    }
    fn url(&self) -> String {
        self.page.vm().document_runtime.document_url().to_string()
    }
    fn loading(&self) -> PageLoadingFacts {
        let vm = self.page.vm();
        PageLoadingFacts {
            ready_state: vm.document_runtime.host_document().ready_state(),
            dom_content_loaded: vm.document_runtime.dom_content_loaded_dispatched(),
            pending_network: vm.pending_subresource_request_count() > 0,
            pending_scripts: vm
                .document_runtime
                .has_pending_document_write_external_script_load()
                || vm
                    .document_runtime
                    .has_pending_document_write_stylesheet_blocked_script()
                || vm.has_pending_native_module_job(),
            pending_navigation: vm.has_pending_location_navigation(),
        }
    }
    fn device_pixel_ratio(&self) -> f64 {
        self.page.inspection_device_pixel_ratio()
    }
    fn backend_node_ids(&mut self) -> std::collections::HashMap<DomHandle, u32> {
        let document = self.dom().dom().document_node_id();
        self.page
            .renderer_backend_node_id_map_for_document_handle(document)
    }
    fn live_handle(&mut self, id: u32) -> Option<DomHandle> {
        self.page.live_handle_for_backend_node_id(id)
    }
    fn backend_node_id(&mut self, handle: DomHandle) -> Option<u32> {
        self.page.renderer_backend_node_id_for_live_handle(handle)
    }
    fn focused_node(&self) -> Option<DomHandle> {
        self.page.vm().document_runtime.active_element_handle()
    }
    fn disabled(&self, handle: DomHandle) -> bool {
        self.page.vm().inspection_disabled(handle)
    }
    fn prepare_inspection(&mut self) {
        self.page.vm_mut().prepare_document_inspection();
    }
    fn with_layout(
        &mut self,
        visitor: &mut dyn FnMut(&moli_layout::FrozenLayoutTree<DomHandle>),
    ) -> Result<bool> {
        let viewport = self.page.inspection_viewport();
        self.page.vm_mut().inspect_layout(viewport, visitor)
    }
    fn styles(
        &self,
        properties: &[&str],
        events: &[&str],
        nodes: &[DomHandle],
    ) -> std::collections::HashMap<DomHandle, InspectionStyle> {
        self.page.vm().inspection_styles(properties, events, nodes)
    }
    fn scroll_into_view(&mut self, id: u32) -> Result<RendererScrollIntoViewResult> {
        self.require_input()?;
        let result = self
            .page
            .scroll_node_into_view_if_needed(RendererDomNodeReference::BackendNodeId(id), None);
        self.retain_input(result)
    }
    fn client_rect(&mut self, id: u32) -> Result<Option<RendererDocumentNodeClientRect>> {
        self.page.client_rect_for_backend_node_id(id)
    }
    fn hit_test(&mut self, x: f64, y: f64) -> Result<Option<u32>> {
        self.page
            .document_hit_test(None, x, y, false, false)
            .map(|hit| hit.map(|hit| hit.node.backend_node_id))
    }
    fn focus(&mut self, id: u32) -> Result<RendererDomFocusOutcome> {
        self.require_input()?;
        let result = self
            .page
            .focus_document_node(RendererDomNodeReference::BackendNodeId(id));
        self.retain_input(result)
    }
    fn mouse(
        &mut self,
        x: f64,
        y: f64,
        event: &str,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        modifiers: u8,
    ) -> Result<bool> {
        self.require_input()?;
        let result = self.page.dispatch_mouse_event_at_point_with_pointer(
            x,
            y,
            event,
            button,
            buttons,
            click_count,
            0.0,
            0.0,
            RendererPointerEventProperties::default(),
            modifiers,
            false,
        );
        self.retain_dispatch(result)
    }
    fn key(
        &mut self,
        event: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        repeat: bool,
        insert_text: bool,
    ) -> Result<bool> {
        self.require_input()?;
        let result =
            self.page
                .dispatch_key_event(event, key, code, text, modifiers, repeat, insert_text);
        self.retain_dispatch(result)
    }
    fn activate(&mut self, handle: DomHandle, x: f64, y: f64, modifiers: u8) -> Result<bool> {
        self.require_input()?;
        let result = self.page.vm_mut().activate_control(handle, x, y, modifiers);
        self.retain_dispatch(result)
    }
    fn replace_text(&mut self, handle: DomHandle, text: &str) -> Result<bool> {
        self.require_input()?;
        let result = self.page.vm_mut().replace_control_text(handle, text);
        self.retain_input(result)
    }
    fn submit(&mut self, handle: DomHandle) -> Result<bool> {
        self.require_input()?;
        let result = self
            .page
            .vm_mut()
            .submit_control_form(handle)
            .map(Option::unwrap_or_default);
        self.retain_dispatch(result)
    }
    fn select_values(&mut self, handle: DomHandle, values: &[String]) -> Result<bool> {
        self.require_input()?;
        let result = self.page.vm_mut().select_control_values(handle, values);
        self.retain_input(result)
    }
    fn screenshot_clip(&mut self, handle: DomHandle) -> Result<RendererScreenshotClip> {
        let viewport = self.page.inspection_viewport();
        self.page.vm_mut().node_screenshot_clip(handle, viewport)
    }
    fn capture(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<RendererCaptureScreenshotReply> {
        self.page.capture_screenshot(request)
    }
    fn flush_screenshot(&mut self) -> Result<()> {
        self.page
            .flush_page_action_window(moli_action_window::ActionBarrier::Screenshot)
            .map(|_| ())
    }
}
