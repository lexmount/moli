use std::collections::HashMap;

use super::*;

#[derive(Clone, Copy)]
enum ForeignCloneSource<'a> {
    Dom(&'a NativeDom),
    Host(&'a DomHost),
}

#[derive(Clone, Copy)]
enum ForeignShadowRootMode {
    None,
    All,
    Clonable,
}

impl<'a> ForeignCloneSource<'a> {
    fn dom(self) -> &'a NativeDom {
        match self {
            Self::Dom(dom) => dom,
            Self::Host(host) => host.dom(),
        }
    }

    fn shadow_root_binding(
        self,
        handle: DomHandle,
        mode: ForeignShadowRootMode,
    ) -> Option<(DomHandle, ShadowRootInit, bool, bool)> {
        if matches!(mode, ForeignShadowRootMode::None) {
            return None;
        }
        let Self::Host(host) = self else {
            return None;
        };
        let root = host.shadow_root_handle(handle)?;
        let (init, declarative, available_to_element_internals) = host
            .shadow_roots_by_host
            .borrow()
            .get(&handle)
            .map(|state| {
                (
                    state.init.clone(),
                    state.declarative,
                    state.available_to_element_internals,
                )
            })?;
        if matches!(mode, ForeignShadowRootMode::Clonable) && !init.clonable() {
            return None;
        }
        Some((root, init, declarative, available_to_element_internals))
    }
}

impl DomHost {
    pub fn clone_node(&mut self, handle: DomHandle, deep: bool) -> Option<DomHandle> {
        let document_handle = self.owner_document_handle(handle)?;
        self.clone_node_for_document(document_handle, handle, deep)
    }

    pub fn import_node(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
        deep: bool,
    ) -> Option<DomHandle> {
        self.node(document_handle)
            .is_some_and(Node::is_document)
            .then_some(())?;
        self.clone_node_for_document(document_handle, handle, deep)
    }

    pub fn import_foreign_node(
        &mut self,
        document_handle: DomHandle,
        foreign_dom: &NativeDom,
        handle: DomHandle,
        deep: bool,
    ) -> Option<DomHandle> {
        self.node(document_handle)
            .is_some_and(Node::is_document)
            .then_some(())?;
        self.clone_foreign_node_for_document(
            document_handle,
            ForeignCloneSource::Dom(foreign_dom),
            handle,
            deep,
            ForeignShadowRootMode::None,
            &mut HashMap::new(),
        )
    }

    pub fn import_foreign_node_with_shadow_roots(
        &mut self,
        document_handle: DomHandle,
        foreign_host: &DomHost,
        handle: DomHandle,
        deep: bool,
    ) -> Option<DomHandle> {
        self.node(document_handle)
            .is_some_and(Node::is_document)
            .then_some(())?;
        self.clone_foreign_node_for_document(
            document_handle,
            ForeignCloneSource::Host(foreign_host),
            handle,
            deep,
            ForeignShadowRootMode::All,
            &mut HashMap::new(),
        )
    }

    pub fn import_foreign_node_with_shadow_roots_and_handle_map(
        &mut self,
        document_handle: DomHandle,
        foreign_host: &DomHost,
        handle: DomHandle,
        deep: bool,
        cloned_handles: &mut HashMap<DomHandle, DomHandle>,
    ) -> Option<DomHandle> {
        self.node(document_handle)
            .is_some_and(Node::is_document)
            .then_some(())?;
        self.clone_foreign_node_for_document(
            document_handle,
            ForeignCloneSource::Host(foreign_host),
            handle,
            deep,
            ForeignShadowRootMode::All,
            cloned_handles,
        )
    }

    pub fn import_foreign_node_with_clonable_shadow_roots(
        &mut self,
        document_handle: DomHandle,
        foreign_host: &DomHost,
        handle: DomHandle,
        deep: bool,
    ) -> Option<DomHandle> {
        self.node(document_handle)
            .is_some_and(Node::is_document)
            .then_some(())?;
        self.clone_foreign_node_for_document(
            document_handle,
            ForeignCloneSource::Host(foreign_host),
            handle,
            deep,
            ForeignShadowRootMode::Clonable,
            &mut HashMap::new(),
        )
    }

    pub fn adopt_node(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        self.adopt_node_with_stylesheet_owner_changes(document_handle, handle)
            .map(|(handle, _)| handle)
    }

