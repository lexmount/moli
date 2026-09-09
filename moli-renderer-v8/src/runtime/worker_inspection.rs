use moli_shared_worker::SharedWorkerInstanceId;

use super::{
    CompletedWorkerRuntimeInspectorCommandDispatch, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorResponseSender, RendererServiceWorkerRunIdentity,
    RendererTurnOutputJournal,
};
use crate::worker::WorkerDevToolsHandle;

/// Renderer identity used only to bind an inspection endpoint. A stable
/// ServiceWorker version is insufficient: inspection must name its exact run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererWorkerInspectionTarget {
    Shared(SharedWorkerInstanceId),
    Dedicated(u64),
    Service {
        version_id: u64,
        run: RendererServiceWorkerRunIdentity,
    },
}

impl RendererWorkerInspectionTarget {
    pub fn unavailable_message(&self) -> &'static str {
        match self {
            Self::Shared(_) => "SharedWorkerRuntimeUnavailable",
            Self::Dedicated(_) => "DedicatedWorkerRuntimeUnavailable",
            Self::Service { .. } => "ServiceWorkerRuntimeUnavailable",
        }
    }
}

/// Inspection-only capability for one concrete worker. It retains neither the
/// Context registry nor a Worker owner/JoinHandle. Retirement disposes the
/// underlying task runner; delayed dispatch cannot look up a replacement run.
#[derive(Clone, Debug)]
pub struct RendererWorkerInspectionEndpoint {
    handle: WorkerDevToolsHandle,
    output_journal: Option<RendererTurnOutputJournal>,
    unavailable_message: &'static str,
}

impl RendererWorkerInspectionEndpoint {
    pub(crate) fn new(
        handle: WorkerDevToolsHandle,
        output_journal: Option<RendererTurnOutputJournal>,
        unavailable_message: &'static str,
    ) -> Self {
        Self {
            handle,
            output_journal,
            unavailable_message,
        }
    }

    pub fn attach_session(&self, session_id: Option<String>) -> bool {
        self.handle.attach_runtime_inspector_session(session_id)
    }

    pub fn detach_session(&self, session_id: Option<String>) -> bool {
        self.handle.detach_runtime_inspector_session(session_id)
    }

    pub async fn dispatch_protocol_message(
        &self,
        session_id: Option<String>,
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        if !self.handle.dispatch_runtime_protocol_message(
            session_id,
            raw_json,
            response,
            response_tx,
        ) {
            return Err(self.unavailable_message.to_owned());
        }
        response_rx
            .await
            .map_err(|_| self.unavailable_message.to_owned())?
    }

    pub fn dispatch_protocol_message_to_session(
        self,
        session_id: String,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<
        impl std::future::Future<Output = CompletedWorkerRuntimeInspectorCommandDispatch>,
        String,
    > {
        let output_journal = self
            .output_journal
            .clone()
            .ok_or_else(|| self.unavailable_message.to_owned())?;
        let response =
            response.route_to_worker_devtools_session_output(session_id.clone(), output_journal);
        let error_response = response.clone();
        let settlement = response
            .take_session_response_settlement_receiver()
            .ok_or_else(|| self.unavailable_message.to_owned())?;
        // Bind the output stream and take its single settlement receiver before
        // yielding. Retirement may settle this response before dispatch runs.
        Ok(async move {
            let dispatch = self
                .dispatch_protocol_message(Some(session_id), raw_json, Some(response))
                .await;
            CompletedWorkerRuntimeInspectorCommandDispatch::finish(
                dispatch,
                settlement,
                error_response,
            )
        })
    }
}
