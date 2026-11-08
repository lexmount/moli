use anyhow::{Context, Result, bail};
use moli_protocol_cdp::{CdpCommandParseError, ParsedCdpCommand};
use tokio::sync::{mpsc, oneshot};

use crate::cdp_writer::CdpSocketSink;

const FRONTEND_COMMAND_QUEUE_CAPACITY: usize = 256;

pub(crate) struct CdpFrontendReceivers {
    pub(crate) control_tx: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>,
    pub(crate) control_rx: mpsc::UnboundedReceiver<CdpFrontendControlRequest>,
    pub(crate) command_rx: mpsc::Receiver<CdpFrontendCommand>,
}

#[derive(Clone)]
pub(crate) struct CdpFrontendEndpoint {
    control_tx: mpsc::UnboundedSender<CdpFrontendControlRequest>,
    command_tx: mpsc::Sender<CdpFrontendCommand>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

pub(crate) enum CdpFrontendControlRequest {
    DisposeOrphanedResource(CdpControlResource),
    ExecuteInternalCommand {
        command: Box<ParsedCdpCommand>,
        registration: Option<CdpFrontendRegistration>,
        completion_tx: oneshot::Sender<Result<CdpInternalCommandResult>>,
    },
    Checkpoint,
    AttachBrowser {
        sink: CdpSocketSink,
        completion_tx: oneshot::Sender<Result<CdpAttachedFrontend>>,
    },
    AttachTarget {
        target_id: String,
        sink: CdpSocketSink,
        completion_tx: oneshot::Sender<Result<CdpAttachedFrontend>>,
    },
    DetachBrowser {
        frontend_id: u64,
    },
    DetachTarget {
        frontend_id: u64,
    },
    TargetDestroyed {
        target_id: String,
    },
    ActivateTarget {
        target_id: String,
        completion_tx: oneshot::Sender<Result<()>>,
    },
    CloseTarget {
        target_id: String,
        completion_tx: oneshot::Sender<Result<()>>,
    },
    CreateManagedTarget {
        target_url: String,
        completion_tx: oneshot::Sender<Result<CdpCreatedTargetResult>>,
    },
    EnsureDefaultTargetPublished {
        completion_tx: oneshot::Sender<Result<()>>,
    },
    Shutdown,
}

pub(crate) struct CdpFrontendCommand {
    pub(crate) frontend_id: u64,
    pub(crate) command: Result<ParsedCdpCommand, CdpCommandParseError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdpCreatedTarget {
    pub(crate) page_target_id: String,
    pub(crate) tab_target_id: String,
}

pub(crate) struct CdpInternalCommandResult {
    pub(crate) response: serde_json::Value,
    pub(crate) created_target: Option<CdpCreatedTarget>,
    pub(crate) registration: Option<Result<()>>,
    pub(crate) rollback: Option<CdpControlResourceLease>,
}

pub(crate) struct CdpAttachedFrontend {
    pub(crate) frontend_id: u64,
    pub(crate) rollback: CdpControlResourceLease,
}

impl CdpAttachedFrontend {
    fn into_id(mut self) -> u64 {
        self.rollback.resource = None;
        self.frontend_id
    }
}

pub(crate) struct CdpCreatedTargetResult {
    pub(crate) target: CdpCreatedTarget,
    pub(crate) rollback: CdpControlResourceLease,
}

impl CdpCreatedTargetResult {
    fn into_target(mut self) -> CdpCreatedTarget {
        self.rollback.resource = None;
        self.target
    }
}

impl CdpInternalCommandResult {
    pub(crate) fn commit_resource(&mut self) {
        if let Some(lease) = &mut self.rollback {
            lease.resource = None;
        }
    }
}

pub(crate) enum CdpControlResource {
    Session(String),
    BrowserFrontend {
        session_id: String,
        frontend_id: u64,
    },
    TargetFrontend {
        session_id: String,
        frontend_id: u64,
    },
    Target(String),
}

pub(crate) struct CdpControlResourceLease {
    owner: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>,
    resource: Option<CdpControlResource>,
}

impl CdpControlResourceLease {
    pub(crate) fn new(
        owner: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>,
        resource: CdpControlResource,
    ) -> Self {
        Self {
            owner,
            resource: Some(resource),
        }
    }
}

impl Drop for CdpControlResourceLease {
    fn drop(&mut self) {
        if let Some(resource) = self.resource.take()
            && let Some(owner) = self.owner.upgrade()
        {
            let _ = owner.send(CdpFrontendControlRequest::DisposeOrphanedResource(resource));
        }
    }
}

pub(crate) enum CdpFrontendRegistration {
    Browser {
        frontend_id: u64,
        sink: CdpSocketSink,
    },
    Target {
        frontend_id: u64,
        target_id: String,
        sink: CdpSocketSink,
    },
}

pub(crate) fn cdp_frontend_channel() -> (CdpFrontendEndpoint, CdpFrontendReceivers) {
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (command_tx, command_rx) = mpsc::channel(FRONTEND_COMMAND_QUEUE_CAPACITY);
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    (
        CdpFrontendEndpoint {
            control_tx: control_tx.clone(),
            command_tx,
            shutdown_tx,
        },
        CdpFrontendReceivers {
            control_tx: control_tx.downgrade(),
            control_rx,
            command_rx,
        },
    )
}

impl CdpFrontendEndpoint {
    pub(crate) async fn attach_browser(&self, sink: CdpSocketSink) -> Result<u64> {
        if self.is_shutting_down() {
            bail!("CDP owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::AttachBrowser {
                sink,
                completion_tx,
            })
            .context("CDP owner is no longer available")?;
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => bail!("CDP owner stopped before browser frontend attach"),
            completion = completion_rx => completion
                .context("CDP owner stopped before browser frontend attach")?.map(CdpAttachedFrontend::into_id),
        }
    }

