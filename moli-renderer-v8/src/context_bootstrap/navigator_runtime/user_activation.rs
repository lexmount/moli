use std::{cell::RefCell, collections::HashMap, rc::Rc};

use anyhow::{Result, anyhow};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{
    context_bootstrap::window_accessors::window_child_context_handle,
    document_runtime::DomHandle,
    native_bridge::{OwnerDispatchScope, WindowUserActivationState},
    util::{
        context_host_ptr_from_global_bridge, get_private_value, set_private_value, throw_type_error,
    },
    web_api_interfaces,
};

const OWNER_SLOT: &str = "__moliUserActivationOwner";
const ASSOCIATED_SLOT: &str = "__moliWindowUserActivation";
const STATE_SLOT: &str = "__moliUserActivationState";
type Store = Rc<RefCell<States>>;

#[derive(Default)]
struct States {
    next_id: u64,
    entries: HashMap<u64, (v8::Weak<v8::Object>, Rc<WindowUserActivationState>)>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::UserActivation)]
struct UserActivationObjectDeclaration<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = OWNER_SLOT)]
    owner: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::UserActivation, enumerable)]
struct UserActivationPrototypeDeclaration {
    #[webapi(accessor_property, getter = has_been_active_getter)]
    has_been_active: (),
    #[webapi(accessor_property, getter = is_active_getter)]
    is_active: (),
}

pub(super) fn install_user_activation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "UserActivation" {
        UserActivationPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

// A private anchor retains the shared Window state for its UserActivation
// object. It never retains the host,
// Document, or a V8 global. GC and isolate teardown release the native state.
fn new_anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: Rc<WindowUserActivationState>,
) -> v8::Local<'s, v8::Object> {
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("user activation identity exhausted");
        store.next_id
    };
    let anchor = v8::Object::new(scope);
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        anchor,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store.borrow_mut().entries.insert(id, (weak, state));
    set_private_value(
        scope,
        anchor,
        STATE_SLOT,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
    anchor
}

fn state_for_owner(
    scope: &mut v8::PinScope<'_, '_>,
    owner: OwnerDispatchScope,
) -> Rc<WindowUserActivationState> {
    context_host_ptr_from_global_bridge(scope)
        .and_then(|host| unsafe { &mut *host }.retain_window_user_activation_state(owner))
        .unwrap_or_default()
}

pub(super) fn initialize_window_user_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let owner = window_child_context_handle(scope, window)
        .map(OwnerDispatchScope::Child)
        .unwrap_or(OwnerDispatchScope::Top);
    let state = state_for_owner(scope, owner);
    // The associated object is created with its Window. Navigator can remain
    // lazy: its first getter may run after the Window's realm has retired.
    let activation = new_user_activation(scope, state)?;
    set_private_value(scope, window, ASSOCIATED_SLOT, activation.into());
    Ok(())
}

pub(super) fn bind_navigator_user_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    owner_child: Option<DomHandle>,
    owner_popup: Option<u64>,
) -> Result<()> {
    let window = scope.get_current_context().global(scope);
    let activation = (owner_popup.is_none()
        && window_child_context_handle(scope, window) == owner_child)
        .then(|| get_private_value(scope, window, ASSOCIATED_SLOT))
        .flatten()
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let activation = if let Some(activation) = activation {
        activation
    } else {
        let owner = owner_popup
            .map(OwnerDispatchScope::LightweightPopup)
            .or_else(|| owner_child.map(OwnerDispatchScope::Child))
            .unwrap_or(OwnerDispatchScope::Top);
        let state = state_for_owner(scope, owner);
        new_user_activation(scope, state)?
    };
    set_private_value(scope, backing, ASSOCIATED_SLOT, activation.into());
    Ok(())
}

pub(super) fn associated_user_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    get_private_value(scope, backing, ASSOCIATED_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("Navigator has no associated UserActivation"))
}

fn new_user_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: Rc<WindowUserActivationState>,
) -> Result<v8::Local<'s, v8::Object>> {
    let anchor = new_anchor(scope, state);
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "UserActivation")?;
    UserActivationObjectDeclaration::new(prototype, anchor)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind UserActivation object: {error}"))
}

fn activation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(bool, bool)> {
    if !web_api_interfaces::UserActivation::is_instance(scope, object) {
        throw_type_error(scope, "Illegal invocation");
        return None;
    }
    let anchor = get_private_value(scope, object, OWNER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let id = get_private_value(scope, anchor, STATE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())?
        .u64_value()
        .0;
    let store = scope.get_slot::<Store>()?;
    Some(store.borrow().entries.get(&id)?.1.state())
}

fn has_been_active_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some((_, sticky)) = activation_state(scope, args.this()) {
        rv.set_bool(sticky);
    }
}

fn is_active_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some((active, _)) = activation_state(scope, args.this()) {
        rv.set_bool(active);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_anchors_release_state_on_gc_and_isolate_drop() {
        moli_v8_test_util::ensure_v8();
        let retained = {
            let mut isolate = v8::Isolate::new(Default::default());
            let retained = {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let state = Rc::default();
                let retained = Rc::downgrade(&state);
                new_anchor(scope, state);
                retained
            };
            isolate.low_memory_notification();
            assert!(retained.upgrade().is_none());
            assert!(
                isolate
                    .get_slot::<Store>()
                    .unwrap()
                    .borrow()
                    .entries
                    .is_empty()
            );
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = Rc::default();
            let retained = Rc::downgrade(&state);
            new_anchor(scope, state);
            retained
        };
        assert!(retained.upgrade().is_none());
    }
}
