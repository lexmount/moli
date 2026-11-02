use super::PageVm;
use crate::ace_page_task::*;
use crate::dom::native::DomHost;
use crate::runtime::*;
use anyhow::Result;
use moli_dom::NodeId as DomHandle;

impl PageVm {
    pub(in crate::runtime) fn execute_ace_page_task(
        &mut self,
        task: Box<dyn AcePageTask>,
    ) -> Result<RendererPageReply> {
        let mut result = task.run(self)?;
        if let Some(outcome) = &mut result.input_outcome {
            self.bind_input_dispatch_file_chooser_backend_node_id(outcome);
        }
        Ok(RendererPageReply::AcePageTask(Box::new(result)))
    }

    fn inspection_viewport(&self) -> moli_layout::LayoutViewport {
        let surface = self.document_settings.viewport_surface;
        let defaults = moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
        moli_layout::LayoutViewport::new(
            surface.map_or(defaults.inner_width as u32, |surface| surface.inner_width),
            surface.map_or(defaults.inner_height as u32, |surface| surface.inner_height),
            self.device_pixel_ratio() as f32,
        )
    }
}

impl PageExtensionHost for PageVm {
    fn layout_enabled(&self) -> bool {
        self.layout_configuration.policy.uses_real_layout()
    }
    fn document_identity(&self) -> RendererDocumentLifecycleIdentity {
        self.document_lifecycle.identity()
    }
    fn dom(&self) -> &DomHost {
        self.vm().document_runtime.dom_host()
    }
    fn url(&self) -> String {
        self.vm().document_runtime.document_url().to_string()
    }
    fn loading(&self) -> PageLoadingFacts {
        let vm = self.vm();
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
            pending_navigation: vm.has_pending_javascript_location_navigation(),
        }
    }
    fn device_pixel_ratio(&self) -> f64 {
        self.document_settings.viewport_surface.map_or(
            moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.device_pixel_ratio,
            |surface| surface.device_pixel_ratio,
        )
    }
    fn backend_node_ids(&mut self) -> std::collections::HashMap<DomHandle, u32> {
        self.renderer_backend_node_id_map_for_document_handle(self.dom().dom().document_node_id())
    }
    fn live_handle(&mut self, id: u32) -> Option<DomHandle> {
        self.live_handle_for_backend_node_id(id)
    }
    fn backend_node_id(&mut self, handle: DomHandle) -> Option<u32> {
        self.renderer_backend_node_id_for_live_handle(handle)
    }
    fn focused_node(&self) -> Option<DomHandle> {
        self.vm().document_runtime.active_element_handle()
    }
    fn disabled(&self, handle: DomHandle) -> bool {
        self.vm().inspection_disabled(handle)
    }
    fn prepare_inspection(&mut self) {
        self.vm_mut().prepare_document_inspection();
    }
    fn with_layout(
        &mut self,
        visitor: &mut dyn FnMut(&moli_layout::FrozenLayoutTree<DomHandle>),
    ) -> Result<bool> {
        let viewport = self.inspection_viewport();
        self.vm_mut().inspect_layout(viewport, visitor)
    }
    fn styles(
        &self,
        properties: &[&str],
        events: &[&str],
        nodes: &[DomHandle],
    ) -> std::collections::HashMap<DomHandle, InspectionStyle> {
        self.vm().inspection_styles(properties, events, nodes)
    }
    fn scroll_into_view(&mut self, id: u32) -> Result<RendererScrollIntoViewResult> {
        self.scroll_node_into_view_if_needed(RendererDomNodeReference::BackendNodeId(id), None)
    }
    fn client_rect(&mut self, id: u32) -> Result<Option<RendererDocumentNodeClientRect>> {
        self.client_rect_for_backend_node_id(id)
    }
    fn hit_test(&mut self, x: f64, y: f64) -> Result<Option<u32>> {
        self.document_hit_test(None, x, y, false, false)
            .map(|hit| hit.map(|hit| hit.node.backend_node_id))
    }
    fn focus(&mut self, id: u32) -> Result<RendererDomFocusOutcome> {
        self.focus_document_node(RendererDomNodeReference::BackendNodeId(id))
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
    ) -> Result<RendererInputDispatchOutcome> {
        self.dispatch_mouse_event_at_point_with_pointer(
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
        )
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
    ) -> Result<RendererInputDispatchOutcome> {
        self.dispatch_key_event(event, key, code, text, modifiers, repeat, insert_text)
    }
    fn activate(
        &mut self,
        handle: DomHandle,
        x: f64,
        y: f64,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        self.vm_mut().activate_control(handle, x, y, modifiers)
    }
    fn replace_text(&mut self, handle: DomHandle, text: &str) -> Result<bool> {
        self.vm_mut().replace_control_text(handle, text)
    }
    fn submit(&mut self, handle: DomHandle) -> Result<Option<RendererInputDispatchOutcome>> {
        self.vm_mut().submit_control_form(handle)
    }
    fn select_values(
        &mut self,
        handle: DomHandle,
        values: &[String],
        multiple: bool,
    ) -> Result<bool> {
        self.vm_mut()
            .select_control_values(handle, values, multiple)
    }
    fn screenshot_clip(&mut self, handle: DomHandle) -> Result<RendererScreenshotClip> {
        let viewport = self.inspection_viewport();
        self.vm_mut().node_screenshot_clip(handle, viewport)
    }
    fn capture(
        &mut self,
        request: RendererCaptureScreenshotRequest,
    ) -> Result<RendererCaptureScreenshotReply> {
        self.capture_screenshot(request)
    }
    fn flush_screenshot(&mut self) -> Result<()> {
        self.flush_page_action_window(moli_action_window::ActionBarrier::Screenshot)
            .map(|_| ())
    }
}
