//! Redirect continuation belongs to the requesting script owner. The resource
//! executor waits asynchronously, so one busy owner cannot block other I/O.

use moli_fetch::FetchCancelHandle;
use tokio::sync::oneshot;

use crate::page_task_queue::RendererResourceCompletionSender;
use crate::types::AsyncSubresourceFetchEvent;

#[derive(Debug)]
pub(crate) struct RedirectAdmission(oneshot::Sender<()>);

impl RedirectAdmission {
    pub(crate) fn grant(self) {
        let _ = self.0.send(());
    }
}

#[derive(Clone)]
pub(crate) enum RedirectOwner {
    Window {
        completion_tx: RendererResourceCompletionSender,
        internal_id: u64,
    },
    Worker(crate::worker::WorkerRedirectOwner),
}

impl RedirectOwner {
    pub(crate) fn window(
        completion_tx: RendererResourceCompletionSender,
        internal_id: u64,
    ) -> Self {
        Self::Window {
            completion_tx,
            internal_id,
        }
    }

    fn detached_keepalive(&self) -> bool {
        match self {
            Self::Window { .. } => false,
            Self::Worker(owner) => owner.detached_keepalive(),
        }
    }

    pub(crate) async fn admit(&self, cancel: &FetchCancelHandle) -> Result<(), String> {
        if cancel.is_cancelled() {
            return Err(super::ABORTED_ERROR_TEXT.into());
        }
        let (reply, admitted) = oneshot::channel();
        let admission = RedirectAdmission(reply);
        let posted = match self {
            Self::Window {
                completion_tx,
                internal_id,
            } => completion_tx
                .send_async_subresource_event(AsyncSubresourceFetchEvent::RedirectAdmission {
                    internal_id: *internal_id,
                    admission,
                })
                .is_ok(),
            Self::Worker(owner) => owner.post(admission),
        };
        if !posted {
            return if self.detached_keepalive() {
                Ok(())
            } else {
                Err("request owner retired before redirect admission".into())
            };
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(super::ABORTED_ERROR_TEXT.into()),
            result = admitted => {
                if result.is_ok() || self.detached_keepalive() {
                    Ok(())
                } else {
                    Err("request owner retired before redirect admission".into())
                }
            }
        }
    }
}
