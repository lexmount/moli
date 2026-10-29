use super::ProtocolWorkflowState;
use crate::conn::{InitialDocumentPageInstallResult, InitialDocumentPageOwner};
use anyhow::Result;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn initial_document_page_can_install(self, owner: InitialDocumentPageOwner) -> bool {
        self.connection
            .initial_document_page_owner_can_install_current_page(&owner)
    }

    pub(crate) fn complete_initial_document_page_build(self, owner: InitialDocumentPageOwner) {
        self.connection
            .complete_initial_document_page_build_for_page_owner(&owner);
    }

    pub(crate) fn fail_initial_document_page_build(
        self,
        owner: InitialDocumentPageOwner,
        message: String,
    ) {
        self.connection
            .fail_initial_document_page_build_for_owner(&owner, message);
    }

    pub(crate) fn install_initial_document_page(
        self,
        owner: InitialDocumentPageOwner,
        page: moli_core::page::Page,
        artifacts: moli_core::page::RendererPageCreationArtifacts,
    ) -> (
        Result<InitialDocumentPageInstallResult>,
        Option<moli_core::page::Page>,
    ) {
        self.connection
            .install_initial_loaded_page_for_page_owner(&owner, page, artifacts)
    }
}
