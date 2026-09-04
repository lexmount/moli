use super::Attribute;
use crate::forms::{input_type_has_value_sanitization, sanitize_input_value_for_type};
use indexmap::IndexSet;

#[derive(Debug, Clone, PartialEq)]
pub struct SelectedFile {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub name: String,
    pub last_modified: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionDirection {
    None,
    Forward,
    Backward,
}

impl SelectionDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptElementState {
    force_async: bool,
    already_started: bool,
    parser_inserted_for_prepare: bool,
    script_text_internal_slot: String,
    children_changed_by_api: bool,
}

impl ScriptElementState {
    pub fn new_script_element() -> Self {
        Self {
            force_async: true,
            already_started: false,
            parser_inserted_for_prepare: false,
            script_text_internal_slot: String::new(),
            children_changed_by_api: false,
        }
    }

    pub fn force_async(&self) -> bool {
        self.force_async
    }

    pub fn already_started(&self) -> bool {
        self.already_started
    }

    pub fn parser_inserted_for_prepare(&self) -> bool {
        self.parser_inserted_for_prepare
    }

    pub fn script_text_internal_slot(&self) -> &str {
        &self.script_text_internal_slot
    }

    pub fn children_changed_by_api(&self) -> bool {
        self.children_changed_by_api
    }

    pub fn set_force_async(&mut self, force_async: bool) -> bool {
        if self.force_async == force_async {
            return false;
        }
        self.force_async = force_async;
        true
    }

    pub fn set_already_started(&mut self, already_started: bool) -> bool {
        if self.already_started == already_started {
            return false;
        }
        self.already_started = already_started;
        true
    }

    pub fn set_parser_inserted_for_prepare(&mut self, parser_inserted: bool) -> bool {
        if self.parser_inserted_for_prepare == parser_inserted {
            return false;
        }
        self.parser_inserted_for_prepare = parser_inserted;
        true
    }

    pub fn set_script_text_internal_slot(&mut self, source: &str) -> bool {
        if self.script_text_internal_slot == source {
            return false;
        }
        self.script_text_internal_slot.clear();
        self.script_text_internal_slot.push_str(source);
        true
    }

    pub fn note_children_changed_by_api(&mut self) -> bool {
        if self.children_changed_by_api {
            return false;
        }
        self.children_changed_by_api = true;
        true
    }

    pub fn finish_parsing_children(&mut self, source: &str) -> bool {
        if self.children_changed_by_api {
            return false;
        }
        self.set_script_text_internal_slot(source)
    }

    pub fn note_parser_created(&mut self) -> bool {
        let mut changed = self.set_parser_inserted_for_prepare(true);
        changed |= self.set_force_async(false);
        changed
    }
}

#[derive(Debug, Clone, Default)]
pub struct ElementControlState {
    input_value: Option<String>,
    input_value_dirty: bool,
    input_value_user_edited: bool,
    input_bad_input: bool,
    datalist_text_decoration_initial_value_dirty: Option<bool>,
    autofilled: bool,
    selected_files: Vec<SelectedFile>,
    checked: Option<bool>,
    checked_dirty: bool,
    selected: Option<bool>,
    selected_dirty: bool,
    output_default_value: Option<String>,
    select_explicit_none: bool,
    indeterminate: bool,
    script: Option<Box<ScriptElementState>>,
    link_created_by_parser: bool,
    link_explicitly_enabled: bool,
    selection_start: Option<u32>,
    selection_end: Option<u32>,
    selection_direction: Option<SelectionDirection>,
    media_paused: Option<bool>,
    media_volume: Option<f64>,
    media_muted: Option<bool>,
    media_seeking: Option<bool>,
    media_seek_token: u64,
    media_playback_rate: Option<f64>,
    media_current_time: Option<f64>,
    media_ready_state: Option<u32>,
    media_network_state: Option<u32>,
    media_error_code: Option<u32>,
    image_load_dispatched: bool,
    cryptographic_nonce: Option<String>,
    scroll_top: Option<f64>,
    scroll_left: Option<f64>,
    custom_validation_message: String,
    popover_open: bool,
    dialog_modal: bool,
    dialog_return_value: String,
    custom_states: IndexSet<String>,
}

impl ElementControlState {
    pub fn from_element_parts(
        namespace: &str,
        local_name: &str,
        attributes: &[Attribute],
    ) -> Option<Self> {
        let is_script = local_name == "script"
            && matches!(
                namespace,
                "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
            );
        let is_html_control = namespace == "http://www.w3.org/1999/xhtml"
            && matches!(
                local_name,
                "input" | "textarea" | "option" | "audio" | "video"
            );
        let nonce = attributes
            .iter()
            .find(|attribute| attribute.local_name() == "nonce")
            .map(Attribute::value);
        if !is_script && !is_html_control && nonce.is_none() {
            return None;
        }

        let mut state = Self::default();
        if is_script {
            state.script = Some(Box::new(ScriptElementState::new_script_element()));
        }
        state.cryptographic_nonce = nonce.map(str::to_owned);
        if namespace != "http://www.w3.org/1999/xhtml" {
            return Some(state);
        }

        let attribute = |name: &str| {
            attributes
                .iter()
                .find(|attribute| attribute.local_name() == name)
                .map(Attribute::value)
        };

        if local_name == "input" {
            let input_type = attribute("type").unwrap_or("text");
            state.input_value = Some(sanitize_input_value_for_type(
                input_type,
                attribute("value").unwrap_or_default(),
            ));
            state.checked = Some(attribute("checked").is_some());
            state.selection_start = Some(0);
            state.selection_end = Some(0);
            state.selection_direction = Some(SelectionDirection::None);
        } else if local_name == "textarea" {
            state.selection_start = Some(0);
            state.selection_end = Some(0);
            state.selection_direction = Some(SelectionDirection::None);
        } else if local_name == "option" {
            state.selected = Some(attribute("selected").is_some());
        } else if local_name == "audio" || local_name == "video" {
            state.media_paused = Some(true);
            state.media_volume = Some(1.0);
            state.media_muted = Some(false);
            state.media_seeking = Some(false);
            state.media_playback_rate = Some(1.0);
            state.media_current_time = Some(0.0);
            state.media_ready_state = Some(0);
            state.media_network_state = Some(0);
            state.media_error_code = None;
        }

        Some(state)
    }

