use std::sync::Arc;

use super::*;

impl DomHost {
    pub fn explicit_element_references(
        &self,
        handle: DomHandle,
        attribute: &str,
    ) -> Option<Vec<DomHandle>> {
        self.node(handle)?
            .as_element()?
            .explicit_element_references(attribute)
            .map(<[DomHandle]>::to_vec)
    }

    pub fn set_explicit_element_references(
        &mut self,
        handle: DomHandle,
        attribute: &str,
        references: Vec<DomHandle>,
    ) -> bool {
        let Some(element) = self
            .node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
        else {
            return false;
        };
        element.set_explicit_element_references(attribute, references);
        true
    }

    pub fn get_attribute(&self, handle: DomHandle, name: &str) -> Option<String> {
        self.dom.get_attribute(handle, name)
    }

    pub fn get_attribute_ns(
        &self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<String> {
        self.dom.get_attribute_ns(handle, namespace, local_name)
    }

    pub fn has_attribute_ns(
        &self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> bool {
        self.dom.has_attribute_ns(handle, namespace, local_name)
    }

    pub fn set_attribute(&mut self, handle: DomHandle, name: &str, value: &str) -> bool {
        self.set_attribute_effects(handle, name, value).did_change()
    }

    pub fn set_attribute_utf16_units(
        &mut self,
        handle: DomHandle,
        name: &str,
        value: &str,
        units: Vec<u16>,
    ) -> bool {
        self.set_attribute_utf16_units_effects(handle, name, value, units)
            .did_change()
    }

    pub fn set_attribute_ns(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        value: &str,
    ) -> bool {
        self.set_attribute_ns_effects(handle, namespace, prefix, local_name, local_name, value)
            .did_change()
    }

    pub fn set_attribute_effects(
        &mut self,
        handle: DomHandle,
        name: &str,
        value: &str,
    ) -> DomMutationEffects {
        self.set_attribute_mutation_outcome(handle, name, value)
            .into_effects()
    }

    pub fn set_attribute_mutation_outcome(
        &mut self,
        handle: DomHandle,
        name: &str,
        value: &str,
    ) -> DomAttributeMutationOutcome {
        self.set_attribute_mutation_outcome_with_utf16_units(handle, name, value, None)
    }

    pub fn set_attribute_utf16_units_effects(
        &mut self,
        handle: DomHandle,
        name: &str,
        value: &str,
        units: Vec<u16>,
    ) -> DomMutationEffects {
        self.set_attribute_utf16_units_mutation_outcome(handle, name, value, units)
            .into_effects()
    }

    pub fn set_attribute_utf16_units_mutation_outcome(
        &mut self,
        handle: DomHandle,
        name: &str,
        value: &str,
        units: Vec<u16>,
    ) -> DomAttributeMutationOutcome {
        self.set_attribute_mutation_outcome_with_utf16_units(handle, name, value, Some(units))
    }

    fn set_attribute_mutation_outcome_with_utf16_units(
        &mut self,
        handle: DomHandle,
        name: &str,
        value: &str,
        units: Option<Vec<u16>>,
    ) -> DomAttributeMutationOutcome {
        if name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let records_enabled = self.mutation_records_enabled();
        let prior_value = self.get_attribute(handle, name).map(Arc::from);
        let prior_slot_name = if name.eq_ignore_ascii_case("slot") {
            self.node(handle)
                .and_then(Node::as_element)
                .map(|_| self.slot_name_for_node(handle))
        } else {
            None
        };
        let prior_shadow_slot_name = self.shadow_tree_slot_name_change(handle, name);
        let host_child_slot_assignment_snapshots = if name.eq_ignore_ascii_case("slot") {
            self.node(handle)
                .and_then(Node::parent_node)
                .map(|parent| {
                    let mut slot_names = Vec::new();
                    if let Some(prior_name) = prior_slot_name.as_deref() {
                        slot_names.push(prior_name);
                    }
                    slot_names.push(value);
                    self.slot_assignment_snapshots_for_host_child_names(parent, handle, &slot_names)
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let shadow_slot_assignment_snapshots = prior_shadow_slot_name
            .as_ref()
            .map(|(shadow_root, prior_name, _prior_assigned_nodes)| {
                self.slot_assignment_snapshots_for_shadow_slot_names(
                    *shadow_root,
                    &[prior_name.as_str(), value],
                )
            })
            .unwrap_or_default();
        let changed = if let Some(units) = units {
            self.dom
                .set_attribute_utf16_units(handle, name, value, units)
        } else {
            self.dom.set_attribute(handle, name, value)
        };
        if changed {
            self.invalidate_shadow_slot_name_index_for_attribute(handle, None, name);
            self.sync_select_state_after_option_selected_attribute(handle, name);
            // Only id/name mutations can change document-level named access.
            // The named candidate indexes are monotonic; live lookup validates
            // the current value, so only the new candidate needs recording.
            if name.eq_ignore_ascii_case("id") || name.eq_ignore_ascii_case("name") {
                self.record_named_index_candidate(handle);
            }
            self.record_mutation(MutationScope::QueryState);
            if name.eq_ignore_ascii_case("id") || name.eq_ignore_ascii_case("name") {
                self.update_target_after_indicated_part_mutation(handle);
            }
            let mut effects = self.node_update_effects(handle);
            self.mark_stylesheet_owner_attribute_change(&mut effects, handle, None, name);
            if self.is_script_element(handle) {
                let source_attribute = self.node(handle).and_then(Node::as_element).is_some_and(
                    |element| match element.namespace() {
                        "http://www.w3.org/1999/xhtml" => name.eq_ignore_ascii_case("src"),
                        "http://www.w3.org/2000/svg" => name.eq_ignore_ascii_case("href"),
                        _ => false,
                    },
                );
                if source_attribute
                    && prior_value.as_deref().is_none_or(str::is_empty)
                    && !value.is_empty()
                {
                    effects.mark_script_prepare_trigger(
                        handle,
                        ScriptPrepareTriggerKind::SourceAttributeAdded,
                    );
                } else if name.eq_ignore_ascii_case("async") {
                    effects.mark_script_prepare_trigger(
                        handle,
                        ScriptPrepareTriggerKind::AsyncAttributeAdded,
                    );
                }
            }
            effects.mark_attribute_change(
                handle,
                name,
                None,
                prior_value.clone(),
                Some(value),
                records_enabled,
            );
            if name.eq_ignore_ascii_case("slot") {
                self.record_host_child_slot_changes_from_snapshots(
                    &mut effects,
                    host_child_slot_assignment_snapshots,
                );
            }
            if let Some((shadow_root, prior_name, prior_assigned_nodes)) = prior_shadow_slot_name {
                self.record_slot_assignment_changes_from_snapshots(
                    &mut effects,
                    shadow_slot_assignment_snapshots,
                );
                self.record_slot_changes_for_shadow_tree_slot_name_change(
                    &mut effects,
                    shadow_root,
                    handle,
                    &prior_name,
                    &prior_assigned_nodes,
                );
            }
            return DomAttributeMutationOutcome::new(effects, prior_value);
        }
        if records_enabled && prior_value.as_deref() == Some(value) {
            let mut effects = DomMutationEffects::default();
            effects.queue_attribute_mutation_record(
                handle,
                name,
                None,
                prior_value.clone(),
                Some(value),
            );
            return DomAttributeMutationOutcome::new(effects, prior_value);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value)
    }

    fn sync_select_state_after_option_selected_attribute(&mut self, handle: DomHandle, name: &str) {
        if !name.eq_ignore_ascii_case("selected") || !self.is_html_element_named(handle, "option") {
            return;
        }
        let Some(select) = self.option_nearest_ancestor_select(handle) else {
            return;
        };
        if self
            .node(select)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute("multiple"))
        {
            return;
        }
        if self
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(Element::selected)
        {
            for option in self.select_option_elements(select) {
                let _ = self.set_selected_state_with_dirty(option, option == handle, false);
            }
            let _ = self.set_select_explicit_none_state(select, false);
        }
    }

    pub fn set_attribute_ns_effects(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        _qualified_name: &str,
        value: &str,
    ) -> DomMutationEffects {
        self.set_attribute_ns_mutation_outcome(handle, namespace, prefix, local_name, value)
            .into_effects()
    }

    pub fn set_attribute_ns_mutation_outcome(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        value: &str,
    ) -> DomAttributeMutationOutcome {
        if namespace.is_none() && local_name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let records_enabled = self.mutation_records_enabled();
        let prior_value = self
            .get_attribute_ns(handle, namespace, local_name)
            .map(Arc::from);
        let changed = self
            .dom
            .set_attribute_ns(handle, namespace, prefix, local_name, value);
        if changed {
            self.invalidate_shadow_slot_name_index_for_attribute(handle, namespace, local_name);
            // Namespace-aware calls can still target HTML id/name by local name.
            if local_name.eq_ignore_ascii_case("id") || local_name.eq_ignore_ascii_case("name") {
                self.record_named_index_candidate(handle);
            }
            self.record_mutation(MutationScope::QueryState);
            if local_name.eq_ignore_ascii_case("id") || local_name.eq_ignore_ascii_case("name") {
                self.update_target_after_indicated_part_mutation(handle);
            }
            let mut effects = self.node_update_effects(handle);
            self.mark_stylesheet_owner_attribute_change(
                &mut effects,
                handle,
                namespace,
                local_name,
            );
            effects.mark_attribute_change(
                handle,
                local_name,
                namespace,
                prior_value.clone(),
                Some(value),
                records_enabled,
            );
            return DomAttributeMutationOutcome::new(effects, prior_value);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value)
    }

    pub fn remove_attribute_effects(
        &mut self,
        handle: DomHandle,
        name: &str,
    ) -> DomMutationEffects {
        self.remove_attribute_mutation_outcome(handle, name)
            .into_effects()
    }

    pub fn remove_attribute_mutation_outcome(
        &mut self,
        handle: DomHandle,
        name: &str,
    ) -> DomAttributeMutationOutcome {
        let records_enabled = self.mutation_records_enabled();
        let prior_value = self.get_attribute(handle, name).map(Arc::from);
        if prior_value.is_some() && name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let prior_slot_name = if name.eq_ignore_ascii_case("slot") {
            self.node(handle)
                .and_then(Node::as_element)
                .map(|_| self.slot_name_for_node(handle))
        } else {
            None
        };
        let prior_shadow_slot_name = self.shadow_tree_slot_name_change(handle, name);
        let host_child_slot_assignment_snapshots = if name.eq_ignore_ascii_case("slot") {
            self.node(handle)
                .and_then(Node::parent_node)
                .map(|parent| {
                    let mut slot_names = Vec::new();
                    if let Some(prior_name) = prior_slot_name.as_deref() {
                        slot_names.push(prior_name);
                    }
                    slot_names.push("");
                    self.slot_assignment_snapshots_for_host_child_names(parent, handle, &slot_names)
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let shadow_slot_assignment_snapshots = prior_shadow_slot_name
            .as_ref()
            .map(|(shadow_root, prior_name, _prior_assigned_nodes)| {
                self.slot_assignment_snapshots_for_shadow_slot_names(
                    *shadow_root,
                    &[prior_name.as_str(), ""],
                )
            })
            .unwrap_or_default();
        let removed = self.dom.remove_attribute(handle, name);
        if removed {
            self.invalidate_shadow_slot_name_index_for_attribute(handle, None, name);
            if name.eq_ignore_ascii_case("id") || name.eq_ignore_ascii_case("name") {
                self.record_named_index_candidate(handle);
            }
            self.record_mutation(MutationScope::QueryState);
            if name.eq_ignore_ascii_case("id") || name.eq_ignore_ascii_case("name") {
                self.update_target_after_indicated_part_mutation(handle);
            }
            let mut effects = self.node_update_effects(handle);
            self.mark_stylesheet_owner_attribute_change(&mut effects, handle, None, name);
            effects.mark_attribute_change(
                handle,
                name,
                None,
                prior_value.clone(),
                None,
                records_enabled,
            );
            if name.eq_ignore_ascii_case("slot") {
                self.record_host_child_slot_changes_from_snapshots(
                    &mut effects,
                    host_child_slot_assignment_snapshots,
                );
            }
            if let Some((shadow_root, prior_name, prior_assigned_nodes)) = prior_shadow_slot_name {
                self.record_slot_assignment_changes_from_snapshots(
                    &mut effects,
                    shadow_slot_assignment_snapshots,
                );
                self.record_slot_changes_for_shadow_tree_slot_name_change(
                    &mut effects,
                    shadow_root,
                    handle,
                    &prior_name,
                    &prior_assigned_nodes,
                );
            }
            return DomAttributeMutationOutcome::new(effects, prior_value);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value)
    }

    pub fn remove_attribute_ns_effects(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> DomMutationEffects {
        self.remove_attribute_ns_mutation_outcome(handle, namespace, local_name)
            .into_effects()
    }

    pub fn remove_attribute_ns_mutation_outcome(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> DomAttributeMutationOutcome {
        let records_enabled = self.mutation_records_enabled();
        let prior_value = self
            .get_attribute_ns(handle, namespace, local_name)
            .map(Arc::from);
        if prior_value.is_some() && namespace.is_none() && local_name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let removed = self.dom.remove_attribute_ns(handle, namespace, local_name);
        if removed {
            self.invalidate_shadow_slot_name_index_for_attribute(handle, namespace, local_name);
            if local_name.eq_ignore_ascii_case("id") || local_name.eq_ignore_ascii_case("name") {
                self.record_named_index_candidate(handle);
            }
            self.record_mutation(MutationScope::QueryState);
            if local_name.eq_ignore_ascii_case("id") || local_name.eq_ignore_ascii_case("name") {
                self.update_target_after_indicated_part_mutation(handle);
            }
            let mut effects = self.node_update_effects(handle);
            self.mark_stylesheet_owner_attribute_change(
                &mut effects,
                handle,
                namespace,
                local_name,
            );
            effects.mark_attribute_change(
                handle,
                local_name,
                namespace,
                prior_value.clone(),
                None,
                records_enabled,
            );
            return DomAttributeMutationOutcome::new(effects, prior_value);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value)
    }

    pub fn remove_attribute(&mut self, handle: DomHandle, name: &str) -> bool {
        self.remove_attribute_effects(handle, name).did_change()
    }

    pub fn remove_attribute_ns(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> bool {
        self.remove_attribute_ns_effects(handle, namespace, local_name)
            .did_change()
    }

    fn mark_stylesheet_owner_attribute_change(
        &self,
        effects: &mut DomMutationEffects,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) {
        if self.is_html_element_named(handle, "link") || self.is_inline_style_sheet_owner(handle) {
            effects.mark_stylesheet_owner_attribute_change(
                handle,
                namespace,
                local_name,
                self.dom.stylesheet_candidate_tree_scope_for_node(handle),
            );
        }
    }
}
