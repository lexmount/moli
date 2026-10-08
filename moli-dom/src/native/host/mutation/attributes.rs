use std::sync::Arc;

use super::*;
use crate::native::{Attribute, DomStringValue};
use moli_html_input_type::InputType;

struct InputValueAttributeChange {
    previous: Option<Arc<DomStringValue>>,
    current: DomStringValue,
}

#[derive(Default)]
struct SlotAttributeMutationSnapshots {
    host_child_assignments: Vec<(DomHandle, Vec<DomHandle>)>,
    shadow_slot: Option<(DomHandle, String, Vec<DomHandle>)>,
    shadow_assignments: Vec<(DomHandle, Vec<DomHandle>)>,
}

impl SlotAttributeMutationSnapshots {
    fn record(self, host: &DomHost, effects: &mut DomMutationEffects, handle: DomHandle) {
        host.record_host_child_slot_changes_from_snapshots(effects, self.host_child_assignments);
        if let Some((shadow_root, prior_name, prior_assigned_nodes)) = self.shadow_slot {
            host.record_slot_assignment_changes_from_snapshots(effects, self.shadow_assignments);
            host.record_slot_changes_for_shadow_tree_slot_name_change(
                effects,
                shadow_root,
                handle,
                &prior_name,
                &prior_assigned_nodes,
            );
        }
    }
}

impl InputValueAttributeChange {
    fn record(self, effects: &mut DomMutationEffects, handle: DomHandle, records_enabled: bool) {
        effects.mark_attribute_change(
            handle,
            "value",
            None,
            self.previous,
            Some(Arc::new(self.current)),
            records_enabled,
        );
    }
}

