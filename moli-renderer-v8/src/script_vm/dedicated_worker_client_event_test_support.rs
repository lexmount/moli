//! DedicatedWorker setup and payload support for renderer fixtures.
//!
//! This module may create an exact producer or a structured-clone payload, but
//! it deliberately has no dequeue, authorization, body execution, or task
//! completion authority. Complete DedicatedWorker task tests must use the
//! shared Page selected-task harness and production dispatcher.

use anyhow::{Result, ensure};

use super::ScriptVm;
use crate::page_task_queue::RendererPageDedicatedWorkerClientEventProducer;

impl ScriptVm {
    pub(crate) fn dedicated_worker_message_payload_for_test(
        &mut self,
        value: &str,
    ) -> Result<crate::structured_clone::V8StructuredClonePayload> {
        self.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            let value = v8::String::new(scope, value)
                .ok_or_else(|| anyhow::anyhow!("failed to allocate Worker test message"))?;
            crate::context_bootstrap::structured_serialize_value(scope, value.into())
                .ok_or_else(|| anyhow::anyhow!("failed to serialize Worker test message"))
        })
    }

    pub(crate) fn only_dedicated_worker_client_event_producer_for_test(
        &self,
    ) -> Result<(
        crate::types::DedicatedWorkerId,
        RendererPageDedicatedWorkerClientEventProducer,
    )> {
        let host = self._context_host.borrow();
        let workers = host.worker_execution_contexts_for_test();
        ensure!(
            workers.len() == 1,
            "expected exactly one DedicatedWorker test target, found {}",
            workers.len()
        );
        let worker_id = workers[0].0;
        let producer = host
            .dedicated_worker_client_event_producer_for_test(worker_id)
            .ok_or_else(|| anyhow::anyhow!("exact Worker test producer disappeared"))?;
        Ok((worker_id, producer))
    }

    pub(crate) fn register_loading_dedicated_worker_client_event_producer_for_test(
        &mut self,
    ) -> Result<(
        crate::types::DedicatedWorkerId,
        RendererPageDedicatedWorkerClientEventProducer,
    )> {
        self.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let wrapper = v8::Object::new(scope);
            let creator_storage_key = host
                .active_storage_context(scope, None)
                .storage_key()
                .clone();
            let top_level_site = creator_storage_key.top_level_site().to_owned();
            let owner = host
                .current_runtime_window_execution_context_binding(scope)
                .expect("test Worker should capture the current top Window");
            let outside_settings_load = host
                .register_dedicated_worker_outside_settings_load(owner.dispatch_scope())
                .expect("test Worker should capture its Document script-load authority");
            let worker_id = host.register_loading_worker(
                scope,
                wrapper,
                top_level_site,
                creator_storage_key,
                host.prepare_dedicated_worker_host(
                    "data:text/html,fixture".parse().unwrap(),
                    "data:text/javascript,fixture".parse().unwrap(),
                    String::new(),
                ),
                moli_fetch::RequestCredentialsMode::SameOrigin,
                None,
                outside_settings_load,
                owner,
            );
            let producer = host
                .dedicated_worker_client_event_producer_for_test(worker_id)
                .expect("registered Worker should retain its exact client-event producer");
            Ok((worker_id, producer))
        })
    }

    pub(crate) fn dedicated_worker_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        crate::types::DedicatedWorkerId,
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::RuntimeObservableContextToken,
    )> {
        self._context_host
            .borrow()
            .worker_execution_contexts_for_test()
    }

    pub(crate) fn dedicated_worker_client_event_producer_for_test(
        &self,
        worker_id: crate::types::DedicatedWorkerId,
    ) -> Option<RendererPageDedicatedWorkerClientEventProducer> {
        self._context_host
            .borrow()
            .dedicated_worker_client_event_producer_for_test(worker_id)
    }

    pub(crate) fn forget_dedicated_worker_for_test(
        &mut self,
        worker_id: crate::types::DedicatedWorkerId,
    ) {
        self._context_host.borrow_mut().forget_worker(worker_id);
    }

    pub(crate) fn only_child_document_owner_for_dedicated_worker_test(
        &self,
        context: &str,
    ) -> Result<crate::frame_owner_model::FrameDocumentTaskOwner> {
        let host = self._context_host.borrow();
        let child_handles = host.child_browsing_context_handles_in_document_order();
        ensure!(
            child_handles.len() == 1,
            "{context}: expected exactly one child browsing context, found {}",
            child_handles.len()
        );
        host.current_child_document_task_owner(child_handles[0])
            .ok_or_else(|| anyhow::anyhow!("{context}: expected one current child Document owner"))
    }
}
