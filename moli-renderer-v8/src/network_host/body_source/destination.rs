use crate::native_bridge::{
    LightweightPopupLocalWindowId, OwnerDispatchScope, RuntimeObservableContextToken,
    WindowExecutionContextAccessPolicy, WindowExecutionContextIdentity,
    WindowExecutionContextOwner,
};

/// A concrete Window owns Fetch completion, including popups that share their
/// opener's V8 context. Pending reads trace this destination through V8 rather
/// than retaining another native Context root after realm retirement.
#[derive(Clone)]
pub(super) struct BodyTaskDestination {
    context: v8::Global<v8::Context>,
    owner: Option<WindowExecutionContextIdentity>,
}

impl BodyTaskDestination {
    pub(super) fn capture<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Self {
        let context = object
            .get_creation_context(scope)
            .expect("Body receiver realm");
        let owner = crate::util::context_host_ptr_from_global_bridge(scope).and_then(|host| {
            unsafe { &*host }.window_execution_context_identity_for_v8_context(scope, context)
        });
        Self {
            context: v8::Global::new(scope, context),
            owner,
        }
    }

    pub(super) fn context<'s>(&self, scope: &v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.context)
    }

    pub(super) fn to_value<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Object> {
        let context = self.context(scope);
        let anchor = context.get_extras_binding_object(scope);
        let (kind, address, window, realm, universal) =
            self.owner.map_or((0, 0, 0, 0, false), |owner| {
                let (kind, address, window) = match (owner.owner(), owner.dispatch_scope()) {
                    (WindowExecutionContextOwner::Frame(window), OwnerDispatchScope::Top) => {
                        (1, 0, window.0)
                    }
                    (
                        WindowExecutionContextOwner::Frame(window),
                        OwnerDispatchScope::Child(child),
                    ) => (2, child.index() as u64, window.0),
                    (
                        WindowExecutionContextOwner::LightweightPopup {
                            popup_id,
                            local_window_id,
                        },
                        OwnerDispatchScope::LightweightPopup(address),
                    ) if popup_id == address => (3, address, local_window_id.as_u64()),
                    _ => unreachable!("Body task Window identity"),
                };
                (
                    kind,
                    address,
                    window,
                    owner.realm_token().as_u64(),
                    owner.grants_universal_access(),
                )
            });
        let values = [
            anchor.into(),
            v8::Integer::new_from_unsigned(scope, kind).into(),
            v8::BigInt::new_from_u64(scope, address).into(),
            v8::BigInt::new_from_u64(scope, window).into(),
            v8::BigInt::new_from_u64(scope, realm).into(),
            v8::Boolean::new(scope, universal).into(),
        ];
        let value = v8::Array::new_with_elements(scope, &values);
        crate::util::set_null_prototype(scope, value.into());
        value.into()
    }

    pub(super) fn from_value<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Object>,
    ) -> Self {
        let value = v8::Local::<v8::Array>::try_from(value).expect("Body task destination");
        let anchor = v8::Local::<v8::Object>::try_from(
            value.get_index(scope, 0).expect("Body destination realm"),
        )
        .expect("Body destination realm anchor");
        let context = anchor
            .get_creation_context(scope)
            .expect("Body destination realm");
        let kind = value
            .get_index(scope, 1)
            .and_then(|value| value.uint32_value(scope))
            .expect("Body destination kind");
        let integer = |scope: &mut v8::PinScope<'s, '_>, index| {
            let value = v8::Local::<v8::BigInt>::try_from(
                value
                    .get_index(scope, index)
                    .expect("Body destination identity"),
            )
            .expect("Body destination integer");
            let (value, exact) = value.u64_value();
            assert!(exact, "Body destination integer must be exact");
            value
        };
        let owner = if kind == 0 {
            None
        } else {
            let address = integer(scope, 2);
            let window = integer(scope, 3);
            let (owner, dispatch_scope) = match kind {
                1 => (
                    WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                        window,
                    )),
                    OwnerDispatchScope::Top,
                ),
                2 => (
                    WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                        window,
                    )),
                    OwnerDispatchScope::Child(crate::document_runtime::DomHandle::new(
                        usize::try_from(address).expect("Body child address"),
                    )),
                ),
                3 => (
                    WindowExecutionContextOwner::LightweightPopup {
                        popup_id: address,
                        local_window_id: LightweightPopupLocalWindowId::new(window),
                    },
                    OwnerDispatchScope::LightweightPopup(address),
                ),
                _ => unreachable!("Body destination kind"),
            };
            let realm = RuntimeObservableContextToken::from_raw(integer(scope, 4));
            let access = if value
                .get_index(scope, 5)
                .is_some_and(|value| value.is_true())
            {
                WindowExecutionContextAccessPolicy::Universal
            } else {
                WindowExecutionContextAccessPolicy::EnforceWebOrigin
            };
            Some(WindowExecutionContextIdentity::new(
                owner,
                dispatch_scope,
                realm,
                access,
            ))
        };
        Self {
            context: v8::Global::new(scope, context),
            owner,
        }
    }

    pub(super) fn queue<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
    ) {
        let host = crate::util::context_host_ptr_from_global_bridge(scope);
        match (host, self.owner) {
            (Some(host), Some(owner)) => {
                let host = unsafe { &mut *host };
                if !host.window_execution_context_identity_is_current(owner) {
                    return;
                }
                let dispatch_scope = owner.dispatch_scope();
                let previous = dispatch_scope.enter(scope);
                host.queue_networking_task(scope, callback);
                dispatch_scope.restore(scope, previous);
            }
            (None, None) => {
                crate::worker::queue_worker_networking_task(scope, callback);
            }
            _ => {}
        }
    }
}
