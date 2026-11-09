use crate::frame_owner_model::{FrameDocumentOwner, FrameRealmId};
use crate::module_runtime::ModuleEntryId;
use std::collections::BTreeMap;

use crate::frame_owner_model::LocalWindowId;

use super::document::ChildDocumentModulatorEntry;

#[derive(Default)]
pub(crate) struct ChildDocumentModulatorStore {
    pub(super) documents: BTreeMap<LocalWindowId, ChildDocumentModulatorEntry>,
}

impl ChildDocumentModulatorStore {
    pub(crate) fn mark_module_graph_evaluated(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        root_entry: ModuleEntryId,
    ) -> bool {
        let Some(document_modulator_entry) =
            self.current_document_modulator_entry_mut(owner, realm_id)
        else {
            return false;
        };
        document_modulator_entry
            .document_modulator
            .mark_evaluated(root_entry);
        true
    }
}
