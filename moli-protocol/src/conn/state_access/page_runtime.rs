use anyhow::Result;

use super::ProtocolWorkflowState;
use crate::conn::{CommandWorkflowContext, CompletedRuntimeBindingPageCommandDispatch};
use crate::domains::runtime::{
    CompletedRuntimeCommandDispatch, PageRuntimeCommandContinuation, PageRuntimeCompletion,
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_page_runtime_completion(
        self,
        completed: CompletedRuntimeCommandDispatch,
        context: &mut CommandWorkflowContext,
    ) -> PageRuntimeCompletion {
        let flush = context.context_mut().response_flush().clone();
        crate::domains::runtime::prepare_page_runtime_command_completion(
            self.connection,
            completed,
            &flush,
        )
        .retain(context)
    }

    pub(crate) fn finish_page_runtime_response(
        self,
        continuation: PageRuntimeCommandContinuation,
        response: Result<moli_core::RendererRuntimeInspectorAsyncCompletion>,
        context: &mut CommandWorkflowContext,
    ) -> PageRuntimeCompletion {
        continuation
            .finish_response(self.connection, response)
            .retain(context)
    }

    pub(crate) fn finish_page_runtime_binding_state(
        self,
        continuation: PageRuntimeCommandContinuation,
        result: Result<CompletedRuntimeBindingPageCommandDispatch>,
        context: &mut CommandWorkflowContext,
    ) -> PageRuntimeCompletion {
        continuation
            .finish_binding_state(self.connection, result)
            .retain(context)
    }
}
