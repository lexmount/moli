use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn install_page_task_capabilities(
        &mut self,
        capabilities: crate::native_bridge::JsContextHostPageTaskCapabilities,
    ) {
        self._context_host
            .borrow_mut()
            .install_page_task_capabilities(capabilities);
    }

    /// Bind a low-level ScriptVm fixture to the same complete producer route
    /// set used by PageVm. The retained source owns the unique test consumer;
    /// no fallback transport or optional capability is introduced.
    #[cfg(test)]
    pub(crate) fn install_page_task_residence_for_executor_test(
        &mut self,
        residence: crate::page_task_queue::RendererPageTaskTestResidence,
    ) {
        let source = residence.runtime_source();
        let root_document = residence.root_document();
        let senders = source
            .bound_task_producer_senders(root_document)
            .expect("ScriptVm executor fixture must bind a complete production route set");
        let (page_task_capabilities, _, _, _, _, _) = senders.into_parts();
        self.install_page_task_capabilities(page_task_capabilities);
        assert!(
            self._page_task_residence_for_executor_test
                .replace(residence)
                .is_none(),
            "ScriptVm executor fixture installed Page task routes twice"
        );
    }

    #[cfg(test)]
    pub(crate) async fn wait_for_page_task_executor_work_arrival_for_test(&self) -> bool {
        let Some(residence) = self._page_task_residence_for_executor_test.as_ref() else {
            return false;
        };
        residence.wait_for_owner_task_arrival().await
    }
}
