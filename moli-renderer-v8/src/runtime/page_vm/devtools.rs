use std::collections::BTreeMap;

use moli_page_types::V8InspectorSessionAttach;

use super::*;
use crate::runtime::RendererPageStateCapturePolicy;

pub(in crate::runtime) struct PageDevToolsState {
    pub(super) pending_dom_mutation_event_batches: Vec<RendererDomMutationEventBatch>,
    pub(super) css_agent_sessions: HashMap<Option<String>, RendererCssAgentSessionState>,
    pub(in crate::runtime) inspector_protocol_configurations:
        BTreeMap<DevToolsSessionKey, RendererInspectorProtocolConfiguration>,
}

impl PageDevToolsState {
    pub(super) fn new(env: &PageVmEnvConfig) -> Self {
        Self {
            pending_dom_mutation_event_batches: Vec::new(),
            css_agent_sessions: HashMap::new(),
            inspector_protocol_configurations: env.inspector_protocol_configurations(),
        }
    }
}

impl PageVmEnvConfig {
    pub(super) fn inspector_protocol_configurations(
        &self,
    ) -> BTreeMap<DevToolsSessionKey, RendererInspectorProtocolConfiguration> {
        self.runtime_inspector_session_restore_snapshots
            .iter()
            .filter(|restore| restore.protocol_configuration.requires_restore())
            .map(|restore| {
                (
                    DevToolsSessionKey::from_wire_session_id(
                        restore
                            .inspector_session_id
                            .as_deref()
                            .filter(|id| !id.is_empty()),
                    ),
                    restore.protocol_configuration.clone(),
                )
            })
            .collect()
    }
}

impl PageVm {
    /// Nested Main dispatch cannot end the suspended outer command scope.
    /// Drain its ready prefix, materialize native DOM facts, and use the same
    /// Inspector-state resolution as ordinary owner settlement before replying.
    pub(in crate::runtime) fn publish_nested_command_output_prefix(
        &mut self,
    ) -> Option<super::RendererOutputFence> {
        self.vm().append_live_command_output_prefix();
        self.absorb_pending_dom_mutations_into_output_journal();
        self.record_document_title_change_if_needed();
        if let Some(publication) = self.vm_mut().settle_renderer_output_prefix() {
            let journal = self.vm().renderer_command_output_journal();
            return journal.publish_resolved_prefix_and_declare_fence(publication);
        }
        // Inspector callbacks can publish the prefix while the owner remains
        // paused. An empty pending journal still needs a fence for that concrete
        // tail before an internal reply starts navigation or replaces the Page.
        let cursor = self.renderer_output_tail_cursor()?;
        Some(self.declare_renderer_output_fence(cursor))
    }

