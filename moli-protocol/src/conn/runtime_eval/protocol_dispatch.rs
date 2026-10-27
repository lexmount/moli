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

    #[cfg(test)]
    pub(super) async fn dispatch_runtime_protocol_message_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
    ) -> Result<Vec<Value>> {
        // Direct raw protocol compatibility still accepts id-bearing messages.
        // Internal helpers should call the explicit helper variant so the
        // callback owner is visible at the call site.
        if let Some(command_id) = runtime_protocol_message_id(raw_json) {
            return Ok(self
                .dispatch_runtime_helper_protocol_message_for_session_owner_async(
                    session_id, raw_json, command_id,
                )
                .await?
                .into_iter()
                .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
                .collect());
        }
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let messages = {
            let page = self.loaded_page_mut_for_protocol_access(session_id)?;
            page.dispatch_runtime_protocol_message_for_inspector_session_async(
                inspector_session_id,
                raw_json,
            )
            .await
            .context("runtime inspector dispatch failed")?
            .into_iter()
            .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
            .collect::<Vec<_>>()
        };
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_page_dispatch_done",
                messages = messages.len(),
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        self.ingest_runtime_session_owner_output_updates(session_id);
        if let Some(started) = timing_started {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "runtime_inspector_output_ingested",
                elapsed_ms = started.elapsed().as_millis(),
            );
        }
        Ok(messages)
    }

    #[cfg(test)]
    pub(crate) async fn dispatch_runtime_helper_protocol_message_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id: u64,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json.to_owned())
            .map_err(anyhow::Error::msg)?;
        let owner = CommandOwnerScope::capture(self, session_id);
        let pending = self.start_runtime_protocol_message_for_owner_with_deferred_response(
            &owner, descriptor, command_id,
        )?;
        let completed = pending.wait().await?;
        self.complete_runtime_helper_protocol_message_for_session_owner_async(completed, command_id)
            .await
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

    #[cfg(test)]
    pub(crate) async fn complete_runtime_helper_protocol_message_for_session_owner_async(
        &mut self,
        mut completed: CompletedRuntimeProtocolMessageDispatch,
        command_id: u64,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let response_rx = completed.take_deferred_response_receiver();
        let session_id = completed.owner().session_id().map(str::to_owned);
        let output = self.complete_runtime_protocol_message(completed)?;
        let response_in_output = output.as_ref().is_some_and(|output| {
            renderer_command_turn_frontend_protocol_response(output, command_id).is_some()
        });
        let mut messages = Vec::new();
        let mut runtime_messages = Vec::new();
        if let Some(output) = output {
            let (completion, _) = output.into_completion_and_predecessor();
            let Some(runtime_output) = completion.into_runtime_inspector_output() else {
                anyhow::bail!(
                    "runtime inspector dispatch completed with a non-Runtime renderer reply"
                );
            };
            runtime_messages = runtime_output.into_messages();
        }
        if !response_in_output && let Some(response_rx) = response_rx {
            let response = RuntimeInspectorResponseReady::new(
                command_id,
                session_id.as_deref(),
                response_rx
                    .await
                    .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
            );
            if let Some(response) = self.resolve_runtime_inspector_response_ready(response) {
                let (_, output, renderer_output_predecessor) =
                    response.into_renderer_command_output();
                assert!(
                    renderer_output_predecessor.is_none(),
                    "message-only Runtime test helper cannot discard a concrete output cursor"
                );
                runtime_messages.extend(output.into_messages());
            }
        }
        messages.extend(runtime_messages);
        Ok(messages)
    }

    /// Completes one concrete BiDi channel owner action under its frozen Page
    /// route.
    ///
    /// Stale work is consumed without entering a replacement runtime. The old
    /// Page or detached session owns any renderer-side cleanup; applying an
    /// object-group release to the new attachment would be the more dangerous
    /// outcome because group names belong to the producing runtime.
    pub(crate) async fn complete_bidi_channel_owner_action_with_background_events_async(
        &mut self,
        action: BidiChannelOwnerAction,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let mut driver = crate::conn::ProtocolWorkflowDriver::inline(self);
        match driver
            .with_state(move |state| state.prepare_bidi_channel_action(action))
            .await
        {
            Ok(Some(prepared)) => {
                let output = prepared.run(&mut driver).await;
                driver
                    .with_state(move |state| state.publish_bidi_channel_output(output))
                    .await
                    .map(|events| background_events.extend(events))
                    .ok();
            }
            Ok(None) => {}
            Err(error) => tracing::debug!(?error, "BiDi channel owner unavailable"),
        }
    }

    pub(crate) async fn release_bidi_channel_object_group_for_owner_best_effort_async(
        &mut self,
        owner: &CommandOwnerScope,
        object_group: &str,
    ) {
        let Some(owner) = BidiChannelPageOwner::capture_for_owner(self, owner.clone()) else {
            return;
        };
        self.complete_bidi_channel_owner_action_with_background_events_async(
            BidiChannelOwnerAction::release_object_group(owner, object_group),
            &mut Vec::new(),
        )
        .await;
    }
}