impl DomHost {
    fn mark_script_attribute_set_trigger(
        &self,
        effects: &mut DomMutationEffects,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        previous: Option<&str>,
        value: &str,
    ) {
        if namespace.is_some_and(|namespace| !namespace.is_empty())
            || !self.is_script_element(handle)
        {
            return;
        }
        let source_attribute =
            self.node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| match element.namespace() {
                    "http://www.w3.org/1999/xhtml" => local_name == "src",
                    "http://www.w3.org/2000/svg" => {
                        local_name == "href"
                            && previous.is_none_or(str::is_empty)
                            && !value.is_empty()
                    }
                    _ => false,
                });
        // Every HTML src assignment invokes preparation, including an empty
        // value. The loader owns the already-started guard.
        if source_attribute {
            effects.mark_script_prepare_trigger(
                handle,
                ScriptPrepareTriggerKind::SourceAttributeAdded,
            );
        } else if local_name == "async" {
            effects
                .mark_script_prepare_trigger(handle, ScriptPrepareTriggerKind::AsyncAttributeAdded);
        }
    }

    fn slot_attribute_mutation_snapshots(
        &self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        value: &str,
    ) -> SlotAttributeMutationSnapshots {
        if namespace.is_some_and(|namespace| !namespace.is_empty()) {
            return SlotAttributeMutationSnapshots::default();
        }
        let host_child_assignments = if local_name.eq_ignore_ascii_case("slot") {
            self.node(handle)
                .and_then(Node::parent_node)
                .map(|parent| {
                    self.slot_assignment_snapshots_for_host_child_names(
                        parent,
                        handle,
                        &[&self.slot_name_for_node(handle), value],
                    )
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let shadow_slot = self.shadow_tree_slot_name_change(handle, local_name);
        let shadow_assignments = shadow_slot
            .as_ref()
            .map(|(shadow_root, prior_name, _)| {
                self.slot_assignment_snapshots_for_shadow_slot_names(
                    *shadow_root,
                    &[prior_name, value],
                )
            })
            .unwrap_or_default();
        SlotAttributeMutationSnapshots {
            host_child_assignments,
            shadow_slot,
            shadow_assignments,
        }
    }

    fn input_type_value_attribute_change(
        &self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
        value: Option<&str>,
    ) -> Option<InputValueAttributeChange> {
        if namespace.is_some_and(|namespace| !namespace.is_empty()) || local_name != "type" {
            return None;
        }
        let element = self.node(handle)?.as_element()?;
        let current = element.input_type_change_value_attribute(
            element.input_type(),
            InputType::from_attribute_value(value),
        )?;
        Some(InputValueAttributeChange {
            previous: self
                .dom
                .get_attribute_ns_dom_string_value(handle, None, "value")
                .map(Arc::new),
            current,
        })
    }

    fn input_type_value_attribute_change_for_qualified_name(
        &self,
        handle: DomHandle,
        name: &str,
        value: Option<&str>,
    ) -> Option<InputValueAttributeChange> {
        if !name.eq_ignore_ascii_case("type") {
            return None;
        }
        let element = self.node(handle)?.as_element()?;
        let attribute = element
            .attributes()
            .iter()
            .find(|attribute| attribute.name_matches(name));
        self.input_type_value_attribute_change(
            handle,
            attribute.map(Attribute::namespace),
            attribute.map_or("type", Attribute::local_name),
            value,
        )
    }
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

    pub fn get_attribute_utf16_units(&self, handle: DomHandle, name: &str) -> Option<Vec<u16>> {
        self.dom.get_attribute_utf16_units(handle, name)
    }

    pub fn get_attribute_ns_utf16_units(
        &self,
        handle: DomHandle,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Option<Vec<u16>> {
        self.dom
            .get_attribute_ns_utf16_units(handle, namespace, local_name)
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
        let Some(normalized_name) = self.dom.normalized_attribute_name(handle, name) else {
            return DomAttributeMutationOutcome::new(DomMutationEffects::default(), None, None);
        };
        let attribute = self
            .node(handle)
            .and_then(Node::as_element)
            .and_then(|element| {
                element
                    .attributes()
                    .iter()
                    .find(|attribute| attribute.name_matches(&normalized_name))
            });
        let (local_name, namespace, prior_value) = match attribute {
            Some(attribute) => (
                attribute.local_name().to_owned(),
                (!attribute.namespace().is_empty()).then(|| attribute.namespace().to_owned()),
                self.dom.get_attribute_dom_string_value(handle, name).map(Arc::new),
            ),
            None => (normalized_name, None, None),
        };
        let slot_changes = self.slot_attribute_mutation_snapshots(handle, None, name, value);
        let input_value_change =
            self.input_type_value_attribute_change_for_qualified_name(handle, name, Some(value));
        let new_value = Arc::new(
            units
                .as_deref()
                .map_or_else(|| DomStringValue::from(value), DomStringValue::from_utf16),
        );
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
            self.mark_stylesheet_owner_attribute_change(
                &mut effects, handle, namespace.as_deref(), &local_name,
            );
            self.mark_script_attribute_set_trigger(
                &mut effects, handle, namespace.as_deref(), &local_name,
                prior_value.as_deref().map(DomStringValue::as_str_lossy), value,
            );
            effects.mark_attribute_change(
                handle,
                &local_name,
                namespace.as_deref(),
                prior_value.clone(),
                Some(new_value.clone()),
                records_enabled,
            );
            if let Some(change) = input_value_change {
                change.record(&mut effects, handle, records_enabled);
            }
            slot_changes.record(self, &mut effects, handle);
            return DomAttributeMutationOutcome::new(effects, prior_value, Some(new_value));
        }
        if prior_value.as_ref() == Some(&new_value) {
            let effects = self.same_value_attribute_effects(
                handle,
                &local_name,
                namespace.as_deref(),
                prior_value.clone(),
                new_value.clone(),
                records_enabled,
            );
            return DomAttributeMutationOutcome::new(effects, prior_value, Some(new_value));
        }
        DomAttributeMutationOutcome::new(
            DomMutationEffects::default(),
            prior_value,
            Some(new_value),
        )
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
            .is_some_and(|element| element.has_attribute_ns("", "multiple"))
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
        self.set_attribute_ns_mutation_outcome_with_utf16_units(
            handle, namespace, prefix, local_name, value, None,
        )
    }

    pub fn set_attribute_ns_utf16_units_mutation_outcome(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        value: &str,
        units: Vec<u16>,
    ) -> DomAttributeMutationOutcome {
        self.set_attribute_ns_mutation_outcome_with_utf16_units(
            handle,
            namespace,
            prefix,
            local_name,
            value,
            Some(units),
        )
    }

    fn set_attribute_ns_mutation_outcome_with_utf16_units(
        &mut self,
        handle: DomHandle,
        namespace: Option<&str>,
        prefix: Option<&str>,
        local_name: &str,
        value: &str,
        units: Option<Vec<u16>>,
    ) -> DomAttributeMutationOutcome {
        let namespace = namespace.filter(|namespace| !namespace.is_empty());
        if namespace.is_none() && local_name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let records_enabled = self.mutation_records_enabled();
        let prior_value = self
            .dom
            .get_attribute_ns_dom_string_value(handle, namespace, local_name)
            .map(Arc::new);
        let slot_changes =
            self.slot_attribute_mutation_snapshots(handle, namespace, local_name, value);
        let input_value_change =
            self.input_type_value_attribute_change(handle, namespace, local_name, Some(value));
        let new_value = Arc::new(
            units
                .as_deref()
                .map_or_else(|| DomStringValue::from(value), DomStringValue::from_utf16),
        );
        let changed = if let Some(units) = units {
            self.dom
                .set_attribute_ns_utf16_units(handle, namespace, prefix, local_name, value, units)
        } else {
            self.dom
                .set_attribute_ns(handle, namespace, prefix, local_name, value)
        };
        if changed {
            self.invalidate_shadow_slot_name_index_for_attribute(handle, namespace, local_name);
            if namespace.is_none_or(str::is_empty) {
                self.sync_select_state_after_option_selected_attribute(handle, local_name);
            }
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
            self.mark_script_attribute_set_trigger(
                &mut effects,
                handle,
                namespace,
                local_name,
                prior_value.as_deref().map(DomStringValue::as_str_lossy),
                value,
            );
            effects.mark_attribute_change(
                handle,
                local_name,
                namespace,
                prior_value.clone(),
                Some(new_value.clone()),
                records_enabled,
            );
            if let Some(change) = input_value_change {
                change.record(&mut effects, handle, records_enabled);
            }
            slot_changes.record(self, &mut effects, handle);
            return DomAttributeMutationOutcome::new(effects, prior_value, Some(new_value));
        }
        if prior_value.as_ref() == Some(&new_value) {
            let effects = self.same_value_attribute_effects(
                handle,
                local_name,
                namespace,
                prior_value.clone(),
                new_value.clone(),
                records_enabled,
            );
            return DomAttributeMutationOutcome::new(effects, prior_value, Some(new_value));
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value, Some(new_value))
    }

    fn same_value_attribute_effects(
        &self,
        handle: DomHandle,
        local_name: &str,
        namespace: Option<&str>,
        prior_value: Option<Arc<DomStringValue>>,
        new_value: Arc<DomStringValue>,
        records_enabled: bool,
    ) -> DomMutationEffects {
        let mut effects = DomMutationEffects::default();
        if records_enabled {
            effects.queue_attribute_mutation_record(
                handle,
                local_name,
                namespace,
                prior_value.clone(),
                Some(new_value.clone()),
            );
        }
        // Observer records apply to every same-value write. Meta processing
        // is an independent semantic effect and must not queue a second record.
        if self.is_html_element_named(handle, "meta")
            && namespace.is_none()
            && matches!(local_name, "content" | "http-equiv")
        {
            effects.mark_attribute_change(
                handle,
                local_name,
                namespace,
                prior_value,
                Some(new_value),
                false,
            );
        }
        effects
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
        let prior_value = self
            .dom
            .get_attribute_dom_string_value(handle, name)
            .map(Arc::new);
        if prior_value.is_some() && name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let slot_changes = self.slot_attribute_mutation_snapshots(handle, None, name, "");
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
            slot_changes.record(self, &mut effects, handle);
            return DomAttributeMutationOutcome::new(effects, prior_value, None);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value, None)
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
            .dom
            .get_attribute_ns_dom_string_value(handle, namespace, local_name)
            .map(Arc::new);
        if prior_value.is_some() && namespace.is_none() && local_name.eq_ignore_ascii_case("form") {
            self.reset_parser_form_owner_for_form_attribute_mutation(handle);
        }
        let slot_changes =
            self.slot_attribute_mutation_snapshots(handle, namespace, local_name, "");
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
            slot_changes.record(self, &mut effects, handle);
            return DomAttributeMutationOutcome::new(effects, prior_value, None);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value, None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeDom;

    #[test]
    fn changed_attribute_effects_use_the_actual_attribute_identity() {
        for (html, setter, local_name, namespace, prefix) in [
            (true, "CONTENT", "content", None, None),
            (false, "CONTENT", "CONTENT", None, None),
            (true, "content", "content", Some("urn:test"), None),
            (true, "p:content", "content", Some("urn:test"), Some("p")),
        ] {
            let url = url::Url::parse("https://example.test/").unwrap();
            let dom = if html {
                NativeDom::new_html(url)
            } else {
                NativeDom::new_xml(url)
            };
            let mut host = DomHost::from_dom(dom);
            let meta = host
                .create_element_ns(Some("http://www.w3.org/1999/xhtml"), "meta")
                .unwrap();
            host.set_attribute_ns(meta, namespace, prefix, local_name, "before");
            host.set_mutation_observer_records_enabled(true);
            let effects = host.set_attribute_effects(meta, setter, "after");
            let mutations = effects.style().attribute_mutations();
            assert_eq!(mutations.len(), 1);
            assert_eq!(mutations[0].local_name(), local_name, "{setter}");
            assert_eq!(mutations[0].namespace(), namespace, "{setter}");
            let records = effects.observer_records().records();
            assert_eq!(records.len(), 1);
            let DomMutationRecordKind::Attributes(record) = records[0].kind() else {
                panic!("attribute record")
            };
            assert_eq!(record.local_name(), local_name);
            assert_eq!(record.namespace(), namespace);
            assert_eq!(record.old_value(), Some("before"));
        }
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").unwrap(),
        ));
        let meta = host.create_element("meta");
        let effects =
            host.set_attribute_ns_effects(meta, Some(""), None, "CONTENT", "CONTENT", "after");
        let mutation = &effects.style().attribute_mutations()[0];
        assert_eq!(mutation.local_name(), "CONTENT");
        assert_eq!(mutation.namespace(), None);
    }

    #[test]
    fn same_value_meta_attribute_sets_keep_semantic_effects_without_observers() {
        for records_enabled in [false, true] {
            for namespace_aware in [false, true] {
                for name in ["content", "http-equiv"] {
                    let mut host = DomHost::from_dom(NativeDom::new_html(
                        url::Url::parse("https://example.test/").unwrap(),
                    ));
                    let meta = host.create_element("meta");
                    host.set_attribute(meta, name, "unchanged");
                    host.set_mutation_observer_records_enabled(records_enabled);
                    let effects = if namespace_aware {
                        host.set_attribute_ns_effects(meta, None, None, name, name, "unchanged")
                    } else {
                        host.set_attribute_effects(meta, name, "unchanged")
                    };
                    assert_eq!(
                        effects.style().attribute_mutations().len(),
                        1,
                        "{name}, namespace-aware={namespace_aware}, observers={records_enabled}"
                    );
                    let mutation = &effects.style().attribute_mutations()[0];
                    assert_eq!(mutation.target(), meta);
                    assert_eq!(mutation.local_name(), name);
                    assert_eq!(mutation.namespace(), None);
                    assert_eq!(mutation.old_value(), Some("unchanged"));
                    assert_eq!(mutation.new_value(), Some("unchanged"));
                    assert_eq!(
                        effects.observer_records().records().len(),
                        usize::from(records_enabled)
                    );
                }
            }
        }
    }

    #[test]
    fn same_value_unrelated_attribute_sets_do_not_notify_meta_processing() {
        for (tag, namespace, name) in [
            ("div", None, "content"),
            ("meta", None, "id"),
            ("meta", Some("urn:test"), "content"),
            ("meta", None, "CONTENT"),
        ] {
            let mut host = DomHost::from_dom(NativeDom::new_html(
                url::Url::parse("https://example.test/").unwrap(),
            ));
            let element = host.create_element(tag);
            host.set_attribute_ns_effects(element, namespace, None, name, name, "unchanged");
            let effects =
                host.set_attribute_ns_effects(element, namespace, None, name, name, "unchanged");
            assert!(
                effects.style().attribute_mutations().is_empty(),
                "{tag}, {namespace:?}, {name}"
            );
            assert!(!effects.did_change());
        }
    }

    #[test]
    fn same_value_plain_attribute_sets_keep_existing_attribute_identity() {
        for records_enabled in [true, false] {
            for (html_document, setter, local_name, namespace, prefix, semantic) in [
                (false, "CONTENT", "CONTENT", None, None, false),
                (false, "HTTP-EQUIV", "HTTP-EQUIV", None, None, false),
                (false, "content", "content", None, None, true),
                (true, "CONTENT", "content", None, None, true),
                (true, "HTTP-EQUIV", "http-equiv", None, None, true),
                (true, "content", "content", Some("urn:test"), None, false),
                (
                    true,
                    "http-equiv",
                    "http-equiv",
                    Some("urn:test"),
                    None,
                    false,
                ),
                (
                    true,
                    "p:content",
                    "content",
                    Some("urn:test"),
                    Some("p"),
                    false,
                ),
            ] {
                let url = url::Url::parse("https://example.test/").unwrap();
                let dom = if html_document {
                    NativeDom::new_html(url)
                } else {
                    NativeDom::new_xml(url)
                };
                let mut host = DomHost::from_dom(dom);
                let meta = host
                    .create_element_ns(Some("http://www.w3.org/1999/xhtml"), "meta")
                    .unwrap();
                host.set_attribute_ns(meta, namespace, prefix, local_name, "unchanged");
                if namespace.is_some() && prefix.is_none() {
                    host.set_attribute_ns(meta, None, None, local_name, "unchanged");
                }
                host.set_mutation_observer_records_enabled(records_enabled);
                let effects = host.set_attribute_effects(meta, setter, "unchanged");
                let records = effects.observer_records().records();
                assert_eq!(records.len(), usize::from(records_enabled));
                if records_enabled {
                    let DomMutationRecordKind::Attributes(mutation) = records[0].kind() else {
                        panic!("expected an attribute record");
                    };
                    assert_eq!(mutation.local_name(), local_name, "{setter}");
                    assert_eq!(mutation.namespace(), namespace, "{setter}");
                    assert_eq!(mutation.old_value(), Some("unchanged"));
                }
                assert_eq!(
                    effects.style().attribute_mutations().len(),
                    usize::from(semantic),
                    "{setter}, namespace={namespace:?}, HTML={html_document}, observers={records_enabled}",
                );
            }
        }
    }

    #[test]
    fn attribute_mutation_snapshots_retain_utf16_after_later_writes() {
        for observe in [false, true] {
            for namespace in [None, Some("urn:values")] {
                let mut host = DomHost::from_dom(NativeDom::new_html(
                    url::Url::parse("https://attribute-values.test/").unwrap(),
                ));
                let element = host.create_element("div");
                host.set_mutation_observer_records_enabled(observe);
                let old_units = [0xd800, 0x41];
                let new_units = [0xd801, 0x41];
                for units in [&old_units, &new_units] {
                    let outcome = if let Some(namespace) = namespace {
                        host.set_attribute_ns_utf16_units_mutation_outcome(
                            element,
                            Some(namespace),
                            Some("v"),
                            "data-value",
                            &String::from_utf16_lossy(units),
                            units.to_vec(),
                        )
                    } else {
                        host.set_attribute_utf16_units_mutation_outcome(
                            element,
                            "data-value",
                            &String::from_utf16_lossy(units),
                            units.to_vec(),
                        )
                    };
                    if units == &old_units {
                        assert_eq!(outcome.old_value(), None);
                        continue;
                    }
                    let (effects, old_value, new_value) = outcome.into_parts();
                    let old_value = old_value.unwrap();
                    let new_value = new_value.unwrap();
                    assert_eq!(&*old_value.utf16_units(), &old_units);
                    assert_eq!(&*new_value.utf16_units(), &new_units);
                    let style = &effects.style().attribute_mutations()[0];
                    assert!(Arc::ptr_eq(&old_value, &style.shared_old_value().unwrap()));
                    assert!(Arc::ptr_eq(&new_value, &style.shared_new_value().unwrap()));
                    assert_eq!(
                        effects.observer_records().records().len(),
                        usize::from(observe)
                    );
                    if observe {
                        let DomMutationRecordKind::Attributes(record) =
                            effects.observer_records().records()[0].kind()
                        else {
                            panic!("expected attribute record");
                        };
                        assert!(Arc::ptr_eq(&old_value, &record.shared_old_value().unwrap()));
                        assert!(Arc::ptr_eq(&new_value, &record.shared_new_value().unwrap()));
                    }
                    let removal = if let Some(namespace) = namespace {
                        host.remove_attribute_ns_mutation_outcome(
                            element,
                            Some(namespace),
                            "data-value",
                        )
                    } else {
                        host.remove_attribute_mutation_outcome(element, "data-value")
                    };
                    let (_, removed_value, absent_value) = removal.into_parts();
                    assert_eq!(&*removed_value.unwrap().utf16_units(), &new_units);
                    assert!(absent_value.is_none());
                    host.set_attribute(element, "data-value", "later");
                    assert_eq!(&*old_value.utf16_units(), &old_units);
                    assert_eq!(&*new_value.utf16_units(), &new_units);
                }
            }
        }
    }

    #[test]
    fn unchanged_attribute_records_keep_the_original_utf16_snapshot() {
        for observe in [false, true] {
            let mut host = DomHost::from_dom(NativeDom::new_html(
                url::Url::parse("https://attribute-values.test/").unwrap(),
            ));
            let element = host.create_element("div");
            let units = vec![0xdc00];
            host.set_attribute_utf16_units(element, "data-value", "�", units.clone());
            host.set_mutation_observer_records_enabled(observe);
            let (effects, old_value, new_value) = host
                .set_attribute_utf16_units_mutation_outcome(
                    element,
                    "data-value",
                    "�",
                    units.clone(),
                )
                .into_parts();
            assert!(effects.style().attribute_mutations().is_empty());
            assert_eq!(
                effects.observer_records().records().len(),
                usize::from(observe)
            );
            assert_eq!(&*old_value.unwrap().utf16_units(), &units);
            assert_eq!(&*new_value.unwrap().utf16_units(), &units);
        }
    }
}