    pub(super) fn runtime_inspector_session_restore_snapshots(
        &self,
    ) -> Vec<RendererInspectorSessionRestoreSnapshot> {
        let mut snapshots = self
            .devtools
            .inspector_protocol_configurations
            .iter()
            .map(|(session_key, protocol_configuration)| {
                (
                    session_key.clone(),
                    RendererInspectorSessionRestoreSnapshot {
                        inspector_session_id: session_key.wire_session_id().map(str::to_owned),
                        protocol_configuration: protocol_configuration.clone(),
                        ..RendererInspectorSessionRestoreSnapshot::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for (session_key, state) in self.vm().inspector_v8_session_states() {
            snapshots
                .entry(session_key.clone())
                .or_insert_with(|| RendererInspectorSessionRestoreSnapshot {
                    inspector_session_id: session_key.wire_session_id().map(str::to_owned),
                    ..RendererInspectorSessionRestoreSnapshot::default()
                })
                .v8_attach = V8InspectorSessionAttach::Reattach(state);
        }
        snapshots.into_values().collect()
    }

    /// A debugger callback already holds the document isolate. The nested
    /// handler can capture native metadata/reports, but must not read JS
    /// location objects or snapshot globals through another isolate borrow.
    pub(in crate::runtime) fn capture_nested_page_state(&mut self) -> Result<PageVmStateCapture> {
        self.capture_page_state_with_policy(RendererPageStateCapturePolicy::ProtocolTurn)
    }

    pub(crate) fn configure_document_dom_agent_session(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.set_include_whitespace(
            inspector_session_id,
            document_id,
            include_whitespace,
        );
    }

    pub(in crate::runtime) fn document_dom_agent_includes_whitespace(
        &self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        self.dom_agent_state
            .includes_whitespace(inspector_session_id, self.current_dom_agent_document_id())
    }

    pub(in crate::runtime) fn discard_document_frontend_bindings(
        &mut self,
        inspector_session_id: Option<&str>,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .discard_frontend_bindings(inspector_session_id, document_id);
        self.sync_devtools_dom_mutation_recording_interest();
    }

    pub(in crate::runtime::page_vm) fn register_document_search_results(
        &mut self,
        inspector_session_id: Option<&str>,
        matches: Vec<dom_search::DocumentSearchMatch>,
    ) -> crate::RendererDomSearchRegistration {
        let mut backend_node_ids = Vec::with_capacity(matches.len());
        for search_match in matches {
            let backend_node_id = match search_match {
                dom_search::DocumentSearchMatch::Live(handle) => {
                    let Some(backend_node_id) =
                        self.renderer_backend_node_id_for_live_handle(handle)
                    else {
                        continue;
                    };
                    backend_node_id
                }
                dom_search::DocumentSearchMatch::Generated { host, identity } => {
                    let Some(backend_node_id) =
                        self.renderer_backend_node_id_for_inspector_node(host, identity)
                    else {
                        continue;
                    };
                    backend_node_id
                }
            };
            backend_node_ids.push(backend_node_id);
        }
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.register_search_results(
            inspector_session_id,
            document_id,
            backend_node_ids,
        )
    }

    pub(in crate::runtime) fn document_frontend_node_id_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> u32 {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.frontend_node_id_for_backend_node_id(
            inspector_session_id,
            document_id,
            backend_node_id,
        )
    }

    pub(in crate::runtime) fn document_frontend_node_id_for_backend_node_id_in_whitespace_projection(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        include_whitespace: bool,
        is_whitespace_text: bool,
    ) -> u32 {
        // Chromium keeps the node's backend identity and any search-result
        // position, but represents a node hidden from this session's Inspector
        // projection with frontend id 0. In particular, do not create a binding
        // that another command can use to re-expose the whitespace node.
        if !include_whitespace && is_whitespace_text {
            return 0;
        }
        self.document_frontend_node_id_for_backend_node_id(inspector_session_id, backend_node_id)
    }

    pub(in crate::runtime) fn document_node_is_published(
        &self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> bool {
        self.dom_agent_state.node_is_published(
            inspector_session_id,
            self.current_dom_agent_document_id(),
            backend_node_id,
        )
    }

    pub(in crate::runtime) fn existing_document_frontend_node_id(
        &self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> Option<u32> {
        self.dom_agent_state
            .frontend_node_id_for_existing_backend_node_id(
                inspector_session_id,
                self.current_dom_agent_document_id(),
                backend_node_id,
            )
    }

    pub(in crate::runtime) fn document_frontend_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
    ) -> crate::RendererDomFrontendNodeBindingResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.frontend_node_binding(
            inspector_session_id,
            document_id,
            frontend_node_id,
        )
    }

    /// Resolve an Inspector input without exposing an internal-only binding.
    pub(in crate::runtime) fn published_document_frontend_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
    ) -> crate::RendererDomFrontendNodeBindingResolution {
        match self.document_frontend_node_binding(inspector_session_id, frontend_node_id) {
            crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id)
                if self.document_node_is_published(inspector_session_id, backend_node_id) =>
            {
                crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id)
            }
            _ => crate::RendererDomFrontendNodeBindingResolution::NotFound,
        }
    }

    pub(in crate::runtime) fn register_document_bidi_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        shared_id: String,
        backend_node_id: u32,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.register_bidi_node_binding(
            inspector_session_id,
            document_id,
            shared_id,
            backend_node_id,
        );
    }

