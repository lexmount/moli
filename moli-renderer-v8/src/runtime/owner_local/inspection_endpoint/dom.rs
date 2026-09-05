use super::super::*;

use anyhow::Result;

/// A session-scoped DOM agent capability borrowed from an inspection binding.
/// It grants no Page access, Browser operation or arbitrary renderer command.
pub struct RendererDomInspection<'a> {
    endpoint: &'a RendererInspectionEndpoint,
    attachment: RendererAgentAttachmentId,
    inspector_session_id: Option<String>,
}

impl RendererInspectionEndpoint {
    pub fn dom_inspection(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
    ) -> RendererDomInspection<'_> {
        RendererDomInspection {
            endpoint: self,
            attachment,
            inspector_session_id,
        }
    }
}

impl RendererDomInspection<'_> {
    fn start_page_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.endpoint
            .page_context_cancel_tx
            .with_inspector_admission(|| {
                self.endpoint
                    .devtools_target
                    .main_ref()
                    .enqueue_bound_protocol_page_command(
                        self.endpoint.token,
                        self.endpoint.devtools_agent_token,
                        command,
                        self.inspector_session_id.clone(),
                        self.attachment,
                    )
            })
    }

    pub fn start_child_frame_document_query_selector_for_backend_node_id(
        &self,
        include_whitespace: bool,
        frame_id: String,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentQuerySelectorForChildFrameBackendNodeId {
                inspector_session_id: self.inspector_session_id.clone(),
                include_whitespace,
                frame_id,
                root_backend_node_id,
                selector,
                multiple,
            },
        )
    }

    pub fn start_child_frame_document_root_node_reference(
        &self,
        frame_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::ChildFrameDocumentRootNodeReference {
            inspector_session_id: self.inspector_session_id.clone(),
            frame_id: frame_id.to_owned(),
        })
    }

    pub fn start_child_frame_id_for_default_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::ChildFrameIdForDefaultExecutionContextId(execution_context_id),
        )
    }

    pub fn start_child_frame_owner_node_reference(
        &self,
        frame_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::ChildFrameOwnerNodeReference {
            inspector_session_id: self.inspector_session_id.clone(),
            frame_id: frame_id.to_owned(),
        })
    }

    pub fn start_discard_document_search_results(
        &self,
        search_id: String,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentDiscardSearchResults {
            inspector_session_id: self.inspector_session_id.clone(),
            search_id,
        })
    }

    pub fn start_discard_dom_agent_frontend_bindings(
        &self,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DiscardDomAgentFrontendBindings {
            inspector_session_id: self.inspector_session_id.clone(),
        })
    }

    pub fn start_document_bidi_node_binding(
        &self,
        shared_id: String,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentBidiNodeBinding {
            inspector_session_id: self.inspector_session_id.clone(),
            shared_id,
        })
    }

    pub fn start_document_child_node_snapshot_events_for_backend_node_id(
        &self,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentChildNodeSnapshotEventsForBackendNodeId {
                inspector_session_id: self.inspector_session_id.clone(),
                include_whitespace,
                backend_node_id,
                depth,
                pierce,
            },
        )
    }

    pub fn start_document_frontend_node_binding(
        &self,
        frontend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentFrontendNodeBinding {
            inspector_session_id: self.inspector_session_id.clone(),
            frontend_node_id,
        })
    }

    pub fn start_document_frontend_node_ids_for_backend_node_ids(
        &self,
        backend_node_ids: Vec<u32>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentFrontendNodeIdsForBackendNodeIds {
                inspector_session_id: self.inspector_session_id.clone(),
                backend_node_ids,
            },
        )
    }

    pub fn start_document_geometry_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentGeometryForBackendNodeId {
            backend_node_id,
        })
    }

    pub fn start_document_geometry_for_object_id_in_inspector_session(
        &self,
        object_id: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::document_geometry_for_object_id(
            self.inspector_session_id.clone(),
            object_id.to_owned(),
        ))
    }

    pub fn start_document_hit_test(
        &self,
        x: f64,
        y: f64,
        include_user_agent_shadow_dom: bool,
        ignore_pointer_events_none: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentHitTest {
            inspector_session_id: self.inspector_session_id.clone(),
            x,
            y,
            include_user_agent_shadow_dom,
            ignore_pointer_events_none,
        })
    }

    pub fn start_document_node_attributes_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentNodeAttributesForBackendNodeId { backend_node_id },
        )
    }

    pub fn start_document_node_property_for_backend_node_id(
        &self,
        backend_node_id: u32,
        name: &str,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentNodePropertyForBackendNodeId {
            backend_node_id,
            name: name.to_owned(),
        })
    }

    pub fn start_document_node_snapshot_for_backend_node_id(
        &self,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentNodeSnapshotForBackendNodeId {
            backend_node_id,
            depth,
            pierce,
        })
    }

    pub fn start_document_node_snapshot_for_backend_node_id_in_inspector_session(
        &self,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentNodeSnapshotForBackendNodeIdInInspectorSession {
                inspector_session_id: self.inspector_session_id.clone(),
                include_whitespace,
                backend_node_id,
                depth,
                pierce,
            },
        )
    }

    pub fn start_document_node_snapshot_for_document(
        &self,
        include_whitespace: bool,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentNodeSnapshotForDocument {
            inspector_session_id: self.inspector_session_id.clone(),
            include_whitespace,
            depth,
            pierce,
        })
    }

    pub fn start_document_node_snapshot_for_object_id_in_inspector_session(
        &self,
        include_whitespace: bool,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::document_node_snapshot_for_object_id(
            self.inspector_session_id.clone(),
            include_whitespace,
            object_id.to_owned(),
            depth,
            pierce,
        ))
    }

    pub fn start_document_node_stack_trace(
        &self,
        frontend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentNodeStackTrace {
            inspector_session_id: self.inspector_session_id.clone(),
            frontend_node_id,
        })
    }

    pub fn start_document_node_text_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentNodeTextForBackendNodeId {
            backend_node_id,
        })
    }

    pub fn start_document_perform_search(
        &self,
        query: String,
        include_user_agent_shadow_dom: bool,
        include_whitespace: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentPerformSearch {
            inspector_session_id: self.inspector_session_id.clone(),
            query,
            include_user_agent_shadow_dom,
            include_whitespace,
        })
    }

    pub fn start_document_query_selector_for_backend_node_id_in_inspector_session(
        &self,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentQuerySelectorForBackendNodeId {
            inspector_session_id: self.inspector_session_id.clone(),
            include_whitespace,
            root_backend_node_id,
            selector,
            multiple,
        })
    }

    pub fn start_document_query_selector_for_document_in_inspector_session(
        &self,
        include_whitespace: bool,
        selector: String,
        multiple: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentQuerySelectorForDocument {
            inspector_session_id: self.inspector_session_id.clone(),
            include_whitespace,
            selector,
            multiple,
        })
    }

    pub fn start_document_query_selector_with_child_node_snapshot_events_for_backend_node_id(
        &self,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId {
                inspector_session_id: self.inspector_session_id.clone(),
                include_whitespace,
                root_backend_node_id,
                selector,
                multiple,
            },
        )
    }

    pub fn start_document_search_results(
        &self,
        search_id: String,
        from_index: usize,
        to_index: usize,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentGetSearchResults {
            inspector_session_id: self.inspector_session_id.clone(),
            search_id,
            from_index,
            to_index,
        })
    }

    pub fn start_edit_document_node(
        &self,
        edit: RendererDomEdit,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::EditDocumentNode {
            inspector_session_id: self.inspector_session_id.clone(),
            edit,
        })
    }

    pub fn start_focus_document_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::FocusDocumentBackendNode { backend_node_id })
    }

    pub fn start_focus_document_node_for_object_id(
        &self,
        object_id: String,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::focus_document_node_for_object_id(
            self.inspector_session_id.clone(),
            object_id,
        ))
    }

    pub fn start_mutate_document_backend_node_attribute(
        &self,
        backend_node_id: u32,
        mutation: RendererDomAttributeMutation,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::MutateDocumentBackendNodeAttribute {
            backend_node_id,
            mutation,
        })
    }

    pub fn start_outer_html_for_backend_node_id(
        &self,
        backend_node_id: u32,
        include_shadow_dom: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::OuterHtmlForBackendNodeId {
            backend_node_id,
            include_shadow_dom,
        })
    }

    pub fn start_outer_html_for_document(
        &self,
        include_shadow_dom: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::OuterHtmlForDocument { include_shadow_dom })
    }

    pub fn start_outer_html_for_object_id_in_inspector_session(
        &self,
        object_id: &str,
        include_shadow_dom: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::outer_html_for_object_id(
            self.inspector_session_id.clone(),
            object_id.to_owned(),
            include_shadow_dom,
        ))
    }

    pub fn start_remove_document_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::RemoveDocumentBackendNodeId {
            backend_node_id,
        })
    }

    pub fn start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
        &self,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::resolve_runtime_object_for_backend_node_id(
                self.inspector_session_id.clone(),
                backend_node_id,
                execution_context_id,
                object_group.map(str::to_owned),
            ),
        )
    }

    pub fn start_scroll_backend_node_into_view_if_needed(
        &self,
        backend_node_id: u32,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::ScrollBackendNodeIntoViewIfNeeded {
            backend_node_id,
            rect,
        })
    }

    pub fn start_scroll_node_into_view_if_needed_for_object_id_in_inspector_session(
        &self,
        object_id: &str,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::scroll_object_node_into_view_if_needed(
            self.inspector_session_id.clone(),
            object_id.to_owned(),
            rect,
        ))
    }

    pub fn start_set_document_node_stack_traces_enabled(
        &self,
        enabled: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DocumentSetNodeStackTracesEnabled {
            inspector_session_id: self.inspector_session_id.clone(),
            enabled,
        })
    }

    pub fn start_set_file_input_files_for_backend_node_id(
        &self,
        backend_node_id: u32,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::SetFileInputFilesForBackendNodeId {
            backend_node_id,
            files,
            append,
        })
    }

    pub fn start_set_file_input_files_for_object_id_in_inspector_session(
        &self,
        object_id: &str,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::set_file_input_files_for_object_id(
            self.inspector_session_id.clone(),
            object_id.to_owned(),
            files,
            append,
        ))
    }
    pub fn start_register_document_bidi_node_binding(
        &self,
        shared_id: String,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::RegisterDocumentBidiNodeBinding {
            inspector_session_id: self.inspector_session_id.clone(),
            shared_id,
            backend_node_id,
        })
    }
    pub fn start_document_bidi_node_shared_id_for_backend_node_id(
        &self,
        backend_node_id: u32,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DocumentBidiNodeSharedIdForBackendNodeId {
                inspector_session_id: self.inspector_session_id.clone(),
                backend_node_id,
            },
        )
    }
}
