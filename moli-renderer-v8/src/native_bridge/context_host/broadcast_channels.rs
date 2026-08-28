use super::*;
use crate::{broadcast_channel_runtime::SharedBroadcastChannelRegistry, types::BroadcastChannelId};

impl JsContextHost {
    pub(crate) fn broadcast_channel_registry(&self) -> SharedBroadcastChannelRegistry {
        self.broadcast_channel_registry.clone()
    }

    pub(crate) fn close_owned_broadcast_channels(&mut self) {
        let channel_ids = self
            .broadcast_channel_wrappers
            .drain()
            .map(|(channel_id, _)| channel_id);
        for channel_id in channel_ids {
            self.broadcast_channel_registry
                .close_broadcast_channel(channel_id);
        }
    }

    pub(crate) fn close_broadcast_channels_for_child_context(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) {
        let channel_ids: Vec<_> = self
            .broadcast_channel_wrappers
            .iter()
            .filter_map(|(channel_id, entry)| {
                matches!(
                    entry.identity.dispatch_scope(),
                    OwnerDispatchScope::Child(child_handle) if child_handle == handle
                )
                .then_some(*channel_id)
            })
            .collect();
        for channel_id in channel_ids {
            self.broadcast_channel_wrappers.remove(&channel_id);
            self.broadcast_channel_registry
                .close_broadcast_channel(channel_id);
        }
    }

    pub(crate) fn close_broadcast_channels_for_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        let channel_ids = self
            .broadcast_channel_wrappers
            .iter()
            .filter_map(|(channel_id, entry)| {
                (entry.identity.owner() == owner).then_some(*channel_id)
            })
            .collect::<Vec<_>>();
        let closed_count = channel_ids.len();
        for channel_id in channel_ids {
            self.broadcast_channel_wrappers.remove(&channel_id);
            self.broadcast_channel_registry
                .close_broadcast_channel(channel_id);
        }
        closed_count
    }

    pub(crate) fn close_broadcast_channels_for_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        let channel_ids = self
            .broadcast_channel_wrappers
            .iter()
            .filter_map(|(channel_id, entry)| {
                (entry.identity.realm_token() == context_token).then_some(*channel_id)
            })
            .collect::<Vec<_>>();
        let closed_count = channel_ids.len();
        for channel_id in channel_ids {
            self.broadcast_channel_wrappers.remove(&channel_id);
            self.broadcast_channel_registry
                .close_broadcast_channel(channel_id);
        }
        closed_count
    }

    pub(crate) fn register_broadcast_channel_wrapper(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        channel_id: BroadcastChannelId,
        channel: v8::Local<'_, v8::Object>,
        identity: WindowExecutionContextIdentity,
    ) {
        self.broadcast_channel_wrappers.insert(
            channel_id,
            BroadcastChannelWrapperEntry {
                identity,
                context: v8::Global::new(scope, scope.get_current_context()),
                wrapper: v8::Global::new(scope, channel),
            },
        );
    }

    pub(crate) fn forget_broadcast_channel_wrapper(&mut self, channel_id: BroadcastChannelId) {
        self.broadcast_channel_wrappers.remove(&channel_id);
    }

    pub(crate) fn broadcast_channel_wrapper<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        channel_id: BroadcastChannelId,
    ) -> Option<(
        OwnerDispatchScope,
        RuntimeObservableContextToken,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        let stale_owner = self
            .broadcast_channel_wrappers
            .get(&channel_id)
            .map(|entry| entry.identity)
            .filter(|identity| !self.window_execution_context_identity_is_current(*identity));
        if let Some(identity) = stale_owner {
            self.broadcast_channel_wrappers.remove(&channel_id);
            self.broadcast_channel_registry
                .close_broadcast_channel(channel_id);
            tracing::debug!(
                ?channel_id,
                ?identity,
                "closed BroadcastChannel wrapper for retired execution context"
            );
            return None;
        }
        let entry = self.broadcast_channel_wrappers.get(&channel_id)?;
        Some((
            entry.identity.dispatch_scope(),
            entry.identity.realm_token(),
            v8::Local::new(scope, &entry.context),
            v8::Local::new(scope, &entry.wrapper),
        ))
    }

    /// Resolves a wrapper for a task already authorized by the Page arbiter.
    ///
    /// This deliberately does not rediscover currentness. It only verifies
    /// that the channel id still names the same captured binding; a channel
    /// closed before its selected turn is therefore a harmless no-op.
    pub(crate) fn authorized_broadcast_channel_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        channel_id: BroadcastChannelId,
        expected: WindowExecutionContextIdentity,
    ) -> Option<(
        OwnerDispatchScope,
        RuntimeObservableContextToken,
        v8::Local<'s, v8::Context>,
        v8::Local<'s, v8::Object>,
    )> {
        let entry = self.broadcast_channel_wrappers.get(&channel_id)?;
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
}