    pub(in crate::runtime) fn document_bidi_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        shared_id: &str,
    ) -> crate::RendererDomBidiNodeBindingResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .bidi_node_binding(inspector_session_id, document_id, shared_id)
    }

    pub(in crate::runtime) fn document_bidi_node_shared_id_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> crate::RendererDomBidiNodeSharedIdResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .bidi_node_shared_id_for_backend_node_id(
                inspector_session_id,
                document_id,
                backend_node_id,
            )
    }

    pub(in crate::runtime) fn document_search_results(
        &mut self,
        inspector_session_id: Option<&str>,
        search_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> crate::RendererDomSearchResultsResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.search_results_slice(
            inspector_session_id,
            document_id,
            search_id,
            from_index,
            to_index,
        )
    }

    pub(in crate::runtime) fn discard_document_search_results(
        &mut self,
        inspector_session_id: Option<&str>,
        search_id: &str,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .discard_search_results(inspector_session_id, document_id, search_id);
    }

    pub(in crate::runtime) fn set_document_node_stack_traces_enabled(
        &mut self,
        inspector_session_id: Option<&str>,
        enabled: bool,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.set_node_stack_traces_enabled(
            inspector_session_id,
            document_id,
            enabled,
        );
    }

    pub(in crate::runtime) fn document_node_stack_trace(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
    ) -> crate::RendererDomNodeStackTraceResolution {
        let document_id = self.current_dom_agent_document_id();
        let crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id) =
            self.published_document_frontend_node_binding(inspector_session_id, frontend_node_id)
        else {
            return crate::RendererDomNodeStackTraceResolution::MissingNode;
        };
        let Some(key) = self
            .dom_agent_state
            .backend_node_key_for_id(backend_node_id)
        else {
            return crate::RendererDomNodeStackTraceResolution::MissingNode;
        };
        if Some(key.document_id) != document_id
            || key.inspector_identity.is_some()
            || self
                .vm()
                .document_runtime
                .dom_host()
                .node(key.handle)
                .is_none()
        {
            return crate::RendererDomNodeStackTraceResolution::MissingNode;
        }
        crate::RendererDomNodeStackTraceResolution::Found(
            self.dom_agent_state.node_creation_stack_trace(
                inspector_session_id,
                document_id,
                key.document_id,
                key.handle,
            ),
        )
    }

    pub(in crate::runtime) fn bind_file_chooser_activation_backend_node_id(
        &mut self,
        activation: &mut crate::RendererPendingFileChooserActivation,
    ) -> bool {
        let Some((handle, document_id)) = activation.live_node_source() else {
            return activation.backend_node_id != 0;
        };
        activation.node_id = Some(handle);
        activation.backend_node_id =
            self.renderer_backend_node_id_for_node_key(document_id, handle);
        assert!(
            self.dom_agent_state
                .retain_detached_backend_node_resolution(activation.backend_node_id),
            "file chooser backend node id must exist before it is exposed"
        );
        true
    }
}

impl PageVm {
    pub(in crate::runtime) fn renderer_backend_node_id_for_inspector_node(
        &mut self,
        host: DomHandle,
        inspector_identity: moli_page_types::DocumentNodeInspectorIdentity,
    ) -> Option<u32> {
        let document_id = self.vm().document_id_for_live_node_handle(host)?;
        Some(self.dom_agent_state.backend_node_id_for_inspector_node(
            document_id,
            host,
            inspector_identity,
        ))
    }

    pub(super) fn current_dom_agent_document_id(&self) -> Option<DocumentId> {
        self.vm()
            .current_main_document_task_owner()
            .map(|owner| owner.document_id)
            .or_else(|| {
                let document_handle = self
                    .vm()
                    .document_runtime
                    .dom_host()
                    .dom()
                    .document_node_id();
                self.vm().document_id_for_live_node_handle(document_handle)
            })
    }
}