    pub(crate) async fn attach_target(
        &self,
        target_id: String,
        sink: CdpSocketSink,
    ) -> Result<u64> {
        if self.is_shutting_down() {
            bail!("CDP target owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::AttachTarget {
                target_id,
                sink,
                completion_tx,
            })
            .context("CDP target owner is no longer available")?;
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => bail!("CDP target owner stopped before target frontend attach"),
            completion = completion_rx => completion
                .context("CDP target owner stopped before target frontend attach")?.map(CdpAttachedFrontend::into_id),
        }
    }

    pub(crate) async fn command(
        &self,
        frontend_id: u64,
        command: Result<ParsedCdpCommand, CdpCommandParseError>,
    ) -> bool {
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => false,
            result = self.command_tx.send(CdpFrontendCommand { frontend_id, command }) => result.is_ok(),
        }
    }

    pub(crate) fn detach_browser(&self, frontend_id: u64) {
        let _ = self
            .control_tx
            .send(CdpFrontendControlRequest::DetachBrowser { frontend_id });
    }

    pub(crate) fn detach_target(&self, frontend_id: u64) {
        let _ = self
            .control_tx
            .send(CdpFrontendControlRequest::DetachTarget { frontend_id });
    }

    pub(crate) fn target_destroyed(&self, target_id: String) {
        let _ = self
            .control_tx
            .send(CdpFrontendControlRequest::TargetDestroyed { target_id });
    }

    pub(crate) async fn activate_target(&self, target_id: String) -> Result<()> {
        if self.is_shutting_down() {
            bail!("CDP target owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::ActivateTarget {
                target_id,
                completion_tx,
            })
            .context("CDP target owner is no longer available")?;
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => bail!("CDP target owner stopped before target activation"),
            completion = completion_rx => completion
                .context("CDP target owner stopped before target activation")?,
        }
    }

    pub(crate) async fn close_target(&self, target_id: String) -> Result<()> {
        if self.is_shutting_down() {
            bail!("CDP target owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::CloseTarget {
                target_id,
                completion_tx,
            })
            .context("CDP target owner is no longer available")?;
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => bail!("CDP target owner stopped before target close"),
            completion = completion_rx => completion
                .context("CDP target owner stopped before target close")?,
        }
    }

    pub(crate) async fn create_managed_target(
        &self,
        target_url: String,
    ) -> Result<CdpCreatedTarget> {
        if self.is_shutting_down() {
            bail!("CDP target owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::CreateManagedTarget {
                target_url,
                completion_tx,
            })
            .context("CDP target owner is no longer available")?;
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => bail!("CDP target owner stopped before target creation"),
            completion = completion_rx => completion
                .context("CDP target owner stopped before target creation")?.map(CdpCreatedTargetResult::into_target),
        }
    }

    pub(crate) async fn ensure_default_target_published(&self) -> Result<()> {
        if self.is_shutting_down() {
            bail!("CDP target owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::EnsureDefaultTargetPublished { completion_tx })
            .context("CDP owner is no longer available")?;
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => bail!("CDP owner stopped before default target creation"),
            completion = completion_rx => completion
                .context("CDP owner stopped before default target creation")?,
        }
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.control_tx.send(CdpFrontendControlRequest::Shutdown);
    }

    pub(crate) async fn wait_for_shutdown(&self) {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        if *shutdown_rx.borrow() {
            return;
        }
        let _ = shutdown_rx.changed().await;
    }

    fn is_shutting_down(&self) -> bool {
        *self.shutdown_tx.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_a_ready_internal_result_reclaims_its_original_resource() {
        for target in [false, true] {
            let (owner, mut requests) = mpsc::unbounded_channel();
            let resource = if target {
                CdpControlResource::Target("TID-orphan".into())
            } else {
                CdpControlResource::Session("SID-orphan".into())
            };
            let result = CdpInternalCommandResult {
                response: serde_json::json!({"id": 1, "result": {}}),
                created_target: None,
                registration: None,
                rollback: Some(CdpControlResourceLease::new(owner.downgrade(), resource)),
            };
            let (reply, receive) = oneshot::channel::<Result<CdpInternalCommandResult>>();
            assert!(reply.send(Ok(result)).is_ok());
            drop(receive);
            match requests
                .try_recv()
                .expect("a result dropped before consumption must reclaim ownership")
            {
                CdpFrontendControlRequest::DisposeOrphanedResource(CdpControlResource::Target(
                    id,
                )) => {
                    assert!(target);
                    assert_eq!(id, "TID-orphan");
                }
                CdpFrontendControlRequest::DisposeOrphanedResource(
                    CdpControlResource::Session(id),
                ) => {
                    assert!(!target);
                    assert_eq!(id, "SID-orphan");
                }
                _ => panic!("cleanup must address the original resource"),
            }
            assert!(
                requests.try_recv().is_err(),
                "one resource has one cleanup obligation"
            );
        }
    }

    #[test]
    fn unconsumed_frontend_and_target_callbacks_keep_their_cleanup_obligations() {
        let (owner, mut requests) = mpsc::unbounded_channel();
        let (reply, receive) = oneshot::channel::<Result<CdpAttachedFrontend>>();
        assert!(
            reply
                .send(Ok(CdpAttachedFrontend {
                    frontend_id: 7,
                    rollback: CdpControlResourceLease::new(
                        owner.downgrade(),
                        CdpControlResource::BrowserFrontend {
                            session_id: "SID-buffered".into(),
                            frontend_id: 7,
                        }
                    ),
                }))
                .is_ok()
        );
        drop(receive);
        assert!(
            matches!(requests.try_recv(), Ok(CdpFrontendControlRequest::DisposeOrphanedResource(
            CdpControlResource::BrowserFrontend { frontend_id: 7, session_id }
        )) if session_id == "SID-buffered")
        );

        let (reply, receive) = oneshot::channel::<Result<CdpCreatedTargetResult>>();
        assert!(
            reply
                .send(Ok(CdpCreatedTargetResult {
                    target: CdpCreatedTarget {
                        page_target_id: "TID-buffered".into(),
                        tab_target_id: "TAB-buffered".into()
                    },
                    rollback: CdpControlResourceLease::new(
                        owner.downgrade(),
                        CdpControlResource::Target("TID-buffered".into())
                    ),
                }))
                .is_ok()
        );
        drop(receive);
        assert!(
            matches!(requests.try_recv(), Ok(CdpFrontendControlRequest::DisposeOrphanedResource(
            CdpControlResource::Target(target)
        )) if target == "TID-buffered")
        );

        let adopted = CdpAttachedFrontend {
            frontend_id: 8,
            rollback: CdpControlResourceLease::new(
                owner.downgrade(),
                CdpControlResource::TargetFrontend {
                    session_id: "SID-adopted".into(),
                    frontend_id: 8,
                },
            ),
        };
        assert_eq!(adopted.into_id(), 8);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn adopted_control_resources_do_not_roll_back_and_leases_do_not_keep_the_owner_alive() {
        let (owner, mut requests) = mpsc::unbounded_channel();
        let mut result = CdpInternalCommandResult {
            response: serde_json::json!({}),
            created_target: None,
            registration: None,
            rollback: Some(CdpControlResourceLease::new(
                owner.downgrade(),
                CdpControlResource::Session("SID-adopted".into()),
            )),
        };
        result.commit_resource();
        drop(result);
        assert!(requests.try_recv().is_err());
        let lease = CdpControlResourceLease::new(
            owner.downgrade(),
            CdpControlResource::Session("SID-closed".into()),
        );
        drop(owner);
        assert!(requests.is_closed());
        drop(lease);
        assert!(matches!(
            requests.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }
}
