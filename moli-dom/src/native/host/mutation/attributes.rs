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
        let records_enabled = self.mutation_records_enabled();
        let Some(normalized_name) = self.dom.normalized_attribute_name(handle, name) else {
            return DomAttributeMutationOutcome::new(DomMutationEffects::default(), None);
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
                Some(Arc::from(attribute.value())),
            ),
            None => (normalized_name, None, None),
        };
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
        let changed = self.dom.set_attribute(handle, name, value);
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
                &mut effects,
                handle,
                namespace.as_deref(),
                &local_name,
            );
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
                &local_name,
                namespace.as_deref(),
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
        if prior_value.as_deref() == Some(value) {
            let name = local_name.as_str();
            let namespace = namespace.as_deref();
            let mut effects = DomMutationEffects::default();
            if self.is_html_element_named(handle, "meta")
                && namespace.is_none()
                && matches!(name, "content" | "http-equiv")
            {
                // Processing a meta declaration is observable even when its
                // serialized attribute value has not changed.
                effects.mark_attribute_change(
                    handle,
                    name,
                    namespace,
                    prior_value.clone(),
                    Some(value),
                    records_enabled,
                );
            } else if records_enabled {
                effects.queue_attribute_mutation_record(
                    handle,
                    name,
                    namespace,
                    prior_value.clone(),
                    Some(value),
                );
            }
            return DomAttributeMutationOutcome::new(effects, prior_value);
        }
        DomAttributeMutationOutcome::new(DomMutationEffects::default(), prior_value)
    }

    fn sync_select_state_after_option_selected_attribute(&mut self, handle: DomHandle, name: &str) {
        if !name.eq_ignore_ascii_case("selected") || !self.is_html_element_named(handle, "option") {
            return;
        }
        let Some(select) = self.owner_select_for_option(handle) else {
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
        let namespace = namespace.filter(|namespace| !namespace.is_empty());
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
        if prior_value.as_deref() == Some(value)
            && namespace.is_none_or(str::is_empty)
            && self.is_html_element_named(handle, "meta")
            && matches!(local_name, "content" | "http-equiv")
        {
            let mut effects = DomMutationEffects::default();
            effects.mark_attribute_change(
                handle,
                local_name,
                None,
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
}
