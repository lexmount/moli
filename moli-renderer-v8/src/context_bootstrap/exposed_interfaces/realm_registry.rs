use std::cell::{Ref, RefCell};
use std::rc::Rc;

use anyhow::{Result, anyhow};

use super::metadata::{InterfaceId, RealmKind};

/// Publication state contains no V8 roots. The Context owns intrinsic identities
/// through a traced embedder-data array, including after Window detachment.
pub(super) enum RealmInterfaceEntry {
    Uninitialized,
    Materializing,
    Ready,
    Failed,
}

pub(super) struct IntrinsicInterfaceRegistry {
    realm_kind: RealmKind,
    entries: RefCell<Vec<RealmInterfaceEntry>>,
}

// Renderer-owned tagged Context slot, reserved exclusively for Web API identities.
// rusty_v8 offsets this index past its debugger and Rust context-slot bookkeeping.
// Storing a JS value here gives V8 the ownership edge without a native Global root.
const INTRINSIC_OBJECTS_CONTEXT_SLOT: i32 = 0;
const OBJECTS_PER_INTERFACE: usize = 3;

fn intrinsic_objects<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Array>> {
    // Registry initialization always sets this slot before any identity lookup.
    scope
        .get_current_context()
        .get_embedder_data(scope, INTRINSIC_OBJECTS_CONTEXT_SLOT)
        .and_then(|value| v8::Local::try_from(value).ok())
}

impl IntrinsicInterfaceRegistry {
    fn new(interface_count: usize, realm_kind: RealmKind) -> Self {
        Self {
            realm_kind,
            entries: RefCell::new(
                (0..interface_count)
                    .map(|_| RealmInterfaceEntry::Uninitialized)
                    .collect(),
            ),
        }
    }

    pub(super) fn initialize_for_current_context(
        scope: &mut v8::PinScope<'_, '_>,
        interface_count: usize,
        realm_kind: RealmKind,
    ) -> Result<Rc<Self>> {
        let context = scope.get_current_context();
        if let Some(registry) = context.get_slot::<Self>() {
            registry.validate_size(interface_count)?;
            if registry.realm_kind != realm_kind {
                return Err(anyhow!(
                    "realm interface registry kind changed from {:?} to {realm_kind:?}",
                    registry.realm_kind
                ));
            }
            return Ok(registry);
        }
        let count = interface_count
            .checked_mul(OBJECTS_PER_INTERFACE)
            .and_then(|count| i32::try_from(count).ok())
            .ok_or_else(|| anyhow!("too many realm interface objects"))?;
        let objects = v8::Array::new(scope, count);
        if objects.set_prototype(scope, v8::null(scope).into()) != Some(true) {
            return Err(anyhow!("failed to initialize realm interface storage"));
        }
        context.set_embedder_data(INTRINSIC_OBJECTS_CONTEXT_SLOT, objects.into());
        let registry = Rc::new(Self::new(interface_count, realm_kind));
        context.set_slot(registry.clone());
        Ok(registry)
    }

    pub(super) fn for_current_context(
        scope: &mut v8::PinScope<'_, '_>,
        interface_count: usize,
    ) -> Result<Rc<Self>> {
        let registry = scope
            .get_current_context()
            .get_slot::<Self>()
            .ok_or_else(|| anyhow!("realm interface registry is not initialized"))?;
        registry.validate_size(interface_count)?;
        Ok(registry)
    }

    fn validate_size(&self, interface_count: usize) -> Result<()> {
        let current_count = self.entries.borrow().len();
        if current_count != interface_count {
            return Err(anyhow!(
                "realm interface registry size changed from {current_count} to {interface_count}"
            ));
        }
        Ok(())
    }

    pub(super) const fn realm_kind(&self) -> RealmKind {
        self.realm_kind
    }

