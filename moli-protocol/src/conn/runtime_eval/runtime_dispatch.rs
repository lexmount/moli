use anyhow::Result;

use super::*;
use anyhow::Context as _;

impl CdpConnection {
    pub async fn evaluate_runtime_expression_with_await_async(
        &mut self,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
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
    ) -> Result<Value> {
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
    ) -> Result<Value> {
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
    ) -> Result<Value> {
        let payload = {
            let page = self.loaded_page_mut_for_protocol_access(session_id)?;
            page.evaluate_runtime_expression_without_navigation_follow_with_await_async(
                expression,
                await_promise,
            )
            .await
            .context("runtime evaluation failed")?
        };
        self.ingest_runtime_session_owner_output_updates(session_id);
        Ok(payload)
    }

    #[cfg(test)]
    pub async fn dispatch_runtime_protocol_message_async(
        &mut self,
        raw_json: &str,
    ) -> Result<Vec<Value>> {
        self.dispatch_runtime_protocol_message_for_session_owner_async(None, raw_json)
            .await
    }

    pub(crate) fn start_runtime_enable_events_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<PendingRuntimeEnableEventsDispatch> {
        let route = self
            .runtime_protocol_message_page_route_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        let pending = page
            .start_runtime_enable_events_for_inspector_session(inspector_session_id.as_deref())
            .context("runtime enable event replay failed")?;
        Ok(PendingRuntimeEnableEventsDispatch {
            owner: owner.clone(),
            route,
            pending,
        })
    }

    pub(crate) fn complete_runtime_enable_events(
        &mut self,
        completed: CompletedRuntimeEnableEventsDispatch,
    ) -> Result<RuntimeEnableEventsReplay> {
        let owner = completed.owner;
        let page = self.runtime_protocol_message_started_page_mut(&completed.route)?;
        let output = page
            .finish_runtime_enable_output(completed.completion)
            .context("runtime enable event replay failed")?;
        let attachment_id = output.renderer_agent_attachment_id();
        if attachment_id != Some(completed.route.renderer_agent_attachment_id) {
            anyhow::bail!("Runtime.enable completed from an unexpected renderer attachment");
        }
        self.prepare_runtime_subscription_events_for_owner(&owner, output)
            .map_err(anyhow::Error::msg)
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

    pub(super) fn runtime_protocol_message_page_route_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<RuntimeProtocolMessagePageRoute> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_owner(owner)?;
        let slot = self.runtime_session_owner_slot_for_owner(owner)?;
        let renderer_agent_attachment_id = slot.current_renderer_attachment()?.id();
        Some(RuntimeProtocolMessagePageRoute {
            browser_context_id,
            target_id,
            renderer_agent_attachment_id,
        })
    }

    pub(super) fn runtime_protocol_message_started_slot_mut(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) -> Result<&mut TargetRuntimeSlot> {
        let browser_context = self
            .browser_context_by_id_mut(&route.browser_context_id)
            .context(NoDocumentLoaded)?;
        let slot = if browser_context.active_target_id() == route.target_id.as_deref() {
            &mut browser_context.active_page_target_mut().runtime_slot
        } else {
            let target_id = route.target_id.as_deref().context(NoDocumentLoaded)?;
            browser_context
                .background_target_mut(target_id)
                .map(|target| &mut target.runtime_slot)
                .context(NoDocumentLoaded)?
        };
        if slot
            .current_renderer_attachment()
            .map(|attachment| attachment.id())
            != Some(route.renderer_agent_attachment_id)
        {
            anyhow::bail!("Renderer attachment changed");
        }
        Ok(slot)
    }

    pub(super) fn runtime_protocol_message_started_page_mut(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
    ) -> Result<&mut Page> {
        self.runtime_protocol_message_started_slot_mut(route)?
            .loaded_page_mut()
            .context(NoDocumentLoaded)
    }