    pub fn adopt_node_with_stylesheet_owner_changes(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<(DomHandle, Vec<DomStylesheetOwnerChange>)> {
        self.node(document_handle)
            .is_some_and(Node::is_document)
            .then_some(())?;
        self.node(handle).filter(|node| !node.is_document())?;
        let owners = self
            .dom
            .mark_subtree_tree_scope_collecting_stylesheet_owners(
                handle,
                Some(document_handle),
                false,
                false,
            );
        let owner_changes = owners
            .into_iter()
            .map(|owner| {
                DomStylesheetOwnerChange::owner_document_changed(
                    owner,
                    self.dom.stylesheet_candidate_tree_scope_for_node(owner),
                )
            })
            .collect();
        self.record_mutation(MutationScope::QueryState);
        Some((handle, owner_changes))
    }

    fn clone_node_for_document(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
        deep: bool,
    ) -> Option<DomHandle> {
        self.clone_node_for_document_inner(document_handle, handle, deep, false)
    }

    fn clone_node_for_document_inner(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
        deep: bool,
        allow_shadow_root: bool,
    ) -> Option<DomHandle> {
        enum LocalCloneAttach {
            Root,
            AppendTo(DomHandle),
            ShadowRoot {
                host: DomHandle,
                init: ShadowRootInit,
                declarative: bool,
                available_to_element_internals: bool,
            },
        }

        struct LocalCloneFrame {
            source: DomHandle,
            document_handle: DomHandle,
            clone_children: bool,
            allow_shadow_root: bool,
            attach: LocalCloneAttach,
        }

        let mut root_clone = None;
        let mut stack = vec![LocalCloneFrame {
            source: handle,
            document_handle,
            clone_children: deep,
            allow_shadow_root,
            attach: LocalCloneAttach::Root,
        }];
        while let Some(frame) = stack.pop() {
            if self.is_shadow_root(frame.source) && !frame.allow_shadow_root {
                return None;
            }
            let node = self.node(frame.source)?.clone();
            let clone = self.clone_node_shell_for_document(frame.document_handle, &node, true)?;
            match frame.attach {
                LocalCloneAttach::Root => root_clone = Some(clone),
                LocalCloneAttach::AppendTo(parent) => {
                    let _ = self.dom.append_child(parent, clone);
                }
                LocalCloneAttach::ShadowRoot {
                    host,
                    init,
                    declarative,
                    available_to_element_internals,
                } => {
                    self.dom.register_stylesheet_candidate_tree_scope(clone);
                    self.shadow_roots_by_host.borrow_mut().insert(
                        host,
                        ShadowRootState {
                            handle: clone,
                            init,
                            declarative,
                            available_to_element_internals,
                        },
                    );
                    self.shadow_hosts_by_root.borrow_mut().insert(clone, host);
                }
            }
            let clone_document_handle = if node.is_document() {
                clone
            } else {
                frame.document_handle
            };

            if let Some((source_shadow_root, init, declarative, available_to_element_internals)) =
                node.as_element()
                    .and_then(|_| self.shadow_root_handle(frame.source))
                    .and_then(|source_shadow_root| {
                        self.shadow_roots_by_host
                            .borrow()
                            .get(&frame.source)
                            .map(|state| {
                                (
                                    source_shadow_root,
                                    state.init.clone(),
                                    state.declarative,
                                    state.available_to_element_internals,
                                )
                            })
                            .filter(|(_, init, _, _)| init.clonable())
                    })
            {
                stack.push(LocalCloneFrame {
                    source: source_shadow_root,
                    document_handle: clone_document_handle,
                    clone_children: true,
                    allow_shadow_root: true,
                    attach: LocalCloneAttach::ShadowRoot {
                        host: clone,
                        init,
                        declarative,
                        available_to_element_internals,
                    },
                });
            }

            if !frame.clone_children {
                continue;
            }
            for child in self.dom.child_ids_reversed(frame.source) {
                stack.push(LocalCloneFrame {
                    source: child,
                    document_handle: clone_document_handle,
                    clone_children: true,
                    allow_shadow_root: false,
                    attach: LocalCloneAttach::AppendTo(clone),
                });
            }
            if let Some(template_contents) = node
                .as_element()
                .and_then(|element| element.template_contents())
            {
                let cloned_contents = self
                    .node(clone)
                    .and_then(Node::as_element)
                    .and_then(Element::template_contents)?;
                let contents_document = self
                    .node(cloned_contents)
                    .and_then(Node::owner_document)
                    .unwrap_or(frame.document_handle);
                for child in self.dom.child_ids_reversed(template_contents) {
                    stack.push(LocalCloneFrame {
                        source: child,
                        document_handle: contents_document,
                        clone_children: true,
                        allow_shadow_root: false,
                        attach: LocalCloneAttach::AppendTo(cloned_contents),
                    });
                }
            }
        }

        root_clone
    }

    fn clone_foreign_node_for_document(
        &mut self,
        document_handle: DomHandle,
        source: ForeignCloneSource<'_>,
        handle: DomHandle,
        deep: bool,
        shadow_root_mode: ForeignShadowRootMode,
        cloned_handles: &mut HashMap<DomHandle, DomHandle>,
    ) -> Option<DomHandle> {
        enum ForeignCloneAttach {
            Root,
            AppendTo(DomHandle),
            ShadowRoot {
                host: DomHandle,
                init: ShadowRootInit,
                declarative: bool,
                available_to_element_internals: bool,
            },
        }

        struct ForeignCloneFrame {
            source: DomHandle,
            document_handle: DomHandle,
            clone_children: bool,
            attach: ForeignCloneAttach,
        }

        let foreign_dom = source.dom();
        let mut root_clone = None;
        let mut pending_form_owner_links = Vec::new();
        let mut stack = vec![ForeignCloneFrame {
            source: handle,
            document_handle,
            clone_children: deep,
            attach: ForeignCloneAttach::Root,
        }];
        while let Some(frame) = stack.pop() {
            let node = foreign_dom.node(frame.source)?.clone();
            let clone = self.clone_node_shell_for_document(frame.document_handle, &node, false)?;
            cloned_handles.insert(frame.source, clone);
            if let Some(source_owner) = node
                .as_element()
                .and_then(Element::parser_associated_form_owner)
            {
                pending_form_owner_links.push((clone, source_owner));
            }
            match frame.attach {
                ForeignCloneAttach::Root => root_clone = Some(clone),
                ForeignCloneAttach::AppendTo(parent) => {
                    let _ = self.dom.append_child(parent, clone);
                }
                ForeignCloneAttach::ShadowRoot {
                    host,
                    init,
                    declarative,
                    available_to_element_internals,
                } => {
                    self.dom.register_stylesheet_candidate_tree_scope(clone);
                    self.shadow_roots_by_host.borrow_mut().insert(
                        host,
                        ShadowRootState {
                            handle: clone,
                            init,
                            declarative,
                            available_to_element_internals,
                        },
                    );
                    self.shadow_hosts_by_root.borrow_mut().insert(clone, host);
                }
            }

            if let Some((source_shadow_root, init, declarative, available_to_element_internals)) =
                source.shadow_root_binding(frame.source, shadow_root_mode)
            {
                stack.push(ForeignCloneFrame {
                    source: source_shadow_root,
                    document_handle: frame.document_handle,
                    clone_children: true,
                    attach: ForeignCloneAttach::ShadowRoot {
                        host: clone,
                        init,
                        declarative,
                        available_to_element_internals,
                    },
                });
            }

            if !frame.clone_children {
                continue;
            }
            for child in foreign_dom.child_ids_reversed(frame.source) {
                stack.push(ForeignCloneFrame {
                    source: child,
                    document_handle: frame.document_handle,
                    clone_children: true,
                    attach: ForeignCloneAttach::AppendTo(clone),
                });
            }
            if let Some(template_contents) = node
                .as_element()
                .and_then(|element| element.template_contents())
            {
                let cloned_contents = self
                    .node(clone)
                    .and_then(Node::as_element)
                    .and_then(Element::template_contents)?;
                let contents_document = self
                    .node(cloned_contents)
                    .and_then(Node::owner_document)
                    .unwrap_or(frame.document_handle);
                for child in foreign_dom.child_ids_reversed(template_contents) {
                    stack.push(ForeignCloneFrame {
                        source: child,
                        document_handle: contents_document,
                        clone_children: true,
                        attach: ForeignCloneAttach::AppendTo(cloned_contents),
                    });
                }
            }
        }

        for (clone, source_owner) in pending_form_owner_links {
            let Some(cloned_owner) = cloned_handles.get(&source_owner).copied() else {
                continue;
            };
            let Some(clone_element) = self
                .node_mut(clone)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                continue;
            };
            let _ = clone_element.set_parser_associated_form_owner(Some(cloned_owner));
        }

        root_clone
    }

