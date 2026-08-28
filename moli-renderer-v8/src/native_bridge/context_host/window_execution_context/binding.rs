use super::super::{
    JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken,
    current_runtime_observable_context_token,
};
use super::{WindowExecutionContextIdentity, WindowExecutionContextOwner};

/// Stable registry address for one Window realm registration.
///
/// The registry remains authoritative for the access policy. Keeping policy
/// out of this locator prevents copied bindings from becoming a second source
/// of truth. Every registered owner now names a concrete Window realm; related
/// auxiliary contexts own their own Page and therefore enter through that
/// Page's registry instead of aliasing the opener.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct WindowExecutionContextLocator {
    owner: WindowExecutionContextOwner,
    dispatch_scope: OwnerDispatchScope,
    realm_token: RuntimeObservableContextToken,
}

impl WindowExecutionContextLocator {
    const fn new(
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
    ) -> Self {
        Self {
            owner,
            dispatch_scope,
            realm_token,
        }
    }

    const fn owner(self) -> WindowExecutionContextOwner {
        self.owner
    }

    const fn dispatch_scope(self) -> OwnerDispatchScope {
        self.dispatch_scope
    }

    pub(super) const fn realm_token(self) -> RuntimeObservableContextToken {
        self.realm_token
    }

    fn resolve_identity(self, host: &JsContextHost) -> Option<WindowExecutionContextIdentity> {
        if !host.window_execution_context_owner_is_current(self.owner, self.dispatch_scope) {
            return None;
        }
        let registration = host
            .window_execution_context_realms
            .registration(self.dispatch_scope, self.realm_token)?;
        (registration.owner == self.owner).then(|| {
            WindowExecutionContextIdentity::new(
                self.owner,
                self.dispatch_scope,
                self.realm_token,
                registration.access_policy,
            )
        })
    }
}

/// Strict binding to one registered Window realm.
///
/// Unlike a WindowProxy facade this value cannot be projected to another
/// owner. Consumers that also need an operation target must carry that target
/// separately and validate the relationship when accepting the operation.
#[derive(Clone)]
pub(crate) struct WindowExecutionContextBinding {
    locator: WindowExecutionContextLocator,
    context: v8::Global<v8::Context>,
}

impl WindowExecutionContextBinding {
    pub(crate) fn new(
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
        realm_token: RuntimeObservableContextToken,
        context: v8::Global<v8::Context>,
    ) -> Self {
        Self {
            locator: WindowExecutionContextLocator::new(owner, dispatch_scope, realm_token),
            context,
        }
    }

    pub(crate) fn owner(&self) -> WindowExecutionContextOwner {
        self.locator.owner()
    }

    pub(crate) fn dispatch_scope(&self) -> OwnerDispatchScope {
        self.locator.dispatch_scope()
    }

    pub(crate) fn realm_token(&self) -> RuntimeObservableContextToken {
        self.locator.realm_token()
    }

    pub(super) fn locator(&self) -> WindowExecutionContextLocator {
        self.locator
    }

    pub(crate) fn context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.context)
    }

    pub(crate) fn context_global(&self) -> &v8::Global<v8::Context> {
        &self.context
    }

    pub(crate) fn duplicate(&self, scope: &mut v8::PinScope<'_, '_>) -> Self {
        let context = self.context(scope);
        let context = v8::Global::new(scope, context);
        Self {
            locator: self.locator,
            context,
        }
    }

    pub(crate) fn resolve_identity(
        &self,
        host: &JsContextHost,
    ) -> Option<WindowExecutionContextIdentity> {
        self.locator.resolve_identity(host)
    }

    pub(crate) fn is_current(&self, host: &JsContextHost) -> bool {
        self.resolve_identity(host).is_some()
    }

    pub(crate) fn with_current_scope<R>(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, OwnerDispatchScope) -> R,
    ) -> Option<R> {
        // The dispatch slot owns the host Rc for this call. Use its stable raw
        // pointer only long enough to validate before author code can run; a
        // RefCell guard cannot cross `op`, because an event listener may
        // synchronously reenter the same host.
        let identity = self.resolve_identity(unsafe { &*host_ptr })?;
        let context = self.context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        // A retained v8::Global can outlive registry retirement. Also verify
        // that the entered context still carries the exact token resolved
        // above, so a replacement realm can never inherit this delivery.
        if current_runtime_observable_context_token(scope) != Some(identity.realm_token()) {
            return None;
        }
        let dispatch_scope = identity.dispatch_scope();
        let previous_dispatch_scope = dispatch_scope.enter(scope);
        let result = op(scope, dispatch_scope);
        dispatch_scope.restore(scope, previous_dispatch_scope);
        Some(result)
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        WindowExecutionContextOwner,
        OwnerDispatchScope,
        RuntimeObservableContextToken,
        v8::Global<v8::Context>,
    ) {
        (
            self.locator.owner(),
            self.locator.dispatch_scope(),
            self.locator.realm_token(),
            self.context,
        )
    }
}