    pub fn input_value(&self) -> Option<&str> {
        self.input_value.as_deref()
    }

    pub fn input_value_dirty(&self) -> bool {
        self.input_value_dirty
    }

    pub fn input_value_user_edited(&self) -> bool {
        self.input_value_user_edited
    }

    pub fn input_bad_input(&self) -> bool {
        self.input_bad_input
    }

    pub fn datalist_text_decoration_initial_value_dirty(&self) -> Option<bool> {
        self.datalist_text_decoration_initial_value_dirty
    }

    pub fn autofilled(&self) -> bool {
        self.autofilled
    }

    pub fn checked(&self) -> Option<bool> {
        self.checked
    }

    pub fn selected_files(&self) -> &[SelectedFile] {
        &self.selected_files
    }

    pub fn checked_dirty(&self) -> bool {
        self.checked_dirty
    }

    pub fn selected(&self) -> Option<bool> {
        self.selected
    }

    pub fn output_default_value(&self) -> Option<&str> {
        self.output_default_value.as_deref()
    }

    pub fn indeterminate(&self) -> bool {
        self.indeterminate
    }

    pub fn select_explicit_none(&self) -> bool {
        self.select_explicit_none
    }

    pub fn script_force_async(&self) -> bool {
        self.script
            .as_deref()
            .is_some_and(ScriptElementState::force_async)
    }

    pub fn script_already_started(&self) -> bool {
        self.script
            .as_deref()
            .is_some_and(ScriptElementState::already_started)
    }

    pub fn script_parser_inserted_for_prepare(&self) -> bool {
        self.script
            .as_deref()
            .is_some_and(ScriptElementState::parser_inserted_for_prepare)
    }

    pub fn script_text_internal_slot(&self) -> &str {
        self.script
            .as_deref()
            .map(ScriptElementState::script_text_internal_slot)
            .unwrap_or_default()
    }

    pub fn script_children_changed_by_api(&self) -> bool {
        self.script
            .as_deref()
            .is_some_and(ScriptElementState::children_changed_by_api)
    }

    pub fn link_explicitly_enabled(&self) -> bool {
        self.link_explicitly_enabled
    }

    pub fn link_created_by_parser(&self) -> bool {
        self.link_created_by_parser
    }

    pub fn selection_start(&self) -> Option<u32> {
        self.selection_start
    }

    pub fn selection_end(&self) -> Option<u32> {
        self.selection_end
    }

    pub fn selection_direction(&self) -> Option<&str> {
        self.selection_direction.map(SelectionDirection::as_str)
    }

    pub fn media_paused(&self) -> Option<bool> {
        self.media_paused
    }

    pub fn media_volume(&self) -> Option<f64> {
        self.media_volume
    }

