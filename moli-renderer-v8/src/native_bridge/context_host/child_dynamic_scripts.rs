use super::*;

use crate::{
    dom::native::Node,
    frame_owner_model::{
        FrameDocumentDynamicClassicScriptExecutionAction, FrameDocumentUnboundScriptWork,
        FrameRealmId, PendingChildDynamicDocumentScript,
    },
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

    pub(crate) fn queue_child_dynamic_inline_classic_script_for_current_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner_document_handle: DomHandle,
        script_handle: DomHandle,
        source: String,
    ) -> bool {
        let Some(child_handle) =
            self.child_browsing_context_handle_by_document_handle(scope, owner_document_handle)
        else {
            return false;
        };
        if !self.child_browsing_context_is_live(child_handle)
            || self.child_browsing_context_document_handle(child_handle)
                != Some(owner_document_handle)
            || self.dom_host().owner_document_handle(script_handle) != Some(owner_document_handle)
        {
            return false;
        }
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
        else {
            return false;
        };
        let realm_id = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner);
        let script_nonce = self
            .dom_host()
            .node(script_handle)
            .and_then(Node::as_element)
            .and_then(|element| element.cryptographic_nonce())
            .map(str::to_owned)
            .or_else(|| self.dom_host().get_attribute(script_handle, "nonce"));
        let script_integrity = self.dom_host().get_attribute(script_handle, "integrity");
        self.queue_child_document_script_work_with_realm_prerequisite(
            FrameDocumentUnboundScriptWork::DynamicClassic(PendingChildDynamicDocumentScript {
                child_handle,
                owner,
                realm_id,
                script_handle,
                source,
                script_nonce,
                script_integrity,
            }),
        )
        .is_some()
    }

    pub(crate) fn child_dynamic_classic_script_execution_action_for_owner(
        &self,
        work: &PendingChildDynamicDocumentScript,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentDynamicClassicScriptExecutionAction> {
        let current_realm_id = self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(work.owner);
        if current_realm_id != Some(realm_id) {
            tracing::debug!(
                child_handle = ?work.child_handle,
                owner = ?work.owner,
                expected_realm_id = ?realm_id,
                current_realm_id = ?current_realm_id,
                script_handle = ?work.script_handle,
                "dropping child dynamic classic script whose materialized FrameRealm is no longer current"
            );
            return None;
        };
        if work.realm_id.is_some_and(|expected| expected != realm_id) {
            tracing::debug!(
                child_handle = ?work.child_handle,
                owner = ?work.owner,
                expected_realm_id = ?work.realm_id,
                current_realm_id = ?realm_id,
                script_handle = ?work.script_handle,
                "dropping child dynamic classic script with stale FrameRealm"
            );
            return None;
        }
        let mut job = self
            .frame_owner_store
            .child_dynamic_classic_script_job_for_owner(
                work.child_handle,
                work.owner.local_window_id,
                work.owner.document_id,
                Some(work.script_handle),
                work.source.clone(),
            )?;
        job.script_nonce = work.script_nonce.clone();
        job.script_integrity = work.script_integrity.clone();
        Some(FrameDocumentDynamicClassicScriptExecutionAction::new(
            work.execution_target(realm_id),
            job,
        ))
    }
}
