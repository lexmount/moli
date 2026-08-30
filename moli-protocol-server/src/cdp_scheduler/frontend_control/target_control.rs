use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{cdp_frontend::CdpCreatedTarget, cdp_frontend_router::CdpFrontendRouter};

use super::super::{CdpScheduler, ProtocolOutputSequence};

pub(super) struct CdpFrontendTargetControl {
    next_command_id: u64,
    browser_control_session_id: Option<String>,
}

impl Default for CdpFrontendTargetControl {
    fn default() -> Self {
        Self {
            next_command_id: u64::MAX,
            browser_control_session_id: None,
        }
    }
}

impl CdpFrontendTargetControl {
    pub(super) async fn attach_target(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_id: &str,
    ) -> Result<String> {
        self.attach_target_session(scheduler, frontend_router, target_id)
            .await
    }

    async fn attach_target_session(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_id: &str,
    ) -> Result<String> {
        let control_session_id = self
            .ensure_browser_control_session(scheduler, frontend_router)
            .await?;
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                Some(&control_session_id),
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let session_id = response["result"]["sessionId"]
            .as_str()
            .with_context(|| format!("CDP target {target_id} did not return an attach session"))?
            .to_owned();
        Ok(session_id)
    }

    pub(super) async fn attach_browser(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
    ) -> Result<String> {
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                None,
                "Target.attachToBrowserTarget",
                json!({}),
            )
            .await?;
        response["result"]["sessionId"]
            .as_str()
            .map(str::to_owned)
            .context("CDP browser frontend did not return an attach session")
    }

    pub(super) async fn detach_frontend_session(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        session_id: &str,
    ) {
        if let Err(error) = self
            .execute_command(
                scheduler,
                frontend_router,
                None,
                "Target.detachFromTarget",
                json!({ "sessionId": session_id }),
            )
            .await
        {
            tracing::debug!(session_id, ?error, "failed to detach CDP frontend session");
        }
    }

    pub(super) async fn activate_target(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_id: &str,
    ) -> Result<()> {
        self.execute_command(
            scheduler,
            frontend_router,
            None,
            "Target.activateTarget",
            json!({ "targetId": target_id }),
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn create_managed_target(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_url: &str,
    ) -> Result<CdpCreatedTarget> {
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                None,
                "Target.createTarget",
                json!({ "url": target_url }),
            )
            .await?;
        let page_target_id = response["result"]["targetId"]
            .as_str()
            .map(str::to_owned)
            .context("Target.createTarget returned no targetId")?;
        let tab_target_id = scheduler
            .conn
            .tab_target_id_for_page_target_id(&page_target_id)
            .map(str::to_owned)
            .context("created page target has no tab target")?;
        Ok(CdpCreatedTarget {
            page_target_id,
            tab_target_id,
        })
    }

    pub(super) async fn close_target(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_id: &str,
    ) -> Result<()> {
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                None,
                "Target.closeTarget",
                json!({ "targetId": target_id }),
            )
            .await?;
        if response["result"]["success"].as_bool() == Some(false) {
            bail!("Target.closeTarget rejected target {target_id}");
        }
        Ok(())
    }

    async fn ensure_browser_control_session(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
    ) -> Result<String> {
        if let Some(control_session_id) = self.browser_control_session_id.as_ref() {
            return Ok(control_session_id.clone());
        }
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                None,
                "Target.attachToBrowserTarget",
                json!({}),
            )
            .await?;
        let session_id = response["result"]["sessionId"]
            .as_str()
            .context("CDP browser target did not return an attach session")?
            .to_owned();
        self.browser_control_session_id = Some(session_id.clone());
        Ok(session_id)
    }

    pub(super) fn ensure_default_target_is_published(&mut self, scheduler: &mut CdpScheduler) {
        scheduler.conn.publish_default_browser_target();
    }

    async fn execute_command(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let command_id = self.next_command_id;
        self.next_command_id = self.next_command_id.wrapping_sub(1);
        let mut command = json!({
            "id": command_id,
            "method": method,
            "params": params,
        });
        if let Some(session_id) = session_id {
            command["sessionId"] = json!(session_id);
        }
        let outcome = scheduler
            .conn
            .process_message_with_turn_outcome_async(&command.to_string())
            .await;
        let (protocol_events, scheduler_events, renderer_output_predecessor) =
            outcome.into_protocol_event_parts();
        assert!(
            renderer_output_predecessor.is_none(),
            "frontend-control attach/detach commands must not execute renderer work"
        );
        let mut response = None;
        let mut passthrough_events = Vec::new();
        for event in protocol_events {
            if event.protocol_message_id() == Some(command_id) {
                response = Some(event.into_protocol_message());
            } else {
                passthrough_events.push(event);
            }
        }
        let output = scheduler
            .finish_command_dispatch_output_flush(scheduler_events, None)
            .await;
        assert!(
            output.is_empty(),
            "a frontend-control command without a Runtime barrier cannot release held output"
        );
        if method == "Target.attachToBrowserTarget"
            && let Some(session_id) = response
                .as_ref()
                .and_then(|message| message.pointer("/result/sessionId"))
                .and_then(Value::as_str)
        {
            frontend_router
                .register_private_session(session_id.to_owned())
                .context("failed to register CDP internal control session")?;
        }
        frontend_router.enqueue_protocol_output_sequence(
            ProtocolOutputSequence::from_background_events(passthrough_events),
        );
        control_command_result(response, method)
    }
}

fn control_command_result(response: Option<Value>, method: &str) -> Result<Value> {
    let response = response.with_context(|| format!("{method} returned no response"))?;
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown protocol error");
        bail!("CDP frontend control command {method} failed: {message}");
    }
    Ok(response)
}
