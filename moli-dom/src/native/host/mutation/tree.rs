use super::*;

impl DomHost {
    pub fn append_child(&mut self, parent: DomHandle, child: DomHandle) -> bool {
        self.append_child_effects(parent, child).did_change()
    }

    /// Raw tree splice for internal clone/import construction before the
    /// constructed subtree is exposed through a Web mutation surface.
    pub fn append_child_without_mutation_effects(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        self.insert_before_without_mutation_effects(parent, child, None)
    }

    /// Raw tree splice for parser/clone staging trees that are not yet exposed
    /// through a Web mutation surface.
    pub fn insert_before_without_mutation_effects(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        let previous_shadow_root = self.containing_shadow_root(child);
        let inserted = self.dom.insert_before(parent, child, reference_child);
        if inserted {
            if let Some(shadow_root) = previous_shadow_root {
                self.invalidate_shadow_slot_name_index(shadow_root);
            }
            self.invalidate_shadow_slot_name_index_for_tree_parent(parent);
            // Internal clone/import construction is intentionally silent to
            // mutation observers, but any query index that was already
            // materialized must still remain a complete view of the host.
            // The subtree is not web-observable until its owner exposes it, so
            // this does not advance query_version or emit mutation effects.
            self.record_query_index_candidates_in_subtree(child);
        }
        inserted
    }

