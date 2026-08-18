use tokio::sync::oneshot;

use super::host::RendererSharedWorkerHost;
use crate::runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender};

impl RendererSharedWorkerHost {
    pub(super) async fn dispatch_runtime_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.dispatch_runtime_protocol_message_with_optional_deferred_response(
            inspector_session_id,
            raw_json,
            None,
        )
        .await
    }

    pub(super) async fn dispatch_runtime_protocol_message_with_deferred_response(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.dispatch_runtime_protocol_message_with_optional_deferred_response(
            inspector_session_id,
            raw_json,
            Some(deferred_response),
        )
        .await
    }

    async fn dispatch_runtime_protocol_message_with_optional_deferred_response(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let Some(handle) = self.running_devtools_handle() else {
            return Err("SharedWorkerRuntimeUnavailable".to_owned());
        };
        let (response_tx, response_rx) = oneshot::channel();
        if !handle.dispatch_runtime_protocol_message(
            inspector_session_id,
            raw_json,
            deferred_response,
            response_tx,
        ) {
            return Err("SharedWorkerRuntimeUnavailable".to_owned());
        }
        response_rx
            .await
            .map_err(|_| "SharedWorkerRuntimeUnavailable".to_owned())?
    }

    pub(super) fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.running_devtools_handle()
            .is_some_and(|handle| handle.detach_runtime_inspector_session(inspector_session_id))
    }
}
