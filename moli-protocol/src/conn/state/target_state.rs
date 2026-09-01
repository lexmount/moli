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
    navigation::{PageNavigationHistoryEntry, TargetNavigationHistoryState},
    page_resource::TargetPageResourceStore,
    page_slot::DocumentStartScript,
    pending_renderer_command::{
        DuplicatePendingRendererCommand, PendingRendererCommandRegistry,
        PreparedRendererCallDispatch, PreparedRendererCallReplay, PreparedRendererCallTermination,
        RegisterRendererCallError, RendererCallIdExhausted, RendererCommandCorrelation,
        RendererCommandDescriptor,
    },
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetCrashState {
    crashed: bool,
}

impl TargetCrashState {
    pub(crate) fn mark_crashed(&mut self) {
        self.crashed = true;
    }

    pub(crate) fn clear(&mut self) {
        self.crashed = false;
    }

    pub(crate) fn is_crashed(self) -> bool {
        self.crashed
    }
}

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TargetWindowSurfaceState {
    #[default]
    Normal,
    Maximized,
    Minimized,
    Fullscreen,
}

impl TargetWindowSurfaceState {
    pub(crate) fn document_hidden(self) -> bool {
        matches!(self, Self::Minimized)
    }

    pub(crate) fn is_fullscreen(self) -> bool {
        matches!(self, Self::Fullscreen)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Maximized => "maximized",
            Self::Minimized => "minimized",
            Self::Fullscreen => "fullscreen",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetWindowSurfaceGeometry {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetInitialEmptyDocumentCreator {
    target_id: String,
    security_origin: String,
    secure_context_type: String,
}

impl TargetInitialEmptyDocumentCreator {
    pub(crate) fn new(
        target_id: String,
        security_origin: String,
        secure_context_type: String,
    ) -> Self {
        Self {
            target_id,
            security_origin,
            secure_context_type,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn security_origin(&self) -> &str {
        &self.security_origin
    }

    pub(crate) fn secure_context_type(&self) -> &str {
        &self.secure_context_type
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetInitialEmptyDocumentState {
    target_id: String,
    loader_id: String,
    initial_url: String,
    creator: Option<TargetInitialEmptyDocumentCreator>,
    storage_key: Option<moli_storage_key::MoliStorageKey>,
    materialized: bool,
    exited: bool,
    pending_cross_document_navigation: bool,
}

impl TargetInitialEmptyDocumentState {
    fn new(
        target_id: String,
        initial_url: String,
        creator: Option<TargetInitialEmptyDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        let loader_id = format!("LID-INITIAL-{target_id}");
        Self {
            target_id,
            loader_id,
            initial_url,
            creator,
            storage_key,
            materialized: false,
            exited: false,
            pending_cross_document_navigation: false,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn initial_url(&self) -> &str {
        &self.initial_url
    }

    pub(crate) fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub(crate) fn creator(&self) -> Option<&TargetInitialEmptyDocumentCreator> {
        self.creator.as_ref()
    }

    pub(crate) fn storage_key(&self) -> Option<&moli_storage_key::MoliStorageKey> {
        self.storage_key.as_ref()
    }

    pub(crate) fn materialized(&self) -> bool {
        self.materialized
    }

    pub(crate) fn exited(&self) -> bool {
        self.exited
    }

    pub(crate) fn pending_cross_document_navigation(&self) -> bool {
        self.pending_cross_document_navigation
    }

    pub(crate) fn is_on_initial_empty_document(&self) -> bool {
        !self.exited
    }

    fn mark_materialized(&mut self) {
        if !self.exited {
            self.materialized = true;
        }
    }

    fn mark_pending_cross_document_navigation(&mut self) {
        if !self.exited {
            self.pending_cross_document_navigation = true;
        }
    }

    fn clear_pending_cross_document_navigation(&mut self) {
        self.pending_cross_document_navigation = false;
    }

    fn mark_exited(&mut self) {
        self.exited = true;
        self.pending_cross_document_navigation = false;
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TargetOwnerState {
    pub(crate) initial_empty_document: Option<TargetInitialEmptyDocumentState>,
    pub(crate) committed_document_title: Option<String>,
    pub(crate) next_document_start_script_id: u32,
    pub(crate) document_start_scripts: Vec<(String, DocumentStartScript)>,
    pub(crate) navigation_history_state: TargetNavigationHistoryState,
    pub(crate) page_resource_store: TargetPageResourceStore,
    pub(crate) runtime_observable_state: TargetRuntimeObservableState,
    pub(crate) console_output_state: TargetConsoleOutputState,
    pub(crate) audits_storage_state: TargetAuditsStorageState,
    pub(crate) log_storage_state: TargetLogStorageState,
    pub(crate) target_crash_state: TargetCrashState,
    pub(crate) window_surface_state: TargetWindowSurfaceState,
    pub(crate) window_surface_geometry: TargetWindowSurfaceGeometry,
    pub(crate) attached_child_frame_ids: HashSet<String>,
}

impl TargetOwnerState {
    pub(crate) fn committed_document_title(&self) -> Option<&str> {
        self.committed_document_title.as_deref()
    }

    pub(crate) fn commit_document_title(&mut self, title: String) -> bool {
        let changed = self.committed_document_title.as_deref().unwrap_or_default() != title;
        self.committed_document_title = Some(title.clone());
        self.navigation_history_state
            .refresh_current_entry_title(title);
        changed
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

    pub(crate) fn begin_initial_empty_document(
        &mut self,
        target_id: String,
        initial_url: String,
        creator: Option<TargetInitialEmptyDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        if !is_initial_empty_document_url(&initial_url) {
            self.initial_empty_document = None;
            return;
        }
        if self.navigation_history_state.is_empty() {
            let entry = PageNavigationHistoryEntry {
                id: self.navigation_history_state.allocate_entry_id(),
                url: initial_url.clone(),
                user_typed_url: initial_url.clone(),
                title: String::new(),
                transition_type: "auto_toplevel".to_owned(),
                document_sequence_number: None,
            };
            self.navigation_history_state.seed_entry(entry);
        }
        self.initial_empty_document = Some(TargetInitialEmptyDocumentState::new(
            target_id,
            initial_url,
            creator,
            storage_key,
        ));
    }

    pub(crate) fn mark_initial_empty_document_materialized(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_materialized();
        }
    }

    pub(crate) fn mark_initial_empty_document_pending_cross_document_navigation(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_pending_cross_document_navigation();
        }
    }

    pub(crate) fn clear_initial_empty_document_pending_cross_document_navigation(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.clear_pending_cross_document_navigation();
        }
    }

    pub(crate) fn mark_initial_empty_document_exited(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_exited();
        }
    }

    pub(crate) fn initial_empty_document_state(&self) -> Option<&TargetInitialEmptyDocumentState> {
        self.initial_empty_document.as_ref()
    }

    pub(crate) fn initial_empty_document_url_if_current(&self) -> Option<&str> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .map(TargetInitialEmptyDocumentState::initial_url)
    }

    pub(crate) fn initial_empty_document_loader_id_if_current(&self) -> Option<&str> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .map(TargetInitialEmptyDocumentState::loader_id)
    }

    pub(crate) fn initial_empty_document_storage_key_if_current(
        &self,
    ) -> Option<&moli_storage_key::MoliStorageKey> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .and_then(TargetInitialEmptyDocumentState::storage_key)
    }

    pub(crate) fn is_on_initial_empty_document(&self) -> Option<bool> {
        self.initial_empty_document_state()
            .map(TargetInitialEmptyDocumentState::is_on_initial_empty_document)
    }

    pub(crate) fn can_install_current_initial_empty_document_page(&self) -> bool {
        self.initial_empty_document_state()
            .is_none_or(TargetInitialEmptyDocumentState::is_on_initial_empty_document)
    }

    pub(crate) fn initial_empty_document_pending_cross_document_navigation(&self) -> bool {
        self.initial_empty_document_state()
            .is_some_and(TargetInitialEmptyDocumentState::pending_cross_document_navigation)
    }

    pub(crate) fn has_materialized_current_initial_empty_document(&self) -> bool {
        self.initial_empty_document_state()
            .is_some_and(|state| state.is_on_initial_empty_document() && state.materialized())
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        let initial_empty_document = self.initial_empty_document.as_ref().map(|state| {
            let creator = state.creator().map(|creator| {
                json!({
                    "targetId": creator.target_id(),
                    "securityOrigin": creator.security_origin(),
                    "secureContextType": creator.secure_context_type(),
                })
            });
            json!({
                "targetId": state.target_id(),
                "initialUrl": state.initial_url(),
                "creator": creator,
                "materialized": state.materialized(),
                "exited": state.exited(),
                "pendingCrossDocumentNavigation": state.pending_cross_document_navigation(),
                "isOnInitialEmptyDocument": state.is_on_initial_empty_document(),
            })
        });
        json!({
            "initialEmptyDocument": initial_empty_document,
            "documentStartScriptCount": self.document_start_scripts.len(),
            "sessionOwnedDocumentStartScriptCount": self
                .document_start_scripts
                .iter()
                .filter(|(_, script)| script.devtools_session.is_some())
                .count(),
            "retainedPageResourceBodyBytes": self.page_resource_store.retained_body_bytes(),
            "windowSurfaceState": self.window_surface_state.label(),
            "attachedChildFrameIdCount": self.attached_child_frame_ids.len(),
            "targetCrashed": self.target_crash_state.is_crashed(),
            "isDefault": self.is_default(),
        })
    }

    fn navigation_history_entry_for_page_snapshot(
        &mut self,
        page_snapshot: (String, String),
    ) -> PageNavigationHistoryEntry {
        let (url, title) = page_snapshot;
        PageNavigationHistoryEntry {
            id: self.navigation_history_state.allocate_entry_id(),
            user_typed_url: url.clone(),
            url,
            title,
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        }
    }

    fn reconcile_navigation_history_page_snapshot(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) {
        let Some(mut page_snapshot) = page_snapshot else {
            return;
        };
        if let Some(title) = self.committed_document_title() {
            page_snapshot.1 = title.to_owned();
        }
        if !self.navigation_history_state.is_empty() {
            let (_, title) = page_snapshot;
            self.navigation_history_state
                .refresh_current_entry_title(title);
            return;
        }
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.navigation_history_state.seed_entry(entry);
    }

    pub(crate) fn refresh_current_navigation_history_title(&mut self, title: String) -> bool {
        self.navigation_history_state
            .refresh_current_entry_title(title)
    }

    pub(crate) fn mark_next_navigation_history_replace_current(&mut self) {
        self.navigation_history_state.mark_replace_current();
    }

    pub(crate) fn mark_next_navigation_history_replace_initial_empty_document(&mut self) {
        self.navigation_history_state
            .mark_replace_initial_empty_document();
    }

    pub(crate) fn mark_next_navigation_history_traverse_to_entry(&mut self, entry_id: i32) {
        self.navigation_history_state
            .mark_traverse_to_entry(entry_id);
    }

    pub(crate) fn clear_pending_navigation_history_update(&mut self) {
        self.navigation_history_state.clear_pending_update();
    }

    pub(crate) fn navigation_history_entry_url(
        &mut self,
        page_snapshot: Option<(String, String)>,
        entry_id: i32,
    ) -> Option<String> {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.entry_url(entry_id)
    }

    pub(crate) fn navigation_history_snapshot(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> (usize, Vec<PageNavigationHistoryEntry>) {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.snapshot()
    }

    pub(crate) fn reset_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> bool {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.prune_all_but_current()
    }

    pub(crate) fn can_reset_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> bool {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.can_prune_all_but_current()
    }

    pub(crate) fn record_loaded_page_navigation_history(
        &mut self,
        page_snapshot: (String, String),
    ) {
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.navigation_history_state.record_loaded_entry(entry);
    }

    pub(crate) fn clear_loaded_document_context_state(&mut self) {
        self.clear_attached_child_frame_ids();
    }

    pub(crate) fn clear_committed_document_navigation_state(&mut self) {
        self.committed_document_title = None;
        self.clear_observable_output_state();
        self.clear_loaded_document_context_state();
    }

    pub(crate) fn record_same_document_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
        url: String,
        mut title: String,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        if let Some(committed_title) = self.committed_document_title() {
            title = committed_title.to_owned();
        }
        let _ =
            self.navigation_history_state
                .record_same_document_update(url, title, history_update);
    }

    pub(crate) fn clear_observable_output_state(&mut self) {
        self.runtime_observable_state.clear();
        self.console_output_state.clear();
        self.audits_storage_state.reset_for_new_document();
        self.log_storage_state.reset_for_new_document();
    }

    pub(crate) fn set_window_surface_state(&mut self, state: TargetWindowSurfaceState) {
        self.window_surface_state = state;
    }

    pub(crate) fn set_window_surface_geometry(
        &mut self,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) {
        if let Some(width) = width {
            self.window_surface_geometry.width = width;
        }
        if let Some(height) = height {
            self.window_surface_geometry.height = height;
        }
        if let Some(x) = x {
            self.window_surface_geometry.x = x;
        }
        if let Some(y) = y {
            self.window_surface_geometry.y = y;
        }
    }

    pub(crate) fn window_document_hidden(&self) -> bool {
        self.window_surface_state.document_hidden()
    }

    pub(crate) fn window_fullscreen(&self) -> bool {
        self.window_surface_state.is_fullscreen()
    }

    pub(crate) fn is_default(&self) -> bool {
        self.initial_empty_document.is_none()
            && self.committed_document_title.is_none()
            && self.next_document_start_script_id == 0
            && self.document_start_scripts.is_empty()
            && self.navigation_history_state == TargetNavigationHistoryState::default()
            && self.page_resource_store.is_empty()
            && self.runtime_observable_state == TargetRuntimeObservableState::default()
            && self.console_output_state == TargetConsoleOutputState::default()
            && self.log_storage_state.is_empty()
            && !self.target_crash_state.is_crashed()
            && self.window_surface_state == TargetWindowSurfaceState::default()
            && self.window_surface_geometry == TargetWindowSurfaceGeometry::default()
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

fn is_initial_empty_document_url(raw_url: &str) -> bool {
    url::Url::parse(raw_url)
        .ok()
        .as_ref()
        .is_some_and(moli_url::is_about_blank)
}
