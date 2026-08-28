use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Result, anyhow};

use super::metadata::{InterfaceId, RealmKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RealmInterfaceState {
    Uninitialized,
    Materializing,
    Finalizing,
    Ready,
    Failed,
}

pub(crate) struct IntrinsicInterfaceRegistry {
    realm_kind: RealmKind,
    states: RefCell<Vec<RealmInterfaceState>>,
    objects: RefCell<Vec<Option<RealmInterfaceObjects>>>,
}

struct RealmInterfaceObjects {
    constructor: RealmInterfaceObjectHandle,
    prototype: RealmInterfaceObjectHandle,
    public_interface: RealmInterfaceObjectHandle,
}

enum RealmInterfaceObjectHandle {
    Strong(v8::Global<v8::Object>),
    Weak(v8::Weak<v8::Object>),
}

impl RealmInterfaceObjectHandle {
    fn new(scope: &mut v8::PinScope<'_, '_>, object: v8::Local<'_, v8::Object>) -> Self {
        if crate::util::page_context_is_detached_document(scope.get_current_context()) {
            Self::Weak(v8::Weak::new(scope, object))
        } else {
            Self::Strong(v8::Global::new(scope, object))
        }
    }

    fn to_local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
        match self {
            Self::Strong(object) => Some(v8::Local::new(scope, object)),
            Self::Weak(object) => object.to_local(scope),
        }
    }

    fn weaken(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        let Self::Strong(object) = self else {
            return;
        };
        let object = v8::Local::new(scope, &*object);
        *self = Self::Weak(v8::Weak::new(scope, object));
    }
}

impl IntrinsicInterfaceRegistry {
    fn new(interface_count: usize, realm_kind: RealmKind) -> Self {
        Self {
            realm_kind,
            states: RefCell::new(vec![RealmInterfaceState::Uninitialized; interface_count]),
            objects: RefCell::new((0..interface_count).map(|_| None).collect()),
        }
    }

    pub(super) fn initialize_for_current_context(
        scope: &mut v8::PinScope<'_, '_>,
        interface_count: usize,
        realm_kind: RealmKind,
    ) -> Result<Rc<Self>> {
        let context = scope.get_current_context();
        if let Some(registry) = context.get_slot::<Self>() {
            if registry.states.borrow().len() != interface_count {
                return Err(anyhow!(
                    "realm interface registry size changed from {} to {interface_count}",
                    registry.states.borrow().len()
                ));
            }
            if registry.realm_kind != realm_kind {
                return Err(anyhow!(
                    "realm interface registry kind changed from {:?} to {realm_kind:?}",
                    registry.realm_kind
                ));
            }
            return Ok(registry);
        }
        let registry = Rc::new(Self::new(interface_count, realm_kind));
        if let Some(previous) = context.set_slot(registry.clone()) {
            if previous.states.borrow().len() != interface_count
                || previous.realm_kind != realm_kind
            {
                return Err(anyhow!(
                    "realm interface registry was concurrently initialized with incompatible metadata"
                ));
            }
            Ok(previous)
        } else {
            Ok(registry)
        }
    }

    pub(super) fn for_current_context(
        scope: &mut v8::PinScope<'_, '_>,
        interface_count: usize,
    ) -> Result<Rc<Self>> {
        let registry = scope
            .get_current_context()
            .get_slot::<Self>()
            .ok_or_else(|| anyhow!("realm interface registry is not initialized"))?;
        if registry.states.borrow().len() != interface_count {
            return Err(anyhow!(
                "realm interface registry size changed from {} to {interface_count}",
                registry.states.borrow().len()
            ));
        }
        Ok(registry)
    }

    pub(super) fn state(&self, id: InterfaceId) -> Option<RealmInterfaceState> {
        self.states.borrow().get(id.index()).copied()
    }

    pub(super) fn set_state(&self, id: InterfaceId, state: RealmInterfaceState) -> Result<()> {
        let interface_count = self.states.borrow().len();
        let mut states = self.states.borrow_mut();
        let slot = states.get_mut(id.index()).ok_or_else(|| {
            anyhow!(
                "interface state id {} is out of range for {interface_count} entries",
                id.index()
            )
        })?;
        *slot = state;
        Ok(())
    }

