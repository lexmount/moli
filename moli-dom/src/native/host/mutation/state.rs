use super::*;
use crate::native::{CustomElementState, SelectedFile};

impl DomHost {
    pub fn set_element_prefix(&mut self, handle: DomHandle, prefix: Option<String>) -> bool {
        let (previous_qualified_name, current_qualified_name) = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            let previous_qualified_name = element.qualified_name();
            if !element.set_prefix(prefix) {
                return false;
            }
            (previous_qualified_name, element.qualified_name())
        };
        self.rekey_qualified_name_query_index_candidate(
            handle,
            &previous_qualified_name,
            &current_qualified_name,
        );
        self.record_mutation(MutationScope::QueryState);
        true
    }

    pub fn set_custom_element_is_name(
        &mut self,
        handle: DomHandle,
        is_name: Option<String>,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_custom_element_is_name(is_name)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn set_custom_element_state(
        &mut self,
        handle: DomHandle,
        state: CustomElementState,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_custom_element_state(state)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn custom_state_names(&self, handle: DomHandle) -> Vec<String> {
        self.node(handle)
            .and_then(Node::as_element)
            .map(|element| element.custom_states().iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn has_custom_state(&self, handle: DomHandle, state: &str) -> bool {
        self.node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_custom_state(state))
    }

    pub fn insert_custom_state(&mut self, handle: DomHandle, state: &str) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.insert_custom_state(state)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn remove_custom_state(&mut self, handle: DomHandle, state: &str) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.remove_custom_state(state)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn clear_custom_states(&mut self, handle: DomHandle) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.clear_custom_states()
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn set_input_value(&mut self, handle: DomHandle, value: &str) -> bool {
        let has_connected_datalist =
            self.is_connected(handle) && self.input_datalist_handle(handle).is_some();
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            let mut did_change =
                element.prepare_datalist_text_decoration_for_value_mutation(has_connected_datalist);
            did_change |= element.set_input_value(value);
            did_change
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_input_value_with_dirty(
        &mut self,
        handle: DomHandle,
        value: &str,
        dirty: bool,
    ) -> bool {
        let has_connected_datalist =
            self.is_connected(handle) && self.input_datalist_handle(handle).is_some();
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            let mut did_change =
                element.prepare_datalist_text_decoration_for_value_mutation(has_connected_datalist);
            did_change |= element.set_input_value_with_dirty(value, dirty);
            did_change
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_input_value_from_user_edit(&mut self, handle: DomHandle, value: &str) -> bool {
        let has_connected_datalist =
            self.is_connected(handle) && self.input_datalist_handle(handle).is_some();
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            let mut did_change =
                element.prepare_datalist_text_decoration_for_value_mutation(has_connected_datalist);
            did_change |= element.set_input_value_from_user_edit(value);
            did_change
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_autofilled_state(&mut self, handle: DomHandle, autofilled: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_autofilled(autofilled)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn set_input_files(&mut self, handle: DomHandle, files: Vec<SelectedFile>) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selected_files(files)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_selection_range(&mut self, handle: DomHandle, start: u32, end: u32) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selection_range(start, end)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_selection_range_with_direction(
        &mut self,
        handle: DomHandle,
        start: u32,
        end: u32,
        direction: &str,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selection_range_with_direction(start, end, direction)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_selection_start(&mut self, handle: DomHandle, start: u32) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selection_start(start)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_selection_end(&mut self, handle: DomHandle, end: u32) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selection_end(end)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_selection_direction(&mut self, handle: DomHandle, direction: &str) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selection_direction(direction)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_custom_validation_message(&mut self, handle: DomHandle, message: &str) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_custom_validation_message(message)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_popover_open(&mut self, handle: DomHandle, open: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_popover_open(open)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_dialog_modal(&mut self, handle: DomHandle, modal: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_dialog_modal(modal)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_dialog_return_value(&mut self, handle: DomHandle, value: &str) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_dialog_return_value(value)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_paused(&mut self, handle: DomHandle, paused: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_paused(paused)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_volume(&mut self, handle: DomHandle, volume: f64) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_volume(volume)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_muted(&mut self, handle: DomHandle, muted: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_muted(muted)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_seeking(&mut self, handle: DomHandle, seeking: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_seeking(seeking)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn media_seek_token(&self, handle: DomHandle) -> Option<u64> {
        self.node(handle)
            .and_then(Node::as_element)
            .filter(|element| element.is_html_media())
            .map(Element::media_seek_token)
    }

    pub fn advance_media_seek_token(&mut self, handle: DomHandle) -> Option<u64> {
        self.node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
            .and_then(Element::advance_media_seek_token)
    }

    pub fn set_media_playback_rate(&mut self, handle: DomHandle, rate: f64) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_playback_rate(rate)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_current_time(&mut self, handle: DomHandle, current_time: f64) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_current_time(current_time)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_ready_state(&mut self, handle: DomHandle, ready_state: u32) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_ready_state(ready_state)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_network_state(&mut self, handle: DomHandle, network_state: u32) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_network_state(network_state)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_media_error_code(&mut self, handle: DomHandle, error_code: Option<u32>) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_media_error_code(error_code)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_checked_state(&mut self, handle: DomHandle, checked: bool) -> bool {
        !self
            .set_checked_state_changed_handles(handle, checked)
            .is_empty()
    }

    pub fn set_checked_state_changed_handles(
        &mut self,
        handle: DomHandle,
        checked: bool,
    ) -> Vec<DomHandle> {
        self.set_checked_state_internal(handle, checked, None)
    }

    pub fn set_checked_state_with_dirty(
        &mut self,
        handle: DomHandle,
        checked: bool,
        dirty: bool,
    ) -> bool {
        !self
            .set_checked_state_with_dirty_changed_handles(handle, checked, dirty)
            .is_empty()
    }

    pub fn set_checked_state_with_dirty_changed_handles(
        &mut self,
        handle: DomHandle,
        checked: bool,
        dirty: bool,
    ) -> Vec<DomHandle> {
        self.set_checked_state_internal(handle, checked, Some(dirty))
    }

    fn set_checked_state_internal(
        &mut self,
        handle: DomHandle,
        checked: bool,
        dirty: Option<bool>,
    ) -> Vec<DomHandle> {
        let mut changed_handles = self.uncheck_checked_radio_peers(handle, checked, dirty);

        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return changed_handles;
            };
            match dirty {
                Some(dirty) => element.set_checked_with_dirty(checked, dirty),
                None => element.set_checked(checked),
            }
        };
        if did_change {
            changed_handles.push(handle);
        }
        if !changed_handles.is_empty() {
            self.record_mutation(MutationScope::LocalState);
        }
        changed_handles
    }

    pub fn checked_state_change_candidate_handles(
        &self,
        handle: DomHandle,
        checked: bool,
    ) -> Vec<DomHandle> {
        if self.node(handle).and_then(Node::as_element).is_none() {
            return Vec::new();
        }
        let mut handles = Vec::from([handle]);
        handles.extend(self.checked_radio_peer_handles(handle, checked));
        handles
    }

    fn checked_radio_peer_handles(&self, handle: DomHandle, checked: bool) -> Vec<DomHandle> {
        if !checked {
            return Vec::new();
        }
        self.radio_group_members(handle)
            .into_iter()
            .filter(|radio_handle| *radio_handle != handle)
            .filter(|radio_handle| {
                self.node(*radio_handle)
                    .and_then(Node::as_element)
                    .is_some_and(Element::checked)
            })
            .collect()
    }

    fn uncheck_checked_radio_peers(
        &mut self,
        handle: DomHandle,
        checked: bool,
        dirty: Option<bool>,
    ) -> Vec<DomHandle> {
        let mut changed_handles = Vec::new();
        for radio_handle in self.checked_radio_peer_handles(handle, checked) {
            let changed = self
                .node_mut(radio_handle)
                .and_then(|node| node.data_mut().as_element_mut())
                .is_some_and(|element| match dirty {
                    Some(dirty) => element.set_checked_with_dirty(false, dirty),
                    None => element.set_checked(false),
                });
            if changed {
                changed_handles.push(radio_handle);
            }
        }
        changed_handles
    }

    pub(super) fn checked_radio_form_owner_snapshots_in_subtrees(
        &self,
        roots: &[DomHandle],
    ) -> Vec<(DomHandle, Option<DomHandle>)> {
        roots
            .iter()
            .flat_map(|root| {
                self.collect_matching_elements(*root, true, |candidate| {
                    self.node(candidate)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_input()
                                && element.input_type() == "radio"
                                && element.checked()
                        })
                })
            })
            .map(|radio| (radio, self.form_control_owner(radio)))
            .collect()
    }

    pub(super) fn normalize_checked_radio_groups_after_form_owner_changes(
        &mut self,
        snapshots: &[(DomHandle, Option<DomHandle>)],
    ) {
        let mut changed_handles = Vec::new();
        for &(radio, previous_form_owner) in snapshots.iter().rev() {
            let is_still_checked_radio =
                self.node(radio)
                    .and_then(Node::as_element)
                    .is_some_and(|element| {
                        element.is_html_input()
                            && element.input_type() == "radio"
                            && element.checked()
                    });
            if !is_still_checked_radio || self.form_control_owner(radio) == previous_form_owner {
                continue;
            }
            changed_handles.extend(self.uncheck_checked_radio_peers(radio, true, None));
        }
        if !changed_handles.is_empty() {
            self.record_mutation(MutationScope::LocalState);
        }
    }

    pub fn set_selected_state(&mut self, handle: DomHandle, selected: bool) -> bool {
        self.set_selected_state_with_dirty(handle, selected, true)
    }

    pub fn set_selected_state_with_dirty(
        &mut self,
        handle: DomHandle,
        selected: bool,
        dirty: bool,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_selected_with_dirty(selected, dirty)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn set_select_explicit_none_state(
        &mut self,
        handle: DomHandle,
        explicit_none: bool,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_select_explicit_none(explicit_none)
        };
        if did_change {
            self.record_mutation(MutationScope::QueryState);
        }
        did_change
    }

    pub fn set_output_default_value_state(
        &mut self,
        handle: DomHandle,
        value: Option<String>,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_output_default_value(value)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_indeterminate_state(&mut self, handle: DomHandle, indeterminate: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_indeterminate(indeterminate)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_script_force_async(&mut self, handle: DomHandle, force_async: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_script_force_async(force_async)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_script_already_started(&mut self, handle: DomHandle, already_started: bool) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_script_already_started(already_started)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_script_parser_inserted_for_prepare(
        &mut self,
        handle: DomHandle,
        parser_inserted: bool,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_script_parser_inserted_for_prepare(parser_inserted)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_script_text_internal_slot(&mut self, handle: DomHandle, source: &str) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_script_text_internal_slot(source)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn note_script_children_changed_by_api(&mut self, handle: DomHandle) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.note_script_children_changed_by_api()
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn finish_parsing_script_children(&mut self, handle: DomHandle) -> bool {
        let Some(source) = self.dom.direct_text_content(handle) else {
            return false;
        };
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.finish_parsing_script_children(&source)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn finish_parsing_link_children(&mut self, handle: DomHandle) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.finish_parsing_link_children()
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_cryptographic_nonce(&mut self, handle: DomHandle, nonce: Option<String>) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_cryptographic_nonce(nonce)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_link_explicitly_enabled(
        &mut self,
        handle: DomHandle,
        explicitly_enabled: bool,
    ) -> bool {
        let did_change = {
            let Some(element) = self
                .node_mut(handle)
                .and_then(|node| node.data_mut().as_element_mut())
            else {
                return false;
            };
            element.set_link_explicitly_enabled(explicitly_enabled)
        };
        if did_change {
            self.record_mutation(MutationScope::LocalState);
        }
        did_change
    }

    pub fn set_subtree_script_already_started(&mut self, root: DomHandle, already_started: bool) {
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            let _ = self.set_script_already_started(handle, already_started);
            stack.extend(self.child_handles_reversed(handle));
        }
    }

    pub fn set_select_value(&mut self, handle: DomHandle, value: &str) -> bool {
        if !self.is_html_element_named(handle, "select") {
            return false;
        }

        let options = self.select_option_elements(handle);
        let mut matched = false;
        let mut did_change = false;
        for option in options {
            let should_select = !matched && self.option_value(option).as_deref() == Some(value);
            did_change |= self.set_selected_state(option, should_select);
            if should_select {
                matched = true;
            }
        }

        did_change |= self.set_select_explicit_none_state(handle, !matched);

        did_change
    }
}