    pub(super) fn consume_runtime_protocol_message_completion(
        &mut self,
        route: &RuntimeProtocolMessagePageRoute,
        completion: moli_core::page::CompletedPageCommand,
    ) -> Result<RendererCommandTurnOutput> {
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
        output.context("runtime inspector dispatch failed")
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
    ) -> Result<SharedWorkerRuntimeTargetRoute> {
        let session_id = session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
        let route = self
            .session_route(Some(session_id))
            .ok_or(RuntimeInspectorError::UnknownSession)?;
        match route {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let target = self
                    .browser_context_by_id(&browser_context_id)
                    .and_then(|context| context.shared_worker_target(&target_id))
                    .ok_or(RuntimeInspectorError::UnknownSession)?;
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
                    .ok_or(RuntimeInspectorError::UnknownSession)?;
                Ok(SharedWorkerRuntimeTargetRoute {
                    browser_context_id,
                    worker: WorkerRuntimeTarget::Dedicated(target.renderer_instance_id),
                })
            }
            _ => Err(RuntimeInspectorError::UnknownSession.into()),
        }
    }

    pub(crate) fn run_dedicated_worker_if_waiting_for_debugger_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<bool> {
        let session_id = session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
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
            .ok_or(RuntimeInspectorError::UnknownSession)?;
        Ok(renderer_runtime.run_dedicated_worker_if_waiting_for_debugger_for_devtools(instance_id))
    }

    pub(super) fn service_worker_runtime_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<ServiceWorkerRuntimeTargetRoute> {
        let session_id = session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self
            .session_route(Some(session_id))
            .ok_or(RuntimeInspectorError::UnknownSession)?
        else {
            return Err(RuntimeInspectorError::UnknownSession.into());
        };
        let target = self
            .browser_context_by_id(&browser_context_id)
            .and_then(|context| context.service_worker_target(&target_id))
            .ok_or(RuntimeInspectorError::UnknownSession)?;
        Ok(ServiceWorkerRuntimeTargetRoute {
            browser_context_id,
            version_id: target.renderer_version_id,
        })
    }

    pub(crate) fn start_shared_worker_runtime_protocol_message_for_session(
        &mut self,
        session_id: Option<&str>,
        raw_json: String,
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch> {
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
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch> {
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
    ) -> Result<PendingSharedWorkerRuntimeProtocolMessageDispatch> {
        let route = self.shared_worker_runtime_target_for_session(session_id)?;
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or(RuntimeInspectorError::UnknownSession)?;
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
                    inspector_session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
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
                    inspector_session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
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
                return Err(RuntimeInspectorError::SessionResponseSenderMissing.into());
            }
        };
        Ok(PendingSharedWorkerRuntimeProtocolMessageDispatch {
            scope: WorkerRuntimeScope::capture(self, session_id)?,
            session_id: session_id.map(str::to_owned),
            pending,
            response_route,
        })
    }

    pub(crate) fn complete_shared_worker_runtime_protocol_message_for_session(
        &mut self,
        mut completed: CompletedSharedWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
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
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch> {
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
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch> {
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
    ) -> Result<PendingServiceWorkerRuntimeProtocolMessageDispatch> {
        let route = self.service_worker_runtime_target_for_session(session_id)?;
        let renderer_runtime = self
            .browser_context_by_id(&route.browser_context_id)
            .map(|context| context.renderer_runtime())
            .ok_or(RuntimeInspectorError::UnknownSession)?;
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
                        inspector_session_id.ok_or(RuntimeInspectorError::UnknownSession)?;
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
                    Err(RuntimeInspectorError::SessionResponseSenderMissing.into())
                }
            }
        });
        Ok(PendingServiceWorkerRuntimeProtocolMessageDispatch {
            scope: WorkerRuntimeScope::capture(self, session_id)?,
            session_id: session_id.map(str::to_owned),
            pending,
            response_route,
        })
    }

    pub(crate) fn complete_service_worker_runtime_protocol_message_for_session(
        &mut self,
        mut completed: CompletedServiceWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
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
