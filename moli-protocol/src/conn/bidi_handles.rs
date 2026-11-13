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
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BidiHandles {
    binding: Option<Binding>,
    exports: HashMap<String, Export>,
    references: HashMap<String, usize>,
    // Node sharedIds are Document identities, not exported object handles.
    // This lookup only supports native node probes; stale-node history lives
    // separately in webdriver_node_references.
    nodes: HashMap<String, Export>,
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

    fn export(&mut self, native: String, realm: String) -> String {
        let handle = next_reference("handle");
        *self.references.entry(native.clone()).or_default() += 1;
        self.exports
            .insert(handle.clone(), Export { native, realm });
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
    fn disown(&mut self, handle: &str, realm: Option<&str>) -> Option<String> {
        self.resolve(handle, realm)?;
        let entry = self.exports.remove(handle)?;
        let count = self
            .references
            .get_mut(&entry.native)
            .expect("export reference count");
        *count -= 1;
        if *count != 0 {
            return None;
        }
        self.references.remove(&entry.native);
        Some(entry.native)
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
        self.nodes.retain(|_, entry| entry.realm != realm);
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
    ) -> Option<String> {
        let binding = Binding::capture(self, owner)?;
        self.with_bidi_session(owner, |state| {
            state.bidi_handles.bind(binding);
            state.bidi_handles.export(native, realm.to_owned())
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
    ) -> Option<String> {
        self.resolve_bidi_handle(owner, handle, realm)?;
        self.with_bidi_session(owner, |state| state.bidi_handles.disown(handle, realm))
            .flatten()
    }

    pub(crate) fn export_bidi_node_reference(
        &mut self,
        owner: &CommandOwnerScope,
        native: String,
        realm: &str,
    ) -> String {
        let alias = next_reference("node");
        self.register_bidi_node_alias(owner, alias.clone(), native, realm.to_owned());
        alias
    }

    pub(crate) fn register_bidi_node_alias(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: String,
        native: String,
        realm: String,
    ) {
        let Some(binding) = Binding::capture(self, owner) else {
            return;
        };
        self.with_bidi_session(owner, |state| {
            state.bidi_handles.bind(binding);
            state
                .bidi_handles
                .nodes
                .insert(shared_id, Export { native, realm });
        });
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
    use super::BidiHandles;

    #[test]
    fn exports_are_explicit_unique_realm_checked_and_reference_counted() {
        let mut handles = BidiHandles::default();
        assert_eq!(handles.resolve("native-id", None), None);
        let first = handles.export("native-id".into(), "realm-A".into());
        let second = handles.export("native-id".into(), "realm-A".into());
        assert_ne!(first, second);
        assert_eq!(handles.resolve(&first, Some("realm-B")), None);
        assert_eq!(handles.disown(&first, Some("realm-B")), None);
        assert_eq!(handles.disown(&first, Some("realm-A")), None);
        assert_eq!(handles.resolve(&second, Some("realm-A")), Some("native-id"));
        assert_eq!(
            handles.disown(&second, Some("realm-A")),
            Some("native-id".into())
        );
        assert!(handles.references.is_empty());
        assert!(handles.exports.is_empty());
    }

    #[test]
    fn clear_and_realm_retirement_cannot_revive_reused_native_ids() {
        let mut handles = BidiHandles::default();
        let old = handles.export("same-native-id".into(), "realm-A".into());
        let retained = handles.export("other-id".into(), "realm-B".into());
        handles.clear_realm("realm-A");
        assert_eq!(handles.resolve(&old, None), None);
        assert_eq!(handles.resolve(&retained, None), Some("other-id"));
        handles.clear();
        let new = handles.export("same-native-id".into(), "realm-A".into());
        assert_ne!(old, new);
        assert_eq!(handles.resolve(&old, None), None);
    }
}
