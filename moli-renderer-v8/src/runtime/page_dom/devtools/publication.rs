use super::super::*;
use super::child_frame_nodes::child_frame_document_contains_live_handle;
use crate::runtime::page_generated_dom::user_agent_shadow_node_snapshot;
use crate::runtime::page_surface::{
    RendererDocumentChildNodeSnapshotEvent, RendererDocumentChildNodeSnapshotEvents,
};
use crate::runtime::page_surface::{
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentNodeReference,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents,
};
use moli_page_types::{DocumentNodeInspectorIdentity, DocumentNodeObjectSnapshot};

impl PageVm {
    pub(crate) fn document_node_snapshot_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        depth: i32,
        pierce: bool,
        bind_frontend_nodes: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        let mut snapshot = self.document_node_snapshot_for_live_handle_for_session(
            inspector_session_id,
            handle,
            depth,
            pierce,
            true,
            self.document_dom_agent_includes_whitespace(inspector_session_id),
        );
        if bind_frontend_nodes && let Some(snapshot) = snapshot.as_mut() {
            self.assign_renderer_dom_agent_ids_to_snapshot(
                inspector_session_id,
                &mut snapshot.snapshot,
                self.document_dom_agent_includes_whitespace(inspector_session_id),
            );
        }
        Ok(snapshot)
    }

    pub(crate) fn request_document_node_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<(Option<u32>, Option<RendererDocumentChildNodeSnapshotEvents>)> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok((None, None));
        };
        let Some(backend_node_id) = self.renderer_backend_node_id_for_live_handle(handle) else {
            return Ok((None, None));
        };
        let (resolution, events) =
            self.document_node_ids_with_publication(inspector_session_id, &[backend_node_id]);
        let node_id = match resolution {
            RendererDocumentFrontendNodeIdsResolution::Found(ids) => ids[0].unwrap_or(0),
            RendererDocumentFrontendNodeIdsResolution::DocumentNotBound => 0,
        };
        Ok((Some(node_id), events))
    }

    pub(crate) fn document_node_exists_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Result<bool> {
        let Some(key) = self.current_renderer_backend_node_key_for_id(backend_node_id) else {
            return Ok(false);
        };
        // The registry has validated physical node lifetime and document ownership.
        // Inspector identities also need their generated representation to exist.
        if key.inspector_identity.is_none() {
            return Ok(true);
        }
        Ok(self
            .document_node_snapshot_for_backend_node_id(backend_node_id, 0, false)?
            .is_some())
    }

    pub(crate) fn document_node_snapshot_for_backend_node_id(
        &mut self,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(key) = self.current_renderer_backend_node_key_for_id(backend_node_id) else {
            return Ok(None);
        };
        let mut snapshot = match key.inspector_identity {
            None => self.document_node_snapshot_for_live_handle(key.handle, depth, pierce),
            Some(DocumentNodeInspectorIdentity::MarkerPseudoElement) => {
                self.document_marker_pseudo_element_snapshot_for_live_host(None, key.handle)
            }
            Some(identity @ DocumentNodeInspectorIdentity::UserAgentShadowTreeNode { .. }) => self
                .document_user_agent_shadow_node_snapshot_for_live_host(
                    None, key.handle, identity, depth, true,
                ),
        };
        if let Some(snapshot) = snapshot.as_mut() {
            self.assign_renderer_dom_agent_ids_to_snapshot(None, &mut snapshot.snapshot, true);
        }
        Ok(snapshot)
    }

    pub(crate) fn document_node_snapshot_for_node_in_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        depth: i32,
        pierce: bool,
    ) -> Result<Option<DocumentNodeObjectSnapshot>> {
        let Some(backend_node_id) = self.backend_node_id_for_document_node_reference(reference)
        else {
            return Ok(None);
        };
        self.configure_document_dom_agent_session(inspector_session_id, include_whitespace);
        // Preserve the inspector identity on the backend key: snapshots also
        // represent pseudo-elements and user-agent shadow nodes.
        self.document_node_snapshot_for_backend_node_id_in_inspector_session(
            inspector_session_id,
            backend_node_id,
            depth,
            pierce,
        )
    }

    pub(crate) fn document_node_snapshot_for_document(
        &mut self,
        inspector_session_id: Option<&str>,
        depth: i32,
        pierce: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        let handle = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let mut snapshot = self.document_node_snapshot_for_live_handle_for_session(
            inspector_session_id,
            handle,
            depth,
            pierce,
            true,
            self.document_dom_agent_includes_whitespace(inspector_session_id),
        )?;
        self.assign_renderer_dom_agent_ids_to_snapshot(
            inspector_session_id,
            &mut snapshot.snapshot,
            self.document_dom_agent_includes_whitespace(inspector_session_id),
        );
        self.mark_document_snapshot_published(inspector_session_id, &snapshot.snapshot, depth);
        Some(snapshot)
    }

    pub(crate) fn document_node_snapshots_for_dom_snapshot(
        &mut self,
        depth: i32,
        pierce: bool,
    ) -> Vec<DocumentNodeObjectSnapshot> {
        let mut snapshots = Vec::new();
        let document_handle = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        if let Some(root) = self.document_node_snapshot_for_live_handle_for_session(
            None,
            document_handle,
            depth,
            pierce,
            false,
            true,
        ) {
            snapshots.push(root);
        }

        let child_documents = self.vm().live_child_document_handles_in_snapshot_order();
        for (frame_id, owner_node_id, document_handle) in child_documents {
            if let Some(mut snapshot) = self.document_node_snapshot_for_live_handle_for_session(
                None,
                document_handle,
                depth,
                pierce,
                false,
                true,
            ) {
                snapshot.frame_id = Some(frame_id);
                snapshot.owner_node_id = Some(owner_node_id);
                snapshots.push(snapshot);
            }
        }
        snapshots
    }

    pub(crate) fn document_child_node_snapshot_events_for_live_handles(
        &mut self,
        inspector_session_id: Option<&str>,
        handles: &[DomHandle],
        depth: i32,
        pierce: bool,
    ) -> Option<RendererDocumentChildNodeSnapshotEvents> {
        let top_snapshot_node_id = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let mut events = Vec::with_capacity(handles.len());
        for &handle in handles {
            let parent_backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
            let parent_frontend_node_id = self.document_frontend_node_id_for_backend_node_id(
                inspector_session_id,
                parent_backend_node_id,
            );
            let snapshots = self.live_document_child_node_snapshots_for_handle(
                inspector_session_id,
                handle,
                depth,
                pierce,
            )?;
            let child_count = snapshots.len();
            self.mark_document_node_children_requested(
                inspector_session_id,
                parent_backend_node_id,
                child_count,
            );
            self.append_document_child_snapshot_event(
                inspector_session_id,
                RendererDocumentChildNodeSnapshotEvent {
                    parent_frontend_node_id,
                    snapshots,
                },
                depth,
                &mut events,
            );
        }
        Some(RendererDocumentChildNodeSnapshotEvents {
            top_snapshot_node_id,
            events,
        })
    }

    pub(crate) fn document_child_node_snapshot_events_for_node(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        depth: i32,
        pierce: bool,
    ) -> Option<RendererDocumentChildNodeSnapshotEvents> {
        let backend_node_id = self.backend_node_id_for_document_node_reference(reference)?;
        self.configure_document_dom_agent_session(inspector_session_id, include_whitespace);
        let handle = self.live_handle_for_backend_node_id(backend_node_id)?;
        let mut events = Vec::new();
        let mut pending = vec![(handle, depth)];
        while let Some((handle, depth)) = pending.pop() {
            let dom_host = self.vm().document_runtime.dom_host();
            if !matches!(
                dom_host.node(handle)?.kind(),
                NodeData::Element(_) | NodeData::Document(_) | NodeData::DocumentFragment(_)
            ) {
                continue;
            }
            let backend = self.renderer_backend_node_id_for_live_handle(handle)?;
            if self.document_node_children_requested(inspector_session_id, backend) {
                if depth != 0 {
                    let next = if depth > 0 { depth - 1 } else { depth };
                    let dom_host = self.vm().document_runtime.dom_host();
                    let children = dom_host
                        .child_handles(handle)
                        .filter(|child| {
                            include_whitespace || !inspector_whitespace_text_node(dom_host, *child)
                        })
                        .collect::<Vec<_>>();
                    pending.extend(children.into_iter().rev().map(|child| (child, next)));
                }
            } else {
                let mut snapshots = self.document_child_node_snapshot_events_for_live_handles(
                    inspector_session_id,
                    &[handle],
                    depth,
                    pierce,
                )?;
                events.append(&mut snapshots.events);
            }
        }
        Some(RendererDocumentChildNodeSnapshotEvents {
            top_snapshot_node_id: self
                .vm()
                .document_runtime
                .dom_host()
                .dom()
                .document_node_id(),
            events,
        })
    }

    fn inspector_parent_backend_node_id(
        &mut self,
        backend_node_id: u32,
        child_owners: &HashMap<DomHandle, DomHandle>,
    ) -> Option<(u32, bool)> {
        let key = self.current_renderer_backend_node_key_for_id(backend_node_id)?;
        if key.inspector_identity == Some(DocumentNodeInspectorIdentity::MarkerPseudoElement) {
            return self
                .renderer_backend_node_id_for_live_handle(key.handle)
                .map(|id| (id, true));
        }
        if let Some(identity) = key.inspector_identity {
            let host = live_document_node_snapshot(
                self.vm().document_runtime.dom_host(),
                key.handle,
                0,
                None,
                false,
            )?;
            let snapshot = user_agent_shadow_node_snapshot(
                self.vm().document_runtime.dom_host(),
                &host,
                identity,
                0,
                true,
            )?;
            return match snapshot.inspector_parent_identity {
                Some(parent) => self
                    .renderer_backend_node_id_for_inspector_node(key.handle, parent)
                    .map(|id| (id, false)),
                None => self
                    .renderer_backend_node_id_for_live_handle(key.handle)
                    .map(|id| (id, true)),
            };
        }
        let dom_host = self.vm().document_runtime.dom_host();
        let parent = dom_host
            .dom()
            .parent_node(key.handle)
            .or_else(|| dom_host.shadow_root_host(key.handle))
            .or_else(|| child_owners.get(&key.handle).copied())?;
        let associated = dom_host.dom().parent_node(key.handle).is_none();
        self.renderer_backend_node_id_for_live_handle(parent)
            .map(|id| (id, associated))
    }

    fn document_node_path_snapshot_events(
        &mut self,
        inspector_session_id: Option<&str>,
        root: DomHandle,
        backend_node_ids: impl IntoIterator<Item = u32>,
    ) -> Option<RendererDocumentChildNodeSnapshotEvents> {
        let mut backend_node_ids = backend_node_ids.into_iter().peekable();
        backend_node_ids.peek()?;
        let top_snapshot_node_id = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let root_backend = self.renderer_backend_node_id_for_live_handle(root)?;
        let child_owners = self
            .inspector_child_document_handles()
            .into_iter()
            .map(|(owner, document)| (document, owner))
            .collect::<HashMap<_, _>>();
        let mut events = Vec::new();
        // A later stale result must not discard paths already prepared for
        // earlier results in the same batch.
        'nodes: for backend_node_id in backend_node_ids {
            if self.document_node_is_published(inspector_session_id, backend_node_id) {
                continue;
            }
            let mut current = backend_node_id;
            let mut path = Vec::new();
            let mut refresh_owner = false;
            while current != root_backend {
                let Some((parent, associated)) =
                    self.inspector_parent_backend_node_id(current, &child_owners)
                else {
                    // A retained search result can leave the Document between
                    // performSearch and getSearchResults. Publish its detached
                    // Inspector root before expanding the remaining path.
                    if refresh_owner
                        || !self.document_node_is_published(inspector_session_id, current)
                    {
                        let Some(mut snapshot) = self
                            .document_node_snapshot_for_backend_node_id_in_inspector_session(
                                inspector_session_id,
                                current,
                                0,
                                false,
                            )
                            .ok()
                            .flatten()
                            .map(|node| node.snapshot)
                        else {
                            continue 'nodes;
                        };
                        snapshot.parent_id = None;
                        snapshot.parent_frontend_node_id = None;
                        self.assign_renderer_dom_agent_ids_to_snapshot(
                            inspector_session_id,
                            &mut snapshot,
                            self.document_dom_agent_includes_whitespace(inspector_session_id),
                        );
                        self.append_document_child_snapshot_event(
                            inspector_session_id,
                            RendererDocumentChildNodeSnapshotEvent {
                                parent_frontend_node_id: 0,
                                snapshots: vec![snapshot],
                            },
                            0,
                            &mut events,
                        );
                    }
                    break;
                };
                path.push((parent, current, refresh_owner));
                // Associated roots are sent in the owner's snapshot, not in
                // its ordinary children. Re-publish that owner through its
                // parent if a ShadowRoot or child Document appeared later.
                if !associated && self.document_node_is_published(inspector_session_id, parent) {
                    break;
                }
                refresh_owner = associated;
                current = parent;
            }
            for (parent, child, refresh_owner) in path.into_iter().rev() {
                if !refresh_owner && self.document_node_is_published(inspector_session_id, child) {
                    continue;
                }
                let Some(key) = self.current_renderer_backend_node_key_for_id(parent) else {
                    continue 'nodes;
                };
                if key.inspector_identity.is_none() {
                    let Some(mut parent_events) = self
                        .document_child_node_snapshot_events_for_live_handles(
                            inspector_session_id,
                            &[key.handle],
                            0,
                            false,
                        )
                    else {
                        continue 'nodes;
                    };
                    events.append(&mut parent_events.events);
                } else {
                    let Some(mut snapshot) = self
                        .document_node_snapshot_for_backend_node_id_in_inspector_session(
                            inspector_session_id,
                            parent,
                            1,
                            true,
                        )
                        .ok()
                        .flatten()
                        .map(|node| node.snapshot)
                    else {
                        continue 'nodes;
                    };
                    self.assign_renderer_dom_agent_ids_to_snapshot(
                        inspector_session_id,
                        &mut snapshot,
                        self.document_dom_agent_includes_whitespace(inspector_session_id),
                    );
                    let Some(parent_frontend_node_id) = snapshot.frontend_node_id else {
                        continue 'nodes;
                    };
                    self.mark_document_node_children_requested(
                        inspector_session_id,
                        parent,
                        snapshot.child_count,
                    );
                    self.append_document_child_snapshot_event(
                        inspector_session_id,
                        RendererDocumentChildNodeSnapshotEvent {
                            parent_frontend_node_id,
                            snapshots: snapshot.children,
                        },
                        0,
                        &mut events,
                    );
                }
            }
        }
        Some(RendererDocumentChildNodeSnapshotEvents {
            top_snapshot_node_id,
            events,
        })
    }

    pub(crate) fn document_query_selector_for_node(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorResolution {
        let Some(root_backend_node_id) =
            self.backend_node_id_for_document_node_reference(reference)
        else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        self.configure_document_dom_agent_session(inspector_session_id, include_whitespace);
        let Some(root) = self.live_handle_for_backend_node_id(root_backend_node_id) else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    pub(crate) fn document_query_selector_for_child_frame_node(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
        frame_id: &str,
        reference: RendererDomNodeReference,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorResolution {
        let Some(root_backend_node_id) =
            self.backend_node_id_for_document_node_reference(reference)
        else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        self.configure_document_dom_agent_session(inspector_session_id, include_whitespace);
        let Some(document_handle) = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)
        else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        let Some(root) = self.live_handle_for_backend_node_id(root_backend_node_id) else {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        };
        if !child_frame_document_contains_live_handle(
            self.vm().document_runtime.dom_host(),
            document_handle,
            root,
        ) {
            return RendererDocumentQuerySelectorResolution::MissingRoot;
        }

        self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    pub(crate) fn document_query_selector_with_child_node_snapshot_events_for_live_root_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        root: DomHandle,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
        let query_selector_resolution = self.document_query_selector_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        );
        let backend_node_ids = match &query_selector_resolution {
            RendererDocumentQuerySelectorResolution::Found(nodes) => nodes
                .iter()
                .map(|node| node.backend_node_id)
                .collect::<Vec<_>>(),
            RendererDocumentQuerySelectorResolution::MissingRoot
            | RendererDocumentQuerySelectorResolution::InvalidSelector(_) => Vec::new(),
        };
        let child_node_snapshot_events =
            self.document_node_path_snapshot_events(inspector_session_id, root, backend_node_ids);
        RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
            child_node_snapshot_events,
            query_selector_resolution,
        }
    }

    pub(crate) fn document_search_results_with_child_node_snapshot_events(
        &mut self,
        inspector_session_id: Option<&str>,
        search_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> (
        crate::RendererDomSearchResultsResolution,
        Option<RendererDocumentChildNodeSnapshotEvents>,
    ) {
        let mut resolution =
            self.document_search_results(inspector_session_id, search_id, from_index, to_index);
        let crate::RendererDomSearchResultsResolution::Found(nodes) = &mut resolution else {
            return (resolution, None);
        };
        let backend_ids = nodes
            .iter()
            .map(|node| node.backend_node_id)
            .collect::<Vec<_>>();
        let (publication, events) =
            self.document_node_ids_with_publication(inspector_session_id, &backend_ids);
        if let RendererDocumentFrontendNodeIdsResolution::Found(ids) = publication {
            for (node, id) in nodes.iter_mut().zip(ids) {
                node.frontend_node_id = id.unwrap_or(0);
            }
        }
        (resolution, events)
    }

    pub(crate) fn document_query_selector_with_child_node_snapshot_events_for_node(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        selector: &str,
        multiple: bool,
    ) -> RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
        let Some(root_backend_node_id) =
            self.backend_node_id_for_document_node_reference(reference)
        else {
            return RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
                child_node_snapshot_events: None,
                query_selector_resolution: RendererDocumentQuerySelectorResolution::MissingRoot,
            };
        };
        self.configure_document_dom_agent_session(inspector_session_id, include_whitespace);
        let Some(root) = self.live_handle_for_backend_node_id(root_backend_node_id) else {
            return RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
                child_node_snapshot_events: None,
                query_selector_resolution: RendererDocumentQuerySelectorResolution::MissingRoot,
            };
        };
        self.document_query_selector_with_child_node_snapshot_events_for_live_root_handle(
            inspector_session_id,
            root,
            selector,
            multiple,
        )
    }

    fn document_node_snapshot_for_live_handle(
        &mut self,
        handle: DomHandle,
        depth: i32,
        pierce: bool,
    ) -> Option<DocumentNodeObjectSnapshot> {
        self.document_node_snapshot_for_live_handle_for_session(
            None, handle, depth, pierce, true, true,
        )
    }

    pub(in crate::runtime::page_dom) fn project_renderer_dom_agent_ids_to_snapshot(
        &mut self,
        inspector_session_id: Option<&str>,
        snapshot: &mut DocumentNodeSnapshot,
        include_whitespace: bool,
        bind: bool,
    ) {
        let mut stack = vec![snapshot];
        while let Some(snapshot) = stack.pop() {
            let backend_node_id = if let Some(inspector_identity) = snapshot.inspector_identity {
                self.renderer_backend_node_id_for_inspector_node(
                    snapshot.node_id,
                    inspector_identity,
                )
            } else {
                self.renderer_backend_node_id_for_live_handle(snapshot.node_id)
            };
            if let Some(backend_node_id) = backend_node_id {
                let frontend_node_id = if bind {
                    self.document_frontend_node_id_for_backend_node_id_in_whitespace_projection(
                        inspector_session_id,
                        backend_node_id,
                        include_whitespace,
                        inspector_whitespace_text_snapshot(snapshot),
                    )
                } else {
                    // Internal snapshots may allocate handles without sending
                    // nodes to the client. describeNode only exposes published IDs.
                    self.existing_document_frontend_node_id(inspector_session_id, backend_node_id)
                        .filter(|_| {
                            self.document_node_is_published(inspector_session_id, backend_node_id)
                        })
                        .unwrap_or(0)
                };
                snapshot.backend_node_id = Some(backend_node_id);
                snapshot.frontend_node_id = Some(frontend_node_id);
            }
            snapshot.frame_id = self
                .vm()
                .child_browsing_context_frame_id_by_owner_node_id(snapshot.node_id);
            snapshot.parent_frontend_node_id = if let Some(parent_identity) =
                snapshot.inspector_parent_identity
            {
                self.renderer_backend_node_id_for_inspector_node(snapshot.node_id, parent_identity)
                    .map(|parent_backend_node_id| {
                        self.existing_document_frontend_node_id(
                            inspector_session_id,
                            parent_backend_node_id,
                        )
                        .filter(|_| {
                            bind || self.document_node_is_published(
                                inspector_session_id,
                                parent_backend_node_id,
                            )
                        })
                        .unwrap_or(0)
                    })
            } else {
                snapshot.parent_id.and_then(|parent_id| {
                    self.renderer_backend_node_id_for_live_handle(parent_id)
                        .map(|parent_backend_node_id| {
                            self.existing_document_frontend_node_id(
                                inspector_session_id,
                                parent_backend_node_id,
                            )
                            .filter(|_| {
                                bind || self.document_node_is_published(
                                    inspector_session_id,
                                    parent_backend_node_id,
                                )
                            })
                            .unwrap_or(0)
                        })
                })
            };
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

    pub(crate) fn backend_node_id_for_document_node_reference(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Option<u32> {
        let backend_node_id = match reference {
            RendererDomNodeReference::BackendNodeId(id) => id,
            RendererDomNodeReference::InternalFrontendNodeBinding {
                inspector_session_id,
                frontend_node_id,
            } => match self
                .document_frontend_node_binding(inspector_session_id.as_deref(), frontend_node_id)
            {
                RendererDomFrontendNodeBindingResolution::BackendNodeId(id) => id,
                RendererDomFrontendNodeBindingResolution::NotFound => return None,
            },
            RendererDomNodeReference::FrontendNodeId {
                inspector_session_id,
                frontend_node_id,
            } => match self.published_document_frontend_node_binding(
                inspector_session_id.as_deref(),
                frontend_node_id,
            ) {
                RendererDomFrontendNodeBindingResolution::BackendNodeId(id) => id,
                RendererDomFrontendNodeBindingResolution::NotFound => {
                    return None;
                }
            },
        };
        Some(backend_node_id)
    }

    fn document_node_ids_with_publication(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_ids: &[u32],
    ) -> (
        RendererDocumentFrontendNodeIdsResolution,
        Option<RendererDocumentChildNodeSnapshotEvents>,
    ) {
        // Freeze earlier DOM facts before new nodes become visible to this
        // session; their notifications must not reference a future publication.
        self.flush_pending_dom_mutation_event_batches();
        let root = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        if !self
            .renderer_backend_node_id_for_live_handle(root)
            .is_some_and(|id| self.document_node_is_published(inspector_session_id, id))
        {
            return (
                RendererDocumentFrontendNodeIdsResolution::DocumentNotBound,
                None,
            );
        }
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let mut ids = vec![None; backend_node_ids.len()];
        let mut pending = Vec::new();
        for (index, &backend_node_id) in backend_node_ids.iter().enumerate() {
            let Some(key) = self.current_renderer_backend_node_key_for_id(backend_node_id) else {
                continue;
            };
            let visible = match key.inspector_identity {
                None => {
                    include_whitespace
                        || !inspector_whitespace_text_node(
                            self.vm().document_runtime.dom_host(),
                            key.handle,
                        )
                }
                Some(DocumentNodeInspectorIdentity::MarkerPseudoElement) => self
                    .vm()
                    .marker_pseudo_element_is_generated_for_document_snapshot(key.handle),
                Some(identity @ DocumentNodeInspectorIdentity::UserAgentShadowTreeNode { .. }) => {
                    live_document_node_snapshot(
                        self.vm().document_runtime.dom_host(),
                        key.handle,
                        0,
                        None,
                        false,
                    )
                    .and_then(|host| {
                        user_agent_shadow_node_snapshot(
                            self.vm().document_runtime.dom_host(),
                            &host,
                            identity,
                            0,
                            true,
                        )
                    })
                    .is_some_and(|snapshot| {
                        include_whitespace || !inspector_whitespace_text_snapshot(&snapshot)
                    })
                }
            };
            if !visible {
                continue;
            }
            if self.document_node_is_published(inspector_session_id, backend_node_id) {
                ids[index] =
                    self.existing_document_frontend_node_id(inspector_session_id, backend_node_id);
            } else {
                pending.push((index, backend_node_id));
            }
        }
        let events = self.document_node_path_snapshot_events(
            inspector_session_id,
            root,
            pending.iter().map(|(_, id)| *id),
        );
        for (index, backend_node_id) in pending {
            if self.document_node_is_published(inspector_session_id, backend_node_id) {
                ids[index] =
                    self.existing_document_frontend_node_id(inspector_session_id, backend_node_id);
            }
        }
        (
            RendererDocumentFrontendNodeIdsResolution::Found(ids),
            events,
        )
    }

    pub(crate) fn document_frontend_node_ids_for_backend_node_ids(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_ids: &[u32],
    ) -> RendererDocumentFrontendNodeIdsResolution {
        let (resolution, events) =
            self.document_node_ids_with_publication(inspector_session_id, backend_node_ids);
        if let Some(events) = events {
            self.queue_document_child_snapshot_events(inspector_session_id, events);
        }
        resolution
    }

    pub(in crate::runtime::page_dom) fn published_document_node_reference_for_live_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
    ) -> Option<RendererDocumentNodeReference> {
        let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
        let node_id = match self.document_frontend_node_ids_for_backend_node_ids(
            inspector_session_id,
            &[backend_node_id],
        ) {
            RendererDocumentFrontendNodeIdsResolution::Found(ids) => ids[0].unwrap_or(0),
            RendererDocumentFrontendNodeIdsResolution::DocumentNotBound => 0,
        };
        Some(RendererDocumentNodeReference {
            node_id,
            backend_node_id,
        })
    }

    pub(crate) fn child_frame_owner_node_reference_by_frame_id(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: &str,
    ) -> Option<RendererDocumentNodeReference> {
        let handle = self
            .vm()
            .child_browsing_context_owner_node_id_by_frame_id(frame_id)?;
        self.published_document_node_reference_for_live_handle(inspector_session_id, handle)
    }

    pub(crate) fn child_frame_document_root_node_reference_by_frame_id(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: &str,
    ) -> Option<RendererDocumentNodeReference> {
        let handle = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)?;
        self.document_node_reference_for_live_handle(inspector_session_id, handle)
    }

    fn document_node_reference_for_live_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: DomHandle,
    ) -> Option<RendererDocumentNodeReference> {
        let backend_node_id = self.renderer_backend_node_id_for_live_handle(handle)?;
        let node_id = self
            .document_frontend_node_id_for_backend_node_id(inspector_session_id, backend_node_id);
        Some(RendererDocumentNodeReference {
            node_id,
            backend_node_id,
        })
    }
}
