use super::*;

impl CdpConnection {
    pub(crate) fn validate_runtime_remote_object_ids_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_ids: &[String],
    ) -> Result<(), String> {
        if object_ids.is_empty() {
            return Ok(());
        }
        let Some(owner) = self.runtime_remote_object_owner_identity_for_session(session_id) else {
            return Ok(());
        };
        for object_id in object_ids {
            // V8 remote object ids are scoped to an Inspector session. Two
            // sessions connected to the same context can therefore emit the
            // same wire id for different objects. Prefer the current
            // session's registration before using the cross-owner check to
            // reject a handle borrowed from another session.
            if self.runtime_remote_object_id_known_for_session_owner(session_id, object_id) {
                continue;
            }
            if self.runtime_remote_object_id_known_for_different_owner(&owner, object_id) {
                return Err("Cannot find object with given id".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn validate_runtime_remote_object_ids_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_ids: &[String],
    ) -> Result<(), String> {
        if owner.session_id().is_some() {
            return self.validate_runtime_remote_object_ids_for_session_owner(
                owner.session_id(),
                object_ids,
            );
        }
        if object_ids.is_empty() {
            return Ok(());
        }
        let Some(owner_identity) = self.runtime_remote_object_owner_identity_for_owner(owner)
        else {
            return Ok(());
        };
        for object_id in object_ids {
            if self.runtime_remote_object_id_known_for_owner(owner, object_id) {
                continue;
            }
            if self.runtime_remote_object_id_known_for_different_owner(&owner_identity, object_id) {
                return Err("Cannot find object with given id".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn runtime_remote_object_id_known_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> bool {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target.has_runtime_remote_object_id(owner_session_id, object_id);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target.has_runtime_remote_object_id(owner_session_id, object_id);
        }
        self.target_devtools_session_state_for_session(session_id)
            .is_some_and(|state| state.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn runtime_remote_object_id_known_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> bool {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_id_known_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_owner(owner)
            .is_some_and(|state| state.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        value: &Value,
    ) {
        let object_ids = runtime_remote_object_ids_in_value(value);
        self.register_runtime_remote_object_ids_for_session_owner(session_id, object_ids);
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        value: &Value,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_ids_from_value_for_session_owner(
                owner.session_id(),
                value,
            );
            return;
        }
        let object_ids = runtime_remote_object_ids_in_value(value);
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.register_runtime_remote_object_ids(object_ids)
        });
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
        &mut self,
        session_id: Option<&str>,
        value: &Value,
        object_group: &str,
    ) {
        let object_ids = runtime_remote_object_ids_in_value(value);
        self.register_runtime_remote_object_ids_for_session_owner_with_group(
            session_id,
            object_ids,
            object_group,
        );
    }

    pub(crate) fn register_runtime_remote_object_ids_from_value_for_owner_with_group(
        &mut self,
        owner: &CommandOwnerScope,
        value: &Value,
        object_group: &str,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_ids_from_value_for_session_owner_with_group(
                owner.session_id(),
                value,
                object_group,
            );
            return;
        }
        let object_ids = runtime_remote_object_ids_in_value(value);
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.register_runtime_remote_object_ids_with_group(object_ids, object_group)
        });
    }

    pub(crate) fn runtime_remote_object_group_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> Option<String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_group(owner_session_id, object_id)
                .map(str::to_owned);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_group(owner_session_id, object_id)
                .map(str::to_owned);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .runtime_remote_object_group(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_group_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> Option<String> {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_group_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_owner(owner)?
            .runtime_remote_object_group(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_realm_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> Option<String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_realm(owner_session_id, object_id)
                .map(str::to_owned);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_realm(owner_session_id, object_id)
                .map(str::to_owned);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .runtime_remote_object_realm(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_realm_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> Option<String> {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_realm_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_owner(owner)?
            .runtime_remote_object_realm(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_alias_for_session_owner(
        &self,
        session_id: Option<&str>,
        object_id: &str,
    ) -> Option<String> {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_alias(owner_session_id, object_id)
                .map(str::to_owned);
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(session_id)
        {
            return target
                .runtime_remote_object_alias(owner_session_id, object_id)
                .map(str::to_owned);
        }
        self.target_devtools_session_state_for_session(session_id)?
            .runtime_remote_object_alias(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn runtime_remote_object_alias_for_owner(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
    ) -> Option<String> {
        if owner.session_id().is_some() {
            return self
                .runtime_remote_object_alias_for_session_owner(owner.session_id(), object_id);
        }
        self.target_devtools_session_state_for_owner(owner)?
            .runtime_remote_object_alias(object_id)
            .map(str::to_owned)
    }

    pub(crate) fn register_runtime_remote_object_alias_for_owner_with_realm(
        &mut self,
        owner: &CommandOwnerScope,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_alias_for_session_owner_with_realm(
                owner.session_id(),
                alias_id,
                object_id,
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.register_runtime_remote_object_alias_with_realm(alias_id, object_id, realm_id);
        });
    }

    pub(crate) fn unregister_runtime_remote_object_ids_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        object_ids: &[String],
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_ids(owner_session_id, object_ids);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_ids(owner_session_id, object_ids);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.unregister_runtime_remote_object_ids(object_ids);
        });
    }

    pub(crate) fn unregister_runtime_remote_object_ids_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        object_ids: &[String],
    ) {
        if owner.session_id().is_some() {
            self.unregister_runtime_remote_object_ids_for_session_owner(
                owner.session_id(),
                object_ids,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.unregister_runtime_remote_object_ids(object_ids)
        });
    }

    pub(crate) fn unregister_runtime_remote_object_group_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        object_group: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_group(owner_session_id, object_group);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.unregister_runtime_remote_object_group(owner_session_id, object_group);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.unregister_runtime_remote_object_group(object_group);
        });
    }

    pub(crate) fn unregister_runtime_remote_object_group_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        object_group: &str,
    ) {
        if owner.session_id().is_some() {
            self.unregister_runtime_remote_object_group_for_session_owner(
                owner.session_id(),
                object_group,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.unregister_runtime_remote_object_group(object_group)
        });
    }

    pub(crate) fn clear_runtime_remote_object_tracking_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_object_tracking(owner_session_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_object_tracking(owner_session_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.clear_runtime_remote_object_tracking();
        });
    }

    pub(crate) fn clear_runtime_remote_object_tracking_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) {
        if owner.session_id().is_some() {
            self.clear_runtime_remote_object_tracking_for_session_owner(owner.session_id());
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(
            owner,
            DevToolsSessionState::clear_runtime_remote_object_tracking,
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

    pub(crate) fn clear_runtime_remote_objects_for_realm_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        realm_id: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_objects_for_realm(owner_session_id, realm_id);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.clear_runtime_remote_objects_for_realm(owner_session_id, realm_id);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.clear_runtime_remote_objects_for_realm(realm_id);
        });
    }

    pub(crate) fn clear_runtime_remote_objects_for_realm_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.clear_runtime_remote_objects_for_realm_for_session_owner(
                owner.session_id(),
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.clear_runtime_remote_objects_for_realm(realm_id)
        });
    }

    pub(crate) fn register_runtime_remote_object_ids_for_session_owner_with_realm(
        &mut self,
        session_id: Option<&str>,
        object_ids: Vec<String>,
        realm_id: &str,
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_realm(
                owner_session_id,
                object_ids,
                realm_id,
            );
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_realm(
                owner_session_id,
                object_ids,
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_ids_with_realm(object_ids, realm_id);
        });
    }

    pub(crate) fn register_runtime_remote_object_ids_for_owner_with_realm(
        &mut self,
        owner: &CommandOwnerScope,
        object_ids: Vec<String>,
        realm_id: &str,
    ) {
        if owner.session_id().is_some() {
            self.register_runtime_remote_object_ids_for_session_owner_with_realm(
                owner.session_id(),
                object_ids,
                realm_id,
            );
            return;
        }
        if object_ids.is_empty() {
            return;
        }
        let _ = self.with_target_devtools_session_state_for_owner_mut(owner, |state| {
            state.register_runtime_remote_object_ids_with_realm(object_ids, realm_id)
        });
    }

    pub(crate) fn register_runtime_remote_object_alias_for_session_owner_with_realm(
        &mut self,
        session_id: Option<&str>,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_alias_with_realm(
                owner_session_id,
                alias_id,
                object_id,
                realm_id,
            );
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_alias_with_realm(
                owner_session_id,
                alias_id,
                object_id,
                realm_id,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_alias_with_realm(alias_id, object_id, realm_id);
        });
    }

    pub(super) fn register_runtime_remote_object_ids_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        object_ids: Vec<String>,
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_for_session(owner_session_id, object_ids);
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_for_session(owner_session_id, object_ids);
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_ids(object_ids);
        });
    }

    pub(super) fn register_runtime_remote_object_ids_for_session_owner_with_group(
        &mut self,
        session_id: Option<&str>,
        object_ids: Vec<String>,
        object_group: &str,
    ) {
        if object_ids.is_empty() {
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.shared_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_group(
                owner_session_id,
                object_ids,
                object_group,
            );
            return;
        }
        if let Some(owner_session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(session_id)
        {
            target.register_runtime_remote_object_ids_with_group(
                owner_session_id,
                object_ids,
                object_group,
            );
            return;
        }
        let _ = self.with_target_devtools_session_state_for_session_mut(session_id, |state| {
            state.register_runtime_remote_object_ids_with_group(object_ids, object_group);
        });
    }

    pub(super) fn runtime_remote_object_owner_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<RuntimeRemoteObjectOwnerIdentity> {
        if let Some(CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        }) = self.session_route(session_id)
        {
            let target = self
                .browser_context_by_id(&browser_context_id)?
                .shared_worker_target(&target_id)?;
            return Some(RuntimeRemoteObjectOwnerIdentity::SharedWorker {
                browser_context_id,
                instance_id: target.renderer_instance_id,
                session_id: session_id?.to_owned(),
            });
        }
        if let Some(CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        }) = self.session_route(session_id)
        {
            let target = self
                .browser_context_by_id(&browser_context_id)?
                .dedicated_worker_target(&target_id)?;
            return Some(RuntimeRemoteObjectOwnerIdentity::DedicatedWorker {
                browser_context_id,
                instance_id: target.renderer_instance_id,
                session_id: session_id?.to_owned(),
            });
        }
        if let Some(CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        }) = self.session_route(session_id)
        {
            let target = self
                .browser_context_by_id(&browser_context_id)?
                .service_worker_target(&target_id)?;
            return Some(RuntimeRemoteObjectOwnerIdentity::ServiceWorker {
                browser_context_id,
                version_id: target.renderer_version_id,
                session_id: session_id?.to_owned(),
            });
        }
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
        let devtools_session_id =
            self.target_devtools_attached_session_id_for_session(session_id)?;
        Some(RuntimeRemoteObjectOwnerIdentity::Page {
            browser_context_id,
            target_id,
            devtools_session_id,
        })
    }

    pub(super) fn runtime_remote_object_owner_identity_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<RuntimeRemoteObjectOwnerIdentity> {
        if owner.session_id().is_some() {
            return self.runtime_remote_object_owner_identity_for_session(owner.session_id());
        }
        let (browser_context_id, target_id) = self.target_owner_identity_for_owner(owner)?;
        Some(RuntimeRemoteObjectOwnerIdentity::Page {
            browser_context_id,
            target_id,
            devtools_session_id: None,
        })
    }

    pub(super) fn runtime_remote_object_id_known_for_different_owner(
        &self,
        owner: &RuntimeRemoteObjectOwnerIdentity,
        object_id: &str,
    ) -> bool {
        for browser_context in self.browser_contexts() {
            let current_page_owner = match owner {
                RuntimeRemoteObjectOwnerIdentity::Page {
                    browser_context_id,
                    target_id,
                    devtools_session_id,
                } if browser_context_id == &browser_context.id => Some((
                    target_id
                        .as_deref()
                        .or_else(|| browser_context.active_target_id()),
                    devtools_session_id.as_deref(),
                )),
                _ => None,
            };
            if browser_context.page_targets.iter().any(|target| {
                if current_page_owner
                    .is_some_and(|(target_id, _)| target_id == Some(target.target_id()))
                {
                    target.has_runtime_remote_object_id_for_different_session(
                        current_page_owner.and_then(|(_, session_id)| session_id),
                        object_id,
                    )
                } else {
                    target.has_runtime_remote_object_id(object_id)
                }
            }) {
                return true;
            }

            for target in browser_context.shared_worker_targets.values() {
                let shared_worker_is_current_owner = matches!(
                    owner,
                    RuntimeRemoteObjectOwnerIdentity::SharedWorker {
                        browser_context_id,
                        instance_id,
                        session_id,
                    } if browser_context_id == &browser_context.id
                        && instance_id == &target.renderer_instance_id
                        && target.is_session(session_id)
                );
                if !shared_worker_is_current_owner
                    && target.any_session_has_runtime_remote_object_id(object_id)
                {
                    return true;
                }
            }
            for target in browser_context.dedicated_worker_targets.values() {
                let dedicated_worker_is_current_owner = matches!(
                    owner,
                    RuntimeRemoteObjectOwnerIdentity::DedicatedWorker {
                        browser_context_id,
                        instance_id,
                        session_id,
                    } if browser_context_id == &browser_context.id
                        && *instance_id == target.renderer_instance_id
                        && target.is_session(session_id)
                );
                if !dedicated_worker_is_current_owner
                    && target.any_session_has_runtime_remote_object_id(object_id)
                {
                    return true;
                }
            }
            for target in browser_context.service_worker_targets.values() {
                let service_worker_is_current_owner = matches!(
                    owner,
                    RuntimeRemoteObjectOwnerIdentity::ServiceWorker {
                        browser_context_id,
                        version_id,
                        session_id,
                    } if browser_context_id == &browser_context.id
                        && *version_id == target.renderer_version_id
                        && target.is_session(session_id)
                );
                if !service_worker_is_current_owner
                    && target.any_session_has_runtime_remote_object_id(object_id)
                {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) async fn release_worker_runtime_remote_objects_for_session_best_effort_async(
        &mut self,
        session_id: &str,
    ) {
        let service_worker = matches!(
            self.session_route(Some(session_id)),
            Some(CdpSessionRoute::ServiceWorkerTarget { .. })
        );
        let cleanup_plan = if service_worker {
            self.service_worker_target_for_session_mut(Some(session_id))
                .map(|target| target.take_runtime_remote_object_cleanup_plan(session_id))
        } else {
            self.shared_worker_target_for_session_mut(Some(session_id))
                .map(|target| target.take_runtime_remote_object_cleanup_plan(session_id))
        };
        let Some((object_groups, object_ids)) = cleanup_plan else {
            return;
        };
        if object_groups.is_empty() && object_ids.is_empty() {
            return;
        }

        let mut command_id = SHARED_WORKER_RUNTIME_REMOTE_OBJECT_CLEANUP_COMMAND_ID_BASE;
        for object_group in object_groups {
            let raw_json = json!({
                "id": command_id,
                "method": "Runtime.releaseObjectGroup",
                "params": { "objectGroup": object_group }
            })
            .to_string();
            let release = if service_worker {
                self.dispatch_service_worker_runtime_helper_protocol_message_for_session_async(
                    Some(session_id),
                    &raw_json,
                    command_id,
                )
                .await
            } else {
                self.dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
                    Some(session_id),
                    &raw_json,
                    command_id,
                )
                .await
            };
            if let Err(error) = release {
                tracing::warn!(
                    object_group = %object_group,
                    error = %error,
                    "failed to release worker Runtime object group during target detach"
                );
            }
            command_id = command_id.saturating_add(1);
        }
        for object_id in object_ids {
            let raw_json = json!({
                "id": command_id,
                "method": "Runtime.releaseObject",
                "params": { "objectId": object_id }
            })
            .to_string();
            let release = if service_worker {
                self.dispatch_service_worker_runtime_helper_protocol_message_for_session_async(
                    Some(session_id),
                    &raw_json,
                    command_id,
                )
                .await
            } else {
                self.dispatch_shared_worker_runtime_helper_protocol_message_for_session_async(
                    Some(session_id),
                    &raw_json,
                    command_id,
                )
                .await
            };
            if let Err(error) = release {
                tracing::warn!(
                    object_id = %object_id,
                    error = %error,
                    "failed to release worker Runtime object during target detach"
                );
            }
            command_id = command_id.saturating_add(1);
        }
    }
}
