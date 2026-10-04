use moli_core::page::RendererCommandTurnOutput;

use super::{CdpConnection, CdpRendererOwnerTurnOutcome, CdpTurnOutcome, CommandDispatchContext};

/// A renderer acknowledgement whose concrete output is already in transport.
/// Activity never installs a full historical PageState snapshot: another
/// command or navigation may have completed before this acknowledgement arrives.
pub struct CompletedDocumentActivityUpdate {
    pub(super) completion: anyhow::Result<Box<RendererCommandTurnOutput>>,
}

impl CdpConnection {
    pub fn complete_document_activity_update(
        &mut self,
        update: CompletedDocumentActivityUpdate,
    ) -> CdpRendererOwnerTurnOutcome {
        let mut context = CommandDispatchContext::default();
        match update.completion {
            Ok(output) => {
                context.consume_renderer_command_turn_output(*output);
            }
            Err(error) => tracing::debug!(%error, "document activity owner update ended"),
        }
        CdpTurnOutcome::new_with_protocol_events(Vec::new(), self.take_scheduler_events())
            .with_renderer_output_predecessor(context.take_renderer_output_predecessor())
    }
}
