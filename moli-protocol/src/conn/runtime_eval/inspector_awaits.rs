use super::*;

impl CdpConnection {
    /// Registers a CDP request as awaiting a deferred V8 inspector reply.
    /// Used by `Runtime.evaluate`/`Runtime.callFunctionOn` when `awaitPromise=true`
    /// is dispatched directly to V8 inspector. V8 calls back after the promise
    /// settles, and the reply is routed via [`Self::route_inspector_messages_into`].
    #[cfg(test)]
    pub(crate) fn register_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.try_register_pending_inspector_await_with_object_group_for_owner(
            cdp_request_id,
            &owner,
            None,
        )
        .expect("pending Inspector await frontend command id must be unique per session");
    }

    pub(crate) fn try_register_pending_inspector_await_with_object_group_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let session_id = owner.session_id();
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.try_register_pending_inspector_await(
                owner_session_id,
                cdp_request_id,
                session_id,
                object_group,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.try_register_pending_inspector_await(
                owner_session_id,
                cdp_request_id,
                session_id,
                object_group,
            );
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.try_register_pending_inspector_await(cdp_request_id, session_id, object_group)
        })
        .unwrap_or(Ok(()))
    }

    pub(crate) fn try_register_renderer_call_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target
                .try_register_renderer_call(
                    owner_session_id,
                    cdp_request_id,
                    dispatched_attachment_id,
                    descriptor,
                )
                .ok_or_else(|| "UnknownSession".to_owned())?
                .map_err(|error| error.to_string());
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target
                .try_register_renderer_call(
                    owner_session_id,
                    cdp_request_id,
                    dispatched_attachment_id,
                    descriptor,
                )
                .ok_or_else(|| "UnknownSession".to_owned())?
                .map_err(|error| error.to_string());
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.try_register_renderer_call(cdp_request_id, dispatched_attachment_id, descriptor)
        })
        .ok_or_else(|| "UnknownSession".to_owned())?
        .map_err(|error| error.to_string())
    }

    pub(super) fn try_register_renderer_call_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, String> {
        if owner.session_id().is_some() {
            return self.try_register_renderer_call_for_session_owner(
                owner.session_id(),
                cdp_request_id,
                dispatched_attachment_id,
                descriptor,
            );
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.try_register_renderer_call(cdp_request_id, dispatched_attachment_id, descriptor)
        })
        .ok_or_else(|| "UnknownSession".to_owned())?
        .map_err(|error| error.to_string())
    }

    pub(crate) fn take_renderer_call_for_frontend_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.take_renderer_call_for_frontend(cdp_request_id)
        })
        .flatten()
    }

    pub(crate) fn take_renderer_call_for_frontend_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        if owner.session_id().is_some() {
            return self.take_renderer_call_for_frontend_for_session_owner(
                owner.session_id(),
                cdp_request_id,
            );
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.take_renderer_call_for_frontend(cdp_request_id)
        })
        .flatten()
    }

    pub(super) fn renderer_call_for_frontend_for_session_owner(
        &self,
        session_id: Option<&str>,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.renderer_call_for_frontend(owner_session_id, cdp_request_id);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .renderer_call_for_frontend(cdp_request_id)
    }

    pub(super) fn renderer_command_descriptor_for_renderer_if_attachment_matches_for_session_owner(
        &self,
        session_id: Option<&str>,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.renderer_command_descriptor_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.renderer_command_descriptor_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.target_devtools_session_state_for_session(session_id)?
            .renderer_command_descriptor_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(super) fn renderer_command_descriptor_for_renderer_if_attachment_matches_for_owner(
        &self,
        owner: &CommandOwnerScope,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        if owner.session_id().is_some() {
            return self
                .renderer_command_descriptor_for_renderer_if_attachment_matches_for_session_owner(
                    owner.session_id(),
                    renderer_call_id,
                    dispatched_attachment_id,
                );
        }
        self.target_devtools_session_state_for_owner(owner)?
            .renderer_command_descriptor_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn renderer_runtime_command_cause_for_frontend(
        &self,
        session_id: Option<&str>,
        cdp_request_id: u64,
    ) -> Option<RendererRuntimeCommandCausalIdentity> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.renderer_runtime_command_cause_for_owner(&owner, cdp_request_id)
    }

    pub(crate) fn renderer_runtime_command_cause_for_owner(
        &self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
    ) -> Option<RendererRuntimeCommandCausalIdentity> {
        let correlation = if owner.session_id().is_some() {
            self.renderer_call_for_frontend_for_session_owner(owner.session_id(), cdp_request_id)
        } else {
            self.target_devtools_session_state_for_owner(owner)?
                .renderer_call_for_frontend(cdp_request_id)
        }?;
        Some(RendererRuntimeCommandCausalIdentity::new(
            self.target_renderer_runtime_inspector_session_id_for_owner(owner),
            correlation.renderer_call_id().get(),
        ))
    }

    pub(super) fn take_renderer_call_for_frontend_if_matches_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cdp_request_id: u64,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend_if_matches(
                owner_session_id,
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.take_renderer_call_for_frontend_if_matches(
                owner_session_id,
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.take_renderer_call_for_frontend_if_matches(
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            )
        })
        .flatten()
    }

    pub(super) fn take_renderer_call_for_frontend_if_matches_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        cdp_request_id: u64,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if owner.session_id().is_some() {
            return self.take_renderer_call_for_frontend_if_matches_for_session_owner(
                owner.session_id(),
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.take_renderer_call_for_frontend_if_matches(
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            )
        })
        .flatten()
    }

    pub(crate) fn take_renderer_call_if_correlation_matches_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        correlation: RendererCommandCorrelation,
    ) -> bool {
        self.take_renderer_call_for_frontend_if_matches_for_session_owner(
            session_id,
            correlation.frontend_command_id().get(),
            correlation.renderer_call_id(),
            correlation.dispatched_attachment_id(),
        ) == Some(correlation)
    }

    pub(crate) fn take_renderer_call_if_correlation_matches_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        correlation: RendererCommandCorrelation,
    ) -> bool {
        if owner.session_id().is_some() {
            return self.take_renderer_call_if_correlation_matches_for_session_owner(
                owner.session_id(),
                correlation,
            );
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.take_renderer_call_for_frontend_if_matches(
                correlation.frontend_command_id().get(),
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            )
        })
        .flatten()
            == Some(correlation)
    }

    pub(super) fn take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.take_frontend_command_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.take_frontend_command_for_renderer_if_attachment_matches(
                owner_session_id,
                renderer_call_id,
                dispatched_attachment_id,
            );
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
        })
        .flatten()
    }

    pub(super) fn take_frontend_command_for_renderer_if_attachment_matches_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        if owner.session_id().is_some() {
            return self
                .take_frontend_command_for_renderer_if_attachment_matches_for_session_owner(
                    owner.session_id(),
                    renderer_call_id,
                    dispatched_attachment_id,
                );
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
        })
        .flatten()
    }

    pub(super) fn prepare_renderer_call_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        descriptor: RendererCommandDescriptor,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Result<
        (
            RendererCommandCorrelation,
            String,
            RendererRuntimeInspectorResponseSender,
            RuntimeProtocolResponseRoute,
        ),
        String,
    > {
        let raw_json = descriptor.frontend_payload().to_owned();
        let response_delivery = descriptor.response_delivery();
        let prepared = self.try_register_renderer_call_for_session_owner(
            session_id,
            cdp_request_id,
            dispatched_attachment_id,
            descriptor,
        )?;
        let correlation = prepared.correlation();
        match self.rewrite_runtime_inspector_command_for_session_owner(
            session_id,
            &raw_json,
            Some((
                FrontendCommandId::new(cdp_request_id),
                correlation.renderer_call_id(),
            )),
        ) {
            Ok(raw_json) => {
                let (correlation, response_sender, response_receiver) = prepared.into_parts();
                Ok((
                    correlation,
                    raw_json,
                    response_sender,
                    RuntimeProtocolResponseRoute::for_registered_delivery(
                        response_delivery,
                        response_receiver,
                    ),
                ))
            }
            Err(error) => {
                let removed = self
                    .take_renderer_call_for_frontend_for_session_owner(session_id, cdp_request_id);
                debug_assert_eq!(removed, Some(correlation));
                Err(error)
            }
        }
    }

    pub(super) fn prepare_renderer_call_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        descriptor: RendererCommandDescriptor,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Result<
        (
            RendererCommandCorrelation,
            String,
            RendererRuntimeInspectorResponseSender,
            RuntimeProtocolResponseRoute,
        ),
        String,
    > {
        let raw_json = descriptor.frontend_payload().to_owned();
        let response_delivery = descriptor.response_delivery();
        let prepared = self.try_register_renderer_call_for_owner(
            owner,
            cdp_request_id,
            dispatched_attachment_id,
            descriptor,
        )?;
        let correlation = prepared.correlation();
        match self.rewrite_runtime_inspector_command_for_owner(
            owner,
            &raw_json,
            Some((
                FrontendCommandId::new(cdp_request_id),
                correlation.renderer_call_id(),
            )),
        ) {
            Ok(raw_json) => {
                let (correlation, response_sender, response_receiver) = prepared.into_parts();
                Ok((
                    correlation,
                    raw_json,
                    response_sender,
                    RuntimeProtocolResponseRoute::for_registered_delivery(
                        response_delivery,
                        response_receiver,
                    ),
                ))
            }
            Err(error) => {
                let removed = self.take_renderer_call_for_frontend_for_owner(owner, cdp_request_id);
                debug_assert_eq!(removed, Some(correlation));
                Err(error)
            }
        }
    }

    pub(super) fn rewrite_runtime_inspector_command_for_session_owner(
        &self,
        session_id: Option<&str>,
        raw_json: &str,
        command_id_rewrite: Option<(FrontendCommandId, RendererCallId)>,
    ) -> Result<String, String> {
        let owner_target_id = self
            .runtime_context_owner_identity_for_session(session_id)
            .and_then(|(_, target_id)| target_id);
        rewrite_runtime_inspector_command_for_renderer(
            raw_json,
            command_id_rewrite,
            owner_target_id.as_deref(),
        )
    }

    pub(super) fn rewrite_runtime_inspector_command_for_owner(
        &self,
        owner: &CommandOwnerScope,
        raw_json: &str,
        command_id_rewrite: Option<(FrontendCommandId, RendererCallId)>,
    ) -> Result<String, String> {
        if owner.session_id().is_some() {
            return self.rewrite_runtime_inspector_command_for_session_owner(
                owner.session_id(),
                raw_json,
                command_id_rewrite,
            );
        }
        let owner_target_id = self
            .target_owner_identity_for_owner(owner)
            .and_then(|(_, target_id)| target_id);
        rewrite_runtime_inspector_command_for_renderer(
            raw_json,
            command_id_rewrite,
            owner_target_id.as_deref(),
        )
    }

    pub(crate) fn register_runtime_await_job_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
        object_group: Option<&str>,
        action: &'static str,
    ) {
        let session_id = owner.session_id();
        let job = RuntimeAwaitJob::new(cdp_request_id, owner, object_group, action);
        let trace_fields = job.trace_fields();
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        match self.pending_runtime_await_jobs.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(job);
            }
            Entry::Occupied(_) => {
                tracing::error!(
                    cdp_request_id,
                    session_id,
                    "runtime await trace job already exists for frontend command"
                );
                return;
            }
        }
        self.record_runtime_await_trace(
            "runtime_await_job_start",
            Some(cdp_request_id),
            session_id,
            trace_fields,
        );
    }

    pub(crate) fn trace_runtime_await_pending_registered(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        self.record_runtime_await_trace(
            "runtime_await_pending_registered",
            Some(cdp_request_id),
            session_id,
            self.runtime_await_job_trace_fields(cdp_request_id, session_id),
        );
    }

    pub(crate) fn trace_runtime_await_initial_dispatch_done(
        &mut self,
        cdp_request_id: Option<u64>,
        session_id: Option<&str>,
        messages: usize,
        saw_current_response: bool,
    ) {
        self.record_runtime_await_trace(
            "runtime_await_initial_dispatch_done",
            cdp_request_id,
            session_id,
            json!({
                "messages": messages,
                "matchingResponseSeen": saw_current_response,
                "job": cdp_request_id
                    .map(|id| self.runtime_await_job_trace_fields(id, session_id)),
            }),
        );
    }

    pub(crate) fn complete_runtime_await_job(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        let Some(job) = self.pending_runtime_await_jobs.remove(&key) else {
            return;
        };
        let session_id = job.session_id();
        let fields = job.trace_fields();
        self.record_runtime_await_trace(
            "runtime_await_completed",
            Some(cdp_request_id),
            session_id.as_deref(),
            fields,
        );
    }

    pub(crate) fn cancel_runtime_await_job(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        reason: &'static str,
    ) {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        let Some(job) = self.pending_runtime_await_jobs.remove(&key) else {
            return;
        };
        let session_id = job.session_id();
        let mut fields = job.trace_fields();
        if let Some(object) = fields.as_object_mut() {
            object.insert("reason".to_owned(), json!(reason));
        }
        self.record_runtime_await_trace(
            "runtime_await_cancelled",
            Some(cdp_request_id),
            session_id.as_deref(),
            fields,
        );
    }

    pub(super) fn runtime_await_job_trace_fields(
        &self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Value {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        self.pending_runtime_await_jobs
            .get(&key)
            .map(RuntimeAwaitJob::trace_fields)
            .unwrap_or_else(|| json!({}))
    }

    pub(crate) fn runtime_await_owner_route_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<CdpSessionRoute> {
        if let Some(route) = self.session_route(session_id) {
            return Some(route);
        }
        self.target_owner_identity_for_session(session_id).and_then(
            |(browser_context_id, target_id)| {
                target_id.map(|target_id| CdpSessionRoute::PageTarget {
                    browser_context_id,
                    target_id,
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                })
            },
        )
    }

    pub(crate) fn next_internal_runtime_command_id(&mut self) -> u64 {
        let id = self.next_internal_runtime_command_id;
        self.next_internal_runtime_command_id = self
            .next_internal_runtime_command_id
            .checked_add(1)
            .expect("internal Runtime command id space exhausted");
        id
    }

    pub(crate) fn next_bidi_channel_object_group(&mut self) -> String {
        format!(
            "{BIDI_CHANNEL_OBJECT_GROUP_PREFIX}{}",
            self.next_internal_runtime_command_id()
        )
    }

    #[cfg(test)]
    pub(crate) fn register_pending_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        listener: BidiChannelListenerResidence,
    ) {
        assert_eq!(
            listener.owner().session_id(),
            session_id,
            "BiDi listener residence must be registered under its exact Page attachment"
        );
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_pending_bidi_channel_listener(
                cdp_request_id,
                session_id,
                Some(BIDI_SCRIPT_RESULT_OBJECT_GROUP),
                listener,
            );
        });
    }

    pub(super) fn register_pending_bidi_channel_listener_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
        listener: BidiChannelListenerResidence,
    ) {
        assert_eq!(
            listener.owner().command_owner(),
            owner,
            "BiDi listener residence must be registered under its exact Page owner"
        );
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.register_pending_bidi_channel_listener(
                cdp_request_id,
                owner.session_id(),
                Some(BIDI_SCRIPT_RESULT_OBJECT_GROUP),
                listener,
            );
        });
    }

    pub(crate) fn publish_bidi_channel_listener_start(
        &mut self,
        listener: BidiChannelListenerResidence,
    ) {
        self.publish_bidi_channel_owner_action(BidiChannelOwnerAction::start_listener(listener));
    }

    pub(super) fn publish_bidi_channel_object_group_release(
        &mut self,
        owner: BidiChannelPageOwner,
        object_group: impl Into<String>,
    ) {
        self.publish_bidi_channel_owner_action(BidiChannelOwnerAction::release_object_group(
            owner,
            object_group,
        ));
    }

    pub(super) fn publish_bidi_channel_owner_action(&mut self, action: BidiChannelOwnerAction) {
        let publish_sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::bidi_channel_owner_action(
            publish_sequence,
            action,
        );
        self.scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(crate) fn forget_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) {
        self.cancel_runtime_await_job(cdp_request_id, session_id, "forgotten");
        let _ = self.remove_pending_inspector_await_for_cancellation(cdp_request_id, session_id);
    }

    pub(crate) fn forget_pending_inspector_await_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) {
        self.cancel_runtime_await_job(cdp_request_id, owner.session_id(), "forgotten");
        let _ =
            self.remove_pending_inspector_await_for_cancellation_for_owner(cdp_request_id, owner);
    }

    pub(crate) fn claim_pending_inspector_await_for_scheduler_deferred_reply(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) -> Option<ClaimedPendingInspectorAwait> {
        let claimed = self
            .remove_pending_inspector_await_for_owner(cdp_request_id, owner)
            .map(|entry| ClaimedPendingInspectorAwait {
                command_id: cdp_request_id,
                owner: owner.clone(),
                entry,
            })?;
        let key = PendingRendererCommandKey::new(owner.session_id(), cdp_request_id);
        match self.claimed_pending_inspector_await_owners.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(ClaimedPendingInspectorAwaitOwner::from_claimed(&claimed));
            }
            Entry::Occupied(_) => {
                panic!("claimed pending Inspector await owner must be unique per session");
            }
        }
        Some(claimed)
    }

    pub(super) fn remove_claimed_pending_inspector_await_owner(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Option<ClaimedPendingInspectorAwaitOwner> {
        let key = PendingRendererCommandKey::new(session_id, cdp_request_id);
        self.claimed_pending_inspector_await_owners.remove(&key)
    }

    pub(super) fn drain_claimed_pending_inspector_await_owners_for_session(
        &mut self,
        session_id: Option<&str>,
    ) -> Vec<ClaimedPendingInspectorAwaitOwner> {
        let to_remove = self
            .claimed_pending_inspector_await_owners
            .iter()
            .filter_map(|(key, owner)| {
                owner
                    .matches_session_owner(session_id)
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        to_remove
            .into_iter()
            .filter_map(|key| self.claimed_pending_inspector_await_owners.remove(&key))
            .collect()
    }

    pub(super) fn drain_claimed_pending_inspector_await_owners_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Vec<ClaimedPendingInspectorAwaitOwner> {
        let to_remove = self
            .claimed_pending_inspector_await_owners
            .iter()
            .filter_map(|(key, claimed)| claimed.matches_owner(owner).then_some(key.clone()))
            .collect::<Vec<_>>();
        to_remove
            .into_iter()
            .filter_map(|key| self.claimed_pending_inspector_await_owners.remove(&key))
            .collect()
    }

    pub(super) fn push_claimed_pending_inspector_await_owner_errors(
        &mut self,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        owners: Vec<ClaimedPendingInspectorAwaitOwner>,
        reason: &'static str,
    ) {
        for owner in owners {
            self.cancel_runtime_await_job(owner.command_id, owner.session_id(), reason);
            if let Some(correlation) = owner.renderer_correlation {
                let _ = self.take_renderer_call_for_frontend_if_matches_for_owner(
                    &owner.owner,
                    correlation.frontend_command_id().get(),
                    correlation.renderer_call_id(),
                    correlation.dispatched_attachment_id(),
                );
            }
            if let Some(object_group) = owner.bidi_channel_object_group.as_deref() {
                self.unregister_runtime_remote_object_group_for_owner(&owner.owner, object_group);
                continue;
            }
            let mut response = RuntimeInspectorResponseReady::for_owner(
                owner.command_id,
                &owner.owner,
                Err(reason.to_owned()),
            );
            if let Some(correlation) = owner.renderer_correlation {
                response.bind_renderer_call_id(correlation.renderer_call_id());
            }
            background_events.push(BackgroundProtocolEvent::runtime_inspector_response_ready(
                response,
            ));
        }
    }

    #[cfg(test)]
    pub(crate) fn has_claimed_pending_inspector_awaits_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.claimed_pending_inspector_await_owners
            .values()
            .any(|owner| owner.matches_session_owner(session_id))
    }

    #[cfg(test)]
    pub(crate) fn has_unclaimed_pending_inspector_awaits_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        self.target_devtools_session_state_for_session(session_id)
            .is_some_and(DevToolsSessionState::has_pending_inspector_awaits)
    }

    pub(crate) fn complete_claimed_pending_inspector_await_for_scheduler_deferred_reply(
        &mut self,
        claimed: Option<ClaimedPendingInspectorAwait>,
        protocol_events: &[BackgroundProtocolEvent],
    ) {
        let Some(claimed) = claimed else {
            return;
        };
        let ClaimedPendingInspectorAwait {
            command_id,
            owner,
            entry,
        } = claimed;
        let session_id = owner.session_id().map(str::to_owned);
        self.remove_claimed_pending_inspector_await_owner(command_id, session_id.as_deref());
        self.complete_runtime_await_job(command_id, session_id.as_deref());
        self.apply_completed_pending_inspector_await_entry(&owner, entry, protocol_events);
    }

    pub(crate) fn cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
        &mut self,
        claimed: Option<ClaimedPendingInspectorAwait>,
        reason: &'static str,
    ) {
        let Some(claimed) = claimed else {
            return;
        };
        let ClaimedPendingInspectorAwait {
            command_id,
            owner,
            entry,
        } = claimed;
        let session_id = owner.session_id().map(str::to_owned);
        self.remove_claimed_pending_inspector_await_owner(command_id, session_id.as_deref());
        self.cancel_runtime_await_job(command_id, session_id.as_deref(), reason);
        if let Some(correlation) = entry.renderer_correlation() {
            let _ = self.take_renderer_call_for_frontend_if_matches_for_owner(
                &owner,
                correlation.frontend_command_id().get(),
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            );
        }
        if let Some(listener) = entry.bidi_channel_listener() {
            self.unregister_runtime_remote_object_group_for_owner(
                &owner,
                listener.channel_object_group(),
            );
        }
    }

    pub(super) fn apply_completed_pending_inspector_await_entry(
        &mut self,
        owner: &CommandOwnerScope,
        entry: PendingInspectorAwait,
        protocol_events: &[BackgroundProtocolEvent],
    ) {
        if let Some(object_group) = entry.object_group() {
            for event in protocol_events {
                if let Some((_, _, BackgroundCommandResponsePayloadRef::Success { result })) =
                    event.command_response_payload_ref()
                {
                    self.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner,
                        result,
                        object_group,
                    );
                } else if let Some(message) = event.protocol_message() {
                    self.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner,
                        message,
                        object_group,
                    );
                }
            }
        } else {
            for event in protocol_events {
                if let Some((_, _, BackgroundCommandResponsePayloadRef::Success { result })) =
                    event.command_response_payload_ref()
                {
                    self.register_runtime_remote_object_ids_from_value_for_owner(owner, result);
                } else if let Some(message) = event.protocol_message() {
                    self.register_runtime_remote_object_ids_from_value_for_owner(owner, message);
                }
            }
        }
        if let Some(listener) = entry.bidi_channel_listener() {
            self.unregister_runtime_remote_object_group_for_owner(
                owner,
                listener.channel_object_group(),
            );
        }
    }

    pub(super) fn remove_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Option<PendingInspectorAwait> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            return target.remove_pending_inspector_await(owner_session_id, cdp_request_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            return target.remove_pending_inspector_await(owner_session_id, cdp_request_id);
        }
        self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.remove_pending_inspector_await(cdp_request_id)
        })
        .flatten()
    }

    pub(super) fn remove_pending_inspector_await_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) -> Option<PendingInspectorAwait> {
        if owner.session_id().is_some() {
            return self.remove_pending_inspector_await(cdp_request_id, owner.session_id());
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.remove_pending_inspector_await(cdp_request_id)
        })
        .flatten()
    }

    pub(super) fn remove_pending_inspector_await_for_cancellation(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
    ) -> Option<PendingInspectorAwait> {
        let entry = self.remove_pending_inspector_await(cdp_request_id, session_id);
        if let Some(correlation) = entry
            .as_ref()
            .and_then(PendingInspectorAwait::renderer_correlation)
        {
            self.discard_renderer_call_for_session_owner_if_matches(session_id, correlation);
        } else if entry.is_none() {
            let _ =
                self.take_renderer_call_for_frontend_for_session_owner(session_id, cdp_request_id);
        }
        entry
    }

    pub(super) fn remove_pending_inspector_await_for_cancellation_for_owner(
        &mut self,
        cdp_request_id: u64,
        owner: &CommandOwnerScope,
    ) -> Option<PendingInspectorAwait> {
        let entry = self.remove_pending_inspector_await_for_owner(cdp_request_id, owner);
        if let Some(correlation) = entry
            .as_ref()
            .and_then(PendingInspectorAwait::renderer_correlation)
        {
            let _ = self.take_renderer_call_for_frontend_if_matches_for_owner(
                owner,
                correlation.frontend_command_id().get(),
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            );
        } else if entry.is_none() {
            let _ = self.take_renderer_call_for_frontend_for_owner(owner, cdp_request_id);
        }
        entry
    }

    pub(super) fn discard_renderer_call_for_session_owner_if_matches(
        &mut self,
        session_id: Option<&str>,
        correlation: RendererCommandCorrelation,
    ) {
        let _ = self.take_renderer_call_for_frontend_if_matches_for_session_owner(
            session_id,
            correlation.frontend_command_id().get(),
            correlation.renderer_call_id(),
            correlation.dispatched_attachment_id(),
        );
    }

    pub fn has_pending_inspector_awaits(&self) -> bool {
        if !self.claimed_pending_inspector_await_owners.is_empty() {
            return true;
        }
        self.browser_contexts().any(|browser_context| {
            browser_context
                .page_targets
                .iter()
                .any(|target| target.has_pending_inspector_awaits())
                || browser_context
                    .shared_worker_targets
                    .values()
                    .any(SharedWorkerTargetState::has_pending_inspector_awaits)
                || browser_context
                    .dedicated_worker_targets
                    .values()
                    .any(|target| target.has_pending_inspector_awaits())
                || browser_context
                    .service_worker_targets
                    .values()
                    .any(ServiceWorkerTargetState::has_pending_inspector_awaits)
        })
    }

    pub fn has_pending_inspector_awaits_for_session_owner(&self, session_id: Option<&str>) -> bool {
        if self
            .claimed_pending_inspector_await_owners
            .values()
            .any(|owner| owner.matches_session_owner(session_id))
        {
            return true;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.has_pending_inspector_awaits_for_session(owner_session_id);
        }
        self.target_devtools_session_state_for_session(session_id)
            .is_some_and(DevToolsSessionState::has_pending_inspector_awaits)
    }

    pub(crate) fn fail_pending_inspector_awaits_from_shared_worker_target_session_background_events_into(
        out: &mut Vec<BackgroundProtocolEvent>,
        target: &mut SharedWorkerTargetState,
        owner_session_id: &str,
        reason: &'static str,
    ) {
        for (cdp_id, entry) in target.drain_pending_inspector_awaits_for_session(owner_session_id) {
            if let Some(listener) = entry.bidi_channel_listener() {
                let object_owner_session_id = entry.session_id().unwrap_or(owner_session_id);
                target.unregister_runtime_remote_object_group(
                    object_owner_session_id,
                    listener.channel_object_group(),
                );
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        for correlation in target.terminate_renderer_calls_for_session(owner_session_id, reason) {
            push_terminated_renderer_call_error_background_events(
                out,
                vec![correlation],
                Some(owner_session_id),
                reason,
            );
        }
    }

    pub(crate) fn fail_pending_inspector_awaits_from_service_worker_target_state_background_events_into(
        out: &mut Vec<BackgroundProtocolEvent>,
        target: &mut ServiceWorkerTargetState,
        reason: &'static str,
    ) {
        for (cdp_id, entry) in target.drain_pending_inspector_awaits() {
            if let Some(listener) = entry.bidi_channel_listener() {
                if let Some(session_id) = entry.session_id() {
                    target.unregister_runtime_remote_object_group(
                        session_id,
                        listener.channel_object_group(),
                    );
                }
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        for (session_id, correlation) in target.terminate_renderer_calls(reason) {
            push_terminated_renderer_call_error_background_events(
                out,
                vec![correlation],
                Some(&session_id),
                reason,
            );
        }
    }

    pub(crate) fn fail_pending_inspector_awaits_for_session_owner_background_events_into(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        claimed_background_events: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
        reason: &'static str,
    ) {
        let claimed = self.drain_claimed_pending_inspector_await_owners_for_session(session_id);
        self.push_claimed_pending_inspector_await_owner_errors(
            claimed_background_events,
            claimed,
            reason,
        );
        if let Some(owner_session_id) = session_id
            && self.shared_worker_target_for_session(session_id).is_some()
        {
            let drained = self
                .shared_worker_target_for_session_mut(session_id)
                .map(|target| target.drain_pending_inspector_awaits_for_session(owner_session_id))
                .unwrap_or_default();
            let mut listener_groups_to_unregister = Vec::new();
            for (cdp_id, entry) in drained {
                self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
                if let Some(listener) = entry.bidi_channel_listener() {
                    listener_groups_to_unregister.push((
                        entry.session_id().map(str::to_owned),
                        listener.channel_object_group().to_owned(),
                    ));
                    continue;
                }
                push_pending_inspector_await_error_background_event(
                    out,
                    cdp_id,
                    entry.session_id(),
                    reason,
                );
            }
            if let Some(target) = self.shared_worker_target_for_session_mut(session_id) {
                for (entry_session_id, object_group) in listener_groups_to_unregister {
                    let object_owner_session_id =
                        entry_session_id.as_deref().unwrap_or(owner_session_id);
                    target.unregister_runtime_remote_object_group(
                        object_owner_session_id,
                        &object_group,
                    );
                }
                let terminated =
                    target.terminate_renderer_calls_for_session(owner_session_id, reason);
                push_terminated_renderer_call_error_background_events(
                    out,
                    terminated,
                    Some(owner_session_id),
                    reason,
                );
            }
            return;
        }
        if let Some(owner_session_id) = session_id
            && self.service_worker_target_for_session(session_id).is_some()
        {
            let drained = self
                .service_worker_target_for_session_mut(session_id)
                .map(|target| target.drain_pending_inspector_awaits_for_session(owner_session_id))
                .unwrap_or_default();
            let mut listener_groups_to_unregister = Vec::new();
            for (cdp_id, entry) in drained {
                self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
                if let Some(listener) = entry.bidi_channel_listener() {
                    listener_groups_to_unregister.push((
                        entry.session_id().map(str::to_owned),
                        listener.channel_object_group().to_owned(),
                    ));
                    continue;
                }
                push_pending_inspector_await_error_background_event(
                    out,
                    cdp_id,
                    entry.session_id(),
                    reason,
                );
            }
            if let Some(target) = self.service_worker_target_for_session_mut(session_id) {
                for (entry_session_id, object_group) in listener_groups_to_unregister {
                    let object_owner_session_id =
                        entry_session_id.as_deref().unwrap_or(owner_session_id);
                    target.unregister_runtime_remote_object_group(
                        object_owner_session_id,
                        &object_group,
                    );
                }
                let terminated =
                    target.terminate_renderer_calls_for_session(owner_session_id, reason);
                push_terminated_renderer_call_error_background_events(
                    out,
                    terminated,
                    Some(owner_session_id),
                    reason,
                );
            }
            return;
        }
        let drained = self
            .with_target_devtools_session_state_for_session_mut(session_id, |state| {
                state.drain_pending_inspector_awaits()
            })
            .unwrap_or_default();
        for (cdp_id, entry) in drained {
            self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
            if let Some(listener) = entry.bidi_channel_listener() {
                self.unregister_runtime_remote_object_group_for_session_owner(
                    entry.session_id(),
                    listener.channel_object_group(),
                );
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        let terminated = self
            .with_target_devtools_session_state_for_session_mut(session_id, |state| {
                state.terminate_all_renderer_calls(reason)
            })
            .unwrap_or_default();
        push_terminated_renderer_call_error_background_events(out, terminated, session_id, reason);
    }

    pub(crate) fn fail_pending_inspector_awaits_for_owner_background_events_into(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        claimed_background_events: &mut Vec<BackgroundProtocolEvent>,
        owner: &CommandOwnerScope,
        reason: &'static str,
    ) {
        if owner.session_id().is_some() {
            self.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out,
                claimed_background_events,
                owner.session_id(),
                reason,
            );
            return;
        }

        let claimed = self.drain_claimed_pending_inspector_await_owners_for_owner(owner);
        self.push_claimed_pending_inspector_await_owner_errors(
            claimed_background_events,
            claimed,
            reason,
        );
        let drained = self
            .with_target_devtools_session_state_for_owner_mut(owner, |state| {
                state.drain_pending_inspector_awaits()
            })
            .unwrap_or_default();
        for (cdp_id, entry) in drained {
            self.cancel_runtime_await_job(cdp_id, entry.session_id(), reason);
            if let Some(listener) = entry.bidi_channel_listener() {
                self.unregister_runtime_remote_object_group_for_owner(
                    owner,
                    listener.channel_object_group(),
                );
                continue;
            }
            push_pending_inspector_await_error_background_event(
                out,
                cdp_id,
                entry.session_id(),
                reason,
            );
        }
        let terminated = self
            .with_target_devtools_session_state_for_owner_mut(owner, |state| {
                state.terminate_all_renderer_calls(reason)
            })
            .unwrap_or_default();
        push_terminated_renderer_call_error_background_events(out, terminated, None, reason);
    }
}
