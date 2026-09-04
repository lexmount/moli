use super::super::{
    LightweightPopupLocalWindowId, OwnerDispatchScope, RuntimeObservableContextToken,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowExecutionContextOwner {
    Frame(crate::frame_owner_model::LocalWindowId),
    LightweightPopup {
        popup_id: u64,
        local_window_id: LightweightPopupLocalWindowId,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum WindowExecutionContextAccessPolicy {
    #[default]
    EnforceWebOrigin,
    Universal,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::native_bridge::context_host) struct WindowExecutionContextRealmRegistration {
    pub(in crate::native_bridge::context_host) owner: WindowExecutionContextOwner,
    pub(in crate::native_bridge::context_host) access_policy: WindowExecutionContextAccessPolicy,
}

impl WindowExecutionContextRealmRegistration {
    pub(in crate::native_bridge::context_host) fn new(
        owner: WindowExecutionContextOwner,
        access_policy: WindowExecutionContextAccessPolicy,
    ) -> Self {
        Self {
            owner,
            access_policy,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::native_bridge::context_host) struct WindowExecutionContextScopedRealmRegistration {
    pub(in crate::native_bridge::context_host) dispatch_scope: OwnerDispatchScope,
    pub(in crate::native_bridge::context_host) registration:
        WindowExecutionContextRealmRegistration,
}

impl WindowExecutionContextScopedRealmRegistration {
    pub(in crate::native_bridge::context_host) fn new(
        dispatch_scope: OwnerDispatchScope,
        registration: WindowExecutionContextRealmRegistration,
    ) -> Self {
        Self {
            dispatch_scope,
            registration,
        }
    }
}

#[derive(Default)]
pub(in crate::native_bridge::context_host) struct WindowExecutionContextRealmRecords {
    // A V8 context token identifies one concrete realm. Lightweight popups
    // currently share their opener's concrete context, so their entries are
    // explicitly scoped aliases rather than competing concrete registrations.
    pub(in crate::native_bridge::context_host) concrete_by_token:
        HashMap<RuntimeObservableContextToken, WindowExecutionContextScopedRealmRegistration>,
    lightweight_popup_aliases: HashMap<
        (OwnerDispatchScope, RuntimeObservableContextToken),
        WindowExecutionContextRealmRegistration,
    >,
}

impl WindowExecutionContextRealmRecords {
    pub(in crate::native_bridge::context_host) fn registration(
        &self,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
    ) -> Option<WindowExecutionContextRealmRegistration> {
        match dispatch_scope {
            OwnerDispatchScope::LightweightPopup(_) => self
                .lightweight_popup_aliases
                .get(&(dispatch_scope, realm_token))
                .copied(),
            OwnerDispatchScope::Top | OwnerDispatchScope::Child(_) => self
                .concrete_by_token
                .get(&realm_token)
                .filter(|registered| registered.dispatch_scope == dispatch_scope)
                .map(|registered| registered.registration),
        }
    }

    pub(in crate::native_bridge::context_host) fn concrete_registration(
        &self,
        realm_token: RuntimeObservableContextToken,
    ) -> Option<WindowExecutionContextScopedRealmRegistration> {
        self.concrete_by_token.get(&realm_token).copied()
    }

    pub(in crate::native_bridge::context_host) fn register(
        &mut self,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
        registration: WindowExecutionContextRealmRegistration,
    ) -> Result<(), WindowExecutionContextScopedRealmRegistration> {
        let candidate =
            WindowExecutionContextScopedRealmRegistration::new(dispatch_scope, registration);
        match dispatch_scope {
            OwnerDispatchScope::LightweightPopup(_) => {
                match self
                    .lightweight_popup_aliases
                    .get(&(dispatch_scope, realm_token))
                {
                    Some(registered) if *registered != registration => {
                        return Err(WindowExecutionContextScopedRealmRegistration::new(
                            dispatch_scope,
                            *registered,
                        ));
                    }
                    Some(_) => return Ok(()),
                    None => {}
                }
                self.lightweight_popup_aliases
                    .insert((dispatch_scope, realm_token), registration);
            }
            OwnerDispatchScope::Top | OwnerDispatchScope::Child(_) => {
                match self.concrete_by_token.get(&realm_token) {
                    Some(registered) if *registered != candidate => return Err(*registered),
                    Some(_) => return Ok(()),
                    None => {}
                }
                self.concrete_by_token.insert(realm_token, candidate);
            }
        }
        Ok(())
    }

    pub(in crate::native_bridge::context_host) fn remove(
        &mut self,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
    ) {
        match dispatch_scope {
            OwnerDispatchScope::LightweightPopup(_) => {
                self.lightweight_popup_aliases
                    .remove(&(dispatch_scope, realm_token));
            }
            OwnerDispatchScope::Top | OwnerDispatchScope::Child(_) => {
                if self
                    .concrete_by_token
                    .get(&realm_token)
                    .is_some_and(|registered| registered.dispatch_scope == dispatch_scope)
                {
                    self.concrete_by_token.remove(&realm_token);
                }
            }
        }
    }

    pub(in crate::native_bridge::context_host) fn remove_token(
        &mut self,
        realm_token: RuntimeObservableContextToken,
    ) -> usize {
        let concrete_count = usize::from(self.concrete_by_token.remove(&realm_token).is_some());
        let alias_count_before = self.lightweight_popup_aliases.len();
        self.lightweight_popup_aliases
            .retain(|(_, token), _| *token != realm_token);
        concrete_count + alias_count_before.saturating_sub(self.lightweight_popup_aliases.len())
    }

    pub(in crate::native_bridge::context_host) fn retire_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        self.concrete_by_token
            .retain(|_, registered| registered.registration.owner != owner);
        self.lightweight_popup_aliases
            .retain(|_, registered| registered.owner != owner);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct WindowExecutionContextIdentity {
    owner: WindowExecutionContextOwner,
    dispatch_scope: OwnerDispatchScope,
    realm_token: RuntimeObservableContextToken,
    access_policy: WindowExecutionContextAccessPolicy,
}

impl WindowExecutionContextIdentity {
    pub(crate) fn new(
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
        access_policy: WindowExecutionContextAccessPolicy,
    ) -> Self {
        Self {
            owner,
            dispatch_scope,
            realm_token,
            access_policy,
        }
    }

    pub(crate) fn owner(self) -> WindowExecutionContextOwner {
        self.owner
    }

    pub(crate) fn dispatch_scope(self) -> OwnerDispatchScope {
        self.dispatch_scope
    }

    pub(crate) fn realm_token(self) -> RuntimeObservableContextToken {
        self.realm_token
    }

    pub(crate) fn grants_universal_access(self) -> bool {
        self.access_policy == WindowExecutionContextAccessPolicy::Universal
    }

    /// Encodes an exact callback-realm identity for storage in an internal V8
    /// private slot. Lightweight popup realms alias their opener's V8 context,
    /// so retaining only that context would lose the LocalWindow generation
    /// and dispatch address captured when the callback was converted.
    pub(crate) fn serialize_for_internal_slot(self) -> String {
        let policy = u8::from(self.grants_universal_access());
        match (self.owner, self.dispatch_scope) {
            (WindowExecutionContextOwner::Frame(local_window_id), OwnerDispatchScope::Top) => {
                format!(
                    "frame:{}:top:{}:{policy}",
                    local_window_id.0,
                    self.realm_token.as_u64()
                )
            }
            (
                WindowExecutionContextOwner::Frame(local_window_id),
                OwnerDispatchScope::Child(child_handle),
            ) => format!(
                "frame:{}:child:{}:{}:{policy}",
                local_window_id.0,
                child_handle.index(),
                self.realm_token.as_u64()
            ),
            (
                WindowExecutionContextOwner::LightweightPopup {
                    popup_id,
                    local_window_id,
                },
                OwnerDispatchScope::LightweightPopup(dispatch_popup_id),
            ) => {
                debug_assert_eq!(popup_id, dispatch_popup_id);
                format!(
                    "popup:{popup_id}:{}:{}:{policy}",
                    local_window_id.as_u64(),
                    self.realm_token.as_u64()
                )
            }
            _ => unreachable!("Window execution-context owner and dispatch scope diverged"),
        }
    }

    pub(crate) fn deserialize_from_internal_slot(serialized: &str) -> Option<Self> {
        fn access_policy(value: &str) -> Option<WindowExecutionContextAccessPolicy> {
            match value {
                "0" => Some(WindowExecutionContextAccessPolicy::EnforceWebOrigin),
                "1" => Some(WindowExecutionContextAccessPolicy::Universal),
                _ => None,
            }
        }

        let parts = serialized.split(':').collect::<Vec<_>>();
        match parts.as_slice() {
            ["frame", local_window_id, "top", realm_token, policy] => Some(Self::new(
                WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                    local_window_id.parse().ok()?,
                )),
                OwnerDispatchScope::Top,
                RuntimeObservableContextToken::from_raw(realm_token.parse().ok()?),
                access_policy(policy)?,
            )),
            [
                "frame",
                local_window_id,
                "child",
                child_handle,
                realm_token,
                policy,
            ] => {
                let child_handle = child_handle.parse::<u64>().ok()?;
                let child_handle = usize::try_from(child_handle).ok()?;
                Some(Self::new(
                    WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                        local_window_id.parse().ok()?,
                    )),
                    OwnerDispatchScope::Child(crate::document_runtime::DomHandle::new(
                        child_handle,
                    )),
                    RuntimeObservableContextToken::from_raw(realm_token.parse().ok()?),
                    access_policy(policy)?,
                ))
            }
            ["popup", popup_id, local_window_id, realm_token, policy] => {
                let popup_id = popup_id.parse().ok()?;
                Some(Self::new(
                    WindowExecutionContextOwner::LightweightPopup {
                        popup_id,
                        local_window_id: LightweightPopupLocalWindowId::new(
                            local_window_id.parse().ok()?,
                        ),
                    },
                    OwnerDispatchScope::LightweightPopup(popup_id),
                    RuntimeObservableContextToken::from_raw(realm_token.parse().ok()?),
                    access_policy(policy)?,
                ))
            }
            _ => None,
        }
    }

    pub(in crate::native_bridge::context_host) fn access_policy(
        self,
    ) -> WindowExecutionContextAccessPolicy {
        self.access_policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_identity_internal_slot_encoding_round_trips_every_owner_shape() {
        let identities = [
            WindowExecutionContextIdentity::new(
                WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(7)),
                OwnerDispatchScope::Top,
                RuntimeObservableContextToken::from_raw(11),
                WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            ),
            WindowExecutionContextIdentity::new(
                WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(13)),
                OwnerDispatchScope::Child(crate::document_runtime::DomHandle::new(17)),
                RuntimeObservableContextToken::from_raw(19),
                WindowExecutionContextAccessPolicy::Universal,
            ),
            WindowExecutionContextIdentity::new(
                WindowExecutionContextOwner::LightweightPopup {
                    popup_id: 23,
                    local_window_id: LightweightPopupLocalWindowId::new(29),
                },
                OwnerDispatchScope::LightweightPopup(23),
                RuntimeObservableContextToken::from_raw(31),
                WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            ),
        ];

        for identity in identities {
            assert_eq!(
                WindowExecutionContextIdentity::deserialize_from_internal_slot(
                    &identity.serialize_for_internal_slot()
                ),
                Some(identity)
            );
        }
        assert_eq!(
            WindowExecutionContextIdentity::deserialize_from_internal_slot("popup:23:29:31:2"),
            None
        );
        assert_eq!(
            WindowExecutionContextIdentity::deserialize_from_internal_slot("not-an-identity"),
            None
        );
    }
}
