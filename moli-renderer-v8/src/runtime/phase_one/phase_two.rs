use super::loop_protocol::ParseTimePhaseTransitionReason;
use super::scaffold::debug_assert_phase_one_execution_context_for;
use super::*;

impl ConcurrentParseTimeRuntime {
    pub(super) async fn into_phase_two_execution(
        mut self,
        started: Instant,
        reason: ParseTimePhaseTransitionReason,
    ) -> Result<(
        PageVm,
        Vec<PostParsePageOwnedWork>,
        PageVmInitStage,
        Instant,
    )> {
        debug_assert_phase_one_execution_context_for(
            &self.page_vm.local_executor,
            "parse-time phase transition handoff",
        );
        self.publish_target_lifecycle_phase();
        let (stranded_parse_time_document_script_events, stranded_parse_time_lifecycle_work) =
            self.retire_main_parser_continuation();
        self.page_vm
            .vm_mut()
            .document_runtime
            .dom_host_mut()
            .ensure_html_document_shell();
        self.page_vm.vm_mut().sync_live_document_style_sources();
        let execution = match reason {
            ParseTimePhaseTransitionReason::ParserCompleted => {
                let mut scheduler = std::mem::take(&mut self.state.scheduler);
                scheduler.absorb_stranded_parse_time_document_script_events(
                    stranded_parse_time_document_script_events,
                );
                // A parser-inserted external script can suspend the main
                // parser and let the document.write continuation consume its
                // remaining input. Defer-like handoffs discovered by that
                // continuation are accepted synchronously but leave their
                // source/graph starts queued. Ordinary ParserDriver handoffs
                // start immediately, so this EOF boundary must normalize both
                // paths before sealing the one parser-deferred queue.
                self.page_vm
                    .vm_mut()
                    .start_pending_main_parser_deferred_scripts()?;
                let parser_deferred_marker = self
                    .page_vm
                    .seal_main_parser_deferred_scripts(self.parser_document_owner);
                if self.page_vm.vm().current_main_document_task_owner()
                    != Some(self.parser_document_owner)
                {
                    return Err(anyhow::anyhow!(
                        "main document owner changed while finalizing parser-deferred scripts"
                    ));
                }
                // The runtime already owns the DomHost (single-DOM authority).
                // Read directly from the runtime's live document for post-parse planning.
                let handoff = scheduler
                    .finalize_live_parser_post_parse_handoff(
                        self.page_vm.vm_mut().document_runtime.dom_host(),
                    )
                    .await;
                let mut execution = handoff.into_page_owned_work();
                execution.extend(stranded_parse_time_lifecycle_work);
                if let Some(marker) = parser_deferred_marker {
                    execution.push(marker);
                }
                execution
            }
            ParseTimePhaseTransitionReason::DocumentReplaced => {
                // The replacement transaction retired the old parser owner. Its
                // phase-one scheduler and stranded events must not be projected
                // onto the replacement document. The replacement handoffs are
                // installed by the post-parse invalidation boundary instead.
                drop(stranded_parse_time_document_script_events);
                drop(stranded_parse_time_lifecycle_work);
                Vec::new()
            }
        };
        // Run late-definition upgrades that became visible during the final parser turn.
        self.page_vm
            .vm_mut()
            .upgrade_late_defined_custom_elements_after_parser_checkpoint()?;
        Ok((self.page_vm, execution, self.stage, started))
    }
}
