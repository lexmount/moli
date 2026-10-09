use super::*;

impl CdpConnection {
    pub async fn evaluate_runtime_expression_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_with_await_for_session_owner_async(
            None,
            expression,
            await_promise,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn evaluate_runtime_expression_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_with_await_for_session_owner_async(
            session_id, expression, false,
        )
        .await
    }

    pub(crate) async fn evaluate_runtime_expression_with_await_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        self.evaluate_runtime_expression_for_session_owner_once_async(
            session_id,
            expression,
            await_promise,
        )
        .await
    }

    pub(super) async fn evaluate_runtime_expression_for_session_owner_once_async(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
        let payload = {
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.evaluate_runtime_expression_without_navigation_follow_with_await_async(
                expression,
                await_promise,
            )
            .await
            .map_err(|error| format!("runtime evaluation failed: {error}"))?
        };
        self.ingest_runtime_session_owner_output_updates(session_id);
        Ok(payload)
    }

    #[cfg(test)]
    pub async fn dispatch_runtime_protocol_message_async(
        &mut self,
        raw_json: &str,
    ) -> Result<Vec<Value>, String> {
        self.dispatch_runtime_protocol_message_for_session_owner_async(None, raw_json)
            .await
    }

    pub(crate) fn start_runtime_enable_events_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<PendingRuntimeEnableEventsDispatch, String> {
        let route = self.runtime_protocol_message_page_route_for_owner(owner)?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_runtime_enable_events_for_inspector_session(inspector_session_id.as_deref())
            .map_err(|error| format!("runtime enable event replay failed: {error}"))?;
        Ok(PendingRuntimeEnableEventsDispatch {
            owner: owner.clone(),
            route,
            pending,
        })
    }

    pub(crate) fn complete_runtime_enable_events(
        &mut self,
        completed: CompletedRuntimeEnableEventsDispatch,
    ) -> Result<RuntimeEnableEventsReplay, String> {
        let owner = completed.owner;
        let page = self.runtime_protocol_message_started_page_mut(&completed.route)?;
        let output = page
            .finish_runtime_enable_output(completed.completion)
            .map_err(|error| format!("runtime enable event replay failed: {error}"))?;
        let attachment_id = output.renderer_agent_attachment_id();
        if attachment_id != Some(completed.route.renderer_agent_attachment_id) {
            return Err(
                "Runtime.enable completed from an unexpected renderer attachment".to_owned(),
            );
        }
        self.prepare_runtime_subscription_events_for_owner(&owner, output)
    }

    /// Project the frozen replay from either an internal completion or an
    /// ordered frontend terminal. The caller has validated the exact attachment.
    pub(crate) fn prepare_runtime_subscription_events_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        output: RendererRuntimeCommandOutput,
    ) -> Result<RuntimeEnableEventsReplay, String> {
        let _ = self.set_renderer_runtime_agent_owns_page_console_api_events_for_owner(owner, true);
        self.prepare_runtime_command_events_for_owner(owner, output)
    }

    pub(crate) fn prepare_runtime_command_events_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        output: RendererRuntimeCommandOutput,
    ) -> Result<RuntimeEnableEventsReplay, String> {
        let session_id = owner.session_id();
        let (_, v8_state_update, messages) = output.into_parts();
        if let Some(state) = v8_state_update
            && !self.merge_v8_inspector_session_state_for_owner(owner, state)
        {
            return Err("Runtime.enable completed after session owner disappeared".to_owned());
        }
        let mut replay = RuntimeEnableEventsReplay::from_renderer_messages(messages);
        self.ingest_runtime_session_owner_output_updates_for_owner(owner);
        for event in replay.events_mut() {
            match event {
                RuntimeEnableReplayEvent::Context(event) => {
                    qualify_runtime_context_protocol_event_for_owner_typed(self, event, owner);
                }
                RuntimeEnableReplayEvent::Background(event) => {
                    event.ensure_protocol_session_id(session_id);
                }
            }
        }
        Ok(replay)
    }

    pub(super) fn runtime_session_owner_page_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut Page, String> {
        self.loaded_page_mut_for_protocol_access(session_id)
    }

    pub(super) fn runtime_session_owner_page_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<&mut Page, String> {
        self.loaded_page_mut_for_protocol_access_for_owner(owner)
    }

    pub(super) fn runtime_session_owner_page_mut_for_interruptible_control_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<&mut Page, String> {
        self.loaded_page_mut_for_interruptible_protocol_access_for_owner(owner)
    }

    pub(super) fn runtime_protocol_message_page_route_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Result<RuntimeProtocolMessagePageRoute, String> {
        let (browser_context_id, target_id) = self
            .target_owner_identity_for_session(session_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        let slot = self.runtime_session_owner_slot(session_id)?;
        let renderer_agent_attachment_id = slot
            .current_renderer_attachment()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .id();
        Ok(RuntimeProtocolMessagePageRoute {
            browser_context_id,
            target_id,
            renderer_agent_attachment_id,
        })
    }

    pub(super) fn runtime_protocol_message_page_route_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<RuntimeProtocolMessagePageRoute, String> {
        let (browser_context_id, target_id) = self
            .target_owner_identity_for_owner(owner)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        let slot = self.runtime_session_owner_slot_for_owner(owner)?;
        let renderer_agent_attachment_id = slot
            .current_renderer_attachment()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?
            .id();
        Ok(RuntimeProtocolMessagePageRoute {
            browser_context_id,
            target_id,
            renderer_agent_attachment_id,
        })
    }

    pub(super) fn runtime_protocol_message_started_slot_mut(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) -> Result<&mut TargetRuntimeSlot, String> {
        let browser_context = self
            .browser_context_by_id_mut(&route.browser_context_id)
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        let slot = if browser_context.active_target_id() == route.target_id.as_deref() {
            &mut browser_context.active_page_target_mut().runtime_slot
        } else {
            let target_id = route
                .target_id
                .as_deref()
                .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
            browser_context
                .background_target_mut(target_id)
                .map(|target| &mut target.runtime_slot)
                .ok_or_else(|| "NoDocumentLoaded".to_owned())?
        };
        if slot
            .current_renderer_attachment()
            .map(|attachment| attachment.id())
            != Some(route.renderer_agent_attachment_id)
        {
            return Err("Renderer attachment changed".to_owned());
        }
        Ok(slot)
    }

    pub(super) fn runtime_protocol_message_started_page_mut(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) -> Result<&mut Page, String> {
        self.runtime_protocol_message_started_slot_mut(route)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())
    }

    pub(super) fn consume_runtime_protocol_message_completion(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<RendererCommandTurnOutput, String> {
        let output = if let Ok(page) = self.runtime_protocol_message_started_page_mut(route) {
            page.finish_runtime_protocol_message_command_turn(completion)
        } else {
            // Completion means the renderer owner has already committed the
            // command's Page state and concrete protocol publication. The
            // target can install a successor attachment before this protocol
            // task resumes (for example, form.submit() followed by a normal
            // command response). Preserve that immutable result; there is
            // simply no current Page cache belonging to this route to update.
            completion.into_runtime_protocol_message_command_turn()
        };
        output.map_err(|error| format!("runtime inspector dispatch failed: {error}"))
    }

    pub(super) fn ingest_runtime_protocol_message_started_route_output_updates(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) {
        if let Ok(slot) = self.runtime_protocol_message_started_slot_mut(route) {
            let _ = slot.ingest_owner_page_observable_output_updates();
        }
    }

    pub(super) fn shared_worker_runtime_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<SharedWorkerRuntimeTargetRoute, String> {
        let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
        let route = self
            .session_route(Some(session_id))
            .ok_or_else(|| "UnknownSession".to_owned())?;
        match route {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let target = self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.shared_worker_target(&target_id))
                    .ok_or_else(|| "UnknownSession".to_owned())?;
                Ok(SharedWorkerRuntimeTargetRoute {
                    browser_context_id,
                    worker: WorkerRuntimeTarget::Shared(target.renderer_instance_id),
                })
            }
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let target = self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.dedicated_worker_target(&target_id))
                    .ok_or_else(|| "UnknownSession".to_owned())?;
                Ok(SharedWorkerRuntimeTargetRoute {
                    browser_context_id,
                    worker: WorkerRuntimeTarget::Dedicated(target.renderer_instance_id),
                })
            }
            _ => Err("UnknownSession".to_owned()),
        }
    }

    pub(crate) fn run_dedicated_worker_if_waiting_for_debugger_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<bool, String> {
        let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
        let route = self.shared_worker_runtime_target_for_session(Some(session_id))?;
        let WorkerRuntimeTarget::Dedicated(instance_id) = route.worker else {
            return Ok(false);
        };
        if let Some(target) = self.dedicated_worker_target_for_session_mut(Some(session_id)) {
            target.discard_main_script_network_replay_for(session_id);
        }
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or_else(|| "UnknownSession".to_owned())?;
        Ok(renderer_runtime.run_dedicated_worker_if_waiting_for_debugger_for_devtools(instance_id))
    }

    pub(super) fn service_worker_runtime_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<ServiceWorkerRuntimeTargetRoute, String> {
        let session_id = session_id.ok_or_else(|| "UnknownSession".to_owned())?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self
            .session_route(Some(session_id))
            .ok_or_else(|| "UnknownSession".to_owned())?
        else {
            return Err("UnknownSession".to_owned());
        };
        let target = self
            .browser_context_by_id(&browser_context_id)
            .and_then(|context| context.service_worker_target(&target_id))
            .ok_or_else(|| "UnknownSession".to_owned())?;
        Ok(ServiceWorkerRuntimeTargetRoute {
            browser_context_id,
            version_id: target.renderer_version_id,
        })
    }

    pub(crate) fn start_shared_worker_runtime_protocol_message_for_session(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
        let raw_json =
            self.rewrite_runtime_inspector_command_for_session_owner(session_id, &raw_json, None)?;
        self.start_shared_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            None,
            RuntimeProtocolResponseRoute::adapter_reply_without_receiver(),
        )
    }

    pub(crate) fn start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
        &mut self,
        session_id: Option<&str>,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
        self.shared_worker_runtime_target_for_session(session_id)?;
        let (_correlation, raw_json, response_sender, response_route) =
            self.prepare_renderer_call_for_session_owner(session_id, descriptor, command_id, None)?;
        self.start_shared_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            Some(response_sender),
            response_route,
        )
    }

    pub(super) fn start_shared_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
        response_sender: Option<RendererRuntimeInspectorResponseSender>,
        response_route: RuntimeProtocolResponseRoute,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch, String> {
        let route = self.shared_worker_runtime_target_for_session(session_id)?;
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or_else(|| "UnknownSession".to_owned())?;
        let worker = route.worker;
        let inspector_session_id = session_id.map(str::to_owned);
        let response_delivery = response_route.delivery();
        let pending: SharedWorkerRuntimeProtocolDispatchFuture = match (
            worker,
            response_sender,
            response_delivery,
        ) {
            (
                WorkerRuntimeTarget::Shared(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::AdapterReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_shared_worker_runtime_protocol_message_with_deferred_response(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                        response,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::adapter_reply)
            }),
            (
                WorkerRuntimeTarget::Shared(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::SessionSink,
            ) => {
                let inspector_session_id =
                    inspector_session_id.ok_or_else(|| "UnknownSession".to_owned())?;
                Box::pin(async move {
                    renderer_runtime
                        .dispatch_shared_worker_runtime_protocol_message_with_devtools_session_response(
                            instance_id,
                            inspector_session_id,
                            raw_json,
                            response,
                        )
                        .await
                        .map(CompletedWorkerRuntimeProtocolDispatch::devtools_session)
                })
            }
            (
                WorkerRuntimeTarget::Shared(instance_id),
                None,
                RendererInspectorResponseDelivery::AdapterReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_shared_worker_runtime_protocol_message(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::adapter_reply)
            }),
            (
                WorkerRuntimeTarget::Dedicated(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::AdapterReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_dedicated_worker_runtime_protocol_message_with_deferred_response(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                        response,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::adapter_reply)
            }),
            (
                WorkerRuntimeTarget::Dedicated(instance_id),
                Some(response),
                RendererInspectorResponseDelivery::SessionSink,
            ) => {
                let inspector_session_id =
                    inspector_session_id.ok_or_else(|| "UnknownSession".to_owned())?;
                Box::pin(async move {
                    renderer_runtime
                        .dispatch_dedicated_worker_runtime_protocol_message_with_devtools_session_response(
                            instance_id,
                            inspector_session_id,
                            raw_json,
                            response,
                        )
                        .await
                        .map(CompletedWorkerRuntimeProtocolDispatch::devtools_session)
                })
            }
            (
                WorkerRuntimeTarget::Dedicated(instance_id),
                None,
                RendererInspectorResponseDelivery::AdapterReply,
            ) => Box::pin(async move {
                renderer_runtime
                    .dispatch_dedicated_worker_runtime_protocol_message(
                        instance_id,
                        inspector_session_id,
                        raw_json,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::adapter_reply)
            }),
            (_, None, RendererInspectorResponseDelivery::SessionSink) => {
                return Err("SessionResponseSenderMissing".to_owned());
            }
        };
        Ok(PendingSharedWorkerRuntimeProtocolMessageDispatch {
            session_id: session_id.map(str::to_owned),
            pending,
            response_route,
        })
    }

    pub(crate) async fn dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id: u64,
    ) -> anyhow::Result<Vec<RendererRuntimeInspectorMessage>> {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json.to_owned())
            .map_err(anyhow::Error::msg)?;
        let pending = self
            .start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
                session_id, descriptor, command_id,
            )
            .map_err(anyhow::Error::msg)?;
        let mut completed = pending.wait().await.map_err(anyhow::Error::msg)?;
        let response_rx = completed.take_deferred_response_receiver();
        let mut messages = self
            .complete_shared_worker_runtime_protocol_message_for_session(completed)
            .map_err(anyhow::Error::msg)?;
        if let Some(response_rx) = response_rx
            && let Some(message) = self
                .await_registered_runtime_inspector_response_for_session_owner_async(
                    session_id,
                    command_id,
                    response_rx,
                )
                .await
        {
            messages.push(message);
        }
        Ok(messages)
    }

    pub(super) async fn dispatch_service_worker_runtime_helper_protocol_message_for_session_async(
        &mut self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id: u64,
    ) -> anyhow::Result<Vec<RendererRuntimeInspectorMessage>> {
        let descriptor = RendererCommandDescriptor::from_synthesized_payload(raw_json.to_owned())
            .map_err(anyhow::Error::msg)?;
        let pending = self
            .start_service_worker_runtime_protocol_message_for_session_with_deferred_response(
                session_id, descriptor, command_id,
            )
            .map_err(anyhow::Error::msg)?;
        let mut completed = pending.wait().await.map_err(anyhow::Error::msg)?;
        let response_rx = completed.take_deferred_response_receiver();
        let mut messages = self
            .complete_service_worker_runtime_protocol_message_for_session(completed)
            .map_err(anyhow::Error::msg)?;
        if let Some(response_rx) = response_rx
            && let Some(message) = self
                .await_registered_runtime_inspector_response_for_session_owner_async(
                    session_id,
                    command_id,
                    response_rx,
                )
                .await
        {
            messages.push(message);
        }
        Ok(messages)
    }

    pub(crate) fn complete_shared_worker_runtime_protocol_message_for_session(
        &mut self,
        mut completed: CompletedSharedWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.restore_frontend_command_ids_in_runtime_messages(
            completed.session_id.as_deref(),
            None,
            &mut completed.dispatch.messages,
        );
        Ok(completed.dispatch.messages)
    }

    pub(crate) fn start_service_worker_runtime_protocol_message_for_session(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
        let raw_json =
            self.rewrite_runtime_inspector_command_for_session_owner(session_id, &raw_json, None)?;
        self.start_service_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            None,
            RuntimeProtocolResponseRoute::adapter_reply_without_receiver(),
        )
    }

    pub(crate) fn start_service_worker_runtime_protocol_message_for_session_with_deferred_response(
        &mut self,
        session_id: Option<&str>,
        descriptor: RendererCommandDescriptor,
        command_id: u64,
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
        self.service_worker_runtime_target_for_session(session_id)?;
        let (_correlation, raw_json, response_sender, response_route) =
            self.prepare_renderer_call_for_session_owner(session_id, descriptor, command_id, None)?;
        self.start_service_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
            session_id,
            raw_json,
            Some(response_sender),
            response_route,
        )
    }

    pub(super) fn start_service_worker_runtime_protocol_message_for_session_with_optional_deferred_response(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
        response_sender: Option<RendererRuntimeInspectorResponseSender>,
        response_route: RuntimeProtocolResponseRoute,
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch, String> {
        let route = self.service_worker_runtime_target_for_session(session_id)?;
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or_else(|| "UnknownSession".to_owned())?;
        let version_id = route.version_id;
        let inspector_session_id = session_id.map(str::to_owned);
        let response_delivery = response_route.delivery();
        let pending: ServiceWorkerRuntimeProtocolDispatchFuture = Box::pin(async move {
            match (response_sender, response_delivery) {
                (Some(response), RendererInspectorResponseDelivery::AdapterReply) => {
                    renderer_runtime
                        .dispatch_service_worker_runtime_protocol_message_with_deferred_response(
                            version_id,
                            inspector_session_id,
                            raw_json,
                            response,
                        )
                        .await
                        .map(CompletedWorkerRuntimeProtocolDispatch::adapter_reply)
                }
                (Some(response), RendererInspectorResponseDelivery::SessionSink) => {
                    let inspector_session_id =
                        inspector_session_id.ok_or_else(|| "UnknownSession".to_owned())?;
                    renderer_runtime
                            .dispatch_service_worker_runtime_protocol_message_with_devtools_session_response(
                                version_id,
                                inspector_session_id,
                                raw_json,
                                response,
                            )
                            .await
                            .map(CompletedWorkerRuntimeProtocolDispatch::devtools_session)
                }
                (None, RendererInspectorResponseDelivery::AdapterReply) => renderer_runtime
                    .dispatch_service_worker_runtime_protocol_message(
                        version_id,
                        inspector_session_id,
                        raw_json,
                    )
                    .await
                    .map(CompletedWorkerRuntimeProtocolDispatch::adapter_reply),
                (None, RendererInspectorResponseDelivery::SessionSink) => {
                    Err("SessionResponseSenderMissing".to_owned())
                }
            }
        });
        Ok(PendingServiceWorkerRuntimeProtocolMessageDispatch {
            session_id: session_id.map(str::to_owned),
            pending,
            response_route,
        })
    }

    pub(crate) fn complete_service_worker_runtime_protocol_message_for_session(
        &mut self,
        mut completed: CompletedServiceWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>, String> {
        self.restore_frontend_command_ids_in_runtime_messages(
            completed.session_id.as_deref(),
            None,
            &mut completed.dispatch.messages,
        );
        Ok(completed.dispatch.messages)
    }

    pub(crate) fn start_moli_diagnostics(
        &mut self,
    ) -> Result<PendingMoliDiagnosticsDispatch, String> {
        let mut pending = Vec::new();

        if let Some(browser_context) = self.browser_context.as_mut() {
            collect_moli_diagnostics_pending_snapshots(browser_context, &mut pending)?;
        }
        for browser_context in &mut self.inactive_browser_contexts {
            collect_moli_diagnostics_pending_snapshots(browser_context, &mut pending)?;
        }

        Ok(PendingMoliDiagnosticsDispatch { pending })
    }

    pub(crate) fn complete_moli_diagnostics(
        &mut self,
        completed: CompletedMoliDiagnosticsDispatch,
    ) -> Value {
        let mut dedicated_worker_loading_count = 0;
        let mut dedicated_worker_running_worker_isolate_count = 0;
        let mut document_context_count = 0;
        let mut isolated_world_context_count = 0;
        let mut child_default_context_count = 0;
        let mut failed_page_snapshot_count = 0;

        for completed in completed.completed {
            let Some(page) = self
                .browser_context_by_id_mut(&completed.browser_context_id)
                .and_then(|browser_context| {
                    if let Some(target_id) = completed.target_id.as_deref() {
                        browser_context
                            .background_target_mut(target_id)
                            .and_then(|target| target.loaded_page_mut())
                    } else {
                        browser_context
                            .active_page_target_mut()
                            .runtime_slot
                            .loaded_page_mut()
                    }
                })
            else {
                failed_page_snapshot_count += 1;
                continue;
            };
            let Ok(completion) = completed.completion else {
                failed_page_snapshot_count += 1;
                continue;
            };
            let Ok(snapshot) = page.finish_page_diagnostics_snapshot(completion) else {
                failed_page_snapshot_count += 1;
                continue;
            };
            document_context_count += snapshot.diagnostics.document_context_count;
            isolated_world_context_count += snapshot.diagnostics.isolated_world_context_count;
            child_default_context_count += snapshot.diagnostics.child_default_context_count;
            dedicated_worker_loading_count += snapshot.diagnostics.dedicated_worker_loading_count;
            dedicated_worker_running_worker_isolate_count += snapshot
                .diagnostics
                .dedicated_worker_running_worker_isolate_count;
        }

        let estimated_document_isolate_count =
            self.estimated_document_isolate_count_for_diagnostics();
        let shared_worker_running_worker_isolate_count = self
            .browser_contexts()
            .map(|browser_context| {
                browser_context
                    .shared_worker_runtime_diagnostics_for_diagnostics()
                    .running_worker_isolate_count
            })
            .sum::<usize>();
        let estimated_worker_isolate_count = dedicated_worker_running_worker_isolate_count
            + shared_worker_running_worker_isolate_count;
        let estimated_live_v8_isolate_count =
            estimated_document_isolate_count + estimated_worker_isolate_count;

        let mut diagnostics = self.moli_memory_diagnostics();
        diagnostics["isolateScope"]["documentContextCount"] = json!(document_context_count);
        diagnostics["isolateScope"]["isolatedWorldContextCount"] =
            json!(isolated_world_context_count);
        diagnostics["isolateScope"]["childDefaultContextCount"] =
            json!(child_default_context_count);
        diagnostics["isolateScope"]["dedicatedWorkerLoadingCount"] =
            json!(dedicated_worker_loading_count);
        diagnostics["isolateScope"]["dedicatedWorkerRunningWorkerIsolateCount"] =
            json!(dedicated_worker_running_worker_isolate_count);
        diagnostics["isolateScope"]["dedicatedWorkerDiagnosticsFailedPageSnapshotCount"] =
            json!(failed_page_snapshot_count);
        diagnostics["isolateScope"]["estimatedWorkerIsolateCount"] =
            json!(estimated_worker_isolate_count);
        diagnostics["isolateScope"]["estimatedLiveV8IsolateCount"] =
            json!(estimated_live_v8_isolate_count);
        diagnostics
    }
}
