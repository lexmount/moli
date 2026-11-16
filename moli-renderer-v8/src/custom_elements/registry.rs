use crate::document_runtime::DomHandle;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) struct ChildCustomElementRegistryKey {
    pub(crate) child_handle: DomHandle,
    pub(crate) document_handle: DomHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomElementRegistryKey {
    Global,
    Child(ChildCustomElementRegistryKey),
    Scoped(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomElementRegistryAssociation {
    Null,
    Registry(CustomElementRegistryKey),
}

impl CustomElementRegistryKey {
    pub(crate) fn is_scoped(self) -> bool {
        matches!(self, Self::Scoped(_))
    }

    pub(crate) fn is_document_default_backed(self) -> bool {
        matches!(self, Self::Global | Self::Child(_))
    }
}

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
        ChildCustomElementRegistryKey, CustomElementRegistryAssociation, CustomElementRegistryKey,
    };
    use crate::document_runtime::DomHandle;

    #[test]
    fn classifies_registry_associations_for_detached_bridge_policy() {
        let global = CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Global);
        let child = CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(
            ChildCustomElementRegistryKey {
                child_handle: DomHandle::new(1),
                document_handle: DomHandle::new(2),
            },
        ));
        let scoped =
            CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Scoped(7));

        assert!(CustomElementRegistryAssociation::Null.is_null_or_scoped_registry());
        assert!(scoped.is_null_or_scoped_registry());
        assert!(scoped.is_scoped_registry());
        assert!(!global.is_null_or_scoped_registry());
        assert!(!child.is_null_or_scoped_registry());
        assert!(global.is_document_default_backed_registry());
        assert!(child.is_document_default_backed_registry());
        assert!(!scoped.is_document_default_backed_registry());
    }
}
