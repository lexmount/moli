use super::*;

use crate::{
    dom::native::Node,
    frame_owner_model::FrameDocumentTaskOwner,
    host::{RuntimeScriptPreparationContext, build_runtime_prepared_script},
    types::{ScriptKind, ScriptMode, ScriptSourceKind},
};

#[derive(Debug)]
pub(crate) struct ChildDynamicInlineClassicScriptStart {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    script_handle: DomHandle,
    source: String,
    script_nonce: Option<String>,
    script_integrity: Option<String>,
}

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

    pub(crate) fn prepare_child_dynamic_inline_classic_script_for_current_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner_document_handle: DomHandle,
        script_handle: DomHandle,
        source: String,
    ) -> Option<ChildDynamicInlineClassicScriptStart> {
        let child_handle =
            self.child_browsing_context_handle_by_document_handle(scope, owner_document_handle)?;
        if !self.child_browsing_context_is_live(child_handle)
            || self.child_browsing_context_document_handle(child_handle)
                != Some(owner_document_handle)
            || self.dom_host().owner_document_handle(script_handle) != Some(owner_document_handle)
        {
            return None;
        }
        let owner = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)?;
        let script_nonce = self
            .dom_host()
            .node(script_handle)
            .and_then(Node::as_element)
            .and_then(|element| element.cryptographic_nonce())
            .map(str::to_owned)
            .or_else(|| self.dom_host().get_attribute(script_handle, "nonce"));
        let script_integrity = self.dom_host().get_attribute(script_handle, "integrity");
        Some(ChildDynamicInlineClassicScriptStart {
            child_handle,
            owner,
            script_handle,
            source,
            script_nonce,
            script_integrity,
        })
    }

    pub(crate) fn execute_child_dynamic_inline_classic_script_on_current_stack(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        work: ChildDynamicInlineClassicScriptStart,
    ) -> anyhow::Result<bool> {
        let ChildDynamicInlineClassicScriptStart {
            child_handle,
            owner,
            script_handle,
            source,
            script_nonce,
            script_integrity,
        } = work;
        if !self.child_browsing_context_is_live(child_handle)
            || self
                .frame_owner_store
                .current_child_document_task_owner(child_handle)
                != Some(owner)
            || self.dom_host().owner_document_handle(script_handle)
                != self.child_browsing_context_document_handle(child_handle)
        {
            return Ok(false);
        }

        let script_context =
            self.ensure_prebootstrapped_child_default_context(scope, child_handle)?;
        if self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
            != Some(owner)
        {
            return Ok(false);
        }
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
            return Ok(false);
        };
        job.script_nonce = script_nonce;
        job.script_integrity = script_integrity;
        self.execute_child_frame_script_job_on_current_stack(scope, script_context, job, false)?;
        Ok(true)
    }
}
