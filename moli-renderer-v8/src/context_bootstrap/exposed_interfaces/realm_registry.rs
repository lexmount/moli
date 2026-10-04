use std::cell::{Ref, RefCell};
use std::rc::Rc;

use anyhow::{Result, anyhow};

use super::metadata::{InterfaceId, RealmKind};

/// Only a completely initialized interface retains intrinsic handles and is
/// visible to callers. State and identities share one realm-owned entry.
pub(super) enum RealmInterfaceEntry {
    Uninitialized,
    Materializing,
    Ready(RealmInterfaceObjects),
    Failed,
}

pub(super) struct IntrinsicInterfaceRegistry {
    realm_kind: RealmKind,
    entries: RefCell<Vec<RealmInterfaceEntry>>,
}

pub(super) struct RealmInterfaceObjects {
    constructor: crate::util::RealmObjectHandle,
    prototype: crate::util::RealmObjectHandle,
    public_interface: crate::util::RealmObjectHandle,
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
        let mut entries = self.entries.borrow_mut();
        let interface_count = entries.len();
        let slot = entries.get_mut(id.index()).ok_or_else(|| {
            anyhow!(
                "interface id {} is out of range for {interface_count} entries",
                id.index()
            )
        })?;
        match slot {
            RealmInterfaceEntry::Ready(_) => {
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
        *slot = RealmInterfaceEntry::Ready(RealmInterfaceObjects {
            constructor: crate::util::RealmObjectHandle::new(scope, constructor),
            prototype: crate::util::RealmObjectHandle::new(scope, prototype),
            public_interface: crate::util::RealmObjectHandle::new(scope, public_interface),
        });
        Ok(())
    }

    pub(super) fn constructor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let entries = self.entries.borrow();
        let RealmInterfaceEntry::Ready(objects) = entries.get(id.index())? else {
            return None;
        };
        objects.constructor.to_local(scope)
    }

    pub(super) fn prototype<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let entries = self.entries.borrow();
        let RealmInterfaceEntry::Ready(objects) = entries.get(id.index())? else {
            return None;
        };
        objects.prototype.to_local(scope)
    }

    pub(super) fn public_interface<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let entries = self.entries.borrow();
        let RealmInterfaceEntry::Ready(objects) = entries.get(id.index())? else {
            return None;
        };
        objects.public_interface.to_local(scope)
    }
}

pub(crate) fn retain_intrinsic_interfaces_in_realm(scope: &mut v8::PinScope<'_, '_>) {
    let context = scope.get_current_context();
    let Some(registry) = context.get_slot::<IntrinsicInterfaceRegistry>() else {
        return;
    };
    for entry in registry.entries.borrow_mut().iter_mut() {
        if let RealmInterfaceEntry::Ready(objects) = entry {
            objects.constructor.retain_in_realm(scope);
            objects.prototype.retain_in_realm(scope);
            objects.public_interface.retain_in_realm(scope);
        }
    }
    crate::util::retain_context_v8_handle_state_for_safe_release(context, registry);
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;

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
        let registry = IntrinsicInterfaceRegistry::new(1, RealmKind::Window);
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
