use super::*;

impl CdpConnection {
    pub(crate) fn ingest_runtime_session_owner_output_updates(&mut self, session_id: Option<&str>) {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.ingest_runtime_session_owner_output_updates_for_owner(&owner)
    }

    pub(crate) fn ingest_runtime_session_owner_output_updates_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) {
        if let Ok(slot) = self.runtime_session_owner_slot_mut_for_owner(owner) {
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
    ) -> Result<Vec<Value>, String> {
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
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.dispatch_runtime_protocol_message_for_inspector_session_async(
                inspector_session_id,
                raw_json,
            )
            .await
            .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?
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
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json.to_owned())?;
        let owner = CommandOwnerScope::capture(self, session_id);
        let pending = self.start_runtime_protocol_message_for_owner_with_deferred_response(
            &owner, descriptor, command_id,
        )?;
        let completed = pending.wait().await?;
        self.complete_runtime_helper_protocol_message_for_session_owner_async(completed, command_id)
            .await
    }

    pub(crate) async fn await_registered_runtime_inspector_response_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        command_id: u64,
        response_rx: RuntimeInspectorResponseReceiver,
    ) -> Option<RendererRuntimeInspectorMessage> {
        let response = crate::conn::RuntimeInspectorResponseReady::new(
            command_id,
            session_id,
            response_rx
                .await
                .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
        );
        let mut response = self.resolve_runtime_inspector_response_ready(response)?;
        if response
            .renderer_agent_attachment_id()
            .is_some_and(|attachment_id| {
                !self.renderer_agent_attachment_is_current_for_session_owner(
                    session_id,
                    attachment_id,
                )
            })
        {
            response.replace_with_error("Execution context was destroyed by navigation");
        }
        let message = response.into_protocol_message_for_typed_runtime_route();
        Some(RendererRuntimeInspectorMessage::protocol(message))
    }

    pub(crate) fn start_runtime_protocol_message_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        raw_json: String,
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let raw_json = self.rewrite_runtime_inspector_command_for_owner(owner, &raw_json, None)?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = match inspector_route {
            RendererInspectorCommandRoute::MainThread => {
                self.runtime_session_owner_page_mut_for_owner(owner)?
            }
            RendererInspectorCommandRoute::Io => {
                self.runtime_session_owner_page_mut_for_interruptible_control_for_owner(owner)?
            }
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
        .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?;
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
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
                self.runtime_session_owner_page_mut_for_owner(owner)
            }
            RendererInspectorCommandRoute::Io => {
                self.runtime_session_owner_page_mut_for_interruptible_control_for_owner(owner)
            }
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
                return Err(format!("runtime inspector dispatch failed: {error}"));
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let raw_json = self.rewrite_runtime_inspector_command_for_owner(owner, &raw_json, None)?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_runtime_protocol_message_for_inspector_session_with_context_resolution(
                inspector_session_id,
                action.to_owned(),
                raw_json,
            )
            .map_err(|error| format!("runtime inspector dispatch failed: {error}"))?;
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
    ) -> Result<PendingRuntimeProtocolMessageDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let (correlation, raw_json, response_sender, response_route) = self
            .prepare_renderer_call_for_owner(
                owner,
                descriptor,
                command_id,
                Some(route.renderer_agent_attachment_id),
            )?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = match self.runtime_session_owner_page_mut_for_owner(owner) {
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
                return Err(format!("runtime inspector dispatch failed: {error}"));
            }
        };
        Ok(PendingRuntimeProtocolMessageDispatch {
            owner: owner.clone(),
            route,
            pending: PendingRuntimeProtocolMessageDispatchKind::Routable(pending),
            response_route,
        })
    }

    pub(crate) async fn complete_runtime_protocol_message_async(
        &mut self,
        completed: CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<Option<RendererCommandTurnOutput>, String> {
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
        let runtime_messages = output.runtime_inspector_output_mut().ok_or_else(|| {
            "runtime inspector dispatch completed with a non-Runtime renderer reply".to_owned()
        })?;
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

    pub(crate) async fn replay_prepared_renderer_calls_after_navigation_async(
        &mut self,
        replays: Vec<SessionRendererCallReplay>,
        new_attachment_id: RendererAgentAttachmentId,
    ) -> Result<Vec<BackgroundProtocolEvent>, String> {
        let mut events = Vec::new();
        for replay in replays {
            let frontend_session_id = replay.frontend_session_id().map(str::to_owned);
            let owner = CommandOwnerScope::capture(self, frontend_session_id.as_deref());
            let renderer_inspector_session_id =
                replay.renderer_inspector_session_id().map(str::to_owned);
            let (correlation, replay, response_delivery, frontend_payload, response_sender) =
                replay.into_replay().into_parts();
            let route = match self.runtime_protocol_message_page_route_for_session_owner(
                frontend_session_id.as_deref(),
            ) {
                Ok(route) => route,
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &error,
                    );
                    continue;
                }
            };
            if route.renderer_agent_attachment_id != new_attachment_id {
                self.settle_renderer_replacement_error(
                    &mut events,
                    frontend_session_id.as_deref(),
                    response_delivery,
                    &response_sender,
                    correlation,
                    "renderer replay attachment is no longer current",
                );
                continue;
            }
            let dispatch = match replay {
                RendererCommandReplay::Inspector(dispatch) => dispatch,
                RendererCommandReplay::PerformanceGetMetrics => {
                    debug_assert_eq!(
                        response_delivery,
                        RendererInspectorResponseDelivery::SessionSink
                    );
                    let pending = self
                        .runtime_session_owner_page_mut(frontend_session_id.as_deref())
                        .map_err(|error| error.to_string())
                        .and_then(|page| {
                            let result = crate::domains::performance::performance_metrics_result(
                                &page.cached_performance_metric_snapshot(),
                            );
                            page.start_performance_get_metrics_from_io_with_response(
                                renderer_inspector_session_id.clone(),
                                result,
                                response_sender.clone(),
                            )
                            .map_err(|error| error.to_string())
                        });
                    let completion = match pending {
                        Ok(pending) => pending.wait().await.map_err(|error| error.to_string()),
                        Err(error) => Err(error),
                    };
                    match completion {
                        Ok(
                            moli_core::page::CompletedDevToolsIoCommandDispatch::SessionResponse {
                                ..
                            },
                        )
                        | Ok(moli_core::page::CompletedDevToolsIoCommandDispatch::Dispatched) => {}
                        Err(error) => {
                            self.settle_renderer_replacement_error(
                                &mut events,
                                frontend_session_id.as_deref(),
                                response_delivery,
                                &response_sender,
                                correlation,
                                &format!("Performance replay dispatch failed: {error}"),
                            );
                        }
                    }
                    continue;
                }
                RendererCommandReplay::SetScriptExecutionDisabled { disabled } => {
                    debug_assert_eq!(
                        response_delivery,
                        RendererInspectorResponseDelivery::SessionSink
                    );
                    let pending = self
                        .runtime_session_owner_page_mut(frontend_session_id.as_deref())
                        .map_err(|error| error.to_string())
                        .and_then(|page| {
                            page.start_set_script_execution_disabled_from_io_with_response(
                                renderer_inspector_session_id.clone(),
                                disabled,
                                response_sender.clone(),
                            )
                            .map_err(|error| error.to_string())
                        });
                    let completion = match pending {
                        Ok(pending) => pending.wait().await.map_err(|error| error.to_string()),
                        Err(error) => Err(error),
                    };
                    match completion {
                        Ok(
                            moli_core::page::CompletedDevToolsIoCommandDispatch::SessionResponse {
                                ..
                            },
                        )
                        | Ok(moli_core::page::CompletedDevToolsIoCommandDispatch::Dispatched) => {}
                        Err(error) => {
                            self.settle_renderer_replacement_error(
                                &mut events,
                                frontend_session_id.as_deref(),
                                response_delivery,
                                &response_sender,
                                correlation,
                                &format!("Emulation replay dispatch failed: {error}"),
                            );
                        }
                    }
                    continue;
                }
            };
            let raw_json = match self.rewrite_runtime_inspector_command_for_session_owner(
                frontend_session_id.as_deref(),
                &frontend_payload,
                Some((
                    correlation.frontend_command_id(),
                    correlation.renderer_call_id(),
                )),
            ) {
                Ok(raw_json) => raw_json,
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &error,
                    );
                    continue;
                }
            };
            let pending = {
                let page = match self.runtime_session_owner_page_mut(frontend_session_id.as_deref())
                {
                    Ok(page) => page,
                    Err(error) => {
                        self.settle_renderer_replacement_error(
                            &mut events,
                            frontend_session_id.as_deref(),
                            response_delivery,
                            &response_sender,
                            correlation,
                            &error,
                        );
                        continue;
                    }
                };
                let dispatch_sender = response_sender.clone();
                match response_delivery {
                    RendererInspectorResponseDelivery::AdapterReply => match dispatch {
                        CdpRendererCommandReplayDispatch::ResolveRuntimeContext => page
                            .start_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
                                renderer_inspector_session_id,
                                "addBinding".to_owned(),
                                raw_json,
                                dispatch_sender,
                            )
                            .map(PendingRuntimeProtocolMessageDispatchKind::Page),
                        CdpRendererCommandReplayDispatch::Direct => page
                            .start_runtime_protocol_message_for_inspector_session_with_deferred_response(
                                renderer_inspector_session_id,
                                raw_json,
                                dispatch_sender,
                            )
                            .map(PendingRuntimeProtocolMessageDispatchKind::Page),
                    },
                    RendererInspectorResponseDelivery::SessionSink => {
                        debug_assert_eq!(
                            dispatch,
                            CdpRendererCommandReplayDispatch::Direct,
                            "the migrated synchronous IO family must replay directly"
                        );
                        page.start_routable_runtime_protocol_message_for_inspector_session(
                            renderer_inspector_session_id,
                            RendererInspectorCommandRoute::Io,
                            None,
                            raw_json,
                            dispatch_sender,
                        )
                        .map(PendingRuntimeProtocolMessageDispatchKind::Routable)
                    }
                }
            };
            let pending = match pending {
                Ok(pending) => PendingRuntimeProtocolMessageDispatch {
                    owner,
                    route,
                    pending,
                    response_route:
                        RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
                            response_delivery,
                        ),
                },
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &format!("runtime inspector replay dispatch failed: {error}"),
                    );
                    continue;
                }
            };
            let completed = match pending.wait().await {
                Ok(completed) => completed,
                Err(error) => {
                    self.settle_renderer_replacement_error(
                        &mut events,
                        frontend_session_id.as_deref(),
                        response_delivery,
                        &response_sender,
                        correlation,
                        &error,
                    );
                    continue;
                }
            };
            let completed_owner = completed.owner().clone();
            let completion = match completed.completion {
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(completion) => {
                    *completion
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector => {
                    continue;
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse {
                    ..
                } => {
                    continue;
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(
                    _,
                ) => {
                    continue;
                }
                moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse {
                    completion,
                    ..
                } => *completion,
            };
            let mut command_turn_output = match self
                .consume_runtime_protocol_message_completion(&completed.route, completion)
            {
                Ok(output) => output,
                Err(error) => {
                    send_renderer_replacement_error(
                        &response_sender,
                        correlation,
                        &format!("runtime inspector replay dispatch failed: {error}"),
                    );
                    continue;
                }
            };
            command_turn_output.bind_renderer_agent_attachment(new_attachment_id);
            self.ingest_runtime_protocol_message_started_route_output_updates(&completed.route);
            let mut command = CommandDispatchContext::default();
            let completion = command.consume_renderer_command_turn_output(command_turn_output);
            events.extend(command.take_protocol_events());
            events.extend(command.take_post_response_events());
            let Some(output) = completion.into_runtime_inspector_output() else {
                send_renderer_replacement_error(
                    &response_sender,
                    correlation,
                    "runtime inspector replay completed with a non-Runtime renderer reply",
                );
                continue;
            };
            if output
                .protocol_response(correlation.renderer_call_id().get())
                .is_some()
            {
                let _ = response_sender.send_output(output);
                continue;
            }
            let _ = self.route_renderer_runtime_command_output_for_owner_into(
                output,
                None,
                &completed_owner,
                &mut events,
            );
        }
        Ok(events)
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
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        let response_rx = completed.take_deferred_response_receiver();
        let session_id = completed.owner().session_id().map(str::to_owned);
        let output = self
            .complete_runtime_protocol_message_async(completed)
            .await?;
        let response_in_output = output.as_ref().is_some_and(|output| {
            renderer_command_turn_frontend_protocol_response(output, command_id).is_some()
        });
        let mut messages = Vec::new();
        let mut runtime_messages = Vec::new();
        if let Some(output) = output {
            let (completion, _) = output.into_completion_and_predecessor();
            let Some(runtime_output) = completion.into_runtime_inspector_output() else {
                return Err(
                    "runtime inspector dispatch completed with a non-Runtime renderer reply"
                        .to_owned(),
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
        let (owner, body) = action.into_parts();
        if !owner.is_current(self) {
            tracing::debug!(
                session_id = owner.session_id(),
                action = ?body,
                "discarding stale BiDi channel owner action"
            );
            return;
        }
        let command_owner = owner.command_owner().clone();
        match body {
            BidiChannelOwnerActionBody::StartListener(listener) => {
                self.start_bidi_channel_listener_once_for_owner_with_background_events_async(
                    &command_owner,
                    BidiChannelListenerResidence::from_boxed(owner, listener),
                    background_events,
                )
                .await;
            }
            BidiChannelOwnerActionBody::ReleaseObjectGroup(object_group) => {
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    &command_owner,
                    &object_group,
                )
                .await;
            }
        }
    }

    pub(crate) async fn release_bidi_channel_object_group_for_owner_best_effort_async(
        &mut self,
        owner: &CommandOwnerScope,
        object_group: &str,
    ) {
        let command_id = self.next_internal_runtime_command_id();
        let raw_json = json!({
            "id": command_id,
            "method": "Runtime.releaseObjectGroup",
            "params": { "objectGroup": object_group }
        })
        .to_string();
        let descriptor = match RendererCommandDescriptor::from_synthesized_payload(raw_json) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                tracing::debug!(%error, object_group, "failed to prepare BiDi object group release");
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let pending = match self
            .start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
                owner,
                "releaseObjectGroup",
                descriptor,
                command_id,
            ) {
            Ok(pending) => pending,
            Err(error) => {
                tracing::debug!(
                    %error,
                    object_group,
                    "failed to start BiDi channel object group release"
                );
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let mut completed = match pending.wait().await {
            Ok(completed) => completed,
            Err(error) => {
                self.forget_pending_inspector_await_for_owner(command_id, owner);
                tracing::debug!(
                    %error,
                    object_group,
                    "BiDi channel object group release dispatch failed"
                );
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let mut renderer_response_rx = completed.take_deferred_response_receiver();
        let messages = match self
            .complete_runtime_protocol_message_async(completed)
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                self.forget_pending_inspector_await_for_owner(command_id, owner);
                tracing::debug!(
                    %error,
                    object_group,
                    "BiDi channel object group release completion failed"
                );
                self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
                return;
            }
        };
        let mut release_events = Vec::new();
        let mut release_post_response_events = Vec::new();
        let response_flush = CommandResponseFlushContext::default();
        let release_response_seen = if let Some(messages) = messages {
            self.route_renderer_command_turn_output_for_owner_into(
                messages,
                Some(command_id),
                owner,
                &response_flush,
                &mut release_events,
                &mut release_post_response_events,
            )
            .0
        } else {
            false
        };
        if !release_response_seen {
            tracing::debug!(
                command_id,
                "internal object group release inspector response was not routed as current command"
            );
        }
        if release_response_seen {
            renderer_response_rx.take();
        }
        release_events.extend(release_post_response_events);
        if let Some(renderer_response_rx) = renderer_response_rx {
            let response = RuntimeInspectorResponseReady::for_owner(
                command_id,
                owner,
                renderer_response_rx
                    .await
                    .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned()),
            );
            if let Some(response) = self.resolve_runtime_inspector_response_ready(response) {
                let (_, output, renderer_output_predecessor) =
                    response.into_renderer_command_output();
                assert!(
                    renderer_output_predecessor.is_none(),
                    "internal object-group cleanup cannot discard a concrete output cursor"
                );
                let release_response_seen = self
                    .route_renderer_runtime_command_output_for_owner_into(
                        output,
                        Some(command_id),
                        owner,
                        &mut release_events,
                    );
                if !release_response_seen {
                    tracing::debug!(
                        command_id,
                        "internal object group release deferred inspector response was not routed as current command"
                    );
                }
            }
        }
        self.unregister_runtime_remote_object_group_for_owner(owner, object_group);
    }

    pub(super) async fn start_bidi_channel_listener_once_for_owner_with_background_events_async(
        &mut self,
        owner: &CommandOwnerScope,
        residence: BidiChannelListenerResidence,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let listener = residence.listener();
        if self.runtime_inspector_response_ready_sender().is_none() {
            self.release_bidi_channel_object_group_for_owner_best_effort_async(
                owner,
                listener.channel_object_group(),
            )
            .await;
            return;
        }
        let command_id = self.next_internal_runtime_command_id();
        let raw_json = bidi_channel_listener_call_function_json(command_id, listener);
        let descriptor = match RendererCommandDescriptor::from_synthesized_payload(raw_json) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                tracing::debug!(%error, "failed to prepare BiDi channel listener command");
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    owner,
                    listener.channel_object_group(),
                )
                .await;
                return;
            }
        };
        let pending = match self
            .start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
                owner,
                "callFunctionOn",
                descriptor,
                command_id,
            ) {
            Ok(pending) => pending,
            Err(error) => {
                tracing::debug!(
                    %error,
                    channel = %listener.properties().channel,
                    "failed to start BiDi channel listener"
                );
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    owner,
                    listener.channel_object_group(),
                )
                .await;
                return;
            }
        };
        self.register_pending_bidi_channel_listener_for_owner(command_id, owner, residence);
        let mut completed = match pending.wait().await {
            Ok(completed) => completed,
            Err(error) => {
                let object_group = self
                    .remove_pending_inspector_await_for_cancellation_for_owner(command_id, owner)
                    .and_then(|entry| {
                        entry
                            .bidi_channel_listener()
                            .map(|listener| listener.channel_object_group().to_owned())
                    });
                tracing::debug!(%error, "BiDi channel listener dispatch failed");
                if let Some(object_group) = object_group {
                    self.release_bidi_channel_object_group_for_owner_best_effort_async(
                        owner,
                        &object_group,
                    )
                    .await;
                }
                return;
            }
        };
        let mut renderer_response_rx = completed.take_deferred_response_receiver();
        let messages = match self
            .complete_runtime_protocol_message_async(completed)
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                let object_group = self
                    .remove_pending_inspector_await_for_cancellation_for_owner(command_id, owner)
                    .and_then(|entry| {
                        entry
                            .bidi_channel_listener()
                            .map(|listener| listener.channel_object_group().to_owned())
                    });
                tracing::debug!(%error, "BiDi channel listener completion failed");
                if let Some(object_group) = object_group {
                    self.release_bidi_channel_object_group_for_owner_best_effort_async(
                        owner,
                        &object_group,
                    )
                    .await;
                }
                return;
            }
        };
        let mut listener_events = Vec::new();
        let mut listener_post_response_events = Vec::new();
        let response_flush = CommandResponseFlushContext::default();
        let listener_response_seen = if let Some(messages) = messages {
            self.route_renderer_command_turn_output_for_owner_into(
                messages,
                Some(command_id),
                owner,
                &response_flush,
                &mut listener_events,
                &mut listener_post_response_events,
            )
            .0
        } else {
            false
        };
        if !listener_response_seen {
            tracing::debug!(
                command_id,
                "BiDi channel listener inspector response was consumed before command response routing"
            );
        }
        if listener_response_seen {
            renderer_response_rx.take();
        }
        listener_events.extend(listener_post_response_events);
        let non_listener_response_count = listener_events
            .iter()
            .filter(|event| event.protocol_message_id().is_some())
            .count();
        if non_listener_response_count > 0 {
            tracing::debug!(
                messages = non_listener_response_count,
                "BiDi channel listener produced non-listener protocol messages on background route"
            );
        }
        background_events.extend(
            listener_events
                .into_iter()
                .filter(|event| event.protocol_message_id().is_none()),
        );
        if let Some(renderer_response_rx) = renderer_response_rx {
            // Listener responses use the same response-ready lane whether the
            // oneshot is already completed or still pending.
            if self.start_or_enqueue_registered_runtime_inspector_response_ready(
                command_id,
                owner,
                renderer_response_rx,
            ) {
                return;
            }
            let object_group = self
                .remove_pending_inspector_await_for_cancellation_for_owner(command_id, owner)
                .and_then(|entry| {
                    entry
                        .bidi_channel_listener()
                        .map(|listener| listener.channel_object_group().to_owned())
                });
            tracing::debug!(
                channel_object_group = object_group.as_deref(),
                "BiDi channel listener started without scheduler runtime response hook"
            );
            if let Some(object_group) = object_group {
                self.release_bidi_channel_object_group_for_owner_best_effort_async(
                    owner,
                    &object_group,
                )
                .await;
            }
        }
    }
}
