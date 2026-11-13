use super::*;

impl CdpConnection {
    pub(crate) fn clear_runtime_context_metadata_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_context_metadata(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_context_metadata(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.clear_runtime_context_metadata();
        });
    }
    pub(crate) fn clear_runtime_context_metadata_for_owner(&mut self, owner: &CommandOwnerScope) {
        if owner.session_id().is_some() {
            self.clear_runtime_context_metadata_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(
            owner,
            DevToolsSessionState::clear_runtime_context_metadata,
        );
    }
    pub(crate) fn record_runtime_contexts_reported_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_reported_to_frontend(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_reported_to_frontend(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.record_runtime_contexts_reported_to_frontend();
        });
    }
    pub(crate) fn record_runtime_contexts_reported_for_owner(&mut self, owner: &CommandOwnerScope) {
        if owner.session_id().is_some() {
            self.record_runtime_contexts_reported_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(
            owner,
            DevToolsSessionState::record_runtime_contexts_reported_to_frontend,
        );
    }
    pub(crate) fn record_runtime_contexts_cleared_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_cleared_for_frontend(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.record_runtime_contexts_cleared_for_frontend(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.record_runtime_contexts_cleared_for_frontend();
        });
    }
    pub(crate) fn record_runtime_contexts_cleared_for_owner(&mut self, owner: &CommandOwnerScope) {
        if owner.session_id().is_some() {
            self.record_runtime_contexts_cleared_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(
            owner,
            DevToolsSessionState::record_runtime_contexts_cleared_for_frontend,
        );
    }
    pub(crate) fn record_runtime_context_protocol_event_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        event: &RuntimeContextProtocolEvent,
    ) {
        if session_id.is_none() {
            return;
        }
        if let Some(target) = self.shared_worker_target_for_session_mut(session_id) {
            match event {
                RuntimeContextProtocolEvent::Created(event) => {
                    target.record_runtime_execution_context_created_event(event);
                }
                RuntimeContextProtocolEvent::Destroyed(event) => {
                    target.record_runtime_execution_context_destroyed_event(event);
                }
                RuntimeContextProtocolEvent::Cleared(_) => {
                    target.record_runtime_execution_contexts_cleared_event();
                }
            }
            return;
        }
        if let Some(target) = self.service_worker_target_for_session_mut(session_id) {
            match event {
                RuntimeContextProtocolEvent::Created(event) => {
                    target.record_runtime_execution_context_created_event(event);
                }
                RuntimeContextProtocolEvent::Destroyed(event) => {
                    target.record_runtime_execution_context_destroyed_event(event);
                }
                RuntimeContextProtocolEvent::Cleared(_) => {
                    target.record_runtime_execution_contexts_cleared_event();
                }
            }
        }
    }
    pub(crate) fn record_runtime_context_protocol_event_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        event: &RuntimeContextProtocolEvent,
    ) {
        if owner.session_id().is_some() {
            self.record_runtime_context_protocol_event_for_session_owner(owner.session_id(), event);
        }
    }
    pub(crate) fn clear_bidi_references_for_realm_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        realm_id: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.clear_bidi_references_for_realm(owner_session_id, realm_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.clear_bidi_references_for_realm(owner_session_id, realm_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.clear_bidi_references_for_realm(realm_id);
        });
    }
    pub(crate) fn clear_bidi_references_for_realm_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.clear_bidi_references_for_realm_for_session_owner(owner.session_id(), realm_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.clear_bidi_references_for_realm(realm_id)
        });
    }
}