    fn clone_node_shell_for_document(
        &mut self,
        document_handle: DomHandle,
        node: &Node,
        allow_document: bool,
    ) -> Option<DomHandle> {
        let clone = match node.data() {
            super::NodeData::Document(document) => {
                if !allow_document {
                    return None;
                }
                self.dom.create_node(
                    super::NodeData::Document(document.clone()),
                    None,
                    false,
                    false,
                )
            }
            super::NodeData::DocumentType(doctype) => {
                self.create_document_type(doctype.name(), doctype.public_id(), doctype.system_id())
            }
            super::NodeData::Text(text) => self.create_text_node(text.data()),
            super::NodeData::CDataSection(cdata) => self.create_cdata_section(cdata.data()),
            super::NodeData::Comment(comment) => self.create_comment(comment.data()),
            super::NodeData::ProcessingInstruction(pi) => {
                self.create_processing_instruction(pi.target(), pi.data())
            }
            super::NodeData::DocumentFragment(_) => self.create_document_fragment(),
            super::NodeData::Element(element) => {
                let clone = if element.namespace() == "http://www.w3.org/1999/xhtml"
                    && element.prefix().is_none()
                {
                    self.create_element(element.local_name())
                } else {
                    let qualified_name = match element.prefix() {
                        Some(prefix) if !prefix.is_empty() => {
                            format!("{prefix}:{}", element.local_name())
                        }
                        _ => element.local_name().to_owned(),
                    };
                    self.create_element_ns(Some(element.namespace()), &qualified_name)?
                };
                self.copy_element_state(node, clone);
                clone
            }
        };

        let owner_document = (!node.is_document()).then_some(document_handle);
        self.dom
            .mark_subtree_tree_scope(clone, owner_document, false, false);
        Some(clone)
    }

