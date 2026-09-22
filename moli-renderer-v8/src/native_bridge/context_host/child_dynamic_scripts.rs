use super::*;

use crate::{
    host::{RuntimeScriptPreparationContext, build_runtime_prepared_script},
    types::{ScriptKind, ScriptMode, ScriptSourceKind},
};

impl JsContextHost {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_child_dynamic_external_classic_script_for_current_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner_document_handle: DomHandle,
        script_handle: DomHandle,
        preparation: &RuntimeScriptPreparationContext,
        source: &str,
        kind: ScriptKind,
        mode: ScriptMode,
        source_kind: ScriptSourceKind,
    ) -> std::result::Result<bool, String> {
        if kind != ScriptKind::Classic || source_kind != ScriptSourceKind::External {
            return Ok(false);
        }
        let Some(child_handle) =
            self.child_browsing_context_handle_by_document_handle(scope, owner_document_handle)
        else {
            return Ok(false);
        };
        // Frame-document scheduling owns ordering and exact Document identities.
        // This load payload is intentionally unbound to the main scheduler.
        let script = build_runtime_prepared_script(
            preparation,
            script_handle,
            0,
            None,
            source,
            source_kind,
            kind,
            mode,
        )?;
        Ok(
            self.queue_child_external_classic_document_script_for_current_document(
                child_handle,
                owner_document_handle,
                script_handle,
                script,
            ),
        )
    }

    pub(crate) fn execute_child_dynamic_inline_classic_script_on_current_stack(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner_document_handle: DomHandle,
        script_handle: DomHandle,
        source: String,
    ) -> anyhow::Result<()> {
        let Some(child_handle) =
            self.child_browsing_context_handle_by_document_handle(scope, owner_document_handle)
        else {
            return Ok(());
        };
        if !self.child_browsing_context_is_live(child_handle)
            || self.child_browsing_context_document_handle(child_handle)
                != Some(owner_document_handle)
            || self.dom_host().owner_document_handle(script_handle) != Some(owner_document_handle)
        {
            return Ok(());
        }
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
        else {
            return Ok(());
        };
        let Some(mut job) = self
            .frame_owner_store
            .child_dynamic_classic_script_job_for_owner(
                child_handle,
                owner.local_window_id,
                owner.document_id,
                Some(script_handle),
                source,
            )
        else {
            return Ok(());
        };
        job.script_nonce = self
            .dom_host()
            .node(script_handle)
            .and_then(crate::dom::native::Node::as_element)
            .and_then(|element| element.cryptographic_nonce())
            .map(str::to_owned)
            .or_else(|| self.dom_host().get_attribute(script_handle, "nonce"));
        job.script_integrity = self.dom_host().get_attribute(script_handle, "integrity");
        // Commit before entering V8: the script can remove and reinsert itself,
        // mutate another pending script, or replace its preparation Document.
        let _ = self
            .dom_host_mut()
            .set_script_already_started(script_handle, true);
        let context = self.ensure_prebootstrapped_child_default_context(scope, child_handle)?;
        if self.current_child_document_task_owner(child_handle) != Some(owner)
            || self.dom_host().owner_document_handle(script_handle) != Some(owner_document_handle)
        {
            return Ok(());
        }
        self.execute_child_frame_script_job_on_current_stack(scope, context, job)
    }
}
