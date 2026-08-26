use crate::devtools_runtime::DevToolsTargetKind;

/// Stable protocol identity for one live CDP target.
///
/// Ownership and association belong to `TargetGraph`; keeping them out of the
/// host avoids conflating Moli's internal Tab/Page relationship with CDP's
/// public `parentId` semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetHost {
    id: String,
    kind: DevToolsTargetKind,
}

impl TargetHost {
    pub(crate) fn new(id: String, kind: DevToolsTargetKind) -> Self {
        Self { id, kind }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn kind(&self) -> DevToolsTargetKind {
        self.kind
    }
}
