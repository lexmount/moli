use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use super::{CustomElementRegistryKey, definition::CustomElementRegistryData};
use crate::native_bridge::{JsContextHost, NativeDomHostId};

struct RegistryBinding {
    wrapper: v8::Weak<v8::Object>,
    data: Weak<CustomElementRegistryData>,
}

/// Isolate-local discovery only. Neither the native owner nor any JS realm is
/// kept alive by this index. DOM associations own their V8-traced references.
#[derive(Default)]
pub(crate) struct CustomElementRegistryBindings {
    registries: RefCell<HashMap<CustomElementRegistryKey, RegistryBinding>>,
    hosts: RefCell<HashMap<NativeDomHostId, Weak<RefCell<JsContextHost>>>>,
    constructions: RefCell<Vec<Weak<super::construction::SharedPendingConstruction>>>,
}

impl CustomElementRegistryBindings {
    pub(super) fn register_construction(
        &self,
        view: &Rc<super::construction::SharedPendingConstruction>,
    ) {
        let mut constructions = self.constructions.borrow_mut();
        constructions.retain(|view| view.strong_count() != 0);
        constructions.push(Rc::downgrade(view));
    }

    pub(crate) fn has_pending_construction(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        constructor: v8::Local<'_, v8::Function>,
    ) -> bool {
        self.constructions
            .borrow()
            .iter()
            .rev()
            .filter_map(Weak::upgrade)
            .any(|view| view.matches(scope, constructor))
    }

    pub(crate) fn take_pending_construction<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        constructor: v8::Local<'_, v8::Function>,
    ) -> Option<super::PendingCustomElementConstruction<'s>> {
        let view = self
            .constructions
            .borrow()
            .iter()
            .rev()
            .filter_map(Weak::upgrade)
            .find(|view| view.matches(scope, constructor))?;
        view.take(scope)
    }

    pub(crate) fn for_isolate(scope: &mut v8::PinScope<'_, '_>) -> Rc<Self> {
        type Store = Rc<CustomElementRegistryBindings>;
        if let Some(store) = scope.get_slot::<Store>() {
            return store.clone();
        }
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    }

    pub(crate) fn register_host(&self, id: NativeDomHostId, host: &Rc<RefCell<JsContextHost>>) {
        let mut hosts = self.hosts.borrow_mut();
        hosts.retain(|_, owner| owner.strong_count() != 0);
        hosts.insert(id, Rc::downgrade(host));
    }

    pub(crate) fn host(&self, id: NativeDomHostId) -> Option<Rc<RefCell<JsContextHost>>> {
        self.hosts.borrow().get(&id)?.upgrade()
    }

    pub(crate) fn register(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        key: CustomElementRegistryKey,
        wrapper: v8::Local<'_, v8::Object>,
        data: &Rc<CustomElementRegistryData>,
    ) {
        let mut registries = self.registries.borrow_mut();
        if registries.len().is_multiple_of(64) {
            registries.retain(|_, binding| {
                !binding.wrapper.is_empty() && binding.data.strong_count() != 0
            });
        }
        registries.insert(
            key,
            RegistryBinding {
                wrapper: v8::Weak::new(scope, wrapper),
                data: Rc::downgrade(data),
            },
        );
    }

    pub(crate) fn data(
        &self,
        key: CustomElementRegistryKey,
    ) -> Option<Rc<CustomElementRegistryData>> {
        self.registries.borrow().get(&key)?.data.upgrade()
    }

    pub(crate) fn wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        key: CustomElementRegistryKey,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.registries.borrow().get(&key)?.wrapper.to_local(scope)
    }
}