    pub fn media_muted(&self) -> Option<bool> {
        self.media_muted
    }

    pub fn media_seeking(&self) -> Option<bool> {
        self.media_seeking
    }

    pub fn media_seek_token(&self) -> u64 {
        self.media_seek_token
    }

    pub fn media_playback_rate(&self) -> Option<f64> {
        self.media_playback_rate
    }

    pub fn media_current_time(&self) -> Option<f64> {
        self.media_current_time
    }

    pub fn media_ready_state(&self) -> Option<u32> {
        self.media_ready_state
    }

    pub fn media_network_state(&self) -> Option<u32> {
        self.media_network_state
    }

    pub fn media_error_code(&self) -> Option<u32> {
        self.media_error_code
    }

    pub fn scroll_top(&self) -> Option<f64> {
        self.scroll_top
    }

    pub fn image_load_dispatched(&self) -> bool {
        self.image_load_dispatched
    }

    pub fn cryptographic_nonce(&self) -> Option<&str> {
        self.cryptographic_nonce.as_deref()
    }

    pub fn scroll_left(&self) -> Option<f64> {
        self.scroll_left
    }

    pub fn custom_validation_message(&self) -> &str {
        &self.custom_validation_message
    }

    pub fn popover_open(&self) -> bool {
        self.popover_open
    }

    pub fn dialog_modal(&self) -> bool {
        self.dialog_modal
    }

    pub fn dialog_return_value(&self) -> &str {
        &self.dialog_return_value
    }

    pub fn custom_states(&self) -> &IndexSet<String> {
        &self.custom_states
    }

    pub fn has_custom_state(&self, state: &str) -> bool {
        self.custom_states.contains(state)
    }

    pub fn set_input_value(&mut self, value: &str) -> bool {
        self.set_input_value_with_state(value, true, false, false)
    }

    pub fn set_input_value_with_dirty(&mut self, value: &str, dirty: bool) -> bool {
        self.set_input_value_with_state(value, dirty, false, false)
    }

    pub fn set_input_value_from_user_edit(&mut self, value: &str, bad_input: bool) -> bool {
        self.set_input_value_with_state(value, true, true, bad_input)
    }

    pub fn prepare_datalist_text_decoration_for_value_mutation(
        &mut self,
        has_connected_datalist: bool,
    ) -> bool {
        let next = if has_connected_datalist {
            Some(
                self.datalist_text_decoration_initial_value_dirty
                    .unwrap_or(self.input_value_dirty),
            )
        } else {
            None
        };
        if self.datalist_text_decoration_initial_value_dirty == next {
            return false;
        }
        self.datalist_text_decoration_initial_value_dirty = next;
        true
    }

    pub fn set_autofilled(&mut self, autofilled: bool) -> bool {
        if self.autofilled == autofilled {
            return false;
        }
        self.autofilled = autofilled;
        true
    }

    fn set_input_value_with_state(
        &mut self,
        value: &str,
        dirty: bool,
        user_edited: bool,
        bad_input: bool,
    ) -> bool {
        if self.input_value.as_deref() == Some(value)
            && self.input_value_dirty == dirty
            && self.input_value_user_edited == user_edited
            && self.input_bad_input == bad_input
        {
            return false;
        }
        self.input_value = Some(value.to_owned());
        self.input_value_dirty = dirty;
        self.input_value_user_edited = user_edited;
        self.input_bad_input = bad_input;
        true
    }

    pub fn set_selected_files(&mut self, files: Vec<SelectedFile>) -> bool {
        if self.selected_files == files {
            return false;
        }
        self.selected_files = files;
        true
    }

    pub fn set_checked(&mut self, checked: bool) -> bool {
        self.set_checked_with_dirty(checked, true)
    }

    pub fn set_checked_with_dirty(&mut self, checked: bool, dirty: bool) -> bool {
        if self.checked == Some(checked) && self.checked_dirty == dirty {
            return false;
        }
        self.checked = Some(checked);
        self.checked_dirty = dirty;
        true
    }

    pub fn set_selected(&mut self, selected: bool) -> bool {
        self.set_selected_with_dirty(selected, true)
    }

    pub fn selected_dirty(&self) -> bool {
        self.selected_dirty
    }

    pub fn set_selected_with_dirty(&mut self, selected: bool, dirty: bool) -> bool {
        if self.selected == Some(selected) && self.selected_dirty == dirty {
            return false;
        }
        self.selected = Some(selected);
        self.selected_dirty = dirty;
        true
    }