    /// Raw removal counterpart to [`Self::insert_before_without_mutation_effects`].
    pub fn remove_child_without_mutation_effects(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> bool {
        let previous_shadow_root = self.containing_shadow_root(child);
        let removed = self.dom.remove_child(parent, child);
        if removed {
            if let Some(shadow_root) = previous_shadow_root {
                self.invalidate_shadow_slot_name_index(shadow_root);
            }
            self.invalidate_shadow_slot_name_index_for_tree_parent(parent);
        }
        removed
    }

    pub fn remove_child(&mut self, parent: DomHandle, child: DomHandle) -> bool {
        self.remove_child_effects(parent, child).did_change()
    }

    pub fn append_child_effects(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> DomMutationEffects {
        self.insert_before_effects(parent, child, None)
    }

    pub fn remove_child_effects(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
    ) -> DomMutationEffects {
        let owner_document = self.owner_document_handle(parent);
        let records_enabled = self.mutation_records_enabled();
        let removed_tree_was_connected = self.is_connected(child);
        let parser_form_owner_resets = self.parser_form_owner_resets_for_removed_subtrees(&[child]);
        let removal_context = self.subtree_removal_context(child);
        let removed_shadow_slot_assignment_snapshots = self
            .slot_assignment_snapshots_for_removed_shadow_tree_slots(&removal_context.shadow_slots);
        // Only compute the prior slot name when the parent actually has a
        // shadow root attached; otherwise no host-child slot snapshot is
        // needed (the case for every mutation on a plain non-shadow parent).
        let prior_slot_name = if self.shadow_root_handle(parent).is_some() {
            self.node(child).map(|_| self.slot_name_for_node(child))
        } else {
            None
        };
        let slot_assignment_snapshots = prior_slot_name
            .as_deref()
            .map(|slot_name| {
                self.slot_assignment_snapshots_for_host_child_names(parent, child, &[slot_name])
            })
            .unwrap_or_default();
        let previous_sibling = self.node(child).and_then(Node::prev_sibling);
        let next_sibling = self.node(child).and_then(Node::next_sibling);
        let previous_textarea_value = self.textarea_value_excluding_children(parent, &[]);
        let candidate_changes = self
            .dom
            .remove_child_with_stylesheet_candidate_changes(parent, child);
        if let Some(candidate_changes) = candidate_changes {
            self.invalidate_shadow_slot_name_index_for_tree_parent(parent);
            self.prune_disconnected_hovered_elements();
            self.reset_parser_form_owners_after_subtree_removal(child, &parser_form_owner_resets);
            let mut effects = DomMutationEffects::changed();
            effects.record_textarea_value_change(
                parent,
                previous_textarea_value.as_deref(),
                self.textarea_value_excluding_children(parent, &[])
                    .as_deref(),
            );
            effects.extend_stylesheet_candidate_changes(candidate_changes);
            self.clear_popover_open_states(&removal_context.open_popovers, &mut effects);
            effects.extend_stylesheet_owner_changes(
                self.sync_shadow_tree_scopes_for_removed_subtree(
                    &removal_context.shadow_hosts,
                    removed_tree_was_connected,
                ),
            );
            self.record_mutation(MutationScope::QueryState);
            if let Some(document) = owner_document {
                self.update_document_target_from_url(document);
            }
            effects.mark_disconnected_root(child);
            if records_enabled {
                effects.mark_child_list_mutation(
                    parent,
                    &[],
                    &[child],
                    previous_sibling,
                    next_sibling,
                );
            } else {
                effects.mark_style_child_list_mutation(
                    parent,
                    &[],
                    &[child],
                    previous_sibling,
                    next_sibling,
                );
            }
            self.mark_stylesheet_owner_contents_change_for_parent(&mut effects, parent);
            self.record_host_child_slot_changes_from_snapshots(
                &mut effects,
                slot_assignment_snapshots,
            );
            self.record_slot_assignment_changes_from_snapshots(
                &mut effects,
                removed_shadow_slot_assignment_snapshots,
            );
            self.record_slot_changes_for_removed_shadow_tree_slots(
                &mut effects,
                &removal_context.shadow_slots,
            );
            return effects;
        }
        DomMutationEffects::default()
    }

    fn clear_popover_open_states(
        &mut self,
        open_popovers: &[DomHandle],
        effects: &mut DomMutationEffects,
    ) {
        let mut did_change = false;
        for &handle in open_popovers {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                continue;
            };
            if element.set_popover_open(false) {
                did_change = true;
                effects.mark_removed_open_popover(handle);
            }
        }
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
    }

    pub fn insert_before(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> bool {
        self.insert_before_effects(parent, child, reference_child)
            .did_change()
    }

    pub fn insert_before_effects(
        &mut self,
        parent: DomHandle,
        child: DomHandle,
        reference_child: Option<DomHandle>,
    ) -> DomMutationEffects {
        let reference_child = if reference_child == Some(child) {
            self.node(child).and_then(Node::next_sibling)
        } else {
            reference_child
        };
        let old_owner_document = self.owner_document_handle(child);
        let records_enabled = self.mutation_records_enabled();
        let inserted_fragment_children = self
            .node(child)
            .filter(|node| node.is_document_fragment())
            .map(|_| self.child_handles(child).collect::<Vec<_>>())
            .unwrap_or_default();
        let single_child_was_connected_before_insert =
            inserted_fragment_children.is_empty() && self.is_connected(child);
        let implicit_removal_slot_state = inserted_fragment_children
            .is_empty()
            .then(|| {
                self.node(child)
                    .and_then(Node::parent_node)
                    .filter(|old_parent| *old_parent != parent || reference_child != Some(child))
                    .map(|old_parent| {
                        let prior_slot_name = if self.shadow_root_handle(old_parent).is_some() {
                            Some(self.slot_name_for_node(child))
                        } else {
                            None
                        };
                        let slot_assignment_snapshots = prior_slot_name
                            .as_deref()
                            .map(|slot_name| {
                                self.slot_assignment_snapshots_for_host_child_names(
                                    old_parent,
                                    child,
                                    &[slot_name],
                                )
                            })
                            .unwrap_or_default();
                        let removal_context = self.subtree_removal_context(child);
                        let removed_shadow_slot_assignment_snapshots = self
                            .slot_assignment_snapshots_for_removed_shadow_tree_slots(
                                &removal_context.shadow_slots,
                            );
                        (
                            old_parent,
                            slot_assignment_snapshots,
                            removal_context,
                            removed_shadow_slot_assignment_snapshots,
                        )
                    })
            })
            .flatten();
        let removal_record = if inserted_fragment_children.is_empty() {
            self.node(child)
                .and_then(Node::parent_node)
                .filter(|old_parent| *old_parent != parent || reference_child != Some(child))
                .map(|old_parent| {
                    let previous_sibling = self.node(child).and_then(Node::prev_sibling);
                    let next_sibling = self.node(child).and_then(Node::next_sibling);
                    (old_parent, vec![child], previous_sibling, next_sibling)
                })
        } else {
            let previous_sibling = inserted_fragment_children
                .first()
                .and_then(|handle| self.node(*handle).and_then(Node::prev_sibling));
            let next_sibling = inserted_fragment_children
                .last()
                .and_then(|handle| self.node(*handle).and_then(Node::next_sibling));
            Some((
                child,
                inserted_fragment_children.clone(),
                previous_sibling,
                next_sibling,
            ))
        };
        let insertion_slot_assignment_snapshots = if self.shadow_root_handle(parent).is_none() {
            Vec::new()
        } else if inserted_fragment_children.is_empty() {
            let slot_name = self.slot_name_for_node(child);
            self.slot_assignment_snapshots_for_host_child_names(parent, child, &[&slot_name])
        } else {
            let mut snapshots = Vec::new();
            for &inserted_child in &inserted_fragment_children {
                let slot_name = self.slot_name_for_node(inserted_child);
                snapshots.extend(self.slot_assignment_snapshots_for_host_child_names(
                    parent,
                    inserted_child,
                    &[&slot_name],
                ));
            }
            snapshots
        };
        let inserted_roots = if inserted_fragment_children.is_empty() {
            vec![child]
        } else {
            inserted_fragment_children.clone()
        };
        let had_implicit_removal = removal_record.is_some();
        let parser_form_owner_resets =
            self.parser_form_owner_resets_for_removed_subtrees(&inserted_roots);
        let checked_radio_form_owners_before_insert =
            self.checked_radio_form_owner_snapshots_in_subtrees(&inserted_roots);
        let inserted_option_owner_snapshots =
            self.option_owner_snapshots_in_subtrees(&inserted_roots);
        let inserted_shadow_slot_assignment_snapshots =
            self.slot_assignment_snapshots_for_inserted_shadow_tree_slots(parent, &inserted_roots);
        let previous_shadow_root = self.containing_shadow_root(child);
        let removed_textarea_value = removal_record.as_ref().map(|(old_parent, removed, _, _)| {
            (
                *old_parent,
                self.textarea_value_excluding_children(*old_parent, &[]),
                self.textarea_value_excluding_children(*old_parent, removed),
            )
        });
        let previous_textarea_value = match &removed_textarea_value {
            Some((old_parent, _, after)) if *old_parent == parent => after.clone(),
            _ => self.textarea_value_excluding_children(parent, &[]),
        };
        let candidate_changes = self.dom.insert_before_with_stylesheet_candidate_changes(
            parent,
            child,
            reference_child,
        );
        if let Some(candidate_changes) = candidate_changes {
            if inserted_fragment_children.is_empty()
                && self.node(child).is_some_and(Node::is_document_fragment)
            {
                // The insertion was valid, but an empty fragment has no
                // mutation payloads or tree invalidations. Keep the success
                // flag used by appendChild/insertBefore callers.
                return DomMutationEffects::changed();
            }
            if let Some(shadow_root) = previous_shadow_root {
                self.invalidate_shadow_slot_name_index(shadow_root);
            }
            self.invalidate_shadow_slot_name_index_for_tree_parent(parent);
            self.reset_parser_form_owners_after_subtree_insertion(
                &inserted_roots,
                had_implicit_removal,
                &parser_form_owner_resets,
            );
            self.normalize_selected_options_after_owner_select_changes(
                &inserted_option_owner_snapshots,
            );
            self.normalize_checked_radio_groups_after_form_owner_changes(
                &checked_radio_form_owners_before_insert,
            );
            self.normalize_checked_radio_groups_after_form_owner_changes(
                &checked_radio_form_owners_before_insert,
            );
            let new_owner_document = self.owner_document_handle(parent);
            let removed_connected_shadow_tree = single_child_was_connected_before_insert
                && implicit_removal_slot_state
                    .as_ref()
                    .is_some_and(|(_, _, context, _)| !context.shadow_hosts.is_empty())
                && (!self.is_connected(child) || new_owner_document != old_owner_document);
            let departed_connected_shadow_tree_document = if removed_connected_shadow_tree {
                old_owner_document
            } else {
                None
            };
            let mut shadow_stylesheet_owner_changes = Vec::new();
            let inserted_shadow_hosts =
                self.record_inserted_subtree_candidates_in_subtrees(&inserted_roots);
            shadow_stylesheet_owner_changes.extend(
                self.sync_shadow_tree_scopes_for_inserted_subtrees(
                    &inserted_roots,
                    &inserted_shadow_hosts,
                    departed_connected_shadow_tree_document,
                ),
            );
            self.record_mutation(MutationScope::QueryState);
            if let Some(document) = old_owner_document {
                self.update_document_target_from_url(document);
            }
            if new_owner_document != old_owner_document
                && let Some(document) = new_owner_document
            {
                self.update_document_target_from_url(document);
            }
            let mut effects =
                self.tree_insertion_effects(parent, child, &inserted_fragment_children);
            effects.extend_stylesheet_candidate_changes(candidate_changes);
            effects.extend_stylesheet_owner_changes(shadow_stylesheet_owner_changes);
            if let Some((old_parent, before, after)) = removed_textarea_value {
                effects.record_textarea_value_change(
                    old_parent,
                    before.as_deref(),
                    after.as_deref(),
                );
            }
            effects.record_textarea_value_change(
                parent,
                previous_textarea_value.as_deref(),
                self.textarea_value_excluding_children(parent, &[])
                    .as_deref(),
            );
            if single_child_was_connected_before_insert && !self.is_connected(child) {
                effects.mark_disconnected_root(child);
                if let Some((_, _, removal_context, _)) = implicit_removal_slot_state.as_ref() {
                    self.clear_popover_open_states(&removal_context.open_popovers, &mut effects);
                }
            } else if single_child_was_connected_before_insert
                && new_owner_document != old_owner_document
                && let Some((_, _, removal_context, _)) = implicit_removal_slot_state.as_ref()
            {
                self.clear_popover_open_states(&removal_context.open_popovers, &mut effects);
            }
            if let Some((old_parent, removed_nodes, previous_sibling, next_sibling)) =
                removal_record
            {
                if records_enabled {
                    effects.mark_child_list_mutation(
                        old_parent,
                        &[],
                        &removed_nodes,
                        previous_sibling,
                        next_sibling,
                    );
                } else {
                    effects.mark_style_child_list_mutation(
                        old_parent,
                        &[],
                        &removed_nodes,
                        previous_sibling,
                        next_sibling,
                    );
                }
                self.mark_stylesheet_owner_contents_change_for_parent(&mut effects, old_parent);
            }
            if let Some((
                old_parent,
                slot_assignment_snapshots,
                removal_context,
                removed_shadow_slot_assignment_snapshots,
            )) = implicit_removal_slot_state
            {
                self.record_host_child_slot_changes_from_snapshots(
                    &mut effects,
                    slot_assignment_snapshots,
                );
                self.record_slot_assignment_changes_from_snapshots(
                    &mut effects,
                    removed_shadow_slot_assignment_snapshots,
                );
                self.record_slot_fallback_child_change(&mut effects, old_parent);
                self.record_slot_changes_for_removed_shadow_tree_slots(
                    &mut effects,
                    &removal_context.shadow_slots,
                );
            }
            if inserted_fragment_children.is_empty() {
                let previous_sibling = self.node(child).and_then(Node::prev_sibling);
                let next_sibling = self.node(child).and_then(Node::next_sibling);
                if records_enabled {
                    effects.mark_child_list_mutation(
                        parent,
                        std::slice::from_ref(&child),
                        &[],
                        previous_sibling,
                        next_sibling,
                    );
                } else {
                    effects.mark_style_child_list_mutation(
                        parent,
                        std::slice::from_ref(&child),
                        &[],
                        previous_sibling,
                        next_sibling,
                    );
                }
            } else {
                let previous_sibling = inserted_fragment_children
                    .first()
                    .and_then(|handle| self.node(*handle).and_then(Node::prev_sibling));
                let next_sibling = inserted_fragment_children
                    .last()
                    .and_then(|handle| self.node(*handle).and_then(Node::next_sibling));
                if records_enabled {
                    effects.mark_child_list_mutation(
                        parent,
                        &inserted_fragment_children,
                        &[],
                        previous_sibling,
                        next_sibling,
                    );
                } else {
                    effects.mark_style_child_list_mutation(
                        parent,
                        &inserted_fragment_children,
                        &[],
                        previous_sibling,
                        next_sibling,
                    );
                }
            }
            self.mark_stylesheet_owner_contents_change_for_parent(&mut effects, parent);
            self.record_host_child_slot_changes_from_snapshots(
                &mut effects,
                insertion_slot_assignment_snapshots,
            );
            self.record_slot_assignment_changes_from_snapshots(
                &mut effects,
                inserted_shadow_slot_assignment_snapshots,
            );
            self.record_slot_fallback_child_change(&mut effects, parent);
            self.record_slot_changes_for_inserted_shadow_tree_slots_in_subtrees(
                &mut effects,
                &inserted_roots,
            );
            return effects;
        }
        DomMutationEffects::default()
    }

    fn option_owner_snapshots_in_subtrees(
        &self,
        roots: &[DomHandle],
    ) -> Vec<(DomHandle, Option<DomHandle>, bool)> {
        roots
            .iter()
            .flat_map(|root| {
                self.collect_matching_elements(*root, true, |handle| {
                    self.is_html_element_named(handle, "option")
                })
            })
            .map(|option| {
                let selected = self
                    .node(option)
                    .and_then(Node::as_element)
                    .is_some_and(Element::selected);
                (
                    option,
                    self.option_nearest_ancestor_select(option),
                    selected,
                )
            })
            .collect()
    }

    fn normalize_selected_options_after_owner_select_changes(
        &mut self,
        snapshots: &[(DomHandle, Option<DomHandle>, bool)],
    ) {
        for &(option, previous_select, was_selected) in snapshots {
            let Some(select) = self.option_nearest_ancestor_select(option) else {
                continue;
            };
            if previous_select == Some(select)
                || !was_selected
                || self
                    .node(select)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.has_attribute("multiple"))
            {
                continue;
            }
            for peer in self.select_option_elements(select) {
                let Some(peer_element) = self.node(peer).and_then(Node::as_element) else {
                    continue;
                };
                let dirty = peer_element.selected_dirty();
                let _ = self.set_selected_state_with_dirty(peer, peer == option, dirty);
            }
            let _ = self.set_select_explicit_none_state(select, false);
        }
    }

    pub fn text_content(&self, handle: DomHandle) -> Option<String> {
        self.dom.text_content(handle)
    }

    pub fn inner_html(&self, handle: DomHandle) -> Option<String> {
        self.dom.inner_html(handle)
    }

    pub fn node_metadata(&self, handle: DomHandle) -> Option<LiveDomNodeMetadata> {
        self.dom.node_metadata(handle)
    }

    pub fn set_text_content(&mut self, handle: DomHandle, value: &str) -> bool {
        self.set_text_content_effects(handle, value).did_change()
    }

    pub fn set_text_content_effects(
        &mut self,
        handle: DomHandle,
        value: &str,
    ) -> DomMutationEffects {
        let Some(node_type) = self.node(handle).map(Node::node_type) else {
            return DomMutationEffects::default();
        };

        match node_type {
            NodeType::Text
            | NodeType::CDataSection
            | NodeType::Comment
            | NodeType::ProcessingInstruction => {
                self.set_character_data_value_effects(handle, value.into(), false)
            }
            NodeType::Element | NodeType::DocumentFragment => {
                let children = self.child_handles(handle).collect::<Vec<_>>();
                let records_enabled = self.mutation_records_enabled();
                let added_text = if value.is_empty() {
                    None
                } else {
                    let owner_document = match self.owner_document_handle(handle) {
                        Some(owner_document) => owner_document,
                        None => return DomMutationEffects::default(),
                    };
                    Some(self.create_text_node_for_document(owner_document, value))
                };
                let mut effects = DomMutationEffects::default();
                for &child in &children {
                    effects.merge(self.remove_child_effects(handle, child));
                }

                if let Some(text_handle) = added_text {
                    effects.merge(self.append_child_effects(handle, text_handle));
                }
                if records_enabled && effects.did_change() {
                    effects.clear_mutation_records();
                    effects.mark_child_list_mutation(
                        handle,
                        added_text.as_slice(),
                        &children,
                        None,
                        None,
                    );
                }
                effects
            }
            NodeType::Document => DomMutationEffects::default(),
            NodeType::DocumentType => DomMutationEffects::default(),
        }
    }

    /// Replace the native DOMString and collect the same effects for UTF-8 and
    /// UTF-16 callers. CharacterData edits may queue a record even when equal.
    pub fn set_character_data_value_effects(
        &mut self,
        handle: DomHandle,
        value: crate::native::DomStringValue,
        force_mutation_record: bool,
    ) -> DomMutationEffects {
        let Some(node) = self.node(handle) else {
            return DomMutationEffects::default();
        };
        let Some(previous) = node.character_data_value() else {
            return DomMutationEffects::default();
        };
        let changed = previous != &value;
        let should_record = self.mutation_records_enabled() && (changed || force_mutation_record);
        if !changed && !should_record {
            return DomMutationEffects::default();
        }
        let old_value = should_record.then(|| previous.clone());
        let is_text = matches!(node.node_type(), NodeType::Text | NodeType::CDataSection);
        let textarea = if changed && is_text {
            self.parent_node(handle).and_then(|parent| {
                self.textarea_value_excluding_children(parent, &[])
                    .map(|value| (parent, value))
            })
        } else {
            None
        };
        let mut effects = if changed {
            *self
                .node_mut(handle)
                .and_then(Node::character_data_value_mut)
                .unwrap() = value;
            self.record_mutation(MutationScope::QueryState);
            let mut effects = self.node_update_effects(handle);
            if is_text {
                effects.mark_style_character_data_mutation(handle);
                if let Some((parent, before)) = textarea {
                    effects.record_textarea_value_change(
                        parent,
                        Some(&before),
                        self.textarea_value_excluding_children(parent, &[])
                            .as_deref(),
                    );
                }
                if let Some(parent) = self.parent_node(handle) {
                    self.mark_stylesheet_owner_contents_change_for_parent(&mut effects, parent);
                }
            }
            effects
        } else {
            DomMutationEffects::default()
        };
        if should_record {
            effects.mark_character_data_mutation(handle, old_value);
        }
        effects
    }

    pub fn connected_script_handles(&self, root: DomHandle) -> Vec<DomHandle> {
        let mut handles = Vec::new();
        self.collect_script_handles_in_shadow_including_subtree(root, true, &mut handles);
        handles
    }

    pub fn script_handles_in_subtree(&self, root: DomHandle) -> Vec<DomHandle> {
        let mut handles = Vec::new();
        self.collect_script_handles_in_shadow_including_subtree(root, false, &mut handles);
        handles
    }

    fn collect_script_handles_in_shadow_including_subtree(
        &self,
        root: DomHandle,
        connected_only: bool,
        out: &mut Vec<DomHandle>,
    ) {
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            let Some(node) = self.node(handle) else {
                continue;
            };
            if node.is_script_element() && (!connected_only || node.flags().connected()) {
                out.push(handle);
            }
            stack.extend(self.child_handles_reversed(handle));
            if let Some(shadow_root) = self.shadow_root_handle(handle) {
                stack.push(shadow_root);
            }
        }
    }

