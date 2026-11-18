use anyhow::Result;
use tokio::sync::oneshot;

use super::{WorkerControlHandle, WorkerHandle};
use crate::runtime::{RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender};

impl WorkerControlHandle {
    pub(crate) fn dispatch_runtime_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>>>,
    ) -> bool {
        self.inspector_tasks.append_protocol_message(
            inspector_session_id,
            raw_json,
            deferred_response,
            response_tx,
        )
    }

    pub(crate) fn attach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.inspector_tasks.append_attach(inspector_session_id)
    }

    pub(crate) fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> tokio::sync::oneshot::Receiver<anyhow::Result<()>> {
        self.inspector_tasks.append_detach(inspector_session_id)
    }

    pub(crate) fn run_if_waiting_for_debugger(&self) -> bool {
        self.inspector_tasks.append_run_if_waiting_for_debugger()
    }
}

impl WorkerHandle {
    pub(crate) fn dispatch_runtime_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>>>,
    ) -> bool {
        self.control.dispatch_runtime_protocol_message(
            inspector_session_id,
            raw_json,
            deferred_response,
            response_tx,
        )
    }

    pub(crate) fn attach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.control
            .attach_runtime_inspector_session(inspector_session_id)
    }

    pub(crate) fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> tokio::sync::oneshot::Receiver<anyhow::Result<()>> {
        self.control
            .detach_runtime_inspector_session(inspector_session_id)
    }

    pub(crate) fn run_if_waiting_for_debugger_for_devtools(&self) -> bool {
        self.control.run_if_waiting_for_debugger()
    }
}
