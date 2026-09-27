use super::*;

impl CdpConnection {
    /// Routes a batch of inspector messages into `out`, demultiplexing by id.
    ///
    /// For each message:
    /// - if it carries an `id` matching a pending inspector await registry entry,
    ///   the entry is consumed and the message is sent with that entry's
    ///   `session_id` (regardless of `current_session_id`);
    /// - otherwise if its `id` matches `current_cmd_id`, the message is sent
    ///   with `current_session_id`;
    /// - otherwise the message is dropped (orphan id; logs at warn);
    /// - notifications (no `id`) are routed as background events with
    ///   `current_session_id`.
    ///
    /// Returns true if a message matching `current_cmd_id` was produced (either
    /// via a pending entry or directly).
    #[cfg(test)]
    pub(crate) fn route_inspector_messages_into(
        &mut self,
        messages: Vec<Value>,
        current_cmd_id: Option<u64>,
        current_session_id: Option<&str>,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        self.route_inspector_messages_with_background_events_into(
            messages,
            current_cmd_id,
            current_session_id,
            response_events,
            background_events,
        )
    }

    #[cfg(test)]
    pub(crate) fn route_inspector_messages_with_background_events_into(
        &mut self,
        messages: Vec<Value>,
        current_cmd_id: Option<u64>,
        current_session_id: Option<&str>,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, current_session_id);
        let mut current_seen = false;
        for message in messages {
            current_seen |= self
                .route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
                    message,
                    current_cmd_id,
                    &owner,
                    response_events,
                    background_events,
                );
        }
        current_seen
    }

    pub(crate) fn route_renderer_runtime_inspector_messages_for_owner_with_background_events_into(
        &mut self,
        messages: Vec<RendererRuntimeInspectorMessage>,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let current_session_id = owner.session_id();
        let mut current_seen = false;
        for message in messages {
            match message {
                RendererRuntimeInspectorMessage::RuntimeContext(event) => {
                    let mut event = RuntimeContextProtocolEvent::from_restore_event(event);
                    qualify_runtime_context_protocol_event_for_owner_typed(self, &mut event, owner);
                    apply_runtime_context_protocol_event_side_effects_for_owner_typed(
                        self, &event, owner,
                    );
                    let mut runtime_context_events = Vec::new();
                    emit_runtime_context_protocol_background_event_typed(
                        &mut runtime_context_events,
                        event,
                        current_session_id,
                    );
                    if current_cmd_id.is_some() {
                        response_events.extend(runtime_context_events);
                    } else {
                        background_events.extend(runtime_context_events);
                    }
                }
                RendererRuntimeInspectorMessage::Protocol(message) => {
                    current_seen |= self
                        .route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
                            message.into_value(),
                            current_cmd_id,
                            owner,
                            response_events,
                            background_events,
                        );
                }
            }
        }
        current_seen
    }

    pub(crate) fn route_renderer_runtime_command_output_for_owner_into(
        &mut self,
        output: RendererRuntimeCommandOutput,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        ordered_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let current_session_id = owner.session_id();
        let attachment_is_current =
            output
                .renderer_agent_attachment_id()
                .is_none_or(|attachment_id| {
                    self.current_renderer_agent_attachment_id_for_owner(owner)
                        == Some(attachment_id)
                });
        if !attachment_is_current {
            tracing::debug!(
                attachment_id = ?output.renderer_agent_attachment_id(),
                session_id = current_session_id,
                "dropping renderer command output from a stale attachment"
            );
            return false;
        }
        if output.renderer_agent_attachment_id().is_some()
            && let Some(state) = output.v8_state_update().cloned()
        {
            let _ = self.merge_v8_inspector_session_state_for_owner(owner, state);
        }
        let mut current_seen = false;
        for message in output.into_messages() {
            let mut response_events = Vec::new();
            let mut background_events = Vec::new();
            current_seen |= self
                .route_renderer_runtime_inspector_messages_for_owner_with_background_events_into(
                    vec![message],
                    current_cmd_id,
                    owner,
                    &mut response_events,
                    &mut background_events,
                );
            ordered_events.extend(response_events);
            ordered_events.extend(background_events);
        }
        current_seen
    }

    pub(crate) fn route_renderer_command_turn_output_for_owner_into(
        &mut self,
        output: RendererCommandTurnOutput,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        response_flush: &CommandResponseFlushContext,
        ordered_events: &mut Vec<BackgroundProtocolEvent>,
        post_response_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> (bool, Option<moli_core::RendererOutputFence>) {
        let mut command = CommandDispatchContext::new(response_flush.clone());
        let completion = command.consume_renderer_command_turn_output(output);
        ordered_events.extend(command.take_protocol_events());
        post_response_events.extend(command.take_post_response_events());
        let renderer_output_predecessor = command.take_renderer_output_predecessor();
        let Some(output) = completion.into_runtime_inspector_output() else {
            tracing::error!("Runtime command turn completed with a non-Runtime reply");
            return (false, renderer_output_predecessor);
        };
        let response_seen = self.route_renderer_runtime_command_output_for_owner_into(
            output,
            current_cmd_id,
            owner,
            ordered_events,
        );
        (response_seen, renderer_output_predecessor)
    }

    pub(super) fn route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
        &mut self,
        message: Value,
        current_cmd_id: Option<u64>,
        owner: &CommandOwnerScope,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let current_session_id = owner.session_id();
        let mut message = message;
        let id = message.get("id").and_then(Value::as_u64);
        match id {
            Some(id) => {
                let entry = self.remove_pending_inspector_await_for_owner(id, owner);
                if let Some(entry) = entry {
                    let response = OwnerRuntimeResponse::from_pending_inspector_await(
                        id, entry, owner, message,
                    );
                    return self.route_owner_runtime_response_into(
                        response,
                        current_cmd_id,
                        response_events,
                        background_events,
                    );
                }
                if Some(id) == current_cmd_id {
                    self.register_runtime_remote_object_ids_from_value_for_owner(owner, &message);
                    let response =
                        BackgroundCommandResponsePayload::from_owned_runtime_inspector_message(
                            message,
                        );
                    response_events.push(BackgroundProtocolEvent::command_response(
                        Some(id),
                        current_session_id,
                        response,
                    ));
                    return true;
                }
                tracing::warn!(
                    id,
                    "dropping inspector reply with no matching pending await"
                );
            }
            None => {
                if message.get("method").and_then(Value::as_str) == Some("Page.windowOpen") {
                    let params = message.get("params").unwrap_or(&Value::Null);
                    let Some(url) = params.get("url").and_then(Value::as_str) else {
                        tracing::warn!("dropping Page.windowOpen without a string url");
                        return false;
                    };
                    let Some(window_name) = params.get("windowName").and_then(Value::as_str) else {
                        tracing::warn!("dropping Page.windowOpen without a string windowName");
                        return false;
                    };
                    let Some(window_features) =
                        params.get("windowFeatures").and_then(Value::as_array)
                    else {
                        tracing::warn!("dropping Page.windowOpen without windowFeatures");
                        return false;
                    };
                    let Some(user_gesture) = params.get("userGesture").and_then(Value::as_bool)
                    else {
                        tracing::warn!("dropping Page.windowOpen without userGesture");
                        return false;
                    };
                    let Some(window_features) = window_features
                        .iter()
                        .map(Value::as_str)
                        .collect::<Option<Vec<_>>>()
                    else {
                        tracing::warn!("dropping Page.windowOpen with a non-string window feature");
                        return false;
                    };
                    let window_features = window_features
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    crate::domains::page::emit_page_window_open_background_events_for_owner(
                        self,
                        background_events,
                        owner,
                        url,
                        window_name,
                        &window_features,
                        user_gesture,
                    );
                    return false;
                }
                self.register_runtime_remote_object_ids_from_value_for_owner(owner, &message);
                if let Some(mut event) =
                    RuntimeContextProtocolEvent::from_context_protocol_message(message.clone())
                {
                    qualify_runtime_context_protocol_event_for_owner_typed(self, &mut event, owner);
                    apply_runtime_context_protocol_event_side_effects_for_owner_typed(
                        self, &event, owner,
                    );
                    let mut runtime_context_events = Vec::new();
                    emit_runtime_context_protocol_background_event_typed(
                        &mut runtime_context_events,
                        event,
                        current_session_id,
                    );
                    if current_cmd_id.is_some() {
                        response_events.extend(runtime_context_events);
                    } else {
                        background_events.extend(runtime_context_events);
                    }
                    return false;
                }
                if let Some(session_id) = current_session_id {
                    message["sessionId"] = json!(session_id);
                } else if let Some(map) = message.as_object_mut() {
                    map.remove("sessionId");
                }
                background_events.push(protocol_message_background_event(message));
            }
        }
        false
    }

    pub(crate) async fn route_scheduler_deferred_runtime_inspector_response_into(
        &mut self,
        mut response: RuntimeInspectorResponseReady,
        owner: &CommandOwnerScope,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> (bool, Option<moli_core::RendererOutputFence>) {
        response.bind_owner(owner);
        let Some(response) = self.resolve_runtime_inspector_response_ready(response) else {
            return (false, None);
        };
        // The V8 response and the renderer turn publication travel over
        // separate channels. Preserve the exact Page-stream cursor while
        // consuming the response so the command completion cannot overtake
        // owner actions (for example popup target creation) produced by the
        // same turn.
        let (current_cmd_id, output, renderer_output_predecessor) =
            response.into_renderer_command_output();
        let (renderer_agent_attachment_id, v8_state_update, messages) = output.into_parts();
        let mut ordered_events = Vec::new();
        let output = RendererRuntimeCommandOutput::from_parts(
            renderer_agent_attachment_id,
            v8_state_update,
            messages,
        );
        let current_seen = self.route_renderer_runtime_command_output_for_owner_into(
            output,
            Some(current_cmd_id),
            owner,
            &mut ordered_events,
        );
        response_events.extend(ordered_events);
        let _ = background_events;
        (current_seen, renderer_output_predecessor)
    }

    pub fn route_registered_runtime_inspector_response_into(
        &mut self,
        mut response: RuntimeInspectorResponseReady,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let owner = response
            .owner()
            .cloned()
            .or_else(|| response.session_id().map(CommandOwnerScope::for_session));
        let Some(owner) = owner else {
            tracing::debug!(
                command_id = response.command_id(),
                "dropping implicit runtime Inspector response without an exact owner"
            );
            return;
        };
        response.bind_owner(&owner);
        let Some(response) = self.resolve_runtime_inspector_response_ready(response) else {
            return;
        };
        let message = response.into_protocol_message_for_typed_runtime_route();
        self.route_runtime_inspector_protocol_message_for_owner_with_background_events_into(
            message,
            None,
            &owner,
            response_events,
            background_events,
        );
    }

    pub(crate) fn resolve_runtime_inspector_response_ready(
        &mut self,
        mut response: RuntimeInspectorResponseReady,
    ) -> Option<RuntimeInspectorResponseReady> {
        if response.has_bound_renderer_call_id() {
            return Some(response);
        }
        let command_id = response.command_id();
        let session_id = response.session_id().map(str::to_owned);
        let owner = response.owner().cloned();
        let correlation = if let Some(renderer_call_id) = response.renderer_call_id() {
            let dispatched_attachment_id = response.renderer_agent_attachment_id();
            // A lease can complete immediately before attachment cutover while
            // its response-ready event is still queued. The registry mapping
            // proves that this exact old lease won before rotation; requiring
            // the attachment to remain current here would lose that response.
            match owner.as_ref() {
                Some(owner) => self.take_renderer_call_for_frontend_if_matches_for_owner(
                    owner,
                    command_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                ),
                None => self.take_renderer_call_for_frontend_if_matches_for_session_owner(
                    session_id.as_deref(),
                    command_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                ),
            }
        } else {
            match owner.as_ref() {
                Some(owner) => self.take_renderer_call_for_frontend_for_owner(owner, command_id),
                None => self.take_renderer_call_for_frontend_for_session_owner(
                    session_id.as_deref(),
                    command_id,
                ),
            }
        };
        let Some(correlation) = correlation else {
            tracing::debug!(
                command_id,
                session_id,
                "dropping runtime Inspector response without a pending renderer correlation"
            );
            return None;
        };
        debug_assert!(
            response.renderer_call_id().is_none()
                || correlation.dispatched_attachment_id()
                    == response.renderer_agent_attachment_id()
        );
        response.bind_renderer_call_id(correlation.renderer_call_id());
        Some(response)
    }

    pub(super) fn restore_frontend_command_ids_in_runtime_messages(
        &mut self,
        session_id: Option<&str>,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        messages: &mut [RendererRuntimeInspectorMessage],
    ) {
        if dispatched_attachment_id.is_some_and(|attachment_id| {
            !self.renderer_agent_attachment_is_current_for_session_owner(session_id, attachment_id)
        }) {
            return;
        }
        for message in messages {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                continue;
            };
            let Some(renderer_call_id) = message.renderer_call_id() else {
                continue;
            };
            let Some(correlation) = self
                .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                    session_id,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            else {
                continue;
            };
            debug_assert_eq!(
                correlation.dispatched_attachment_id(),
                dispatched_attachment_id
            );
            message.value_mut()["id"] = json!(correlation.frontend_command_id().get());
        }
    }

    pub(super) fn restore_frontend_command_ids_in_runtime_messages_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        messages: &mut [RendererRuntimeInspectorMessage],
    ) {
        if dispatched_attachment_id.is_some_and(|attachment_id| {
            self.current_renderer_agent_attachment_id_for_owner(owner) != Some(attachment_id)
        }) {
            return;
        }
        for message in messages {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                continue;
            };
            let Some(renderer_call_id) = message.renderer_call_id() else {
                continue;
            };
            let Some(correlation) = self
                .take_frontend_command_for_renderer_if_attachment_matches_for_owner(
                    owner,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            else {
                continue;
            };
            debug_assert_eq!(
                correlation.dispatched_attachment_id(),
                dispatched_attachment_id
            );
            message.value_mut()["id"] = json!(correlation.frontend_command_id().get());
        }
    }

    /// Resolves terminal responses carried by a concrete renderer DevTools
    /// session stream.
    ///
    /// The command owner, renderer call id, and attachment id form the complete
    /// response authority. Document observations are validated separately, so
    /// a response that won its lease immediately before a navigation can still
    /// settle the session without granting the retired document permission to
    /// publish notifications or mutate replacement-document state.
    pub(crate) fn restore_frontend_command_ids_in_devtools_session_output_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        messages: &mut Vec<RendererRuntimeInspectorMessage>,
        project_runtime_object_ownership: bool,
    ) {
        let session_id = owner.session_id().map(str::to_owned);
        messages.retain_mut(|message| {
            let RendererRuntimeInspectorMessage::Protocol(message) = message else {
                return true;
            };
            let Some(renderer_call_id) = message.renderer_call_id() else {
                return true;
            };
            let descriptor = self
                .renderer_command_descriptor_for_renderer_if_attachment_matches_for_owner(
                    owner,
                    renderer_call_id,
                    dispatched_attachment_id,
                );
            let result_object_group = descriptor.as_ref().and_then(|descriptor| {
                self.runtime_result_object_group_for_renderer_command_descriptor(
                    session_id.as_deref(),
                    descriptor,
                )
            });
            let Some(correlation) = self
                .take_frontend_command_for_renderer_if_attachment_matches_for_owner(
                    owner,
                    renderer_call_id,
                    dispatched_attachment_id,
                )
            else {
                tracing::debug!(
                    session_id = session_id.as_deref(),
                    renderer_call_id = renderer_call_id.get(),
                    attachment_id = ?dispatched_attachment_id.map(RendererAgentAttachmentId::get),
                    "dropping DevTools session response without a live renderer correlation"
                );
                return false;
            };
            let frontend_command_id = correlation.frontend_command_id().get();
            message.value_mut()["id"] = json!(frontend_command_id);
            if project_runtime_object_ownership && message.value().get("result").is_some() {
                if let Some(object_group) = result_object_group.as_deref() {
                    self.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner,
                        message.value(),
                        object_group,
                    );
                } else {
                    self.register_runtime_remote_object_ids_from_value_for_owner(
                        owner,
                        message.value(),
                    );
                }
            }
            self.complete_runtime_await_job(frontend_command_id, session_id.as_deref());
            let _ = self.remove_pending_inspector_await_for_owner(frontend_command_id, owner);
            true
        });
    }

    pub(super) fn runtime_result_object_group_for_renderer_command_descriptor(
        &self,
        session_id: Option<&str>,
        descriptor: &RendererCommandDescriptor,
    ) -> Option<String> {
        let command = serde_json::from_str::<Value>(descriptor.frontend_payload()).ok()?;
        let method = command.get("method")?.as_str()?;
        let params = command.get("params")?.as_object()?;
        match method {
            "Runtime.evaluate" | "Runtime.runScript" => params
                .get("objectGroup")
                .and_then(Value::as_str)
                .map(str::to_owned),
            "Runtime.callFunctionOn" => params
                .get("objectGroup")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    self.runtime_remote_object_group_for_session_owner(
                        session_id,
                        params.get("objectId")?.as_str()?,
                    )
                }),
            "Runtime.getProperties" => self.runtime_remote_object_group_for_session_owner(
                session_id,
                params.get("objectId")?.as_str()?,
            ),
            "Runtime.awaitPromise" => self.runtime_remote_object_group_for_session_owner(
                session_id,
                params.get("promiseObjectId")?.as_str()?,
            ),
            "Runtime.queryObjects" => params
                .get("objectGroup")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    self.runtime_remote_object_group_for_session_owner(
                        session_id,
                        params.get("prototypeObjectId")?.as_str()?,
                    )
                }),
            _ => None,
        }
    }

    pub(super) fn start_or_enqueue_registered_runtime_inspector_response_ready(
        &self,
        command_id: u64,
        owner: &CommandOwnerScope,
        mut response_rx: RuntimeInspectorResponseReceiver,
    ) -> bool {
        let Some(response_tx) = self.runtime_inspector_response_ready_sender() else {
            return false;
        };
        let owner = owner.clone();
        // Keep both completion timings on the same response-ready lane. If the
        // renderer callback has already completed, enqueue it immediately; if
        // not, spawn a waiter that will enqueue the same event later.
        match response_rx.try_recv() {
            Ok(completion) => {
                let _ = response_tx.send(crate::conn::RuntimeInspectorResponseReady::for_owner(
                    command_id,
                    &owner,
                    Ok(completion),
                ));
                return true;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                let _ = response_tx.send(crate::conn::RuntimeInspectorResponseReady::for_owner(
                    command_id,
                    &owner,
                    Err("RuntimeInspectorResponseCanceled".to_owned()),
                ));
                return true;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
        tokio::task::spawn_local(async move {
            let response = response_rx
                .await
                .map_err(|_| "RuntimeInspectorResponseCanceled".to_owned());
            let _ = response_tx.send(crate::conn::RuntimeInspectorResponseReady::for_owner(
                command_id, &owner, response,
            ));
        });
        true
    }

    pub(super) fn route_owner_runtime_response_into(
        &mut self,
        response: OwnerRuntimeResponse,
        current_cmd_id: Option<u64>,
        response_events: &mut Vec<BackgroundProtocolEvent>,
        background_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> bool {
        let command_id = response.command_id;
        self.complete_runtime_await_job(command_id, response.session_id());
        self.trace_owner_runtime_response_route(&response);
        let current_seen = Some(command_id) == current_cmd_id;
        match self.route_bidi_channel_listener_owner_runtime_response(&response) {
            BidiChannelListenerRoute::NotListener => {}
            BidiChannelListenerRoute::Consumed => return current_seen,
            BidiChannelListenerRoute::Event(event) => {
                background_events.push(event);
                return current_seen;
            }
        }
        let routed_session_id = response.session_id().map(str::to_owned);
        if let Some(object_group) = response.object_group() {
            self.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                response.owner(),
                &response.message,
                object_group,
            );
        } else {
            self.register_runtime_remote_object_ids_from_value_for_owner(
                response.owner(),
                &response.message,
            );
        }
        let mut message = response.into_protocol_message();
        if let Some(session_id) = routed_session_id.as_deref() {
            message["sessionId"] = json!(session_id);
        } else if let Some(map) = message.as_object_mut() {
            map.remove("sessionId");
        }
        response_events.push(protocol_message_background_event(message));
        current_seen
    }

    pub(super) fn trace_owner_runtime_response_route(&mut self, response: &OwnerRuntimeResponse) {
        let current_route = self.runtime_await_owner_route_for_session(response.session_id());
        let response_owner_route = response.owner().resolve_route(self);
        if current_route != response_owner_route {
            tracing::debug!(
                command_id = response.command_id,
                session_id = response.session_id(),
                ?response_owner_route,
                current_owner_route = ?current_route,
                "owner runtime response route no longer matches current session owner"
            );
        }
        self.record_runtime_await_trace(
            "owner_runtime_response_route",
            Some(response.command_id),
            response.session_id(),
            json!({
                "ownerRoute": response_owner_route.as_ref().map(|route| format!("{route:?}")),
                "currentOwnerRoute": current_route.as_ref().map(|route| format!("{route:?}")),
            }),
        );
    }

    pub(super) fn route_bidi_channel_listener_owner_runtime_response(
        &mut self,
        response: &OwnerRuntimeResponse,
    ) -> BidiChannelListenerRoute {
        let Some(residence) = response.bidi_channel_listener().cloned() else {
            return BidiChannelListenerRoute::NotListener;
        };
        let owner = residence.owner().clone();
        if !owner.is_current(self) {
            tracing::debug!(
                command_id = response.command_id,
                session_id = owner.session_id(),
                "discarding BiDi channel listener reply for a stale Page attachment"
            );
            return BidiChannelListenerRoute::Consumed;
        }
        let listener = residence.listener();
        let message = &response.message;
        if let Some(error) = message.get("error") {
            tracing::debug!(
                ?error,
                channel = %listener.properties().channel,
                "BiDi channel listener stopped after inspector error"
            );
            self.publish_bidi_channel_object_group_release(
                owner,
                listener.channel_object_group().to_owned(),
            );
            return BidiChannelListenerRoute::Consumed;
        }
        let result = message.get("result").unwrap_or(&Value::Null);
        if let Some(exception_details) = result.get("exceptionDetails") {
            tracing::debug!(
                ?exception_details,
                channel = %listener.properties().channel,
                "BiDi channel listener stopped after JavaScript exception"
            );
            self.publish_bidi_channel_object_group_release(
                owner,
                listener.channel_object_group().to_owned(),
            );
            return BidiChannelListenerRoute::Consumed;
        }
        let remote = result.get("result").unwrap_or(&Value::Null);
        let properties = listener.properties().clone();
        let realm_id = listener.realm_id().clone();
        let data = DevToolsRemoteValue::from_cdp_remote_object(
            remote,
            matches!(properties.ownership, DevToolsResultOwnership::Root),
            Some(realm_id.clone()),
        );
        if let Some(remote_object_id) = data.handle.as_ref().or(data.shared_id.as_ref()) {
            self.register_runtime_remote_object_ids_for_owner_with_realm(
                owner.command_owner(),
                vec![remote_object_id.as_str().to_owned()],
                realm_id.as_str(),
            );
        }
        let event = BackgroundProtocolEvent::immediate_automation_event(
            json!({
                "method": "Moli.scriptMessage",
                "params": {}
            }),
            AutomationEvent::ScriptMessage(ScriptMessageEvent {
                target_id: Some(listener.target_id().clone()),
                realm_id: Some(realm_id),
                channel: properties.channel.clone(),
                data,
            }),
        );
        self.publish_bidi_channel_listener_start(residence);
        BidiChannelListenerRoute::Event(event)
    }

    pub(crate) async fn document_node_snapshot_for_runtime_remote_object_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>, String> {
        let include_whitespace =
            crate::domains::dom::dom_agent_includes_whitespace_for_owner(self, owner);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_node_snapshot_for_object_id_in_inspector_session(
                inspector_session_id,
                include_whitespace,
                object_id,
                depth,
                pierce,
            )
            .map_err(|error| format!("resolve runtime node snapshot failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve runtime node snapshot failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_node_snapshot_for_object_id(completion)
            .map_err(|error| format!("resolve runtime node snapshot failed: {error}"))
    }

    pub(crate) async fn document_node_snapshot_for_backend_node_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>, String> {
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_node_snapshot_for_backend_node_id(backend_node_id, depth, pierce)
                .map_err(|error| format!("resolve backend node snapshot failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve backend node snapshot failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_node_snapshot_for_backend_node_id(completion)
            .map_err(|error| format!("resolve backend node snapshot failed: {error}"))
    }

    pub(crate) async fn register_document_bidi_node_binding_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: &str,
        backend_node_id: u32,
    ) -> Result<(), String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_register_document_bidi_node_binding(
                inspector_session_id,
                shared_id.to_owned(),
                backend_node_id,
            )
            .map_err(|error| format!("register BiDi node binding failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("register BiDi node binding failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_register_document_bidi_node_binding(completion)
            .map_err(|error| format!("register BiDi node binding failed: {error}"))
    }

    pub(crate) async fn document_bidi_node_binding_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: &str,
    ) -> Result<RendererDomBidiNodeBindingResolution, String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_bidi_node_binding(inspector_session_id, shared_id.to_owned())
                .map_err(|error| format!("resolve BiDi node binding failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve BiDi node binding failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_bidi_node_binding(completion)
            .map_err(|error| format!("resolve BiDi node binding failed: {error}"))
    }

    pub(crate) async fn document_bidi_node_shared_id_for_backend_node_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
    ) -> Result<RendererDomBidiNodeSharedIdResolution, String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_document_bidi_node_shared_id_for_backend_node_id(
                inspector_session_id,
                backend_node_id,
            )
            .map_err(|error| format!("resolve BiDi node shared id failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve BiDi node shared id failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.finish_document_bidi_node_shared_id_for_backend_node_id(completion)
            .map_err(|error| format!("resolve BiDi node shared id failed: {error}"))
    }

    pub(crate) async fn runtime_remote_object_for_backend_node_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<Option<Value>, String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let pending = {
            let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
            page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                inspector_session_id,
                backend_node_id,
                execution_context_id,
                object_group,
            )
            .map_err(|error| format!("resolve runtime object for backend node failed: {error}"))?
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("resolve runtime object for backend node failed: {error}"))?;
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let result = page
            .finish_resolve_runtime_object_for_backend_node_id(completion)
            .map_err(|error| format!("resolve runtime object for backend node failed: {error}"))?;

        match result {
            DocumentNodeRuntimeObjectResolution::Found(remote_object) => {
                Ok(Some(remote_object.into_protocol_value()))
            }
            DocumentNodeRuntimeObjectResolution::MissingNode => Ok(None),
            DocumentNodeRuntimeObjectResolution::MissingContext => Err(
                "resolve runtime object for backend node failed: missing execution context"
                    .to_owned(),
            ),
        }
    }
}
