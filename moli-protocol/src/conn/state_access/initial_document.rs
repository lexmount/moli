use super::ProtocolWorkflowState;
use crate::conn::{InitialDocumentPageInstallResult, InitialDocumentPageOwner};
use anyhow::Result;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn initial_document_target_is_current(
        self,
        owner: &crate::conn::CommandOwnerScope,
        host: Option<&crate::conn::PageTargetHostIdentity>,
    ) -> bool {
        !self.connection.target_is_closing_for_owner(owner)
            && self
                .connection
                .page_target_host_identity_for_owner(owner)
                .is_some_and(|current| host.is_some_and(|host| host.matches(&current)))
    }

    pub(crate) fn finish_target_initialization_publication(
        self,
        initialization: crate::conn::target_initialization::TargetInitialDocumentWaiter,
    ) {
        self.connection.finish_target_initialization(initialization);
    }

    pub(crate) fn append_target_initialization_output(
        self,
        initialization: &crate::conn::target_initialization::TargetInitialDocumentWaiter,
        stage: crate::conn::TargetInitializationStage,
        into: &mut crate::conn::CommandDispatchContext,
    ) {
        self.connection
            .append_initialization_output(initialization, stage, Some(into));
    }

    pub(crate) fn publish_target_initialization_setup(
        self,
        initialization: &crate::conn::target_initialization::TargetInitialDocumentWaiter,
    ) {
        self.connection.append_initialization_output(
            initialization,
            crate::conn::TargetInitializationStage::SetupFinished,
            None,
        );
    }

    pub(crate) fn complete_target_initialization_workflow(
        self,
        completed: crate::conn::CompletedCommandWorkflow,
        mut output: crate::conn::CommandDispatchContext,
    ) -> (
        crate::conn::CommandDispatchContext,
        Option<crate::automation::DevToolsError>,
    ) {
        let mut plan = completed.finish(self.connection, &mut output);
        // This task has a typed result. Recovery's internal terminal must not
        // synthesize a second protocol response or discard its accepted output.
        let failure = plan
            .take_command_status()
            .and_then(Result::err)
            .map(|error| crate::conn::initialization_error(error.message));
        crate::conn::dispatch::append_command_output_plan(
            &mut Vec::new(),
            &mut output,
            plan,
            None,
            None,
        );
        (output, failure)
    }

    pub(crate) fn fail_target_initialization(
        self,
        owner: crate::conn::CommandOwnerScope,
        host: crate::conn::PageTargetHostIdentity,
        build: Option<InitialDocumentPageOwner>,
        message: String,
    ) -> bool {
        let current = self
            .connection
            .page_target_host_identity_for_owner(&owner)
            .is_some_and(|current| host.matches(&current));
        if current && let Some(build) = build {
            // The raw owner also checks its exact reservation and whether a
            // build is still pending. Closing a host must not orphan that wait.
            self.connection
                .fail_initial_document_page_build_for_owner(&build, message);
        }
        current && !self.connection.target_is_closing_for_owner(&owner)
    }

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
