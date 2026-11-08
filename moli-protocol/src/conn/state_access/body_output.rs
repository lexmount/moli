use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::automation::DevToolsPrintToPdfTransferMode;
use crate::conn::{CapturedBody, CommandOwnerScope, NoDocumentLoaded, RuntimePageScope};
use crate::domains::command_output::CommandOutputPlan;

#[cfg(test)]
mod tests;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_network_resource_preparation(
        self,
        owner: RuntimePageScope,
        completed: Result<moli_core::page::CompletedPageCommand>,
    ) -> Result<Box<moli_core::page::RendererPreparedNetworkResourceLoad>, CommandOutputPlan> {
        crate::domains::network::finish_resource_preparation(self.connection, &owner, completed)
    }

    pub(crate) fn prepare_pdf_output(
        self,
        owner: CommandOwnerScope,
        completed: Result<moli_core::page::CompletedPageCommand>,
        transfer_mode: DevToolsPrintToPdfTransferMode,
    ) -> Result<crate::domains::page::PreparedPdfOutput, CommandOutputPlan> {
        crate::domains::page::PreparedPdfOutput::prepare(
            self.connection,
            owner,
            completed,
            transfer_mode,
        )
    }

    pub(crate) fn open_io_stream_for_page(
        self,
        owner: RuntimePageScope,
        body: CapturedBody,
    ) -> Result<String> {
        anyhow::ensure!(owner.can_start_operation(self.connection), NoDocumentLoaded);
        self.connection
            .open_io_stream_body_source_for_owner(owner.owner(), body)
    }
}
