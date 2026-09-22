use super::*;
use crate::types::MessagePortId;

impl JsContextHost {
    pub(crate) fn message_port_registry(&self) -> SharedMessagePortRegistry {
        self.message_port_registry.clone()
    }

    pub(crate) fn close_owned_message_ports(&mut self) {
        let port_ids = self
            .message_port_wrappers
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for port_id in port_ids {
            self.retire_message_port(port_id);
        }
    }

    pub(crate) fn register_message_port_wrapper(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        port_id: MessagePortId,
        port: v8::Local<'_, v8::Object>,
        identity: WindowExecutionContextIdentity,
    ) {
        let previous = self.message_port_wrappers.insert(
            port_id,
            MessagePortWrapperEntry {
                identity,
                context: v8::Global::new(scope, scope.get_current_context()),
                wrapper: v8::Global::new(scope, port),
            },
        );
        if let Some(previous) = previous {
            tracing::warn!(
                port_id,
                previous_owner = ?previous.identity,
                current_owner = ?self
                    .message_port_wrappers
                    .get(&port_id)
                    .map(|entry| entry.identity),
                "replaced MessagePort wrapper without a transfer detach"
            );
        }
    }

    pub(crate) fn forget_message_port_wrapper(&mut self, port_id: MessagePortId) {
        self.message_port_wrappers.remove(&port_id);
    }

    pub(crate) fn message_port_wrapper<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        port_id: MessagePortId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.message_port_wrappers
            .get(&port_id)
            .map(|entry| v8::Local::new(scope, &entry.wrapper))
    }

    pub(crate) fn message_port_dispatch_target<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        port_id: MessagePortId,
    ) -> Option<(
        OwnerDispatchScope,
        RuntimeObservableContextToken,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        let stale_owner = self
            .message_port_wrappers
            .get(&port_id)
            .map(|entry| entry.identity)
            .filter(|identity| !self.window_execution_context_identity_is_current(*identity));
        if let Some(identity) = stale_owner {
            self.retire_message_port(port_id);
            tracing::debug!(
                port_id,
                ?identity,
                "closed MessagePort for retired execution context"
            );
            return None;
        }
        let entry = self.message_port_wrappers.get(&port_id)?;
        Some((
            entry.identity.dispatch_scope(),
            entry.identity.realm_token(),
            v8::Local::new(scope, &entry.context),
            v8::Local::new(scope, &entry.wrapper),
        ))
    }

    pub(crate) fn message_port_execution_context_identity(
        &self,
        port_id: MessagePortId,
    ) -> Option<WindowExecutionContextIdentity> {
        self.message_port_wrappers
            .get(&port_id)
            .map(|entry| entry.identity)
    }

    /// Resolve a wrapper only after the Page arbiter has matched the exact
    /// attachment identity captured by the selected task.
    pub(crate) fn authorized_message_port_dispatch_target<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        port_id: MessagePortId,
        expected: WindowExecutionContextIdentity,
    ) -> Option<(
        OwnerDispatchScope,
        RuntimeObservableContextToken,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        let entry = self.message_port_wrappers.get(&port_id)?;
        if entry.identity != expected {
            return None;
        }
        Some((
            entry.identity.dispatch_scope(),
            entry.identity.realm_token(),
            v8::Local::new(scope, &entry.context),
            v8::Local::new(scope, &entry.wrapper),
        ))
    }

    pub(crate) fn retire_message_ports_for_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        let port_ids = self
            .message_port_wrappers
            .iter()
            .filter_map(|(port_id, entry)| (entry.identity.owner() == owner).then_some(*port_id))
            .collect::<Vec<_>>();
        let retired_count = port_ids.len();
        for port_id in port_ids {
            self.retire_message_port(port_id);
        }
        retired_count
    }

    pub(crate) fn retire_message_ports_for_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        let port_ids = self
            .message_port_wrappers
            .iter()
            .filter_map(|(port_id, entry)| {
                (entry.identity.realm_token() == context_token).then_some(*port_id)
            })
            .collect::<Vec<_>>();
        let retired_count = port_ids.len();
        for port_id in port_ids {
            self.retire_message_port(port_id);
        }
        retired_count
    }

    pub(crate) fn retire_message_port(&mut self, port_id: MessagePortId) -> bool {
        let Some(_) = self.message_port_wrappers.remove(&port_id) else {
            return false;
        };
        self.message_port_registry.close_message_port(port_id);
        true
    }

    #[cfg(test)]
    pub(crate) fn message_port_execution_context_owners_for_test(
        &self,
    ) -> Vec<(
        MessagePortId,
        WindowExecutionContextOwner,
        RuntimeObservableContextToken,
    )> {
        let mut owners = self
            .message_port_wrappers
            .iter()
            .map(|(port_id, entry)| {
                (
                    *port_id,
                    entry.identity.owner(),
                    entry.identity.realm_token(),
                )
            })
            .collect::<Vec<_>>();
        owners.sort_by_key(|(port_id, _, _)| *port_id);
        owners
    }
}
