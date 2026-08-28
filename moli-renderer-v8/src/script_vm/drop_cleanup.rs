use super::ScriptVm;

impl Drop for ScriptVm {
    fn drop(&mut self) {
        self.cancel_page_context(crate::runtime::RendererPageContextCancelReason::ContextDropped);
        self.close_page_context_resources_for_context_teardown();
        crate::blob::cleanup_owner_resources(self.resource_owner_id);
        self._context_host
            .borrow_mut()
            .child_document_script_schedulers_mut()
            .clear();
        self.child_document_modulator_store.clear();

        self.renderer_document_isolate_teardown
            .unregister_platform_on_context_teardown(&self.renderer_document_isolate);

        self.page_inspector.destroy_all_context_registrations();
        self.page_inspector
            .deactivate_page_vm_binding_for_teardown();

        self.page_default_bridge_ref.take();
        if let Some(document_runtime) = self.document_runtime.take_for_retained_document_host() {
            self._context_host
                .borrow_mut()
                .adopt_retained_document_runtime(document_runtime);
        }
    }
}
