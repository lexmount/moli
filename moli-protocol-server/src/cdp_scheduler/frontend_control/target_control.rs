use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::cdp_frontend_router::CdpFrontendRouter;

use super::super::{CdpScheduler, ProtocolOutputSequence};

pub(super) struct CdpFrontendTargetControl {
    next_command_id: u64,
    page_control_session_id: Option<String>,
    default_target_materialized: bool,
}

struct ForegroundTabRoute {
    browser_context_id: Option<String>,
    page_target_id: String,
    tab_target_id: String,
}

impl Default for CdpFrontendTargetControl {
    fn default() -> Self {
        Self {
            next_command_id: u64::MAX,
            page_control_session_id: None,
            default_target_materialized: false,
        }
    }
}

impl CdpFrontendTargetControl {
    pub(super) async fn attach_page(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_id: &str,
    ) -> Result<String> {
        self.attach_target(scheduler, frontend_router, target_id)
            .await
    }

    pub(super) async fn attach_foreground_tab(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        page_target_id: &str,
    ) -> Result<(String, Option<String>)> {
        let route = foreground_tab_route(scheduler, page_target_id)?;
        let session_id = self
            .attach_target(scheduler, frontend_router, &route.tab_target_id)
            .await?;
        Ok((session_id, route.browser_context_id))
    }

    async fn attach_target(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        target_id: &str,
    ) -> Result<String> {
        let control_session_id = self
            .ensure_page_control_session(scheduler, frontend_router)
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
        if target_id == scheduler.conn.default_target_id() {
            self.default_target_materialized = true;
        }
        Ok(session_id)
    }

    pub(super) async fn follow_foreground_tab_change(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
        page_target_id: String,
    ) {
        let route = match foreground_tab_route(scheduler, &page_target_id) {
            Ok(route) => route,
            Err(error) => {
                tracing::warn!(
                    target_id = page_target_id,
                    ?error,
                    "failed to resolve promoted foreground tab"
                );
                return;
            }
        };
        let frontends = frontend_router
            .foreground_tab_frontends_for_browser_context(route.browser_context_id.as_deref());
        for frontend in frontends {
            if frontend.page_target_id == route.page_target_id {
                continue;
            }
            let new_session_id = match self
                .attach_target(scheduler, frontend_router, &route.tab_target_id)
                .await
            {
                Ok(session_id) => session_id,
                Err(error) => {
                    tracing::warn!(
                        frontend_id = frontend.frontend_id,
                        target_id = route.page_target_id,
                        ?error,
                        "failed to attach promoted foreground tab"
                    );
                    continue;
                }
            };
            self.detach_frontend_session(scheduler, frontend_router, &frontend.base_session_id)
                .await;
            if let Err(error) = frontend_router.replace_foreground_tab_frontend_base(
                frontend.frontend_id,
                route.browser_context_id.clone(),
                route.page_target_id.clone(),
                new_session_id.clone(),
            ) {
                tracing::warn!(
                    frontend_id = frontend.frontend_id,
                    target_id = route.page_target_id,
                    ?error,
                    "failed to bind promoted foreground tab"
                );
                self.detach_frontend_session(scheduler, frontend_router, &new_session_id)
                    .await;
                continue;
            }
            for (method, params) in frontend.replayable_target_commands {
                if let Err(error) = self
                    .execute_command(
                        scheduler,
                        frontend_router,
                        Some(&new_session_id),
                        &method,
                        params,
                    )
                    .await
                {
                    tracing::warn!(
                        frontend_id = frontend.frontend_id,
                        target_id = route.page_target_id,
                        method,
                        ?error,
                        "failed to restore foreground tab target state"
                    );
                }
            }
        }
    }

    pub(super) async fn attach_browser(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
    ) -> Result<String> {
        self.ensure_default_target_is_materialized(scheduler, frontend_router)
            .await?;
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
    ) -> Result<String> {
        self.ensure_default_target_is_materialized(scheduler, frontend_router)
            .await?;
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                None,
                "Target.createTarget",
                json!({ "url": target_url }),
            )
            .await?;
        response["result"]["targetId"]
            .as_str()
            .map(str::to_owned)
            .context("Target.createTarget returned no targetId")
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

    async fn ensure_page_control_session(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
    ) -> Result<String> {
        if let Some(control_session_id) = self.page_control_session_id.as_ref() {
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
        self.page_control_session_id = Some(session_id.clone());
        Ok(session_id)
    }

    async fn ensure_default_target_is_materialized(
        &mut self,
        scheduler: &mut CdpScheduler,
        frontend_router: &CdpFrontendRouter,
    ) -> Result<()> {
        if self.default_target_materialized {
            return Ok(());
        }
        // A fresh connection may reuse its unclaimed default placeholder for
        // the first createTarget. The protocol server publishes that default
        // ID permanently, so materialize it before creating another target.
        let default_target_id = scheduler.conn.default_target_id().to_owned();
        let control_session_id = self
            .ensure_page_control_session(scheduler, frontend_router)
            .await?;
        let response = self
            .execute_command(
                scheduler,
                frontend_router,
                Some(&control_session_id),
                "Target.attachToTarget",
                json!({ "targetId": default_target_id, "flatten": true }),
            )
            .await?;
        let reservation_session_id = response["result"]["sessionId"]
            .as_str()
            .context("default target reservation returned no sessionId")?
            .to_owned();
        self.execute_command(
            scheduler,
            frontend_router,
            Some(&control_session_id),
            "Target.detachFromTarget",
            json!({ "sessionId": reservation_session_id }),
        )
        .await?;
        self.default_target_materialized = true;
        Ok(())
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

fn foreground_tab_route(
    scheduler: &CdpScheduler,
    page_target_id: &str,
) -> Result<ForegroundTabRoute> {
    let (tab_target_id, browser_context_id) = scheduler
        .conn
        .frontend_tab_target_identity(page_target_id)
        .with_context(|| format!("CDP page target {page_target_id} has no tab target"))?;
    Ok(ForegroundTabRoute {
        browser_context_id,
        page_target_id: page_target_id.to_owned(),
        tab_target_id,
    })
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
