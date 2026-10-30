use std::{cell::Cell, rc::Rc};

#[cfg(test)]
use moli_cookie_jar::StoredCookie;
use tokio::sync::mpsc;

use crate::{
    BrowserInstanceControl,
    cdp_frontend::{CdpControlResource, CdpFrontendControlRequest},
    cdp_frontend_router::CdpFrontendRouter,
};

use self::target_control::CdpFrontendTargetControl;

mod target_control;

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct CdpCookieSnapshot {
    profile_backed_cookies: Option<Vec<StoredCookie>>,
}

#[cfg(test)]
impl CdpCookieSnapshot {
    pub(crate) fn from_profile_backed_cookies(cookies: Option<Vec<StoredCookie>>) -> Self {
        Self {
            profile_backed_cookies: cookies,
        }
    }

    pub(crate) fn into_profile_backed_cookies(self) -> Option<Vec<StoredCookie>> {
        self.profile_backed_cookies
    }
}

pub(crate) struct CdpOwnerActorLifecycle {
    pub(crate) instance_control: BrowserInstanceControl,
}

#[derive(Clone, Default)]
pub(super) struct CdpFrontendControlState {
    next_frontend_id: Rc<Cell<u64>>,
    target_control: CdpFrontendTargetControl,
}

impl CdpFrontendControlState {
    pub(super) fn new(owner: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>) -> Self {
        Self {
            target_control: CdpFrontendTargetControl::new(owner),
            ..Self::default()
        }
    }

    pub(super) async fn handle_request(
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
