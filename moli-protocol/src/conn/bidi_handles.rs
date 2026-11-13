//! Only references actually exported by the BiDi adapter belong here. Native
//! CDP object ids and object groups are owned by the Inspector session.
use super::state::DevToolsSessionState;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    BidiChannelPageOwner, CdpConnection, CommandOwnerScope, TargetWorkerProtocolAttachmentIdentity,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Binding {
    Page(Box<BidiChannelPageOwner>),
    Worker(TargetWorkerProtocolAttachmentIdentity),
}

impl Binding {
    fn capture(conn: &CdpConnection, owner: &CommandOwnerScope) -> Option<Self> {
        if let Some(worker) = owner
            .session_id()
            .and_then(|id| conn.committed_worker_protocol_attachment_identity(id))
        {
            return worker.is_current().then(|| Self::Worker(worker.clone()));
        }
        let page = BidiChannelPageOwner::capture_for_owner(conn, owner.clone())?;
        page.can_start_listener(conn)
            .then(|| Self::Page(Box::new(page)))
    }

    fn is_current(&self, conn: &CdpConnection) -> bool {
        match self {
            Self::Page(owner) => owner.can_start_listener(conn),
            Self::Worker(owner) => {
                owner.is_current() && !conn.session_is_closing(owner.session_id())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Export {
    native: String,
    realm: String,
    group: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BidiHandles {
    binding: Option<Binding>,
    exports: HashMap<String, Export>,
    references: HashMap<String, usize>,
    groups: HashMap<String, usize>,
    // Only raw wrapper-backed node aliases need Inspector storage here.
    // Canonical sharedIds and stale-node history have their own renderer and
    // webdriver_node_references bindings.
    nodes: HashMap<String, Export>,
}

/// Cleanup authority exists only for references actually exported by BiDi.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BidiHandleRelease {
    Object(String),
    Group(String),
}

fn next_reference(kind: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("BiDi reference identity exhausted");
    format!("moli-bidi-{kind}-{id}")
}

impl BidiHandles {
    fn bind(&mut self, binding: Binding) {
        if self.binding.as_ref() != Some(&binding) {
            *self = Self {
                binding: Some(binding),
                ..Self::default()
            };
        }
    }

    fn retain(&mut self, entry: &Export) {
        *self.references.entry(entry.native.clone()).or_default() += 1;
        if let Some(group) = &entry.group {
            *self.groups.entry(group.clone()).or_default() += 1;
        }
    }

    fn release(&mut self, entry: Export) -> Option<BidiHandleRelease> {
        let count = self
            .references
            .get_mut(&entry.native)
            .expect("export reference count");
        *count -= 1;
        let last_native = *count == 0;
        if last_native {
            self.references.remove(&entry.native);
        }
        if let Some(group) = entry.group {
            let count = self.groups.get_mut(&group).expect("export group count");
            *count -= 1;
            if *count != 0 {
                return None;
            }
            self.groups.remove(&group);
            return Some(BidiHandleRelease::Group(group));
        }
        last_native.then_some(BidiHandleRelease::Object(entry.native))
    }

    fn export(&mut self, native: String, realm: String, group: Option<String>) -> String {
        let handle = next_reference("handle");
        let entry = Export {
            native,
            realm,
            group,
        };
        self.retain(&entry);
        self.exports.insert(handle.clone(), entry);
        handle
    }

    fn resolve(&self, handle: &str, realm: Option<&str>) -> Option<&str> {
        let entry = self.exports.get(handle)?;
        realm
            .is_none_or(|realm| realm == entry.realm)
            .then_some(entry.native.as_str())
    }

    /// Remove one exported reference. Only its last alias releases the native
    /// reference; state snapshots do not participate in this count.
    fn disown(&mut self, handle: &str, realm: Option<&str>) -> Option<BidiHandleRelease> {
        self.resolve(handle, realm)?;
        let entry = self.exports.remove(handle)?;
        self.release(entry)
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn clear_realm(&mut self, realm: &str) {
        let handles = self
            .exports
            .iter()
            .filter(|(_, entry)| entry.realm == realm)
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        for handle in handles {
            self.disown(&handle, Some(realm));
        }
        let ids = self
            .nodes
            .iter()
            .filter(|(_, entry)| entry.realm == realm)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            let entry = self.nodes.remove(&id).unwrap();
            self.release(entry);
        }
    }
}

impl CdpConnection {
    fn bidi_session(&self, owner: &CommandOwnerScope) -> Option<&DevToolsSessionState> {
        if let Some(id) = owner.session_id() {
            if let Some(worker) = self.shared_worker_target_for_session(Some(id)) {
                return worker.session_state(id);
            }
            if let Some(worker) = self.service_worker_target_for_session(Some(id)) {
                return worker.session_state(id);
            }
        }
        self.target_devtools_session_state_for_owner(owner)
    }

    fn with_bidi_session<T>(
        &mut self,
        owner: &CommandOwnerScope,
        f: impl FnOnce(&mut DevToolsSessionState) -> T,
    ) -> Option<T> {
        if let Some(id) = owner.session_id() {
            if self.shared_worker_target_for_session(Some(id)).is_some() {
                return self
                    .shared_worker_target_for_session_mut(Some(id))?
                    .session_state_mut(id)
                    .map(f);
            }
            if self.service_worker_target_for_session(Some(id)).is_some() {
                return self
                    .service_worker_target_for_session_mut(Some(id))?
                    .session_state_mut(id)
                    .map(f);
            }
        }
        self.with_target_devtools_session_state_for_owner_mut(owner, f)
    }

    pub(crate) fn export_bidi_handle(
        &mut self,
        owner: &CommandOwnerScope,
        native: String,
        realm: &str,
        group: Option<&str>,
    ) -> Option<String> {
        let binding = Binding::capture(self, owner)?;
        self.with_bidi_session(owner, |state| {
            state.bidi_handles.bind(binding);
            state
                .bidi_handles
                .export(native, realm.to_owned(), group.map(str::to_owned))
        })
    }

    pub(crate) fn resolve_bidi_handle(
        &self,
        owner: &CommandOwnerScope,
        handle: &str,
        realm: Option<&str>,
    ) -> Option<String> {
        let handles = &self.bidi_session(owner)?.bidi_handles;
        handles.binding.as_ref()?.is_current(self).then_some(())?;
        handles.resolve(handle, realm).map(str::to_owned)
    }

    pub(crate) fn disown_bidi_handle(
        &mut self,
        owner: &CommandOwnerScope,
        handle: &str,
        realm: Option<&str>,
    ) -> Option<BidiHandleRelease> {
        self.resolve_bidi_handle(owner, handle, realm)?;
        self.with_bidi_session(owner, |state| state.bidi_handles.disown(handle, realm))
            .flatten()
    }

    pub(crate) fn export_bidi_node_reference(
        &mut self,
        owner: &CommandOwnerScope,
        native: String,
        realm: &str,
        group: Option<&str>,
    ) -> String {
        let alias = next_reference("node");
        self.register_bidi_node_alias(owner, alias.clone(), native, realm.to_owned(), group);
        alias
    }

    pub(crate) fn register_bidi_node_alias(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: String,
        native: String,
        realm: String,
        group: Option<&str>,
    ) {
        let Some(binding) = Binding::capture(self, owner) else {
            return;
        };
        self.with_bidi_session(owner, |state| {
            state.bidi_handles.bind(binding);
            let handles = &mut state.bidi_handles;
            if handles.nodes.contains_key(&shared_id) {
                return;
            }
            let entry = Export {
                native,
                realm,
                group: group.map(str::to_owned),
            };
            handles.retain(&entry);
            handles.nodes.insert(shared_id, entry);
        });
    }

    pub(crate) fn bidi_retains_object_group(&self, owner: &CommandOwnerScope, group: &str) -> bool {
        self.bidi_session(owner).is_some_and(|state| {
            state
                .bidi_handles
                .binding
                .as_ref()
                .is_some_and(|binding| binding.is_current(self))
                && state.bidi_handles.groups.contains_key(group)
        })
    }

    pub(crate) fn bidi_node_alias(
        &self,
        owner: &CommandOwnerScope,
        shared_id: &str,
    ) -> Option<String> {
        let handles = &self.bidi_session(owner)?.bidi_handles;
        handles.binding.as_ref()?.is_current(self).then_some(())?;
        handles
            .nodes
            .get(shared_id)
            .map(|entry| entry.native.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{BidiHandleRelease, BidiHandles};

    #[test]
    fn exports_are_explicit_unique_realm_checked_and_reference_counted() {
        let mut handles = BidiHandles::default();
        assert_eq!(handles.resolve("native-id", None), None);
        let first = handles.export("native-id".into(), "realm-A".into(), None);
        let second = handles.export("native-id".into(), "realm-A".into(), None);
        assert_ne!(first, second);
        assert_eq!(handles.resolve(&first, Some("realm-B")), None);
        assert_eq!(handles.disown(&first, Some("realm-B")), None);
        assert_eq!(handles.disown(&first, Some("realm-A")), None);
        assert_eq!(handles.resolve(&second, Some("realm-A")), Some("native-id"));
        assert_eq!(
            handles.disown(&second, Some("realm-A")),
            Some(BidiHandleRelease::Object("native-id".into()))
        );
        assert!(handles.references.is_empty());
        assert!(handles.exports.is_empty());
    }

    #[test]
    fn clear_and_realm_retirement_cannot_revive_reused_native_ids() {
        let mut handles = BidiHandles::default();
        let old = handles.export("same-native-id".into(), "realm-A".into(), None);
        let retained = handles.export("other-id".into(), "realm-B".into(), None);
        handles.clear_realm("realm-A");
        assert_eq!(handles.resolve(&old, None), None);
        assert_eq!(handles.resolve(&retained, None), Some("other-id"));
        handles.clear();
        let new = handles.export("same-native-id".into(), "realm-A".into(), None);
        assert_ne!(old, new);
        assert_eq!(handles.resolve(&old, None), None);
    }
    #[test]
    fn private_groups_outlive_all_exported_handles_and_raw_node_aliases() {
        let mut handles = BidiHandles::default();
        let first = handles.export("first".into(), "realm".into(), Some("operation".into()));
        let second = handles.export("second".into(), "realm".into(), Some("operation".into()));
        assert_eq!(handles.disown(&first, Some("realm")), None);
        assert_eq!(
            handles.disown(&second, Some("realm")),
            Some(BidiHandleRelease::Group("operation".into()))
        );
        assert!(handles.groups.is_empty());
        assert!(handles.references.is_empty());
        let handle = handles.export("node".into(), "realm".into(), Some("node-operation".into()));
        let entry = super::Export {
            native: "node".into(),
            realm: "realm".into(),
            group: Some("node-operation".into()),
        };
        handles.retain(&entry);
        handles.nodes.insert("node-alias".into(), entry);
        assert_eq!(
            handles.disown(&handle, Some("realm")),
            None,
            "disown cannot release a still-exported raw node sharedId"
        );
        assert_eq!(handles.groups.get("node-operation"), Some(&1));
        handles.clear_realm("realm");
        assert!(handles.groups.is_empty());
        assert!(handles.references.is_empty());
        assert!(handles.nodes.is_empty());
    }
}
