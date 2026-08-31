use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHostBridgeRef, RuntimeObservableContextToken},
    script_vm::inspector::DocumentInspectorContextRegistrationId,
};
use moli_page_types::DevToolsSessionKey;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct PageIsolatedWorldContext {
    pub(super) name: String,
    pub(super) devtools_session: Option<DevToolsSessionKey>,
    pub(super) grant_universal_access: bool,
    pub(super) frame_id: Option<String>,
    pub(super) child_handle: Option<DomHandle>,
    pub(super) document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    pub(super) context: v8::Global<v8::Context>,
    pub(super) _bridge_ref: JsContextHostBridgeRef,
    pub(super) runtime_observable_context_token: RuntimeObservableContextToken,
    pub(super) inspector_execution_context_id: Option<i64>,
    pub(super) inspector_execution_context_realm_id: Option<String>,
    pub(super) inspector_context_registration_id: DocumentInspectorContextRegistrationId,
}

pub(super) struct PageIsolatedWorldRegistry {
    contexts: BTreeMap<i64, PageIsolatedWorldContext>,
}

struct InspectorIsolatedContextCreated<'a> {
    inspector_context_id: i64,
    realm_id: Option<String>,
    name: &'a str,
    frame_id: Option<&'a str>,
}

impl PageIsolatedWorldRegistry {
    pub(super) fn new() -> Self {
        Self {
            contexts: BTreeMap::new(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.contexts.len()
    }

    pub(super) fn has_execution_context_id(&self, execution_context_id: i64) -> bool {
        self.contexts.contains_key(&execution_context_id)
    }

    pub(super) fn context(&self, execution_context_id: i64) -> Option<&PageIsolatedWorldContext> {
        self.contexts.get(&execution_context_id)
    }

    pub(super) fn context_mut(
        &mut self,
        execution_context_id: i64,
    ) -> Option<&mut PageIsolatedWorldContext> {
        self.contexts.get_mut(&execution_context_id)
    }

    pub(super) fn remove_context(
        &mut self,
        execution_context_id: i64,
    ) -> Option<PageIsolatedWorldContext> {
        self.contexts.remove(&execution_context_id)
    }

    pub(super) fn insert_context(
        &mut self,
        execution_context_id: i64,
        context: PageIsolatedWorldContext,
    ) -> Option<PageIsolatedWorldContext> {
        self.contexts.insert(execution_context_id, context)
    }

    pub(super) fn contexts(&self) -> impl Iterator<Item = &PageIsolatedWorldContext> {
        self.contexts.values()
    }

    pub(super) fn contexts_with_ids(
        &self,
    ) -> impl Iterator<Item = (i64, &PageIsolatedWorldContext)> {
        self.contexts
            .iter()
            .map(|(execution_context_id, context)| (*execution_context_id, context))
    }

    pub(super) fn execution_context_ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.contexts.keys().copied()
    }

    pub(super) fn execution_context_id_for_scope(
        &self,
        devtools_session: Option<&DevToolsSessionKey>,
        frame_id: Option<&str>,
        name: &str,
    ) -> Option<i64> {
        self.contexts
            .iter()
            .find_map(|(execution_context_id, world)| {
                (world.devtools_session.as_ref() == devtools_session
                    && world.frame_id.as_deref() == frame_id
                    && world.name == name)
                    .then_some(*execution_context_id)
            })
    }

    pub(super) fn has_world_for_scope(&self, frame_id: Option<&str>, name: &str) -> bool {
        self.contexts
            .values()
            .any(|world| world.frame_id.as_deref() == frame_id && world.name == name)
    }

    pub(super) fn execution_context_ids_for_devtools_session(
        &self,
        devtools_session: &DevToolsSessionKey,
    ) -> Vec<i64> {
        self.contexts
            .iter()
            .filter_map(|(execution_context_id, world)| {
                (world.devtools_session.as_ref() == Some(devtools_session))
                    .then_some(*execution_context_id)
            })
            .collect()
    }

    pub(super) fn execution_context_ids_for_name(&self, name: &str) -> Vec<i64> {
        self.contexts
            .iter()
            .filter_map(|(execution_context_id, world)| {
                (world.name == name).then_some(*execution_context_id)
            })
            .collect()
    }

    pub(super) fn pending_inspector_attachment_ids(&self) -> Vec<i64> {
        self.contexts
            .iter()
            .filter_map(|(execution_context_id, world)| {
                world
                    .inspector_execution_context_id
                    .is_none()
                    .then_some(*execution_context_id)
            })
            .collect()
    }

    pub(super) fn inspector_execution_context_id(&self, execution_context_id: i64) -> Option<i64> {
        self.contexts
            .get(&execution_context_id)
            .and_then(|world| world.inspector_execution_context_id)
    }

    pub(super) fn execution_context_id_for_inspector_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        if self.contexts.contains_key(&execution_context_id) {
            return Some(execution_context_id);
        }
        self.contexts.iter().find_map(|(compatibility_id, world)| {
            (world.inspector_execution_context_id == Some(execution_context_id))
                .then_some(*compatibility_id)
        })
    }

