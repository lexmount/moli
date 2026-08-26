use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, oneshot};

use crate::cdp_writer::CdpSocketSink;

const FRONTEND_COMMAND_QUEUE_CAPACITY: usize = 256;

pub(crate) struct CdpFrontendReceivers {
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
    AttachBrowser {
        sink: CdpSocketSink,
        completion_tx: oneshot::Sender<Result<u64>>,
    },
    AttachTarget {
        target_id: String,
        sink: CdpSocketSink,
        completion_tx: oneshot::Sender<Result<u64>>,
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
        completion_tx: oneshot::Sender<Result<CdpCreatedTarget>>,
    },
    EnsureDefaultTarget {
        completion_tx: oneshot::Sender<Result<()>>,
    },
    Shutdown,
}

pub(crate) struct CdpFrontendCommand {
    pub(crate) frontend_id: u64,
    pub(crate) raw: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CdpCreatedTarget {
    pub(crate) page_target_id: String,
    pub(crate) tab_target_id: String,
}

pub(crate) fn cdp_frontend_channel() -> (CdpFrontendEndpoint, CdpFrontendReceivers) {
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (command_tx, command_rx) = mpsc::channel(FRONTEND_COMMAND_QUEUE_CAPACITY);
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    (
        CdpFrontendEndpoint {
            control_tx,
            command_tx,
            shutdown_tx,
        },
        CdpFrontendReceivers {
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
                .context("CDP owner stopped before browser frontend attach")?,
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
                .context("CDP target owner stopped before target frontend attach")?,
        }
    }

    pub(crate) async fn command(&self, frontend_id: u64, raw: String) -> bool {
        tokio::select! {
            biased;
            _ = self.wait_for_shutdown() => false,
            result = self.command_tx.send(CdpFrontendCommand { frontend_id, raw }) => result.is_ok(),
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
                .context("CDP target owner stopped before target creation")?,
        }
    }

    pub(crate) async fn ensure_default_target(&self) -> Result<()> {
        if self.is_shutting_down() {
            bail!("CDP target owner is shutting down");
        }
        let (completion_tx, completion_rx) = oneshot::channel();
        self.control_tx
            .send(CdpFrontendControlRequest::EnsureDefaultTarget { completion_tx })
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
