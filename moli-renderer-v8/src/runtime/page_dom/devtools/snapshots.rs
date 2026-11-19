use super::super::*;
use crate::runtime::page_generated_dom::{
    user_agent_shadow_node_snapshot, user_agent_shadow_root_snapshot,
};
use moli_page_types::{DocumentNodeInspectorIdentity, DocumentNodeObjectSnapshot};
use std::collections::HashMap;

fn marker_pseudo_element_snapshot(
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        // Inspector-only pseudo elements use the originating element handle as
        // their renderer identity input. Backend/frontend ids are allocated
        // from the distinct pseudo-element registry key below.
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(DocumentNodeInspectorIdentity::MarkerPseudoElement),
        inspector_parent_identity: None,
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::Element as u8,
        node_name: "::marker".to_owned(),
        local_name: "::marker".to_owned(),
        node_value: String::new(),
        child_count: 0,
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: None,
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: true,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: Some("marker".to_owned()),
        pseudo_elements: Vec::new(),
        associated: None,
        children: Vec::new(),
    }
}

fn live_document_node_path_for_handle(dom_host: &DomHost, handle: DomHandle) -> Option<Vec<usize>> {
    let owner_document = dom_host.owner_document_handle(handle)?;
    let mut current = handle;
    let mut path = Vec::new();
    while current != owner_document {
        let parent = dom_host.dom().parent_node(current)?;
        let child_index = dom_host.child_index(parent, current)?;
        path.push(child_index);
        current = parent;
    }
    path.reverse();
    Some(path)
}