    pub fn set_output_default_value(&mut self, value: Option<String>) -> bool {
        if self.output_default_value == value {
            return false;
        }
        self.output_default_value = value;
        true
    }

    pub fn set_select_explicit_none(&mut self, explicit_none: bool) -> bool {
        if self.select_explicit_none == explicit_none {
            return false;
        }
        self.select_explicit_none = explicit_none;
        true
    }

    pub fn set_indeterminate(&mut self, indeterminate: bool) -> bool {
        if self.indeterminate == indeterminate {
            return false;
        }
        self.indeterminate = indeterminate;
        true
    }

    pub fn set_script_force_async(&mut self, force_async: bool) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(|script| script.set_force_async(force_async))
    }

    pub fn set_script_already_started(&mut self, already_started: bool) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(|script| script.set_already_started(already_started))
    }

    pub fn set_script_parser_inserted_for_prepare(&mut self, parser_inserted: bool) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(|script| script.set_parser_inserted_for_prepare(parser_inserted))
    }

    pub fn set_script_text_internal_slot(&mut self, source: &str) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(|script| script.set_script_text_internal_slot(source))
    }

    pub fn note_script_children_changed_by_api(&mut self) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(ScriptElementState::note_children_changed_by_api)
    }

    pub fn finish_parsing_script_children(&mut self, source: &str) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(|script| script.finish_parsing_children(source))
    }

    pub fn note_parser_created_script(&mut self) -> bool {
        self.script
            .as_deref_mut()
            .is_some_and(ScriptElementState::note_parser_created)
    }

    pub fn note_parser_created_link(&mut self) -> bool {
        if self.link_created_by_parser {
            return false;
        }
        self.link_created_by_parser = true;
        true
    }

    pub fn finish_parsing_link_children(&mut self) -> bool {
        if !self.link_created_by_parser {
            return false;
        }
        self.link_created_by_parser = false;
        true
    }

    pub fn set_link_explicitly_enabled(&mut self, explicitly_enabled: bool) -> bool {
        if self.link_explicitly_enabled == explicitly_enabled {
            return false;
        }
        self.link_explicitly_enabled = explicitly_enabled;
        true
    }

    pub fn set_selection_range(&mut self, start: u32, end: u32) -> bool {
        self.set_selection_range_with_direction(start, end, "none")
    }

    pub fn set_selection_range_with_direction(
        &mut self,
        start: u32,
        end: u32,
        direction: &str,
    ) -> bool {
        let direction = normalize_selection_direction(direction);
        if self.selection_start == Some(start)
            && self.selection_end == Some(end)
            && self.selection_direction == Some(direction)
        {
            return false;
        }
        self.selection_start = Some(start);
        self.selection_end = Some(end);
        self.selection_direction = Some(direction);
        true
    }

    pub fn set_selection_start(&mut self, start: u32) -> bool {
        if self.selection_start == Some(start) {
            return false;
        }
        self.selection_start = Some(start);
        if self.selection_end.is_none_or(|end| end < start) {
            self.selection_end = Some(start);
        }
        true
    }

    pub fn set_selection_direction(&mut self, direction: &str) -> bool {
        let direction = normalize_selection_direction(direction);
        if self.selection_direction == Some(direction) {
            return false;
        }
        self.selection_direction = Some(direction);
        true
    }

    pub fn set_selection_end(&mut self, end: u32) -> bool {
        if self.selection_end == Some(end) {
            return false;
        }
        self.selection_end = Some(end);
        if self.selection_start.is_none_or(|start| start > end) {
            self.selection_start = Some(end);
        }
        true
    }

    pub fn set_media_paused(&mut self, paused: bool) -> bool {
        if self.media_paused == Some(paused) {
            return false;
        }
        self.media_paused = Some(paused);
        true
    }

    pub fn set_media_volume(&mut self, volume: f64) -> bool {
        if self.media_volume == Some(volume) {
            return false;
        }
        self.media_volume = Some(volume);
        true
    }

    pub fn set_media_muted(&mut self, muted: bool) -> bool {
        if self.media_muted == Some(muted) {
            return false;
        }
        self.media_muted = Some(muted);
        true
    }

    pub fn set_media_seeking(&mut self, seeking: bool) -> bool {
        if self.media_seeking == Some(seeking) {
            return false;
        }
        self.media_seeking = Some(seeking);
        true
    }

    pub fn advance_media_seek_token(&mut self) -> u64 {
        self.media_seek_token = self.media_seek_token.wrapping_add(1).max(1);
        self.media_seek_token
    }

    pub fn set_media_playback_rate(&mut self, rate: f64) -> bool {
        if self.media_playback_rate == Some(rate) {
            return false;
        }
        self.media_playback_rate = Some(rate);
        true
    }

    pub fn set_media_current_time(&mut self, current_time: f64) -> bool {
        if self.media_current_time == Some(current_time) {
            return false;
        }
        self.media_current_time = Some(current_time);
        true
    }

    pub fn set_media_ready_state(&mut self, ready_state: u32) -> bool {
        if self.media_ready_state == Some(ready_state) {
            return false;
        }
        self.media_ready_state = Some(ready_state);
        true
    }

    pub fn set_media_network_state(&mut self, network_state: u32) -> bool {
        if self.media_network_state == Some(network_state) {
            return false;
        }
        self.media_network_state = Some(network_state);
        true
    }

    pub fn set_media_error_code(&mut self, error_code: Option<u32>) -> bool {
        if self.media_error_code == error_code {
            return false;
        }
        self.media_error_code = error_code;
        true
    }

    pub fn set_image_load_dispatched(&mut self, dispatched: bool) -> bool {
        if self.image_load_dispatched == dispatched {
            return false;
        }
        self.image_load_dispatched = dispatched;
        true
    }

    pub fn set_cryptographic_nonce(&mut self, nonce: Option<String>) -> bool {
        if self.cryptographic_nonce == nonce {
            return false;
        }
        self.cryptographic_nonce = nonce;
        true
    }

    pub fn set_scroll_top(&mut self, scroll_top: f64) -> bool {
        if self.scroll_top == Some(scroll_top) {
            return false;
        }
        self.scroll_top = Some(scroll_top);
        true
    }

    pub fn set_scroll_left(&mut self, scroll_left: f64) -> bool {
        if self.scroll_left == Some(scroll_left) {
            return false;
        }
        self.scroll_left = Some(scroll_left);
        true
    }

    pub fn set_custom_validation_message(&mut self, message: &str) -> bool {
        if self.custom_validation_message == message {
            return false;
        }
        self.custom_validation_message = message.to_owned();
        true
    }

    pub fn set_popover_open(&mut self, open: bool) -> bool {
        if self.popover_open == open {
            return false;
        }
        self.popover_open = open;
        true
    }

    pub fn set_dialog_modal(&mut self, modal: bool) -> bool {
        if self.dialog_modal == modal {
            return false;
        }
        self.dialog_modal = modal;
        true
    }

    pub fn set_dialog_return_value(&mut self, value: &str) -> bool {
        if self.dialog_return_value == value {
            return false;
        }
        value.clone_into(&mut self.dialog_return_value);
        true
    }

    pub fn insert_custom_state(&mut self, state: &str) -> bool {
        self.custom_states.insert(state.to_owned())
    }

    pub fn remove_custom_state(&mut self, state: &str) -> bool {
        self.custom_states.shift_remove(state)
    }

    pub fn clear_custom_states(&mut self) -> bool {
        if self.custom_states.is_empty() {
            return false;
        }
        self.custom_states.clear();
        true
    }

    pub fn sync_from_attribute_parts(
        &mut self,
        namespace: &str,
        local_name: &str,
        input_type: &str,
        attribute_name: &str,
        attribute_value: Option<&str>,
    ) {
        if attribute_name == "nonce" {
            self.cryptographic_nonce = attribute_value.map(str::to_owned);
        }
        if namespace != "http://www.w3.org/1999/xhtml" {
            return;
        }
        match (local_name, attribute_name) {
            ("input", "value") => {
                if !self.input_value_dirty {
                    self.input_value = Some(sanitize_input_value_for_type(
                        input_type,
                        attribute_value.unwrap_or_default(),
                    ));
                }
            }
            ("input", "type") => {
                if input_type_has_value_sanitization(input_type) {
                    let current = self.input_value.as_deref().unwrap_or_default();
                    self.input_value = Some(sanitize_input_value_for_type(input_type, current));
                }
            }
            ("input", "checked") => {
                if !self.checked_dirty {
                    self.checked = Some(attribute_value.is_some());
                }
            }
            ("option", "selected") => {
                if !self.selected_dirty {
                    self.selected = Some(attribute_value.is_some());
                }
            }
            (_, "popover") => {
                self.popover_open = false;
            }
            _ => {}
        }
    }
}

fn normalize_selection_direction(direction: &str) -> SelectionDirection {
    match direction {
        "forward" => SelectionDirection::Forward,
        "backward" => SelectionDirection::Backward,
        _ => SelectionDirection::None,
    }
}
