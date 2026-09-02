use crate::runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender};

use super::super::ids::ServiceWorkerVersionId;
use super::ServiceWorkerRuntimeService;

impl ServiceWorkerRuntimeService {
    pub(crate) async fn dispatch_runtime_protocol_message(
        &self,
        version_id: ServiceWorkerVersionId,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(host) = self.running_host_for_version(version_id) else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        host.dispatch_worker_runtime_protocol_message_without_deferred_response(
            inspector_session_id,
            raw_json,
        )
        .await
    }

    pub(crate) async fn dispatch_runtime_protocol_message_with_deferred_response(
        &self,
        version_id: ServiceWorkerVersionId,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(host) = self.running_host_for_version(version_id) else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        host.dispatch_worker_runtime_protocol_message_with_deferred_response(
            inspector_session_id,
            raw_json,
            deferred_response,
        )
        .await
    }

    pub(crate) async fn dispatch_runtime_protocol_message_with_devtools_session_response(
        &self,
        version_id: ServiceWorkerVersionId,
        inspector_session_id: String,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<crate::runtime::CompletedWorkerRuntimeInspectorCommandDispatch, String> {
        let Some(host) = self.running_host_for_version(version_id) else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        let Some(output_journal) = self.target_output_journal(version_id) else {
            return Err("ServiceWorkerRuntimeUnavailable".to_owned());
        };
        let response = response
            .route_to_worker_devtools_session_output(inspector_session_id.clone(), output_journal);
        let error_response = response.clone();
        let settlement = response
            .take_session_response_settlement_receiver()
            .expect("a Worker DevTools response must own one settlement receiver");
        let dispatch = host
            .dispatch_worker_runtime_protocol_message_with_deferred_response(
                Some(inspector_session_id),
                raw_json,
                response,
            )
            .await;
        Ok(
            crate::runtime::CompletedWorkerRuntimeInspectorCommandDispatch::finish(
                dispatch,
                settlement,
                error_response,
            ),
        )
    }

    pub(crate) fn detach_runtime_inspector_session(
        &self,
        version_id: ServiceWorkerVersionId,
        inspector_session_id: Option<String>,
    ) -> bool {
        let Some(host) = self.running_host_for_version(version_id) else {
            return false;
        };
        host.detach_worker_runtime_inspector_session(inspector_session_id)
    }
}
