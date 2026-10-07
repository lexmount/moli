use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    num::{NonZeroI32, NonZeroU64},
    rc::Rc,
    sync::atomic::{AtomicI32, AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct DocumentInspectorContextGroupId(NonZeroI32);

impl DocumentInspectorContextGroupId {
    pub(super) fn next() -> Self {
        static NEXT_CONTEXT_GROUP_ID: AtomicI32 = AtomicI32::new(1);

        let id = NEXT_CONTEXT_GROUP_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                id.checked_add(1).filter(|next| *next > 0)
            })
            .expect("document inspector context group id exhausted");
        Self(
            NonZeroI32::new(id)
                .filter(|id| id.get() > 0)
                .expect("document inspector context group id must be positive"),
        )
    }

    pub(super) fn get(self) -> i32 {
        self.0.get()
    }

    pub(super) fn from_raw(id: i32) -> Self {
        Self(
            NonZeroI32::new(id)
                .filter(|id| id.get() > 0)
                .expect("document inspector context group id must be positive"),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::script_vm) struct DocumentInspectorContextRegistrationId(NonZeroU64);

impl DocumentInspectorContextRegistrationId {
    pub(super) fn next() -> Self {
        static NEXT_CONTEXT_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);

        let id = NEXT_CONTEXT_REGISTRATION_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("document inspector context registration id exhausted");
        Self(
            NonZeroU64::new(id)
                .expect("document inspector context registration id allocation returned zero"),
        )
    }

    #[cfg(test)]
    pub(super) fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Default)]
pub(super) struct DocumentInspectorContextRegistry(
    Rc<RefCell<BTreeMap<DocumentInspectorContextGroupId, DocumentInspectorDefaultContext>>>,
);

struct DocumentInspectorDefaultContext {
    context: v8::Global<v8::Context>,
    registration_id: DocumentInspectorContextRegistrationId,
}

impl DocumentInspectorContextRegistry {
    pub(super) fn set_default_context(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        context: v8::Global<v8::Context>,
        registration_id: DocumentInspectorContextRegistrationId,
    ) {
        self.0.borrow_mut().insert(
            context_group_id,
            DocumentInspectorDefaultContext {
                context,
                registration_id,
            },
        );
    }

    pub(super) fn remove_default_context_if_owned_by(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        registration_id: DocumentInspectorContextRegistrationId,
    ) {
        let mut contexts = self.0.borrow_mut();
        if contexts
            .get(&context_group_id)
            .is_some_and(|context| context.registration_id == registration_id)
        {
            contexts.remove(&context_group_id);
        }
    }

    pub(super) fn remove_default_context(&self, context_group_id: DocumentInspectorContextGroupId) {
        self.0.borrow_mut().remove(&context_group_id);
    }

    pub(super) fn has_default_context(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
    ) -> bool {
        self.0.borrow().contains_key(&context_group_id)
    }

    pub(super) fn default_context_is_owned_by(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        registration_id: DocumentInspectorContextRegistrationId,
    ) -> bool {
        self.0
            .borrow()
            .get(&context_group_id)
            .is_some_and(|context| context.registration_id == registration_id)
    }

