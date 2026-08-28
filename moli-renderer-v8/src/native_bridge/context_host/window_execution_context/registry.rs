use super::super::{OwnerDispatchScope, RuntimeObservableContextToken};
pub(crate) use crate::browsing_context_model::RealmAccessPolicy as WindowExecutionContextAccessPolicy;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WindowExecutionContextOwner {
    Frame(crate::frame_owner_model::LocalWindowId),
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
    // A V8 context token identifies one concrete realm.
    pub(in crate::native_bridge::context_host) concrete_by_token:
        HashMap<RuntimeObservableContextToken, WindowExecutionContextScopedRealmRegistration>,
}

impl WindowExecutionContextRealmRecords {
    pub(in crate::native_bridge::context_host) fn registration(
        &self,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
    ) -> Option<WindowExecutionContextRealmRegistration> {
        self.concrete_by_token
            .get(&realm_token)
            .filter(|registered| registered.dispatch_scope == dispatch_scope)
            .map(|registered| registered.registration)
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
        match self.concrete_by_token.get(&realm_token) {
            Some(registered) if *registered != candidate => return Err(*registered),
            Some(_) => return Ok(()),
            None => {}
        }
        self.concrete_by_token.insert(realm_token, candidate);
        Ok(())
    }

    pub(in crate::native_bridge::context_host) fn remove(
        &mut self,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
    ) {
        if self
            .concrete_by_token
            .get(&realm_token)
            .is_some_and(|registered| registered.dispatch_scope == dispatch_scope)
        {
            self.concrete_by_token.remove(&realm_token);
        }
    }

    pub(in crate::native_bridge::context_host) fn remove_token(
        &mut self,
        realm_token: RuntimeObservableContextToken,
    ) -> usize {
        usize::from(self.concrete_by_token.remove(&realm_token).is_some())
    }

    pub(in crate::native_bridge::context_host) fn retire_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        self.concrete_by_token
            .retain(|_, registered| registered.registration.owner != owner);
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

    pub(in crate::native_bridge::context_host) fn access_policy(
        self,
    ) -> WindowExecutionContextAccessPolicy {
        self.access_policy
    }
}
