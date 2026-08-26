use moli_cookie_jar::StoredCookie;
use tokio::sync::mpsc;

use crate::{cdp_frontend::CdpFrontendControlRequest, cdp_frontend_router::CdpFrontendRouter};

use self::target_control::CdpFrontendTargetControl;
use super::CdpScheduler;

mod target_control;

#[derive(Debug, Clone, Default)]
pub(crate) struct CdpCookieSnapshot {
    profile_backed_cookies: Option<Vec<StoredCookie>>,
}

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
    pub(crate) checkpoint_tx: mpsc::UnboundedSender<CdpCookieSnapshot>,
}

#[derive(Default)]
pub(super) struct CdpFrontendControlState {
    next_frontend_id: u64,
    target_control: CdpFrontendTargetControl,
}

impl CdpFrontendControlState {
    pub(super) async fn handle_request(
        &mut self,
        request: CdpFrontendControlRequest,
        frontend_router: &CdpFrontendRouter,
        scheduler: &mut CdpScheduler,
        owner_lifecycle: Option<&CdpOwnerActorLifecycle>,
    ) -> bool {
        match request {
            CdpFrontendControlRequest::AttachBrowser {
                sink,
                completion_tx,
            } => {
                let result = match self
                    .target_control
                    .attach_browser(scheduler, frontend_router)
                    .await
                {
                    Ok(session_id) => {
                        let frontend_id = self.allocate_frontend_id();
                        match frontend_router.register_browser_frontend(
                            frontend_id,
                            session_id.clone(),
                            sink,
                        ) {
                            Ok(()) => Ok(frontend_id),
                            Err(error) => {
                                self.target_control
                                    .detach_frontend_session(
                                        scheduler,
                                        frontend_router,
                                        &session_id,
                                    )
                                    .await;
                                Err(error)
                            }
                        }
                    }
                    Err(error) => Err(error),
                };
                match completion_tx.send(result) {
                    Ok(()) => {}
                    Err(Ok(frontend_id)) => {
                        if let Some(session_id) =
                            frontend_router.unregister_browser_frontend(frontend_id)
                        {
                            self.target_control
                                .detach_frontend_session(scheduler, frontend_router, &session_id)
                                .await;
                        }
                        send_cookie_checkpoint(scheduler, owner_lifecycle);
                    }
                    Err(Err(_)) => {}
                }
                true
            }
            CdpFrontendControlRequest::AttachTarget {
                target_id,
                sink,
                completion_tx,
            } => {
                let result = match self
                    .target_control
                    .attach_target(scheduler, frontend_router, &target_id)
                    .await
                {
                    Ok(session_id) => {
                        let frontend_id = self.allocate_frontend_id();
                        match frontend_router.register_target_frontend(
                            frontend_id,
                            target_id,
                            session_id.clone(),
                            sink,
                        ) {
                            Ok(()) => Ok(frontend_id),
                            Err(error) => {
                                self.target_control
                                    .detach_frontend_session(
                                        scheduler,
                                        frontend_router,
                                        &session_id,
                                    )
                                    .await;
                                Err(error)
                            }
                        }
                    }
                    Err(error) => Err(error),
                };
                match completion_tx.send(result) {
                    Ok(()) => {}
                    Err(Ok(frontend_id)) => {
                        if let Some(session_id) =
                            frontend_router.unregister_target_frontend(frontend_id)
                        {
                            self.target_control
                                .detach_frontend_session(scheduler, frontend_router, &session_id)
                                .await;
                        }
                        send_cookie_checkpoint(scheduler, owner_lifecycle);
                    }
                    Err(Err(_)) => {}
                }
                true
            }
            CdpFrontendControlRequest::DetachBrowser { frontend_id } => {
                if let Some(session_id) = frontend_router.unregister_browser_frontend(frontend_id) {
                    self.target_control
                        .detach_frontend_session(scheduler, frontend_router, &session_id)
                        .await;
                    send_cookie_checkpoint(scheduler, owner_lifecycle);
                }
                true
            }
            CdpFrontendControlRequest::DetachTarget { frontend_id } => {
                if let Some(session_id) = frontend_router.unregister_target_frontend(frontend_id) {
                    self.target_control
                        .detach_frontend_session(scheduler, frontend_router, &session_id)
                        .await;
                    send_cookie_checkpoint(scheduler, owner_lifecycle);
                }
                true
            }
            CdpFrontendControlRequest::TargetDestroyed { target_id } => {
                frontend_router.unregister_frontends_for_target(&target_id);
                true
            }
            CdpFrontendControlRequest::ActivateTarget {
                target_id,
                completion_tx,
            } => {
                let result = self
                    .target_control
                    .activate_target(scheduler, frontend_router, &target_id)
                    .await;
                let _ = completion_tx.send(result);
                true
            }
            CdpFrontendControlRequest::CloseTarget {
                target_id,
                completion_tx,
            } => {
                let result = self
                    .target_control
                    .close_target(scheduler, frontend_router, &target_id)
                    .await;
                let _ = completion_tx.send(result);
                send_cookie_checkpoint(scheduler, owner_lifecycle);
                true
            }
            CdpFrontendControlRequest::CreateManagedTarget {
                target_url,
                completion_tx,
            } => {
                let result = self
                    .target_control
                    .create_managed_target(scheduler, frontend_router, &target_url)
                    .await;
                if let Err(Ok(target)) = completion_tx.send(result) {
                    let _ = self
                        .target_control
                        .close_target(scheduler, frontend_router, &target.page_target_id)
                        .await;
                }
                send_cookie_checkpoint(scheduler, owner_lifecycle);
                true
            }
            CdpFrontendControlRequest::Shutdown => false,
        }
    }

    fn allocate_frontend_id(&mut self) -> u64 {
        self.next_frontend_id = self
            .next_frontend_id
            .checked_add(1)
            .expect("CDP frontend id space exhausted");
        self.next_frontend_id
    }
}

fn send_cookie_checkpoint(
    scheduler: &mut CdpScheduler,
    owner_lifecycle: Option<&CdpOwnerActorLifecycle>,
) {
    let Some(owner_lifecycle) = owner_lifecycle else {
        return;
    };
    let snapshot =
        CdpCookieSnapshot::from_profile_backed_cookies(scheduler.snapshot_profile_backed_cookies());
    let _ = owner_lifecycle.checkpoint_tx.send(snapshot);
}

#[cfg(test)]
mod tests {
    use super::CdpFrontendControlState;

    #[test]
    #[should_panic(expected = "CDP frontend id space exhausted")]
    fn frontend_ids_never_wrap() {
        let mut state = CdpFrontendControlState {
            next_frontend_id: u64::MAX,
            ..CdpFrontendControlState::default()
        };

        let _ = state.allocate_frontend_id();
    }
}