    pub(super) fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub(super) fn with_default_context<T>(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        op: impl FnOnce(&v8::Global<v8::Context>) -> T,
    ) -> Option<T> {
        self.0
            .borrow()
            .get(&context_group_id)
            .map(|default_context| op(&default_context.context))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeRealmIdentity {
    execution_context_id: i64,
    unique_id: Option<String>,
    frame_id: Option<String>,
    is_default: bool,
}

#[derive(Debug)]
pub(super) struct RuntimeRealmRegistry {
    realms_by_execution_context_id: HashMap<i64, RuntimeRealmIdentity>,
    default_realm: Option<RuntimeRealmIdentity>,
    initial_default_realm: Option<RuntimeRealmIdentity>,
}

impl RuntimeRealmRegistry {
    pub(super) fn new() -> Self {
        Self {
            realms_by_execution_context_id: HashMap::new(),
            default_realm: None,
            initial_default_realm: None,
        }
    }

    pub(super) fn clear(&mut self) {
        self.realms_by_execution_context_id.clear();
        self.default_realm = None;
        self.initial_default_realm = None;
    }

    pub(super) fn record_attached_default_context(
        &mut self,
        execution_context_id: i64,
        unique_id: Option<String>,
    ) {
        let realm = RuntimeRealmIdentity {
            execution_context_id,
            unique_id,
            frame_id: None,
            is_default: true,
        };
        self.realms_by_execution_context_id
            .insert(execution_context_id, realm.clone());
        self.initial_default_realm = Some(realm);
    }

    pub(super) fn default_execution_context_id(&self) -> Option<i64> {
        self.default_realm
            .as_ref()
            .map(|realm| realm.execution_context_id)
    }

    pub(super) fn default_execution_context_realm_id(&self) -> Option<String> {
        self.default_realm
            .as_ref()
            .and_then(|realm| realm.unique_id.clone())
    }

    pub(super) fn initial_default_execution_context_id(&self) -> Option<i64> {
        self.initial_default_realm
            .as_ref()
            .map(|realm| realm.execution_context_id)
    }

    pub(super) fn initial_default_execution_context_realm_id(&self) -> Option<String> {
        self.initial_default_realm
            .as_ref()
            .and_then(|realm| realm.unique_id.clone())
    }

    pub(super) fn record_execution_context_state(
        &mut self,
        messages: &[Value],
        root_frame_id: Option<&str>,
    ) {
        for message in messages {
            match message["method"].as_str() {
                Some("Runtime.executionContextCreated") => {
                    let context = &message["params"]["context"];
                    let Some(execution_context_id) = context["id"].as_i64() else {
                        continue;
                    };
                    let is_default = context["auxData"]["isDefault"].as_bool().unwrap_or(false);
                    let frame_id = context["auxData"]["frameId"].as_str();
                    let is_root_default = frame_id.is_none() || frame_id == root_frame_id;
                    let realm = RuntimeRealmIdentity {
                        execution_context_id,
                        unique_id: context["uniqueId"].as_str().map(str::to_owned),
                        frame_id: frame_id.map(str::to_owned),
                        is_default,
                    };
                    self.realms_by_execution_context_id
                        .insert(execution_context_id, realm.clone());
                    if is_default && is_root_default {
                        self.default_realm = Some(realm.clone());
                        self.initial_default_realm = Some(realm);
                    }
                }
                Some("Runtime.executionContextDestroyed") => {
                    let destroyed_id = message["params"]["executionContextId"].as_i64();
                    let destroyed_unique_id =
                        message["params"]["executionContextUniqueId"].as_str();
                    let matches_destroyed = |realm: &RuntimeRealmIdentity| {
                        destroyed_unique_id.map_or_else(
                            || destroyed_id == Some(realm.execution_context_id),
                            |unique_id| Some(unique_id) == realm.unique_id.as_deref(),
                        )
                    };
                    self.realms_by_execution_context_id
                        .retain(|_, realm| !matches_destroyed(realm));
                    if self.default_realm.as_ref().is_some_and(matches_destroyed) {
                        self.default_realm = None;
                    }
                    if self
                        .initial_default_realm
                        .as_ref()
                        .is_some_and(matches_destroyed)
                    {
                        self.initial_default_realm = None;
                    }
                }
                Some("Runtime.executionContextsCleared") => self.clear(),
                _ => {}
            }
        }
    }

    #[cfg(test)]
    fn realm_for_test(&self, execution_context_id: i64) -> Option<&RuntimeRealmIdentity> {
        self.realms_by_execution_context_id
            .get(&execution_context_id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn document_inspector_context_group_ids_are_unique() {
        let first = DocumentInspectorContextGroupId::next();
        let second = DocumentInspectorContextGroupId::next();

        assert!(first.get() > 0);
        assert!(second.get() > 0);
        assert_ne!(first, second);
        assert_eq!(
            std::mem::size_of::<Option<DocumentInspectorContextGroupId>>(),
            std::mem::size_of::<i32>(),
            "context-group IDs should preserve the NonZeroI32 option niche"
        );
    }

    #[test]
    fn document_inspector_context_registration_ids_are_unique() {
        let first = DocumentInspectorContextRegistrationId::next();
        let second = DocumentInspectorContextRegistrationId::next();

        assert!(first.get() > 0);
        assert!(second.get() > 0);
        assert_ne!(first, second);
        assert_eq!(
            std::mem::size_of::<Option<DocumentInspectorContextRegistrationId>>(),
            std::mem::size_of::<u64>(),
            "context registration IDs should preserve the NonZeroU64 option niche"
        );
    }

    #[test]
    fn stale_default_context_registration_cannot_remove_its_replacement() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let (old_context, current_context) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let old_context = v8::Context::new(scope, Default::default());
            let current_context = v8::Context::new(scope, Default::default());
            (
                v8::Global::new(scope, old_context),
                v8::Global::new(scope, current_context),
            )
        };
        let registry = DocumentInspectorContextRegistry::default();
        let context_group_id = DocumentInspectorContextGroupId::next();
        let old_registration_id = DocumentInspectorContextRegistrationId::next();
        let current_registration_id = DocumentInspectorContextRegistrationId::next();

        registry.set_default_context(context_group_id, old_context, old_registration_id);
        registry.set_default_context(context_group_id, current_context, current_registration_id);
        registry.remove_default_context_if_owned_by(context_group_id, old_registration_id);
        assert_eq!(
            registry.len(),
            1,
            "a stale registration drop must not remove the current default context"
        );

        registry.remove_default_context_if_owned_by(context_group_id, current_registration_id);
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn runtime_realm_registry_tracks_default_context_lifetime() {
        let mut realms = RuntimeRealmRegistry::new();
        realms.record_execution_context_state(
            &[json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": 7,
                        "uniqueId": "realm-7",
                        "auxData": { "isDefault": true }
                    }
                }
            })],
            None,
        );
        assert_eq!(realms.default_execution_context_id(), Some(7));
        assert_eq!(
            realms.default_execution_context_realm_id().as_deref(),
            Some("realm-7")
        );
        assert_eq!(realms.initial_default_execution_context_id(), Some(7));

