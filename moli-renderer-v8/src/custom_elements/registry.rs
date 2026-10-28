use crate::document_runtime::DomHandle;
use crate::native_bridge::NativeDomHostId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CustomElementRegistryKind {
    Global,
    Child(DomHandle),
    Scoped(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CustomElementRegistryKey {
    owner: NativeDomHostId,
    kind: CustomElementRegistryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomElementRegistryAssociation {
    Null,
    Registry(CustomElementRegistryKey),
}

impl CustomElementRegistryKey {
    pub(crate) fn global(owner: NativeDomHostId) -> Self {
        Self {
            owner,
            kind: CustomElementRegistryKind::Global,
        }
    }

    pub(crate) fn child(owner: NativeDomHostId, handle: DomHandle) -> Self {
        Self {
            owner,
            kind: CustomElementRegistryKind::Child(handle),
        }
    }

    pub(crate) fn scoped(owner: NativeDomHostId, id: u32) -> Self {
        Self {
            owner,
            kind: CustomElementRegistryKind::Scoped(id),
        }
    }

    pub(crate) fn owner(self) -> NativeDomHostId {
        self.owner
    }

    pub(crate) fn kind(self) -> CustomElementRegistryKind {
        self.kind
    }

    pub(crate) fn is_global(self) -> bool {
        matches!(self.kind, CustomElementRegistryKind::Global)
    }

    pub(crate) fn is_scoped(self) -> bool {
        matches!(self.kind, CustomElementRegistryKind::Scoped(_))
    }

    pub(crate) fn is_document_default_backed(self) -> bool {
        !self.is_scoped()
    }
}

// Registry associations are copied on ordinary DOM insertion, parsing and
// serialization paths. Keep the owner plus the host-local native slot within
// two words, including the nullable association's discriminant.
const _: () = assert!(std::mem::size_of::<CustomElementRegistryKey>() == 16);
const _: () = assert!(std::mem::size_of::<CustomElementRegistryAssociation>() == 16);

impl CustomElementRegistryAssociation {
    pub(crate) fn is_scoped_registry(self) -> bool {
        matches!(self, Self::Registry(key) if key.is_scoped())
    }

    pub(crate) fn is_null_or_scoped_registry(self) -> bool {
        matches!(self, Self::Null) || self.is_scoped_registry()
    }

    pub(crate) fn is_document_default_backed_registry(self) -> bool {
        matches!(self, Self::Registry(key) if key.is_document_default_backed())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AdoptionCallbackTarget {
    pub(crate) handle: DomHandle,
    pub(crate) old_document: DomHandle,
    pub(crate) new_document: DomHandle,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RegistryAssociationRetarget {
    pub(crate) handle: DomHandle,
    pub(crate) association: CustomElementRegistryAssociation,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CustomElementAdoptionPlan {
    pub(crate) targets: Vec<AdoptionCallbackTarget>,
    pub(crate) registry_retargets: Vec<RegistryAssociationRetarget>,
}

#[cfg(test)]
mod tests {
    use super::{
        CustomElementRegistryAssociation, CustomElementRegistryKey, CustomElementRegistryKind,
        NativeDomHostId,
    };
    use crate::document_runtime::DomHandle;

    #[test]
    fn classifies_registry_associations_for_detached_bridge_policy() {
        let owner = NativeDomHostId::default();
        let global =
            CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::global(owner));
        let child = CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::child(
            owner,
            DomHandle::new(1),
        ));
        let scoped =
            CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::scoped(owner, 7));

        assert!(CustomElementRegistryAssociation::Null.is_null_or_scoped_registry());
        assert!(scoped.is_null_or_scoped_registry());
        assert!(scoped.is_scoped_registry());
        assert!(!global.is_null_or_scoped_registry());
        assert!(!child.is_null_or_scoped_registry());
        assert!(global.is_document_default_backed_registry());
        assert!(child.is_document_default_backed_registry());
        assert!(!scoped.is_document_default_backed_registry());
    }

    #[test]
    fn native_owner_and_kind_distinguish_identical_local_slots() {
        let first = NativeDomHostId::default();
        let second = NativeDomHostId::default();
        let keys = [
            CustomElementRegistryKey::global(first),
            CustomElementRegistryKey::global(second),
            CustomElementRegistryKey::child(first, DomHandle::new(1)),
            CustomElementRegistryKey::child(second, DomHandle::new(1)),
            CustomElementRegistryKey::scoped(first, 1),
            CustomElementRegistryKey::scoped(second, 1),
        ];
        let distinct = keys.into_iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(distinct.len(), keys.len());
        assert_eq!(std::mem::size_of::<CustomElementRegistryKey>(), 16);
        assert_eq!(std::mem::size_of::<CustomElementRegistryAssociation>(), 16);
    }

    #[test]
    fn largest_native_slots_preserve_owner_and_kind() {
        let owner = NativeDomHostId::default();
        let last_node = DomHandle::new(u32::MAX as usize - 1);
        let child = CustomElementRegistryKey::child(owner, last_node);
        let scoped = CustomElementRegistryKey::scoped(owner, u32::MAX);
        assert_eq!(child.owner(), owner);
        assert_eq!(scoped.owner(), owner);
        assert_eq!(child.kind(), CustomElementRegistryKind::Child(last_node));
        assert_eq!(scoped.kind(), CustomElementRegistryKind::Scoped(u32::MAX));
        assert_ne!(child, scoped);
        assert_ne!(child, CustomElementRegistryKey::global(owner));
        assert_ne!(scoped, CustomElementRegistryKey::global(owner));
    }
}