    pub(super) fn entry(&self, id: InterfaceId) -> Option<Ref<'_, RealmInterfaceEntry>> {
        Ref::filter_map(self.entries.borrow(), |entries| entries.get(id.index())).ok()
    }

    pub(super) fn begin_materialization(&self, id: InterfaceId) -> Result<()> {
        self.set_pending_entry(id, RealmInterfaceEntry::Materializing)
    }

    pub(super) fn fail(&self, id: InterfaceId) -> Result<()> {
        self.set_pending_entry(id, RealmInterfaceEntry::Failed)
    }

    fn set_pending_entry(&self, id: InterfaceId, entry: RealmInterfaceEntry) -> Result<()> {
        let mut entries = self.entries.borrow_mut();
        let interface_count = entries.len();
        let slot = entries.get_mut(id.index()).ok_or_else(|| {
            anyhow!(
                "interface id {} is out of range for {interface_count} entries",
                id.index()
            )
        })?;
        if !matches!(
            (&*slot, &entry),
            (
                RealmInterfaceEntry::Uninitialized,
                RealmInterfaceEntry::Materializing
            ) | (
                RealmInterfaceEntry::Materializing,
                RealmInterfaceEntry::Failed
            )
        ) {
            return Err(anyhow!(
                "invalid materialization transition for interface id {}",
                id.index()
            ));
        }
        *slot = entry;
        Ok(())
    }

    /// The only publication point, shared by completed lazy materialization
    /// and eager capture after bootstrap. Keep the prototype handle because
    /// legacy factories share prototypes and author code can mutate properties.
    pub(super) fn publish_ready<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
        constructor: v8::Local<'s, v8::Object>,
        prototype: v8::Local<'s, v8::Object>,
        public_interface: v8::Local<'s, v8::Object>,
    ) -> Result<()> {
        {
            let entries = self.entries.borrow();
            let slot = entries.get(id.index()).ok_or_else(|| {
                anyhow!(
                    "interface id {} is out of range for {} entries",
                    id.index(),
                    entries.len()
                )
            })?;
            match slot {
                RealmInterfaceEntry::Ready => {
                    return Err(anyhow!("interface id {} is already published", id.index()));
                }
                RealmInterfaceEntry::Failed => {
                    return Err(anyhow!(
                        "a previous materialization of interface id {} failed",
                        id.index()
                    ));
                }
                RealmInterfaceEntry::Uninitialized | RealmInterfaceEntry::Materializing => {}
            }
        }
        let objects = intrinsic_objects(scope)
            .ok_or_else(|| anyhow!("realm interface storage is not initialized"))?;
        let first = u32::try_from(id.index() * OBJECTS_PER_INTERFACE)
            .map_err(|_| anyhow!("realm interface object index is out of range"))?;
        for (offset, value) in [constructor, prototype, public_interface]
            .into_iter()
            .enumerate()
        {
            if objects.set_index(scope, first + offset as u32, value.into()) != Some(true) {
                return Err(anyhow!("failed to publish realm interface object"));
            }
        }
        self.entries.borrow_mut()[id.index()] = RealmInterfaceEntry::Ready;
        Ok(())
    }

    fn object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
        offset: usize,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if !matches!(
            self.entries.borrow().get(id.index())?,
            RealmInterfaceEntry::Ready
        ) {
            return None;
        }
        let objects = intrinsic_objects(scope)?;
        let index = u32::try_from(id.index() * OBJECTS_PER_INTERFACE + offset).ok()?;
        objects
            .get_index(scope, index)
            .and_then(|value| v8::Local::try_from(value).ok())
    }

    pub(super) fn constructor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.object(scope, id, 0)
    }

    pub(super) fn prototype<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.object(scope, id, 1)
    }

    pub(super) fn public_interface<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.object(scope, id, 2)
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;

    #[test]
    fn intrinsic_identities_live_with_the_context_not_the_rust_registry() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let id = InterfaceId::from_callback_data(0);
        let (registry, context_root, context_weak, identities) = {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let registry = IntrinsicInterfaceRegistry::initialize_for_current_context(
                scope,
                1,
                RealmKind::Window,
            )
            .unwrap();
            let constructor = v8::Object::new(scope);
            let prototype = v8::Object::new(scope);
            let public_interface = v8::Object::new(scope);
            registry
                .publish_ready(scope, id, constructor, prototype, public_interface)
                .unwrap();
            let identities = [constructor, prototype, public_interface]
                .map(|object| v8::Weak::new(scope, object));
            context.detach_global();
            (
                registry,
                v8::Global::new(scope, context),
                v8::Weak::new(scope, context),
                identities,
            )
        };

        for _ in 0..2 {
            isolate.low_memory_notification();
        }
        {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, &context_root);
            let scope = &mut v8::ContextScope::new(scope, context);
            let values = [
                registry.constructor(scope, id),
                registry.prototype(scope, id),
                registry.public_interface(scope, id),
            ];
            for (value, identity) in values.into_iter().zip(&identities) {
                let identity = identity
                    .to_local(scope)
                    .expect("Context must retain each intrinsic");
                assert!(value.unwrap().strict_equals(identity.into()));
            }
        }

        drop(context_root);
        for _ in 0..2 {
            isolate.low_memory_notification();
        }
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        assert!(
            context_weak.to_local(scope).is_none(),
            "Rust metadata must not root the Context"
        );
        for identity in identities {
            assert!(
                identity.to_local(scope).is_none(),
                "unreachable intrinsic must be collected"
            );
        }
        assert!(matches!(
            &*registry.entry(id).unwrap(),
            RealmInterfaceEntry::Ready
        ));
    }

    #[test]
    fn materialization_rejects_an_out_of_range_interface() {
        let registry = IntrinsicInterfaceRegistry::new(1, RealmKind::Window);
        let error = registry
            .begin_materialization(InterfaceId::from_callback_data(1))
            .expect_err("out-of-range interface must fail");

        assert_eq!(
            error.to_string(),
            "interface id 1 is out of range for 1 entries"
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
        let registry =
            IntrinsicInterfaceRegistry::initialize_for_current_context(scope, 1, RealmKind::Window)
                .unwrap();
        let constructor = v8::Object::new(scope);
        let prototype = v8::Object::new(scope);
        let public_interface = v8::Object::new(scope);

        registry
            .publish_ready(scope, id, constructor, prototype, public_interface)
            .expect("completed realm interface should publish");
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