        realms.record_execution_context_state(
            &[json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {
                    "executionContextId": 7,
                    "executionContextUniqueId": "realm-7"
                }
            })],
            None,
        );
        assert_eq!(realms.default_execution_context_id(), None);
        assert_eq!(realms.initial_default_execution_context_id(), None);
    }

    #[test]
    fn runtime_realm_registry_ignores_child_default_and_clears_inventory() {
        let mut realms = RuntimeRealmRegistry::new();
        realms.record_execution_context_state(
            &[json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": 3,
                        "uniqueId": "realm-3",
                        "auxData": {
                            "isDefault": true,
                            "frameId": "root-frame"
                        }
                    }
                }
            })],
            Some("root-frame"),
        );
        realms.record_execution_context_state(
            &[json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": 11,
                        "auxData": {
                            "isDefault": true,
                            "frameId": "child-frame"
                        }
                    }
                }
            })],
            Some("root-frame"),
        );
        assert_eq!(realms.default_execution_context_id(), Some(3));
        let child = realms.realm_for_test(11).expect("child realm inventory");
        assert_eq!(child.frame_id.as_deref(), Some("child-frame"));
        assert!(child.is_default);

        realms.record_execution_context_state(
            &[json!({ "method": "Runtime.executionContextsCleared" })],
            Some("root-frame"),
        );
        assert_eq!(realms.default_execution_context_id(), None);
        assert_eq!(realms.initial_default_execution_context_id(), None);
    }

    #[test]
    fn runtime_realm_registry_does_not_destroy_reused_numeric_id_with_other_unique_id() {
        let mut realms = RuntimeRealmRegistry::new();
        realms.record_execution_context_state(
            &[json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": 7,
                        "uniqueId": "new-realm",
                        "auxData": { "isDefault": true }
                    }
                }
            })],
            None,
        );

        realms.record_execution_context_state(
            &[json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {
                    "executionContextId": 7,
                    "executionContextUniqueId": "old-realm"
                }
            })],
            None,
        );

        assert_eq!(realms.default_execution_context_id(), Some(7));
        assert_eq!(
            realms
                .realm_for_test(7)
                .and_then(|realm| realm.unique_id.as_deref()),
            Some("new-realm"),
            "unique Inspector realm identity must win over a reused numeric context id"
        );
    }
}
