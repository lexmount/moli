use super::PageScriptHost;
use crate::{
    document_runtime::DomHandle, document_script_scheduler::FrameDocumentReadyActionRoute,
    frame_owner_model::FrameDocumentClassicScriptSourceLoadTask,
    page_task_queue::RendererPageChildClassicScriptSourceLoadTarget,
};

impl PageScriptHost {
    pub(in crate::native_bridge::context_host) fn queue_child_classic_script_source_load_task(
        &mut self,
        handle: DomHandle,
    ) -> bool {
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
        else {
            return false;
        };
        let Some(client) =
            self.child_classic_script_source_load_client_for_owner(handle, owner.document_owner())
        else {
            return false;
        };
        let Some(realm_id) = self.frame_owner_store.ensure_child_realm(handle) else {
            let _ = self.fail_child_classic_source_load_client_before_start(
                handle,
                owner,
                &client,
                "child classic source-load realm reservation failed before fetch start",
            );
            return false;
        };
        if self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner)
            != Some(realm_id)
        {
            let _ = self.fail_child_classic_source_load_client_before_start(
                handle,
                owner,
                &client,
                "child classic source-load reserved another Document realm",
            );
            return false;
        }
        let script_handle = client.metadata().script_handle();
        let task = FrameDocumentClassicScriptSourceLoadTask::from_source_load_client(
            owner,
            realm_id,
            client.clone(),
        );
        let target = RendererPageChildClassicScriptSourceLoadTarget::new(
            handle,
            owner,
            realm_id,
            script_handle,
        );
        if let Err(error) = self
            .page_child_frame_task_sender()
            .send_classic_script_source_load(target, task)
        {
            let task = error.into_task();
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child classic source-load Page route closed before fetch start",
            );
            return false;
        }
        // Fetch preparation does not require an executable V8 context. Keep
        // Chromium's causal order: the exact parser source-start is visible
        // before the realm task that will later authorize script execution.
        // Both tasks share one stable family FIFO, so completion may arrive
        // early but cannot execute the script before that realm is materialized.
        if self
            .request_child_frame_realm_materialization_for_owner(handle, owner)
            .is_none()
        {
            let _ = self.fail_child_classic_source_load_client_before_start(
                handle,
                owner,
                &client,
                "child classic source-load realm admission failed after fetch-start admission",
            );
            return false;
        }
        true
    }

    pub(crate) fn cancel_child_classic_document_script_work(&mut self, handle: DomHandle) {
        let canceled_load_ids = self
            .pending_child_external_classic_document_scripts
            .iter()
            .filter_map(|(load_id, pending)| (pending.child_handle == handle).then_some(*load_id))
            .collect::<Vec<_>>();
        for load_id in &canceled_load_ids {
            let pending = self
                .pending_child_external_classic_document_scripts
                .remove(load_id)
                .expect("collected child classic load should remain pending until cancellation");
            let _ = self
                .frame_owner_store
                .finish_document_request(pending.owner_document_id, pending.owner_request_id);
        }
    }

    pub(crate) fn child_classic_document_script_ready_runner_owner_is_current(
        &self,
        route: &FrameDocumentReadyActionRoute,
    ) -> bool {
        let Some(child_handle) = route.child_handle() else {
            return false;
        };
        self.child_browsing_contexts.contains_key(&child_handle)
            && self
                .frame_parser_classic_scripts
                .has_runner(route.document_owner())
    }
}
