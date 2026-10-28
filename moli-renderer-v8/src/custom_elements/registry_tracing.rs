use super::definition::{CustomElementDefinition, CustomElementRegistryData};
use crate::native_bridge::RuntimeObservableContextToken;
use crate::util::{get_private_value, new_null_prototype_object, set_private_value, v8_string};

const ROOTS_SLOT: &str = "__moliCustomElementRegistryRoots";
const DEFINITIONS_SLOT: &str = "__moliCustomElementRegistryDefinitions";
const PROMISES_SLOT: &str = "__moliCustomElementRegistryPromises";

// The registry object traces constructors, captured callbacks and pending
// resolvers. All wrappers for the same data trace one anchor. Active native
// DOM stores also retain this anchor, as they retained individual values
// before. Retirement releases those execution-owned roots;
// only the V8 graph may then keep the registry's values and realm alive.
pub(super) struct RegistryRoots {
    anchor: v8::Weak<v8::Object>,
    active: Option<v8::Global<v8::Object>>,
    realm_token: Option<RuntimeObservableContextToken>,
}

impl CustomElementRegistryData {
    pub(crate) fn bind_wrapper(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        registry: v8::Local<'_, v8::Object>,
    ) {
        let realm_token = scope
            .get_current_context()
            .get_slot::<RuntimeObservableContextToken>()
            .as_deref()
            .copied();
        let existing = self
            .roots
            .borrow()
            .as_ref()
            .and_then(|roots| roots.anchor.to_local(scope));
        let anchor = existing.unwrap_or_else(|| {
            let anchor = new_null_prototype_object(scope);
            *self.roots.borrow_mut() = Some(RegistryRoots {
                anchor: v8::Weak::new(scope, anchor),
                active: None,
                realm_token,
            });
            anchor
        });
        if let Some(roots) = self.roots.borrow_mut().as_mut()
            && roots.realm_token.is_none()
        {
            roots.realm_token = realm_token;
        }
        set_private_value(scope, registry, ROOTS_SLOT, anchor.into());
    }

    pub(super) fn belongs_to_realm(&self, realm_token: RuntimeObservableContextToken) -> bool {
        self.roots
            .borrow()
            .as_ref()
            .is_some_and(|roots| roots.realm_token == Some(realm_token))
    }

    pub(super) fn retire(&self) {
        self.retired.set(true);
        if let Some(roots) = self.roots.borrow_mut().as_mut() {
            roots.active = None;
        }
    }

    fn roots_map<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        slot: &str,
    ) -> v8::Local<'s, v8::Map> {
        let anchor = self
            .roots
            .borrow()
            .as_ref()
            .and_then(|roots| roots.anchor.to_local(scope))
            .expect("registry operations retain their receiver");
        if !self.retired.get() {
            let mut roots = self.roots.borrow_mut();
            let roots = roots.as_mut().expect("live registry has bound roots");
            roots
                .active
                .get_or_insert_with(|| v8::Global::new(scope, anchor));
        }
        if let Some(map) = get_private_value(scope, anchor, slot)
            .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        {
            return map;
        }
        let map = v8::Map::new(scope);
        set_private_value(scope, anchor, slot, map.into());
        map
    }

    pub(super) fn trace_definition(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        name: &str,
        definition: &CustomElementDefinition,
    ) {
        let constructor = definition
            .constructor
            .to_local(scope)
            .expect("definition construction retains the constructor");
        let mut values = vec![constructor.into()];
        let callbacks = &definition.callbacks;
        for callback in [
            &callbacks.connected,
            &callbacks.disconnected,
            &callbacks.connected_move,
            &callbacks.adopted,
            &callbacks.attribute_changed,
            &callbacks.form_associated,
            &callbacks.form_reset,
            &callbacks.form_disabled,
            &callbacks.form_state_restore,
        ]
        .into_iter()
        .flatten()
        {
            let callback = callback
                .to_local(scope)
                .expect("definition construction retains captured callbacks");
            values.push(callback.into());
        }
        let record = v8::Array::new_with_elements(scope, &values);
        let map = self.roots_map(scope, DEFINITIONS_SLOT);
        let name = v8_string(scope, name).expect("registry name fits in a V8 string");
        let _ = map.set(scope, name.into(), record.into());
    }

    pub(super) fn trace_pending_promise(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        name: &str,
        promise: v8::Local<'_, v8::Promise>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
    ) {
        let record = v8::Array::new_with_elements(scope, &[promise.into(), resolver.into()]);
        let map = self.roots_map(scope, PROMISES_SLOT);
        let name = v8_string(scope, name).expect("registry name fits in a V8 string");
        let _ = map.set(scope, name.into(), record.into());
    }

    pub(super) fn remove_pending_promise_root(&self, scope: &mut v8::PinScope<'_, '_>, name: &str) {
        let map = self.roots_map(scope, PROMISES_SLOT);
        let name = v8_string(scope, name).expect("registry name fits in a V8 string");
        let _ = map.delete(scope, name.into());
    }
}
