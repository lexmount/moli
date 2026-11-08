use crate::cdp_writer::CdpSocketSink;
use std::{cell::Cell, rc::Rc};

use anyhow::{Context as _, Result, bail};
use moli_protocol::ParsedCdpCommand;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, mpsc, oneshot};

use crate::cdp_frontend::{
    CdpAttachedFrontend, CdpCreatedTargetResult, CdpFrontendControlRequest,
    CdpFrontendRegistration, CdpInternalCommandResult,
};

#[derive(Clone)]
pub(super) struct CdpFrontendTargetControl {
    next_command_id: Rc<Cell<u64>>,
    browser_control_session_id: Rc<OnceCell<String>>,
    owner: Option<mpsc::WeakUnboundedSender<CdpFrontendControlRequest>>,
}

impl Default for CdpFrontendTargetControl {
    fn default() -> Self {
        Self {
            next_command_id: Rc::new(Cell::new(u64::MAX)),
            browser_control_session_id: Rc::new(OnceCell::new()),
            owner: None,
        }
    }
}

impl CdpFrontendTargetControl {
    pub(super) fn new(owner: mpsc::WeakUnboundedSender<CdpFrontendControlRequest>) -> Self {
        Self {
            owner: Some(owner),
            ..Self::default()
        }
    }

    pub(super) async fn attach_target(
        &self,
        target_id: &str,
        frontend_id: u64,
        sink: CdpSocketSink,
    ) -> Result<CdpAttachedFrontend> {
        let control_session = self
            .browser_control_session_id
            .get_or_try_init(|| self.attach_browser())
            .await?;
        let response = self
            .execute_command_with_registration(
                Some(control_session),
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                Some(CdpFrontendRegistration::Target {
                    frontend_id,
                    target_id: target_id.to_owned(),
                    sink,
                }),
            )
            .await?;
        self.finish_frontend_attachment(response, frontend_id).await
    }

    pub(super) async fn attach_browser(&self) -> Result<String> {
        let mut response = self
            .execute_command(None, "Target.attachToBrowserTarget", json!({}))
            .await?;
        let session = response
            .response
            .pointer("/result/sessionId")
            .and_then(Value::as_str)
            .context("CDP browser frontend did not return an attach session")?
            .to_owned();
        // This private session transfers to the control owner's OnceCell in
        // the same poll; no frontend registration or callback remains.
        response.commit_resource();
        Ok(session)
    }

    pub(super) async fn attach_browser_frontend(
        &self,
        frontend_id: u64,
        sink: CdpSocketSink,
    ) -> Result<CdpAttachedFrontend> {
        let response = self
            .execute_command_with_registration(
                None,
                "Target.attachToBrowserTarget",
                json!({}),
                Some(CdpFrontendRegistration::Browser { frontend_id, sink }),
            )
            .await?;
        self.finish_frontend_attachment(response, frontend_id).await
    }

    async fn finish_frontend_attachment(
        &self,
        mut response: CdpInternalCommandResult,
        frontend_id: u64,
    ) -> Result<CdpAttachedFrontend> {
        let session = response
            .response
            .pointer("/result/sessionId")
            .and_then(Value::as_str)
            .context("CDP frontend attach returned no session")?
            .to_owned();
        if let Err(error) = response
            .registration
            .take()
            .context("CDP frontend registration was not completed")?
        {
            self.detach_frontend_session(&session).await;
            response.commit_resource();
            return Err(error);
        }
        let rollback = response
            .rollback
            .take()
            .context("CDP frontend attach lost its cleanup obligation")?;
        Ok(CdpAttachedFrontend {
            frontend_id,
            rollback,
        })
    }

    pub(super) async fn detach_frontend_session(&self, session_id: &str) {
        if let Err(error) = self
            .execute_command(
                None,
                "Target.detachFromTarget",
                json!({"sessionId": session_id}),
            )
            .await
        {
            tracing::debug!(session_id, ?error, "failed to detach CDP frontend session");
        }
    }

    pub(super) async fn activate_target(&self, target_id: &str) -> Result<()> {
        self.execute_command(
            None,
            "Target.activateTarget",
            json!({"targetId": target_id}),
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn create_managed_target(
        &self,
        target_url: &str,
    ) -> Result<CdpCreatedTargetResult> {
        let mut response = self
            .execute_command(None, "Target.createTarget", json!({"url": target_url}))
            .await?;
        let target = response
            .created_target
            .take()
            .context("Target.createTarget returned no created target")?;
        let rollback = response
            .rollback
            .take()
            .context("Target.createTarget lost its cleanup obligation")?;
        Ok(CdpCreatedTargetResult { target, rollback })
    }

    pub(super) async fn close_target(&self, target_id: &str) -> Result<()> {
        let response = self
            .execute_command(None, "Target.closeTarget", json!({"targetId": target_id}))
            .await?
            .response;
        if response["result"]["success"].as_bool() == Some(false) {
            bail!("Target.closeTarget rejected target {target_id}");
        }
        Ok(())
    }

    pub(super) fn checkpoint(&self) {
        if let Some(owner) = self
            .owner
            .as_ref()
            .and_then(mpsc::WeakUnboundedSender::upgrade)
        {
            let _ = owner.send(CdpFrontendControlRequest::Checkpoint);
        }
    }

    async fn execute_command(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<CdpInternalCommandResult> {
        self.execute_command_with_registration(session_id, method, params, None)
            .await
    }

    async fn execute_command_with_registration(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        registration: Option<CdpFrontendRegistration>,
    ) -> Result<CdpInternalCommandResult> {
        let command_id = self.next_command_id.get();
        self.next_command_id.set(
            command_id
                .checked_sub(1)
                .expect("CDP internal command id space exhausted"),
        );
        let mut command = json!({"id": command_id, "method": method, "params": params});
        if let Some(session_id) = session_id {
            command["sessionId"] = json!(session_id);
        }
        let command = ParsedCdpCommand::parse_str(command.to_string())
            .context("invalid CDP internal command")?;
        let (completion_tx, completion_rx) = oneshot::channel();
        let owner = self
            .owner
            .as_ref()
            .and_then(mpsc::WeakUnboundedSender::upgrade)
            .context("CDP owner is no longer available")?;
        owner
            .send(CdpFrontendControlRequest::ExecuteInternalCommand {
                command: Box::new(command),
                registration,
                completion_tx,
            })
            .map_err(|_| anyhow::anyhow!("CDP owner is no longer available"))?;
        drop(owner);
        let response = completion_rx
            .await
            .context("CDP owner stopped before internal command completion")??;
        if let Some(error) = response.response.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown protocol error");
            bail!("CDP frontend control command {method} failed: {message}");
        }
        Ok(response)
    }
}
