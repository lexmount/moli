use super::super::*;
use crate::protocol_types::{RuntimeBindingRegistration, RuntimeIsolatedWorldDefinition};

/// A borrowed, session-scoped Runtime realm/binding capability, without Page access.
pub struct RendererRuntimeInspection<'a> {
    endpoint: &'a RendererInspectionEndpoint,
    attachment: RendererAgentAttachmentId,
    inspector_session_id: Option<String>,
}

impl RendererInspectionEndpoint {
    pub fn runtime_inspection(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
    ) -> RendererRuntimeInspection<'_> {
        RendererRuntimeInspection {
            endpoint: self,
            attachment,
            inspector_session_id,
        }
    }
}

impl RendererRuntimeInspection<'_> {
    pub fn start_resolve_blob_object(
        &self,
        object_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::resolve_blob_object(
            self.inspector_session_id.clone(),
            object_id.to_owned(),
        ))
    }

    pub fn start_apply_runtime_protocol_state(
        &self,
        session_restore_snapshots: &[RendererInspectorSessionRestoreSnapshot],
        isolated_worlds: &[RuntimeIsolatedWorldDefinition],
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::apply_runtime_protocol_state(
            self.inspector_session_id.clone(),
            session_restore_snapshots.to_vec(),
            isolated_worlds.to_vec(),
            stored_runtime_bindings.to_vec(),
            session_runtime_bindings.to_vec(),
        ))
    }

    fn start_page_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.endpoint.enqueue_typed_inspection_command(
            self.attachment,
            self.inspector_session_id.clone(),
            command,
        )
    }
    pub fn start_create_isolated_world(
        &self,
        name: &str,
        grant_universal_access: bool,
        frame_id: Option<&str>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::CreateIsolatedWorld {
            name: name.to_owned(),
            grant_universal_access,
            frame_id: frame_id.map(str::to_owned),
        })
    }

    pub fn start_create_isolated_world_runtime_activity(
        &self,
        frame_id: Option<&str>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::CreateIsolatedWorldRuntimeActivity {
            inspector_session_id: self.inspector_session_id.clone(),
            frame_id: frame_id.map(str::to_owned),
            name: name.to_owned(),
            grant_universal_access,
        })
    }

    pub fn start_install_runtime_binding(
        &self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::InstallRuntimeBinding {
            name: name.to_owned(),
            execution_context_name: execution_context_name.map(str::to_owned),
            execution_context_id,
        })
    }

    pub fn start_remove_runtime_binding(
        &self,
        name: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::RemoveRuntimeBinding(name.to_owned()))
    }

    pub fn start_set_runtime_binding_state(
        &self,
        stored_runtime_bindings: &[RuntimeBindingRegistration],
        session_runtime_bindings: &[RuntimeBindingRegistration],
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::SetRuntimeBindingState {
            inspector_session_id: self.inspector_session_id.clone(),
            stored_runtime_bindings: stored_runtime_bindings.to_vec(),
            session_runtime_bindings: session_runtime_bindings.to_vec(),
        })
    }

    pub fn start_add_document_start_script_runtime_activity(
        &self,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::AddDocumentStartScriptRuntimeActivity {
            inspector_session_id: self.inspector_session_id.clone(),
            script: script.clone(),
            run_immediately,
        })
    }

    pub fn start_remove_document_start_script_by_registry_key(
        &self,
        registry_key: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::RemoveDocumentStartScriptByRegistryKey(
            registry_key.to_owned(),
        ))
    }

    pub fn start_default_execution_context_id(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DefaultExecutionContextId)
    }

    pub fn start_default_or_initial_execution_context_id(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DefaultOrInitialExecutionContextId)
    }

    pub fn start_has_isolated_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::HasIsolatedExecutionContextId(
            execution_context_id,
        ))
    }

    pub fn start_ensure_isolated_worlds_attached_to_inspector(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::EnsureIsolatedWorldsAttachedToInspector)
    }

    pub fn start_inspector_execution_context_id_for_isolated_context(
        &self,
        execution_context_id: i64,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::InspectorExecutionContextIdForIsolatedContext(
                execution_context_id,
            ),
        )
    }

    pub fn start_isolated_execution_context_id_for_inspector_context(
        &self,
        execution_context_id: i64,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::IsolatedExecutionContextIdForInspectorContext(
                execution_context_id,
            ),
        )
    }

    pub fn start_runtime_realm_inventory(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::RuntimeRealmInventory)
    }

    pub fn start_child_default_execution_context_id_for_frame_id(
        &self,
        frame_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::ChildDefaultExecutionContextIdForFrameId(frame_id.to_owned()),
        )
    }

    pub fn start_remove_default_runtime_binding(
        &self,
        name: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::RemoveDefaultRuntimeBinding(
            name.to_owned(),
        ))
    }
}
