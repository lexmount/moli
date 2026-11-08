use anyhow::Result;

use super::*;
use anyhow::Context as _;

impl CdpConnection {
    pub(crate) fn ingest_runtime_session_owner_output_updates(&mut self, session_id: Option<&str>) {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.ingest_runtime_session_owner_output_updates_for_owner(&owner)
    }

    pub(crate) fn ingest_runtime_session_owner_output_updates_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) {
        if let Some(slot) = self.runtime_session_owner_slot_mut_for_owner(owner) {
            let _ = slot.ingest_owner_page_observable_output_updates();
        }
    }

    pub(crate) fn runtime_session_owner_frame_id(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.runtime_session_owner_frame_id_for_owner(&owner)
    }

    pub(crate) fn runtime_session_owner_frame_id_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<String> {
        match owner.resolve_route(self)? {
            CdpSessionRoute::Browser => self
                .browser_context
                .as_ref()
                .and_then(|bc| bc.active_target_id_owned()),
            CdpSessionRoute::BrowserContext { browser_context_id } => self
                .browser_context_by_id(&browser_context_id)
                .and_then(|bc| bc.active_target_id_owned()),
            CdpSessionRoute::PageTarget { target_id, .. } => Some(target_id),
            CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
        }
    }

    pub(crate) fn start_runtime_protocol_message_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        self.start_runtime_protocol_message_for_owner_with_access(
            owner,
            raw_json,
            RendererInspectorCommandRoute::MainThread,
        )
    }

    pub(crate) fn start_runtime_io_protocol_message_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        self.start_runtime_protocol_message_for_owner_with_access(
            owner,
            raw_json,
            RendererInspectorCommandRoute::Io,
        )
    }

    pub(super) fn start_runtime_protocol_message_for_owner_with_access(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
        inspector_route: RendererInspectorCommandRoute,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        let route = self
            .runtime_protocol_message_page_route_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let raw_json = self.rewrite_runtime_inspector_command_for_owner(owner, &raw_json, None)?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = match inspector_route {
            RendererInspectorCommandRoute::MainThread => {
                self.loaded_page_mut_for_protocol_access_for_owner(owner)?
            }
            RendererInspectorCommandRoute::Io => self
                .loaded_page_mut_for_owner(owner)
                .context(NoDocumentLoaded)?,
        };
        let pending = match inspector_route {
            RendererInspectorCommandRoute::MainThread => page
                .start_runtime_protocol_message_for_inspector_session(
                    inspector_session_id,
                    raw_json,
                )
                .map(PendingRuntimeProtocolMessageDispatchKind::Page),
            RendererInspectorCommandRoute::Io => page
                .start_runtime_inspector_io_message_without_response(inspector_session_id, raw_json)
                .map(PendingRuntimeProtocolMessageDispatchKind::Routable),
        }
        .context("runtime inspector dispatch failed")?;
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending,
            response_route: RuntimeProtocolResponseRoute::adapter_reply_without_receiver(),
        })
    }

    pub(crate) fn start_runtime_protocol_message_for_owner_with_deferred_response(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        self.start_runtime_protocol_message_for_owner_with_deferred_response_and_access(
            owner,
            descriptor,
            command_id,
            RendererInspectorCommandRoute::MainThread,
        )
    }

    pub(crate) fn start_runtime_io_protocol_message_for_owner_with_deferred_response(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        self.start_runtime_protocol_message_for_owner_with_deferred_response_and_access(
            owner,
            descriptor,
            command_id,
            RendererInspectorCommandRoute::Io,
        )
    }

    pub(super) fn start_runtime_protocol_message_for_owner_with_deferred_response_and_access(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
        inspector_route: RendererInspectorCommandRoute,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        let route = self
            .runtime_protocol_message_page_route_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let (correlation, raw_json, response_sender, response_route) = self
            .prepare_renderer_call_for_owner(
                owner,
                descriptor,
                command_id,
                Some(route.renderer_agent_attachment_id),
            )?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page_result = match inspector_route {
            RendererInspectorCommandRoute::MainThread => {
                self.loaded_page_mut_for_protocol_access_for_owner(owner)
            }
            RendererInspectorCommandRoute::Io => self
                .loaded_page_mut_for_owner(owner)
                .context(NoDocumentLoaded),
        };
        let page = match page_result {
            Ok(page) => page,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(error);
            }
        };
        let pending = match page.start_routable_runtime_protocol_message_for_inspector_session(
            inspector_session_id,
            inspector_route,
            None,
            raw_json,
            response_sender,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(error.context("runtime inspector dispatch failed"));
            }
        };
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Routable(pending),
            response_route,
        })
    }

    pub(crate) fn start_runtime_protocol_message_with_context_resolution_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        action: &str,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        let route = self
            .runtime_protocol_message_page_route_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let raw_json = self.rewrite_runtime_inspector_command_for_owner(owner, &raw_json, None)?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        let pending = page
            .start_runtime_protocol_message_for_inspector_session_with_context_resolution(
                inspector_session_id,
                action.to_owned(),
                raw_json,
            )
            .context("runtime inspector dispatch failed")?;
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Page(pending),
            response_route: RuntimeProtocolResponseRoute::adapter_reply_without_receiver(),
        })
    }

    pub(crate) fn start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
        &mut self,
        owner: &CommandOwnerScope,
        action: &str,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingRuntimeProtocolMessageDispatch> {
        let route = self
            .runtime_protocol_message_page_route_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let (correlation, raw_json, response_sender, response_route) = self
            .prepare_renderer_call_for_owner(
                owner,
                descriptor,
                command_id,
                Some(route.renderer_agent_attachment_id),
            )?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = match self.loaded_page_mut_for_protocol_access_for_owner(owner) {
            Ok(page) => page,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(error);
            }
        };
        let pending = match page.start_routable_runtime_protocol_message_for_inspector_session(
            inspector_session_id,
            RendererInspectorCommandRoute::MainThread,
            Some(action.to_owned()),
            raw_json,
            response_sender,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, command_id);
                debug_assert_eq!(removed, Some(correlation));
                return Err(error.context("runtime inspector dispatch failed"));
            }
        };
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Routable(pending),
            response_route,
        })
    }

    pub(crate) fn complete_runtime_protocol_message(
        &mut self,
        completed: CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<Option<RendererCommandTurnOutput>> {
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let owner = completed.owner().clone();
        let completion = match completed.completion {
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(completion) => {
                *completion
            }
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                completion,
                ..
            } => *completion,
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                ..
            }
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                _,
            ) => {
                return Ok(None);
            }
        };
        let mut output =
            self.consume_runtime_protocol_message_completion(&completed.route, completion)?;
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_page_dispatch_done",
                output_messages = output
                    .runtime_inspector_output()
                    .map_or(0, |messages| messages.len()),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        self.ingest_runtime_protocol_message_started_route_output_updates(&completed.route);
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_output_ingested",
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        let runtime_messages = output
            .runtime_inspector_output_mut()
            .context("runtime inspector dispatch completed with a non-Runtime renderer reply")?;
        runtime_messages
            .bind_renderer_agent_attachment(completed.route.renderer_agent_attachment_id);
        let runtime_messages = runtime_messages.messages_mut();
        self.restore_frontend_command_ids_in_runtime_messages_for_owner(
            &owner,
            Some(completed.route.renderer_agent_attachment_id),
            runtime_messages,
        );
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_command_output_ready",
                output_messages = output
                    .runtime_inspector_output()
                    .map_or(0, |messages| messages.len()),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        Ok(Some(output))
    }

    pub(super) fn settle_renderer_replacement_error(
        &mut self,
        events: &mut Vec<BackgroundProtocolEvent>,
        frontend_session_id: Option<&str>,
        response_delivery: RendererInspectorResponseDelivery,
        response_sender: &RendererRuntimeInspectorResponseSender,
        correlation: RendererCommandCorrelation,
        message: &str,
    ) {
        if response_delivery == RendererInspectorResponseDelivery::AdapterReply {
            send_renderer_replacement_error(response_sender, correlation, message);
            return;
        }

        self.settle_devtools_session_renderer_error(
            events,
            frontend_session_id,
            correlation,
            message,
        );
    }

    pub(super) fn settle_devtools_session_renderer_error(
        &mut self,
        events: &mut Vec<BackgroundProtocolEvent>,
        frontend_session_id: Option<&str>,
        correlation: RendererCommandCorrelation,
        message: &str,
    ) {
        let Some(resolved) = self
            .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                frontend_session_id,
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            )
        else {
            return;
        };
        debug_assert_eq!(resolved, correlation);
        let frontend_command_id = resolved.frontend_command_id().get();
        self.complete_runtime_await_job(frontend_command_id, frontend_session_id);
        let _ = self.remove_pending_inspector_await(frontend_command_id, frontend_session_id);
        let mut response = json!({
            "id": frontend_command_id,
            "error": {
                "code": -32000,
                "message": message,
            },
        });
        if let Some(session_id) = frontend_session_id {
            response["sessionId"] = json!(session_id);
        }
        events.push(protocol_message_background_event(response));
    }

    pub(crate) fn terminate_prepared_renderer_calls_after_navigation(
        &mut self,
        terminations: Vec<SessionRendererCallTermination>,
        reason: &str,
    ) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        for termination in terminations {
            let (frontend_session_id, termination) = termination.into_parts();
            match termination {
                PreparedRendererCallTermination::AdapterReply {
                    correlation,
                    response_sender,
                } => send_renderer_replacement_error(&response_sender, correlation, reason),
                PreparedRendererCallTermination::SessionSink { correlation } => self
                    .settle_devtools_session_renderer_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        correlation,
                        reason,
                    ),
            }
        }
        events
    }
}
