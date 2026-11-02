use super::{CdpFrontendControlRequest, CdpFrontendEndpoint};
use moli_core::ace_page_task::AcePageTask;
use moli_protocol::ace_page_task::{AcePageTaskError, AcePageTaskResult};
use parking_lot::Mutex;
use tokio::sync::oneshot;

pub(crate) struct AcePageTaskRequest {
    target_id: String,
    // Existing control send errors enter anyhow, which requires a Sync
    // envelope. The task itself only needs Send and is taken without locking.
    task: Mutex<Box<dyn AcePageTask>>,
    reply: oneshot::Sender<Result<AcePageTaskResult, AcePageTaskError>>,
}

impl AcePageTaskRequest {
    pub(crate) fn into_parts(
        self,
    ) -> (
        String,
        Box<dyn AcePageTask>,
        oneshot::Sender<Result<AcePageTaskResult, AcePageTaskError>>,
    ) {
        (self.target_id, self.task.into_inner(), self.reply)
    }
}

impl CdpFrontendEndpoint {
    pub(crate) async fn run_ace_page_task(
        &self,
        target_id: String,
        task: Box<dyn AcePageTask>,
    ) -> Result<AcePageTaskResult, AcePageTaskError> {
        if self.is_shutting_down() {
            return Err(AcePageTaskError::new(404, "Browser owner has stopped"));
        }
        let (reply, received) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::AcePageTask(AcePageTaskRequest {
                target_id,
                task: Mutex::new(task),
                reply,
            }))
            .map_err(|_| AcePageTaskError::new(404, "Browser owner is no longer available"))?;
        // Losing a caller never revokes work already admitted by the owner.
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => Err(AcePageTaskError::new(404, "Browser owner has stopped")),
            result = received => result.map_err(|_| AcePageTaskError::new(404, "Browser owner stopped during task"))?,
        }
    }
}
