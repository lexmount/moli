use std::{cell::Cell, rc::Rc};

use tokio::sync::{mpsc, watch};

use crate::{
    BrowserInstanceControl,
    cdp_frontend::{CdpControlResource, CdpFrontendControlRequest},
    cdp_frontend_router::CdpFrontendRouter,
};

use self::target_control::CdpFrontendTargetControl;

mod target_control;

pub(crate) struct CdpOwnerActorLifecycle {
    pub(crate) instance_control: BrowserInstanceControl,
}

#[derive(Clone)]
pub(super) struct CdpFrontendControlState {
    next_frontend_id: Rc<Cell<u64>>,
    target_control: CdpFrontendTargetControl,
    pending_frontend_detaches: watch::Sender<usize>,
}

impl Default for CdpFrontendControlState {
    fn default() -> Self {
        Self {
            next_frontend_id: Rc::default(),
            target_control: CdpFrontendTargetControl::default(),
            pending_frontend_detaches: watch::channel(0).0,
        }
    }
}

struct PendingFrontendDetach(watch::Sender<usize>);

impl Drop for PendingFrontendDetach {
    fn drop(&mut self) {
        self.0.send_modify(|pending| {
            *pending = pending
                .checked_sub(1)
                .expect("balanced frontend detach work");
        });
    }
}

impl CdpFrontendControlState {
    pub(super) fn new(owner: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>) -> Self {
        Self {
            target_control: CdpFrontendTargetControl::new(owner),
            ..Self::default()
        }
    }

    // Register cleanup before the actor spawns the future, so a later attach
    // cannot overtake disposal of the previous frontend's session contexts.
    pub(super) fn handle_request(
        self,
        request: CdpFrontendControlRequest,
        frontend_router: CdpFrontendRouter,
    ) -> impl std::future::Future<Output = ()> {
        let pending_detach = matches!(
            &request,
            CdpFrontendControlRequest::DetachBrowser { .. }
                | CdpFrontendControlRequest::DetachTarget { .. }
        )
        .then(|| {
            self.pending_frontend_detaches.send_modify(|pending| {
                *pending = pending
                    .checked_add(1)
                    .expect("frontend detach work overflow");
            });
            PendingFrontendDetach(self.pending_frontend_detaches.clone())
        });
        let pending_detaches = matches!(
            &request,
            CdpFrontendControlRequest::AttachBrowser { .. }
                | CdpFrontendControlRequest::AttachTarget { .. }
        )
        .then(|| self.pending_frontend_detaches.subscribe());
        async move {
            if let Some(mut pending) = pending_detaches {
                let _ = pending.wait_for(|count| *count == 0).await;
            }
            self.handle_request_inner(request, frontend_router).await;
            drop(pending_detach);
        }
    }

    async fn handle_request_inner(
        self,
        request: CdpFrontendControlRequest,
        frontend_router: CdpFrontendRouter,
    ) {
        match request {
            CdpFrontendControlRequest::DisposeOrphanedResource(resource) => {
                match resource {
                    CdpControlResource::BrowserFrontend {
                        session_id,
                        frontend_id,
                    } => {
                        frontend_router.unregister_browser_frontend(frontend_id);
                        self.target_control
                            .detach_frontend_session(&session_id)
                            .await;
                    }
                    CdpControlResource::TargetFrontend {
                        session_id,
                        frontend_id,
                    } => {
                        frontend_router.unregister_target_frontend(frontend_id);
                        self.target_control
                            .detach_frontend_session(&session_id)
                            .await;
                    }
                    CdpControlResource::Session(session) => {
                        self.target_control.detach_frontend_session(&session).await
                    }
                    CdpControlResource::Target(target) => {
                        if let Err(error) = self.target_control.close_target(&target).await {
                            tracing::debug!(target_id = target, %error, "failed to close unclaimed frontend target");
                        }
                    }
                }
                self.target_control.checkpoint();
            }
            CdpFrontendControlRequest::AttachBrowser {
                sink,
                completion_tx,
            } => {
                let frontend_id = self.allocate_frontend_id();
                let result = self
                    .target_control
                    .attach_browser_frontend(frontend_id, sink)
                    .await;
                let _ = completion_tx.send(result);
            }
            CdpFrontendControlRequest::AttachTarget {
                target_id,
                sink,
                completion_tx,
            } => {
                let frontend_id = self.allocate_frontend_id();
                let result = self
                    .target_control
                    .attach_target(&target_id, frontend_id, sink)
                    .await;
                let _ = completion_tx.send(result);
            }
            CdpFrontendControlRequest::DetachBrowser { frontend_id } => {
                if let Some(session_id) = frontend_router.unregister_browser_frontend(frontend_id) {
                    self.target_control
                        .detach_frontend_session(&session_id)
                        .await;
                    self.target_control.checkpoint();
                }
            }
            CdpFrontendControlRequest::DetachTarget { frontend_id } => {
                if let Some(session_id) = frontend_router.unregister_target_frontend(frontend_id) {
                    self.target_control
                        .detach_frontend_session(&session_id)
                        .await;
                    self.target_control.checkpoint();
                }
            }
            CdpFrontendControlRequest::ActivateTarget {
                target_id,
                completion_tx,
            } => {
                let result = self.target_control.activate_target(&target_id).await;
                let _ = completion_tx.send(result);
            }
            CdpFrontendControlRequest::CloseTarget {
                target_id,
                completion_tx,
            } => {
                let result = self.target_control.close_target(&target_id).await;
                let _ = completion_tx.send(result);
                self.target_control.checkpoint();
            }
            CdpFrontendControlRequest::CreateManagedTarget {
                target_url,
                completion_tx,
            } => {
                let result = self.target_control.create_managed_target(&target_url).await;
                let _ = completion_tx.send(result);
                self.target_control.checkpoint();
            }
            CdpFrontendControlRequest::TargetDestroyed { .. }
            | CdpFrontendControlRequest::EnsureDefaultTargetPublished { .. }
            | CdpFrontendControlRequest::ExecuteInternalCommand { .. }
            | CdpFrontendControlRequest::Checkpoint
            | CdpFrontendControlRequest::Shutdown => {
                unreachable!("finite control requests run in the actor")
            }
        }
    }

    fn allocate_frontend_id(&self) -> u64 {
        let next = self
            .next_frontend_id
            .get()
            .checked_add(1)
            .expect("CDP frontend id space exhausted");
        self.next_frontend_id.set(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::{CdpFrontendControlState, Cell, Rc};

    #[test]
    #[should_panic(expected = "CDP frontend id space exhausted")]
    fn frontend_ids_never_wrap() {
        let state = CdpFrontendControlState {
            next_frontend_id: Rc::new(Cell::new(u64::MAX)),
            ..CdpFrontendControlState::default()
        };

        let _ = state.allocate_frontend_id();
    }
}
