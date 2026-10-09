use super::*;

/// Owns every producer registration for one renderer command turn.
///
/// The scope contains no protocol state. It only decides whether records emitted
/// by lifecycle, V8 Inspector, or child-frame producers belong to the current
/// renderer command. Dropping it on an error or early return uninstalls every
/// registration before a later command can run.
pub(crate) struct RendererCommandTurnOutputScope {
    recorder: Option<RendererCommandTurnOutputRecorder>,
    pending_dom_mutation_batch_start: usize,
    document_lifecycle: RendererDocumentLifecycleJournalHandle,
    document_lifecycle_registered: bool,
    script_vm_scope: Option<crate::script_vm::ScriptVmCommandTurnOutputScope>,
}

impl RendererCommandTurnOutputScope {
    fn close(&mut self) {
        drop(self.script_vm_scope.take());
        if self.document_lifecycle_registered {
            if let Some(recorder) = self.recorder.as_ref() {
                self.document_lifecycle.end_command_turn_output(recorder);
            }
            self.document_lifecycle_registered = false;
        }
    }

    fn finish(
        mut self,
        dom_mutation_batches: Vec<RendererDomMutationEventBatch>,
    ) -> Vec<PendingRendererOutputRecord> {
        self.close();
        let recorder = self
            .recorder
            .take()
            .expect("renderer command-turn output scope recorder was already consumed");
        let mut records = recorder.finish();
        records.extend(dom_mutation_batches.into_iter().map(|batch| {
            PendingRendererOutputRecord::observation(
                None,
                RendererProtocolObservation::DomMutations(batch),
            )
        }));
        records
    }
}

impl Drop for RendererCommandTurnOutputScope {
    fn drop(&mut self) {
        self.close();
    }
}

impl PageVm {
    pub(crate) fn begin_command_turn_output_scope(
        &mut self,
    ) -> Result<RendererCommandTurnOutputScope> {
        // Activity that predates this command belongs before the command's
        // records in the ordinary Page-turn journal. Freeze it there before
        // installing the command recorder; only later batches may be claimed
        // by this command and its response fence.
        self.absorb_pending_dom_mutations_into_output_journal();
        let recorder = RendererCommandTurnOutputRecorder::default();
        let mut scope = RendererCommandTurnOutputScope {
            recorder: Some(recorder.clone()),
            pending_dom_mutation_batch_start: self.pending_dom_mutation_event_batches.len(),
            document_lifecycle: self.document_lifecycle.clone(),
            document_lifecycle_registered: false,
            script_vm_scope: None,
        };
        self.document_lifecycle
            .begin_command_turn_output(recorder.clone())?;
        scope.document_lifecycle_registered = true;
        scope.script_vm_scope = Some(self.vm().begin_command_turn_output(recorder)?);
        Ok(scope)
    }

    pub(crate) fn finish_command_turn_output_scope(
        &mut self,
        scope: RendererCommandTurnOutputScope,
    ) -> Option<RendererOutputCursor> {
        self.flush_pending_dom_mutation_event_batches();
        assert!(
            scope.pending_dom_mutation_batch_start <= self.pending_dom_mutation_event_batches.len(),
            "renderer command cannot drain the pre-command DOM mutation backlog"
        );
        let dom_mutation_batches = self
            .pending_dom_mutation_event_batches
            .split_off(scope.pending_dom_mutation_batch_start);
        let records = scope.finish(dom_mutation_batches);
        // The Page journal owns ready output even when the command must yield
        // or follow a navigation. Continuations retain only the exact cursor
        // needed by the independent internal completion channel.
        (!records.is_empty()).then(|| self.append_renderer_command_output_records(records))
    }
}
