//! State owned for the lifetime of one stable protocol target.

use std::collections::HashSet;

use moli_page_types::{DevToolsSessionKey, FrontendCommandId};
use serde_json::{Value, json};

use crate::{
    devtools_runtime::{
        DevToolsBidiChannelProperties, DevToolsRealmId, DevToolsRemoteHandleId, DevToolsTargetId,
    },
    domains::{
        audits_output_state::TargetAuditsStorageState,
        console_output_state::TargetConsoleOutputState, log_output_state::TargetLogStorageState,
        observable_output::TargetRuntimeObservableState,
    },
};

use super::{
    page_resource::TargetPageResourceStore,
    page_slot::DocumentStartScript,
    pending_renderer_command::{
        DuplicatePendingRendererCommand, PendingRendererCommandRegistry,
        PreparedRendererCallDispatch, PreparedRendererCallReplay, PreparedRendererCallTermination,
        RegisterRendererCallError, RendererCallIdExhausted, RendererCommandCorrelation,
        RendererCommandDescriptor,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingBidiChannelListener {
    target_id: DevToolsTargetId,
    realm_id: DevToolsRealmId,
    channel_handle: DevToolsRemoteHandleId,
    channel_object_group: String,
    properties: DevToolsBidiChannelProperties,
}

impl PendingBidiChannelListener {
    pub(crate) fn new(
        target_id: Option<DevToolsTargetId>,
        realm_id: Option<DevToolsRealmId>,
        channel_handle: DevToolsRemoteHandleId,
        channel_object_group: String,
        properties: DevToolsBidiChannelProperties,
    ) -> Option<Self> {
        Some(Self {
            target_id: target_id?,
            realm_id: realm_id?,
            channel_handle,
            channel_object_group,
            properties,
        })
    }

    pub(crate) fn target_id(&self) -> &DevToolsTargetId {
        &self.target_id
    }

    pub(crate) fn realm_id(&self) -> &DevToolsRealmId {
        &self.realm_id
    }

    pub(crate) fn channel_handle(&self) -> &DevToolsRemoteHandleId {
        &self.channel_handle
    }

    pub(crate) fn channel_object_group(&self) -> &str {
        &self.channel_object_group
    }

    pub(crate) fn properties(&self) -> &DevToolsBidiChannelProperties {
        &self.properties
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingInspectorAwait {
    session_id: Option<String>,
    object_group: Option<String>,
    bidi_channel_listener: Option<crate::conn::BidiChannelListenerResidence>,
    renderer_correlation: Option<RendererCommandCorrelation>,
}

impl PendingInspectorAwait {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn object_group(&self) -> Option<&str> {
        self.object_group.as_deref()
    }

    pub(crate) fn bidi_channel_listener(
        &self,
    ) -> Option<&crate::conn::BidiChannelListenerResidence> {
        self.bidi_channel_listener.as_ref()
    }

    pub(crate) fn renderer_correlation(&self) -> Option<RendererCommandCorrelation> {
        self.renderer_correlation
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TargetPendingInspectorAwaitRegistry {
    entries: PendingRendererCommandRegistry<PendingInspectorAwait>,
}

impl TargetPendingInspectorAwaitRegistry {
    pub(crate) fn try_insert(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let frontend_command_id = FrontendCommandId::new(cdp_request_id);
        self.entries.try_insert(
            frontend_command_id,
            PendingInspectorAwait {
                session_id: session_id.map(str::to_owned),
                object_group: object_group.map(str::to_owned),
                bidi_channel_listener: None,
                renderer_correlation: None,
            },
        )
    }

    pub(crate) fn try_insert_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
        listener: crate::conn::BidiChannelListenerResidence,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let frontend_command_id = FrontendCommandId::new(cdp_request_id);
        let renderer_correlation = self.entries.renderer_call_for_frontend(frontend_command_id);
        self.entries.try_insert(
            frontend_command_id,
            PendingInspectorAwait {
                session_id: session_id.map(str::to_owned),
                object_group: object_group.map(str::to_owned),
                bidi_channel_listener: Some(listener),
                renderer_correlation,
            },
        )
    }

    pub(crate) fn insert_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
        listener: crate::conn::BidiChannelListenerResidence,
    ) {
        self.try_insert_bidi_channel_listener(cdp_request_id, session_id, object_group, listener)
            .expect("pending BiDi listener frontend command id must be unique per session");
    }

    pub(crate) fn try_register_renderer_call(
        &mut self,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, RegisterRendererCallError> {
        let dispatch = self.entries.try_register_renderer_call(
            FrontendCommandId::new(cdp_request_id),
            dispatched_attachment_id,
            descriptor,
        )?;
        if let Some(entry) = self.entries.get_mut(FrontendCommandId::new(cdp_request_id)) {
            entry.renderer_correlation = Some(dispatch.correlation());
        }
        Ok(dispatch)
    }

    pub(crate) fn take_renderer_call_for_frontend(
        &mut self,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.entries
            .take_renderer_call_for_frontend(FrontendCommandId::new(cdp_request_id))
    }

    pub(crate) fn renderer_call_for_frontend(
        &self,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.entries
            .renderer_call_for_frontend(FrontendCommandId::new(cdp_request_id))
    }

    pub(crate) fn renderer_command_descriptor_for_renderer_if_attachment_matches(
        &self,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        self.entries
            .renderer_command_descriptor_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn prepare_renderer_call_replays(
        &mut self,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        new_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallReplay>, RendererCallIdExhausted> {
        self.entries
            .prepare_replays_from_attachment(old_attachment_id, new_attachment_id)
    }

    pub(crate) fn prepare_renderer_call_terminations(
        &mut self,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        terminal_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallTermination>, RendererCallIdExhausted> {
        self.entries
            .prepare_terminations_from_attachment(old_attachment_id, terminal_attachment_id)
    }

    pub(crate) fn terminate_all_renderer_calls(
        &mut self,
        reason: &str,
    ) -> Vec<RendererCommandCorrelation> {
        self.entries.terminate_all_renderer_calls(reason)
    }

    pub(crate) fn take_renderer_call_for_frontend_if_matches(
        &mut self,
        cdp_request_id: u64,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.entries.take_renderer_call_for_frontend_if_matches(
            FrontendCommandId::new(cdp_request_id),
            renderer_call_id,
            dispatched_attachment_id,
        )
    }

    pub(crate) fn take_frontend_command_for_renderer_if_attachment_matches(
        &mut self,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.entries
            .take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn remove(&mut self, cdp_request_id: u64) -> Option<PendingInspectorAwait> {
        self.entries.remove(FrontendCommandId::new(cdp_request_id))
    }

    pub(crate) fn drain_all(&mut self) -> Vec<(u64, PendingInspectorAwait)> {
        let to_remove = self.entries.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        to_remove
            .into_iter()
            .filter_map(|id| {
                self.remove_for_cancellation(id)
                    .map(|entry| (id.get(), entry))
            })
            .collect()
    }

    fn remove_for_cancellation(
        &mut self,
        frontend_command_id: FrontendCommandId,
    ) -> Option<PendingInspectorAwait> {
        let entry = self.entries.remove(frontend_command_id)?;
        if let Some(correlation) = entry.renderer_correlation {
            let removed = self.entries.take_renderer_call_for_frontend_if_matches(
                frontend_command_id,
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            );
            debug_assert_eq!(removed, Some(correlation));
        }
        Some(entry)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TargetOwnerState {
    pub(crate) committed_document_title: Option<String>,
    pub(crate) next_document_start_script_id: u32,
    pub(crate) document_start_scripts: Vec<(String, DocumentStartScript)>,
    pub(crate) page_resource_store: TargetPageResourceStore,
    pub(crate) runtime_observable_state: TargetRuntimeObservableState,
    pub(crate) console_output_state: TargetConsoleOutputState,
    pub(crate) audits_storage_state: TargetAuditsStorageState,
    pub(crate) log_storage_state: TargetLogStorageState,
    pub(crate) attached_child_frame_ids: HashSet<String>,
}

impl TargetOwnerState {
    pub(crate) fn committed_document_title(&self) -> Option<&str> {
        self.committed_document_title.as_deref()
    }

    pub(crate) fn has_bidi_channel_preload_script(&self) -> bool {
        self.document_start_scripts
            .iter()
            .any(|(_, script)| script.has_bidi_channel_argument)
    }

    pub(crate) fn document_start_script_registry_keys_for_session(
        &self,
        devtools_session: &DevToolsSessionKey,
    ) -> Vec<String> {
        self.document_start_scripts
            .iter()
            .filter_map(|(_, script)| {
                (script.devtools_session.as_ref() == Some(devtools_session))
                    .then(|| script.registry_key.clone())
                    .flatten()
            })
            .collect()
    }

    pub(crate) fn remove_document_start_scripts_for_session(
        &mut self,
        devtools_session: &DevToolsSessionKey,
    ) {
        self.document_start_scripts
            .retain(|(_, script)| script.devtools_session.as_ref() != Some(devtools_session));
    }

    pub(crate) fn remove_document_start_script_registry_key_for_session(
        &mut self,
        devtools_session: &DevToolsSessionKey,
        registry_key: &str,
    ) -> bool {
        let Some(index) = self.document_start_scripts.iter().position(|(_, script)| {
            script.devtools_session.as_ref() == Some(devtools_session)
                && script.registry_key.as_deref() == Some(registry_key)
        }) else {
            return false;
        };
        self.document_start_scripts.remove(index);
        true
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        json!({
            "documentStartScriptCount": self.document_start_scripts.len(),
            "sessionOwnedDocumentStartScriptCount": self
                .document_start_scripts
                .iter()
                .filter(|(_, script)| script.devtools_session.is_some())
                .count(),
            "retainedPageResourceBodyBytes": self.page_resource_store.retained_body_bytes(),
            "attachedChildFrameIdCount": self.attached_child_frame_ids.len(),
            "isDefault": self.is_default(),
        })
    }

    pub(crate) fn clear_loaded_document_context_state(&mut self) {
        self.clear_attached_child_frame_ids();
    }

    pub(crate) fn clear_committed_document_navigation_state(&mut self) {
        self.committed_document_title = None;
        self.clear_observable_output_state();
        self.clear_loaded_document_context_state();
    }

    pub(crate) fn clear_observable_output_state(&mut self) {
        self.runtime_observable_state.clear();
        self.console_output_state.clear();
        self.audits_storage_state.reset_for_new_document();
        self.log_storage_state.reset_for_new_document();
    }

    pub(crate) fn is_default(&self) -> bool {
        self.committed_document_title.is_none()
            && self.next_document_start_script_id == 0
            && self.document_start_scripts.is_empty()
            && self.page_resource_store.is_empty()
            && self.runtime_observable_state == TargetRuntimeObservableState::default()
            && self.console_output_state == TargetConsoleOutputState::default()
            && self.log_storage_state.is_empty()
            && self.attached_child_frame_ids.is_empty()
    }

    pub(crate) fn insert_attached_child_frame_id(&mut self, frame_id: String) -> bool {
        self.attached_child_frame_ids.insert(frame_id)
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.attached_child_frame_ids.contains(frame_id)
    }

    pub(crate) fn remove_attached_child_frame_id(&mut self, frame_id: &str) -> bool {
        self.attached_child_frame_ids.remove(frame_id)
    }

    pub(crate) fn clear_attached_child_frame_ids(&mut self) {
        self.attached_child_frame_ids.clear();
    }
}
