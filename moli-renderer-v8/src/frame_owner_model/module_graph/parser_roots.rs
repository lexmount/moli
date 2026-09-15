use crate::{
    document_module_graph::{ModuleEntryId, ModuleMapKey},
    document_runtime::DomHandle,
    document_script_scheduler::ParserPendingScriptId,
    frame_owner_model::{
        FrameDocumentModuleClientReservation, FrameDocumentModuleTerminalBatch,
        FrameDocumentParserRootModuleClient, FrameDocumentTaskOwner, FrameRealmId,
    },
    module_runtime::{ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleRequestRecord},
    planning::PreparedScript,
};
use moli_module_script_tree as module_tree;
use url::Url;

use super::ChildDocumentModulatorStore;

impl ChildDocumentModulatorStore {
    pub(crate) fn reserve_parser_root_module_client(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        client: FrameDocumentParserRootModuleClient,
    ) -> FrameDocumentModuleClientReservation {
        let document_owner = owner.document_owner();
        let document_modulator_entry = self.document_modulator_entry_mut(document_owner, realm_id);
        let reservation = document_modulator_entry
            .document_modulator
            .reserve_frame_document_parser_root_module_client(owner, key, client);
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            entry_id = reservation.entry_id().raw(),
            client_id = reservation.client_id().raw(),
            url = %reservation.key().url(),
            disposition = ?reservation.fetch_disposition(),
            "child parser module root reserved in child document modulator"
        );
        reservation
    }

    pub(crate) fn finish_parser_root_module_fetch(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        result: Result<ModuleGraphFetchedSource, String>,
    ) -> FrameDocumentModuleTerminalBatch {
        let Some(document_modulator_entry) =
            self.current_document_modulator_entry_mut(owner.document_owner(), realm_id)
        else {
            return FrameDocumentModuleTerminalBatch::default();
        };
        document_modulator_entry
            .document_modulator
            .finish_parser_root_module_fetch(request_key, result);
        document_modulator_entry.take_ready_document_modulator_terminal_batches(owner)
    }

    pub(crate) fn record_compiled_parser_root(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: ParserPendingScriptId<crate::frame_owner_model::FrameDocumentOwner>,
        script: PreparedScript,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        source_url: Url,
        entry_id: ModuleEntryId,
        parent_key: ModuleMapKey,
        requests: Vec<ModuleRequestRecord>,
        effective_fetch_metadata: ModuleFetchMetadata,
        load_delay_token: crate::frame_owner_model::ChildDocumentModuleScriptLoadDelay,
    ) -> module_tree::ModuleTreeId {
        assert_eq!(pending_script_id.owner(), owner.document_owner());
        let document_owner = owner.document_owner();
        let document_modulator_entry = self.document_modulator_entry_mut(document_owner, realm_id);
        let tree_id = document_modulator_entry
            .document_modulator
            .record_frame_document_compiled_parser_root(
                owner,
                realm_id,
                pending_script_id.key(),
                script,
                script_handle,
                request_key,
                source_url,
                entry_id,
                parent_key,
                requests,
                effective_fetch_metadata,
                load_delay_token,
            );
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            tree_id = tree_id.0,
            entry_id = entry_id.raw(),
            "child parser module root registered in child document modulator"
        );
        tree_id
    }
}