impl PageVm {
    pub(crate) fn document_node_snapshot_for_backend_node_id_in_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(key) = self.current_renderer_backend_node_key_for_id(backend_node_id) else {
            return Ok(None);
        };
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let snapshot = match key.inspector_identity {
            None => self.document_node_snapshot_for_live_handle_for_session(
                inspector_session_id,
                key.handle,
                depth,
                pierce,
                true,
                include_whitespace,
            ),
            Some(DocumentNodeInspectorIdentity::MarkerPseudoElement) => self
                .document_marker_pseudo_element_snapshot_for_live_host(
                    inspector_session_id,
                    key.handle,
                ),
            Some(identity @ DocumentNodeInspectorIdentity::UserAgentShadowTreeNode { .. }) => self
                .document_user_agent_shadow_node_snapshot_for_live_host(
                    inspector_session_id,
                    key.handle,
                    identity,
                    depth,
                    include_whitespace,
                ),
        };
        Ok(snapshot)
    }

    pub(super) fn live_document_child_node_snapshots_for_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
        depth: i32,
        pierce: bool,
    ) -> Option<Vec<DocumentNodeSnapshot>> {
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let child_handles = {
            let dom_host = self.vm().document_runtime.dom_host();
            dom_host.node(handle)?;
            dom_host
                .child_handles(handle)
                .filter(|child| {
                    include_whitespace || !inspector_whitespace_text_node(dom_host, *child)
                })
                .collect::<Vec<_>>()
        };
        let mut snapshots = Vec::with_capacity(child_handles.len());
        for child_handle in child_handles {
            if let Some(mut snapshot) = live_inspector_document_node_snapshot(
                self.vm().document_runtime.dom_host(),
                child_handle,
                depth,
                Some(handle),
                pierce,
                include_whitespace,
            ) {
                self.project_child_frame_content_documents(
                    &mut snapshot,
                    depth,
                    pierce,
                    include_whitespace,
                );
                self.project_generated_user_agent_shadow_roots(
                    &mut snapshot,
                    depth,
                    pierce,
                    include_whitespace,
                );
                self.project_generated_pseudo_elements(&mut snapshot);
                self.assign_renderer_dom_agent_ids_to_snapshot(
                    inspector_session_id,
                    &mut snapshot,
                    include_whitespace,
                );
                snapshots.push(snapshot);
            }
        }
        Some(snapshots)
    }

    pub(super) fn document_node_snapshot_for_live_handle_for_session(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
        depth: i32,
        pierce: bool,
        include_generated_user_agent_shadow_roots: bool,
        include_whitespace: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        let frame_id = self.vm().child_frame_id_for_live_node_handle(handle);
        let node_path =
            live_document_node_path_for_handle(self.vm().document_runtime.dom_host(), handle);
        let mut snapshot = live_inspector_document_node_snapshot(
            self.vm().document_runtime.dom_host(),
            handle,
            depth,
            None,
            pierce,
            include_whitespace,
        );
        if let Some(snapshot) = snapshot.as_mut() {
            self.project_child_frame_content_documents(snapshot, depth, pierce, include_whitespace);
            if include_generated_user_agent_shadow_roots {
                self.project_generated_user_agent_shadow_roots(
                    snapshot,
                    depth,
                    pierce,
                    include_whitespace,
                );
            }
            self.project_generated_pseudo_elements(snapshot);
            self.project_renderer_dom_agent_ids_to_snapshot(
                inspector_session_id,
                snapshot,
                include_whitespace,
                false,
            );
        }
        snapshot.map(|snapshot| DocumentNodeObjectSnapshot {
            frame_id,
            owner_node_id: None,
            node_path,
            snapshot,
        })
    }

    pub(super) fn document_marker_pseudo_element_snapshot_for_live_host(
        &mut self,
        inspector_session_id: Option<&str>,
        host: DomHandle,
    ) -> Option<DocumentNodeObjectSnapshot> {
        if !self
            .vm()
            .marker_pseudo_element_is_generated_for_document_snapshot(host)
        {
            return None;
        }
        let frame_id = self.vm().child_frame_id_for_live_node_handle(host);
        let originating_element = live_document_node_snapshot(
            self.vm().document_runtime.dom_host(),
            host,
            0,
            None,
            false,
        )?;
        let mut snapshot = marker_pseudo_element_snapshot(&originating_element);
        self.project_renderer_dom_agent_ids_to_snapshot(
            inspector_session_id,
            &mut snapshot,
            true,
            false,
        );
        Some(DocumentNodeObjectSnapshot {
            frame_id,
            owner_node_id: None,
            node_path: None,
            snapshot,
        })
    }

    pub(super) fn document_user_agent_shadow_node_snapshot_for_live_host(
        &mut self,
        inspector_session_id: Option<&str>,
        host: DomHandle,
        identity: DocumentNodeInspectorIdentity,
        depth: i32,
        include_whitespace: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        let frame_id = self.vm().child_frame_id_for_live_node_handle(host);
        let originating_element = live_document_node_snapshot(
            self.vm().document_runtime.dom_host(),
            host,
            0,
            None,
            false,
        )?;
        let mut snapshot = user_agent_shadow_node_snapshot(
            self.vm().document_runtime.dom_host(),
            &originating_element,
            identity,
            depth,
            include_whitespace,
        )?;
        self.project_renderer_dom_agent_ids_to_snapshot(
            inspector_session_id,
            &mut snapshot,
            include_whitespace,
            false,
        );
        Some(DocumentNodeObjectSnapshot {
            frame_id,
            owner_node_id: None,
            node_path: None,
            snapshot,
        })
    }

    pub(in crate::runtime) fn inspector_child_document_handles(
        &self,
    ) -> Vec<(DomHandle, DomHandle)> {
        self.vm()
            .live_child_document_handles_in_snapshot_order()
            .into_iter()
            .filter(|(_, owner, _)| {
                let vm = self.vm();
                vm.child_browsing_context_current_url(*owner)
                    .is_some_and(|url| {
                        child_content_document_belongs_to_top_target(
                            vm.document_runtime.document_url(),
                            &url,
                            vm.child_browsing_context_is_same_origin_with_top(*owner),
                            vm.child_browsing_context_has_opaque_origin(*owner),
                        )
                    })
            })
            .map(|(_, owner, document)| (owner, document))
            .collect()
    }

    fn project_child_frame_content_documents(
        &self,
        snapshot: &mut DocumentNodeSnapshot,
        depth: i32,
        pierce: bool,
        include_whitespace: bool,
    ) {
        let child_documents = self
            .inspector_child_document_handles()
            .into_iter()
            .collect::<HashMap<_, _>>();
        if child_documents.is_empty() {
            return;
        }

        let dom_host = self.vm().document_runtime.dom_host();
        let mut stack = vec![(snapshot, depth)];
        while let Some((snapshot, depth)) = stack.pop() {
            let next_depth = if depth > 0 { depth - 1 } else { depth };
            if snapshot.is_element
                && snapshot.inspector_identity.is_none()
                && let Some(document_handle) = child_documents.get(&snapshot.node_id)
            {
                snapshot.associated = live_inspector_document_node_snapshot(
                    dom_host,
                    *document_handle,
                    if pierce { next_depth } else { 0 },
                    None,
                    pierce,
                    include_whitespace,
                )
                .map(DocumentNodeAssociatedSnapshot::ContentDocument)
                .map(Box::new);
            }
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push((pseudo_element, next_depth));
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push((shadow_root, next_depth));
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push((associated.node_mut(), next_depth));
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push((child, next_depth));
            }
        }
    }

    fn project_generated_user_agent_shadow_roots(
        &self,
        snapshot: &mut DocumentNodeSnapshot,
        depth: i32,
        pierce: bool,
        include_whitespace: bool,
    ) {
        let mut stack = vec![(snapshot, depth)];
        while let Some((snapshot, depth)) = stack.pop() {
            let next_depth = if depth > 0 { depth - 1 } else { depth };
            if snapshot.is_element
                && snapshot.inspector_identity.is_none()
                && let Some(shadow_root) = user_agent_shadow_root_snapshot(
                    self.vm().document_runtime.dom_host(),
                    snapshot,
                    if pierce && depth != 0 { next_depth } else { 0 },
                    include_whitespace,
                )
            {
                snapshot.shadow_roots.push(shadow_root);
            }
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push((pseudo_element, next_depth));
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push((shadow_root, next_depth));
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push((associated.node_mut(), next_depth));
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push((child, next_depth));
            }
        }
    }

    fn project_generated_pseudo_elements(&self, snapshot: &mut DocumentNodeSnapshot) {
        let mut stack = vec![snapshot];
        while let Some(snapshot) = stack.pop() {
            if snapshot.is_element
                && snapshot.inspector_identity.is_none()
                && self
                    .vm()
                    .marker_pseudo_element_is_generated_for_document_snapshot(snapshot.node_id)
            {
                snapshot
                    .pseudo_elements
                    .push(marker_pseudo_element_snapshot(snapshot));
            }
            for pseudo_element in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push(pseudo_element);
            }
            for shadow_root in snapshot.shadow_roots.iter_mut().rev() {
                stack.push(shadow_root);
            }
            if let Some(associated) = snapshot.associated.as_deref_mut() {
                stack.push(associated.node_mut());
            }
            for child in snapshot.children.iter_mut().rev() {
                stack.push(child);
            }
        }
    }

    pub(in crate::runtime) fn assign_renderer_dom_agent_ids_to_snapshot(
        &mut self,
        inspector_session_id: Option<&str>,
        snapshot: &mut DocumentNodeSnapshot,
        include_whitespace: bool,
    ) {
        self.project_renderer_dom_agent_ids_to_snapshot(
            inspector_session_id,
            snapshot,
            include_whitespace,
            true,
        );
    }
}