    pub(super) fn register_objects<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
        constructor: v8::Local<'s, v8::Object>,
        prototype: v8::Local<'s, v8::Object>,
        public_interface: v8::Local<'s, v8::Object>,
    ) -> Result<()> {
        let interface_count = self.objects.borrow().len();
        {
            let objects = self.objects.borrow();
            if let Some(existing) = objects.get(id.index()).and_then(Option::as_ref) {
                let same_constructor = existing
                    .constructor
                    .to_local(scope)
                    .is_some_and(|existing| existing.strict_equals(constructor.into()));
                let same_prototype = existing
                    .prototype
                    .to_local(scope)
                    .is_some_and(|existing| existing.strict_equals(prototype.into()));
                let same_public_interface = existing
                    .public_interface
                    .to_local(scope)
                    .is_some_and(|existing| existing.strict_equals(public_interface.into()));
                if same_constructor && same_prototype && same_public_interface {
                    return Ok(());
                }
                return Err(anyhow!(
                    "interface objects for id {} were replaced after registration",
                    id.index()
                ));
            }
        }
        let mut objects = self.objects.borrow_mut();
        let slot = objects.get_mut(id.index()).ok_or_else(|| {
            anyhow!(
                "interface object id {} is out of range for {interface_count} entries",
                id.index()
            )
        })?;
        debug_assert!(slot.is_none());
        *slot = Some(RealmInterfaceObjects {
            constructor: RealmInterfaceObjectHandle::new(scope, constructor),
            prototype: RealmInterfaceObjectHandle::new(scope, prototype),
            public_interface: RealmInterfaceObjectHandle::new(scope, public_interface),
        });
        Ok(())
    }

    pub(super) fn constructor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let objects = self.objects.borrow();
        let object = objects.get(id.index())?.as_ref()?;
        object.constructor.to_local(scope)
    }

    pub(super) fn prototype<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let objects = self.objects.borrow();
        let object = objects.get(id.index())?.as_ref()?;
        object.prototype.to_local(scope)
    }

    pub(super) fn public_interface<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let objects = self.objects.borrow();
        let object = objects.get(id.index())?.as_ref()?;
        object.public_interface.to_local(scope)
    }

    fn weaken_for_context_teardown(&self, scope: &mut v8::PinScope<'_, '_>) {
        for objects in self.objects.borrow_mut().iter_mut().flatten() {
            objects.constructor.weaken(scope);
            objects.prototype.weaken(scope);
            objects.public_interface.weaken(scope);
        }
    }
}

pub(crate) fn weaken_intrinsic_interface_registry_for_context_teardown(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<Rc<IntrinsicInterfaceRegistry>> {
    let registry = scope
        .get_current_context()
        .get_slot::<IntrinsicInterfaceRegistry>()?;
    registry.weaken_for_context_teardown(scope);
    Some(registry)
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;

    #[test]
    fn set_state_updates_a_registered_interface() {
        let registry = IntrinsicInterfaceRegistry::new(1, RealmKind::Window);
        let id = InterfaceId::from_callback_data(0);

        registry
            .set_state(id, RealmInterfaceState::Ready)
            .expect("registered interface state should update");

        assert_eq!(registry.state(id), Some(RealmInterfaceState::Ready));
    }

    #[test]
    fn set_state_rejects_an_out_of_range_interface() {
        let registry = IntrinsicInterfaceRegistry::new(1, RealmKind::Window);
        let error = registry
            .set_state(
                InterfaceId::from_callback_data(1),
                RealmInterfaceState::Ready,
            )
            .expect_err("out-of-range interface state must fail");

        assert_eq!(
            error.to_string(),
            "interface state id 1 is out of range for 1 entries"
        );
    }

    #[test]
    fn realm_owned_interface_objects_survive_window_proxy_detachment() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let id = InterfaceId::from_callback_data(0);
        let registry = IntrinsicInterfaceRegistry::new(1, RealmKind::Window);
        let constructor = v8::Object::new(scope);
        let prototype = v8::Object::new(scope);
        let public_interface = v8::Object::new(scope);

        registry
            .register_objects(scope, id, constructor, prototype, public_interface)
            .expect("realm interface objects should register");
        context.detach_global();

        assert!(
            registry
                .constructor(scope, id)
                .expect("detached realm constructor")
                .strict_equals(constructor.into())
        );
        assert!(
            registry
                .prototype(scope, id)
                .expect("detached realm prototype")
                .strict_equals(prototype.into())
        );
        assert!(
            registry
                .public_interface(scope, id)
                .expect("detached realm public interface")
                .strict_equals(public_interface.into())
        );
    }
}