    pub fn snapshot_document(&self) -> NativeDom {
        self.dom.clone()
    }

    pub(super) fn tree_insertion_effects(
        &self,
        parent: DomHandle,
        child: DomHandle,
        inserted_fragment_children: &[DomHandle],
    ) -> DomMutationEffects {
        let mut effects = DomMutationEffects::changed();
        if self.node(child).is_some_and(Node::is_document_fragment) {
            for &inserted_child in inserted_fragment_children {
                effects.mark_connected_root(inserted_child);
                if self.is_script_element(inserted_child) {
                    effects.mark_script_prepare_trigger(
                        inserted_child,
                        ScriptPrepareTriggerKind::Connected,
                    );
                } else if self.subtree_can_contain_connected_scripts(inserted_child) {
                    effects.mark_connected_script_root(inserted_child);
                }
            }
        } else if self.is_script_element(child) {
            effects.mark_connected_root(child);
            effects.mark_script_prepare_trigger(child, ScriptPrepareTriggerKind::Connected);
        } else if self.subtree_can_contain_connected_scripts(child) {
            effects.mark_connected_root(child);
            effects.mark_connected_script_root(child);
        } else {
            effects.mark_connected_root(child);
        }
        if self.is_script_element(parent) {
            effects.mark_script_prepare_trigger(parent, ScriptPrepareTriggerKind::ChildInsertion);
        }
        effects
    }

    fn subtree_can_contain_connected_scripts(&self, root: DomHandle) -> bool {
        self.dom.first_child(root).is_some()
            || self
                .shadow_root_handle(root)
                .is_some_and(|shadow_root| self.dom.first_child(shadow_root).is_some())
    }

    pub(super) fn node_update_effects(&self, _handle: DomHandle) -> DomMutationEffects {
        DomMutationEffects::changed()
    }

    fn mark_stylesheet_owner_contents_change_for_parent(
        &self,
        effects: &mut DomMutationEffects,
        parent: DomHandle,
    ) {
        if self.is_inline_style_sheet_owner(parent)
            && !self.is_style_element_parsing_children(parent)
        {
            effects.mark_stylesheet_owner_contents_change(
                parent,
                self.dom.stylesheet_candidate_tree_scope_for_node(parent),
            );
        }
    }
}