    fn copy_element_state(&mut self, source: &Node, clone: DomHandle) {
        let Some(element) = source.as_element() else {
            return;
        };
        let Some(clone_element) = self
            .node_mut(clone)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return;
        };
        for attribute in element.attributes() {
            let _ = clone_element.set_attribute(
                attribute.local_name().to_owned(),
                attribute.namespace().to_owned(),
                attribute.prefix().map(str::to_owned),
                attribute.value().to_owned(),
            );
        }
        let _ =
            clone_element.set_cryptographic_nonce(element.cryptographic_nonce().map(str::to_owned));
        let _ = clone_element
            .set_custom_element_is_name(element.custom_element_is_name().map(str::to_owned));
        let _ = clone_element.mark_undefined_custom_element_candidate_from_identity();
        let _ = clone_element
            .set_input_value_with_dirty(&element.input_value(), element.input_value_dirty());
        let _ = clone_element.set_checked_with_dirty(element.checked(), element.checked_dirty());
        let _ = clone_element.set_selected(element.selected());
        let _ = clone_element.set_indeterminate(element.indeterminate());
        let _ = clone_element.set_script_force_async(element.script_async());
        let _ = clone_element.set_script_already_started(element.script_already_started());
        // Text selection is live user-interaction state; cloned text controls
        // start from their default collapsed 0/0/none selection.
        let _ = clone_element.set_media_paused(!element.media_paused());
        let _ = clone_element.set_media_paused(element.media_paused());
        let _ = clone_element.set_media_volume(-1.0);
        let _ = clone_element.set_media_volume(element.media_volume());
        let _ = clone_element.set_media_muted(!element.media_muted());
        let _ = clone_element.set_media_muted(element.media_muted());
        let _ = clone_element.set_media_seeking(!element.media_seeking());
        let _ = clone_element.set_media_seeking(element.media_seeking());
        let _ = clone_element.set_media_playback_rate(-1.0);
        let _ = clone_element.set_media_playback_rate(element.media_playback_rate());
        let _ = clone_element.set_media_current_time(-1.0);
        let _ = clone_element.set_media_current_time(element.media_current_time());
        let _ = clone_element.set_media_ready_state(u32::MAX);
        let _ = clone_element.set_media_ready_state(element.media_ready_state());
        let _ = clone_element.set_media_network_state(u32::MAX);
        let _ = clone_element.set_media_network_state(element.media_network_state());
        let _ = clone_element.set_media_error_code(element.media_error_code());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEEP_TREE_DEPTH: usize = 4096;
    const DEEP_SHADOW_DEPTH: usize = 1024;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://clone.test/").expect("test URL"),
        ))
    }

    fn append_deep_element_chain(host: &mut DomHost, depth: usize) -> (DomHandle, DomHandle) {
        let root = host.create_element("div");
        let mut parent = root;
        for _ in 0..depth {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }
        let leaf = host.create_text_node("leaf");
        assert!(host.append_child(parent, leaf));
        (root, leaf)
    }

    fn element_chain_depth(host: &DomHost, root: DomHandle) -> usize {
        let mut depth = 0;
        let mut current = root;
        while let Some(child) = host.node(current).and_then(Node::first_child) {
            if !host.node(child).is_some_and(Node::is_element) {
                break;
            }
            depth += 1;
            current = child;
        }
        depth
    }

    #[test]
    fn script_clone_drops_trusted_source_state() {
        let mut host = test_host();
        let script = host.create_element("script");
        assert!(host.set_script_text_internal_slot(script, "trusted-source"));
        assert!(host.note_script_children_changed_by_api(script));
        assert!(host.set_script_already_started(script, true));

        let clone = host.clone_node(script, true).expect("script clone");
        let clone = host
            .node(clone)
            .and_then(Node::as_element)
            .expect("cloned script element");

        assert_eq!(clone.script_text_internal_slot(), "");
        assert!(!clone.script_children_changed_by_api());
        assert!(clone.script_already_started());
    }

    #[test]
    fn deep_clone_and_is_equal_node_walk_iteratively() {
        let mut host = test_host();
        let (root, _) = append_deep_element_chain(&mut host, DEEP_TREE_DEPTH);

        let clone = host.clone_node(root, true).expect("deep clone");
        assert_eq!(element_chain_depth(&host, clone), DEEP_TREE_DEPTH);
        assert!(
            host.node(root)
                .expect("source root")
                .is_equal_node(host.dom(), host.node(clone).expect("cloned root"))
        );
    }

    #[test]
    fn deep_foreign_import_walks_iteratively() {
        let mut source = test_host();
        let (source_root, _) = append_deep_element_chain(&mut source, DEEP_TREE_DEPTH);
        let mut target = test_host();
        let target_document = target.document_handle();

        let imported = target
            .import_foreign_node(target_document, source.dom(), source_root, true)
            .expect("deep import");

        assert_eq!(element_chain_depth(&target, imported), DEEP_TREE_DEPTH);
        assert_eq!(
            target.node(imported).and_then(Node::owner_document),
            Some(target_document)
        );
    }

    #[test]
    fn deep_adopt_remove_and_insert_retarget_walk_iteratively() {
        let mut host = test_host();
        let source_document = host.document_handle();
        let target_document = host.create_detached_xml_document();
        let (root, leaf) = append_deep_element_chain(&mut host, DEEP_TREE_DEPTH);

        assert!(host.append_child(source_document, root));
        assert_eq!(
            host.node(leaf).and_then(Node::owner_document),
            Some(source_document)
        );
        assert!(host.remove_child(source_document, root));
        assert_eq!(host.adopt_node(target_document, root), Some(root));
        assert_eq!(
            host.node(leaf).and_then(Node::owner_document),
            Some(target_document)
        );
        assert!(host.append_child(target_document, root));
        assert_eq!(
            host.node(leaf).and_then(Node::owner_document),
            Some(target_document)
        );
        assert_eq!(element_chain_depth(&host, root), DEEP_TREE_DEPTH);
    }

    #[test]
    fn deep_normalize_walks_iteratively() {
        let mut host = test_host();
        let (root, leaf) = append_deep_element_chain(&mut host, DEEP_TREE_DEPTH);
        let deepest_parent = host
            .node(leaf)
            .and_then(Node::parent_node)
            .expect("leaf parent");
        let empty = host.create_text_node("");
        let tail = host.create_text_node("tail");
        assert!(host.append_child(deepest_parent, empty));
        assert!(host.append_child(deepest_parent, tail));

        let _ = host.normalize_effects(root);

        assert_eq!(host.node(leaf).and_then(Node::node_value), Some("leaftail"));
        assert_eq!(host.node(leaf).and_then(Node::next_sibling), None);
        assert_eq!(element_chain_depth(&host, root), DEEP_TREE_DEPTH);
    }

    #[test]
    fn deep_script_state_walks_iteratively() {
        let mut host = test_host();
        let (root, leaf) = append_deep_element_chain(&mut host, DEEP_TREE_DEPTH);
        let deepest_parent = host
            .node(leaf)
            .and_then(Node::parent_node)
            .expect("leaf parent");
        let script = host.create_element("script");
        assert!(host.append_child(deepest_parent, script));

        host.set_subtree_script_already_started(root, true);

        assert!(
            host.node(script)
                .and_then(Node::as_element)
                .is_some_and(Element::script_already_started)
        );
    }

    #[test]
    fn deep_shadow_tree_scope_retarget_walks_iteratively() {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("section");
        let mut shadow_host = root;
        for _ in 0..DEEP_SHADOW_DEPTH {
            let shadow_root = host
                .attach_shadow_root(shadow_host, "open")
                .expect("shadow root");
            let child_host = host.create_element("div");
            assert!(host.append_child(shadow_root, child_host));
            shadow_host = child_host;
        }

        assert!(host.append_child(document, root));
        assert_eq!(
            host.node(shadow_host).and_then(Node::owner_document),
            Some(document)
        );
        assert!(host.is_connected(shadow_host));
    }
}
