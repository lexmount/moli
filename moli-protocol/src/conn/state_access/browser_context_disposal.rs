use super::ProtocolWorkflowState;
use crate::conn::BrowserContext;
use crate::domains::target::{BrowserContextDisposalIdentity, PreparedBrowserContextDisposal};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_browser_context_worker_disposal(
        self,
        identity: BrowserContextDisposalIdentity,
    ) -> Option<crate::domains::activity::PreparedProtocolOutputs> {
        identity.prepare_worker_disposal(self.connection)
    }

    pub(crate) fn prepare_browser_context_page_close(
        self,
        disposal: PreparedBrowserContextDisposal,
        index: usize,
    ) -> Option<crate::domains::page::TargetTermination> {
        disposal.prepare_page_close(self.connection, index)
    }

    pub(crate) fn finish_browser_context_page_close(
        self,
        disposal: PreparedBrowserContextDisposal,
        completed: crate::domains::page::CompletedTargetTermination,
    ) {
        disposal.finish_page_close(self.connection, completed);
    }

    pub(crate) fn remove_disposed_browser_context(
        self,
        identity: BrowserContextDisposalIdentity,
    ) -> Option<BrowserContext> {
        identity.remove(self.connection)
    }

    pub(crate) fn finish_browser_context_disposal(
        self,
        disposal: PreparedBrowserContextDisposal,
    ) -> crate::domains::command_output::CommandOutputPlan {
        disposal.finish(self.connection)
    }
}

#[cfg(test)]
mod tests;