    pub(super) fn record_inspector_context_state(
        &mut self,
        messages: &[Value],
        root_frame_id: Option<&str>,
    ) {
        for message in messages {
            match message["method"].as_str() {
                Some("Runtime.executionContextsCleared") => self.clear_inspector_context_state(),
                Some("Runtime.executionContextDestroyed") => {
                    let destroyed_id = message["params"]["executionContextId"].as_i64();
                    let destroyed_realm_id = message["params"]["executionContextUniqueId"].as_str();
                    self.clear_destroyed_inspector_context_state(destroyed_id, destroyed_realm_id);
                }
                Some("Runtime.executionContextCreated") => {
                    let Some(created) = InspectorIsolatedContextCreated::from_message(message)
                    else {
                        continue;
                    };
                    let Some(execution_context_id) =
                        self.matching_execution_context_id(&created, root_frame_id)
                    else {
                        continue;
                    };
                    self.set_inspector_execution_context_id(
                        execution_context_id,
                        created.inspector_context_id,
                        created.realm_id,
                    );
                }
                _ => {}
            }
        }
    }

    fn clear_inspector_context_state(&mut self) {
        for world in self.contexts.values_mut() {
            world.inspector_execution_context_id = None;
            world.inspector_execution_context_realm_id = None;
        }
    }

    fn clear_destroyed_inspector_context_state(
        &mut self,
        destroyed_id: Option<i64>,
        destroyed_realm_id: Option<&str>,
    ) {
        for world in self.contexts.values_mut() {
            if world.inspector_execution_context_id == destroyed_id
                || destroyed_realm_id.is_some_and(|realm_id| {
                    Some(realm_id) == world.inspector_execution_context_realm_id.as_deref()
                })
            {
                world.inspector_execution_context_id = None;
                world.inspector_execution_context_realm_id = None;
            }
        }
    }

    fn matching_execution_context_id(
        &self,
        created: &InspectorIsolatedContextCreated<'_>,
        root_frame_id: Option<&str>,
    ) -> Option<i64> {
        if self
            .contexts
            .get(&created.inspector_context_id)
            .is_some_and(|world| self.matches_inspector_context(world, created, root_frame_id))
        {
            return Some(created.inspector_context_id);
        }
        self.contexts
            .iter()
            .find_map(|(execution_context_id, world)| {
                self.matches_inspector_context(world, created, root_frame_id)
                    .then_some(*execution_context_id)
            })
    }

    fn matches_inspector_context(
        &self,
        world: &PageIsolatedWorldContext,
        created: &InspectorIsolatedContextCreated<'_>,
        root_frame_id: Option<&str>,
    ) -> bool {
        world.name == created.name
            && frame_scope_matches(world.frame_id.as_deref(), created.frame_id, root_frame_id)
    }

    pub(super) fn set_inspector_execution_context_id(
        &mut self,
        execution_context_id: i64,
        inspector_execution_context_id: i64,
        inspector_execution_context_realm_id: Option<String>,
    ) {
        if execution_context_id == inspector_execution_context_id {
            if let Some(world) = self.contexts.get_mut(&execution_context_id) {
                world.inspector_execution_context_id = Some(inspector_execution_context_id);
                world.inspector_execution_context_realm_id = inspector_execution_context_realm_id;
            }
            return;
        }

        let Some(mut world) = self.contexts.remove(&execution_context_id) else {
            return;
        };
        world.inspector_execution_context_id = Some(inspector_execution_context_id);
        world.inspector_execution_context_realm_id = inspector_execution_context_realm_id;
        if self.contexts.contains_key(&inspector_execution_context_id) {
            tracing::warn!(
                execution_context_id,
                inspector_execution_context_id,
                "isolated world inspector context id collided while re-keying to native id"
            );
            self.contexts.insert(execution_context_id, world);
            return;
        }
        self.contexts.insert(inspector_execution_context_id, world);
    }
}

impl InspectorIsolatedContextCreated<'_> {
    fn from_message(message: &Value) -> Option<InspectorIsolatedContextCreated<'_>> {
        let context = &message["params"]["context"];
        if context["auxData"]["type"].as_str() != Some("isolated") {
            return None;
        }
        Some(InspectorIsolatedContextCreated {
            inspector_context_id: context["id"].as_i64()?,
            realm_id: context["uniqueId"].as_str().map(str::to_owned),
            name: context["name"].as_str()?,
            frame_id: context["auxData"]["frameId"].as_str(),
        })
    }
}

fn frame_scope_matches(
    world_frame_id: Option<&str>,
    inspector_frame_id: Option<&str>,
    root_frame_id: Option<&str>,
) -> bool {
    match (world_frame_id, inspector_frame_id) {
        (Some(left), Some(right)) => left == right,
        (None, None) => true,
        // Chromium keys inspector isolated worlds by LocalFrame + world name.
        // Moli stores the root frame scope as None, while inspector auxData
        // may still report the root frame id. Only bridge that root alias.
        (None, Some(right)) => root_frame_id == Some(right),
        (Some(_), None) => false,
    }
}
