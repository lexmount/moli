mod attribute;
mod control_state;
mod interface_names;
mod rare_data;
#[cfg(test)]
mod tests;

pub use attribute::Attribute;
pub use control_state::{ElementControlState, SelectedFile};
pub use interface_names::{
    html_element_interface_name, is_mathml_namespace, mathml_element_interface_name,
    svg_element_interface_name,
};

use attribute::{normalized_option_text_content, split_class_names};
use html5ever::{LocalName, Namespace, Prefix};
use indexmap::IndexSet;
use rare_data::ElementRareData;
use thin_vec::ThinVec;

use super::NativeDom;
use super::node::{NativeNodeId, Node};
use crate::custom_elements::is_valid_custom_element_name;
use crate::forms::{
    canonical_input_type, is_valid_number_input_value, sanitize_input_value_for_type,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomElementState {
    Uncustomized,
    Undefined,
    Custom,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementCreationSource {
    Default,
    Parser,
}

fn initial_custom_element_state_for_identity(
    namespace: &str,
    local_name: &str,
    attributes: &[Attribute],
) -> CustomElementState {
    if namespace != "http://www.w3.org/1999/xhtml" {
        return CustomElementState::Uncustomized;
    }
    if is_valid_custom_element_name(local_name)
        || attributes
            .iter()
            .any(|attribute| attribute.name_matches("is"))
    {
        CustomElementState::Undefined
    } else {
        CustomElementState::Uncustomized
    }
}

#[derive(Debug, Clone)]
pub struct Element {
    local_name: LocalName,
    namespace: Namespace,
    prefix: Option<Prefix>,
    attributes: ThinVec<Attribute>,
    rare_data: ElementRareData,
}

impl Element {
    pub fn new_html(local_name: &str) -> Self {
        Self::new(
            local_name.to_ascii_lowercase(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
            Vec::new(),
        )
    }

    pub fn new(
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        attributes: Vec<Attribute>,
    ) -> Self {
        Self::new_with_creation_source(
            local_name,
            namespace,
            prefix,
            attributes,
            ElementCreationSource::Default,
        )
    }

    pub fn new_parser_created(
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        attributes: Vec<Attribute>,
    ) -> Self {
        Self::new_with_creation_source(
            local_name,
            namespace,
            prefix,
            attributes,
            ElementCreationSource::Parser,
        )
    }

    fn new_with_creation_source(
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        attributes: Vec<Attribute>,
        creation_source: ElementCreationSource,
    ) -> Self {
        let custom_element_state =
            initial_custom_element_state_for_identity(&namespace, &local_name, &attributes);
        let mut rare_data = ElementRareData::from_element_parts(
            &namespace,
            &local_name,
            &attributes,
            custom_element_state,
        );
        if creation_source == ElementCreationSource::Parser
            && local_name == "script"
            && matches!(
                namespace.as_str(),
                "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
            )
        {
            rare_data.control_state_mut().note_parser_created_script();
        }
        if creation_source == ElementCreationSource::Parser
            && local_name == "link"
            && namespace == "http://www.w3.org/1999/xhtml"
        {
            rare_data.control_state_mut().note_parser_created_link();
        }
        Self {
            local_name: LocalName::from(local_name),
            namespace: Namespace::from(namespace),
            prefix: prefix.map(Prefix::from),
            attributes: attributes.into(),
            rare_data,
        }
    }

    fn control_state(&self) -> &ElementControlState {
        self.rare_data.control_state()
    }

    fn control_state_mut(&mut self) -> &mut ElementControlState {
        self.rare_data.control_state_mut()
    }

    pub fn local_name(&self) -> &str {
        self.local_name.as_ref()
    }

    pub fn namespace(&self) -> &str {
        self.namespace.as_ref()
    }

    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_ref().map(AsRef::as_ref)
    }

    pub fn set_prefix(&mut self, prefix: Option<String>) -> bool {
        if self.prefix.as_deref() == prefix.as_deref() {
            return false;
        }
        self.prefix = prefix.map(Prefix::from);
        true
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name_matches(name))
            .map(Attribute::value)
    }

    pub fn attribute_ns(&self, namespace: &str, local_name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| {
                attribute.namespace() == namespace && attribute.local_name() == local_name
            })
            .map(Attribute::value)
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.attribute(name).is_some()
    }

    pub fn has_attribute_ns(&self, namespace: &str, local_name: &str) -> bool {
        self.attribute_ns(namespace, local_name).is_some()
    }

    pub fn custom_element_state(&self) -> CustomElementState {
        self.rare_data.custom_element_state()
    }

    pub fn custom_element_is_name(&self) -> Option<&str> {
        self.rare_data.custom_element_is_name()
    }

    pub fn set_custom_element_is_name(&mut self, is_name: Option<String>) -> bool {
        self.rare_data.set_custom_element_is_name(is_name)
    }

    pub fn set_custom_element_state(&mut self, state: CustomElementState) -> bool {
        self.rare_data.set_custom_element_state(state)
    }

    pub fn mark_undefined_custom_element_candidate_from_identity(&mut self) -> bool {
        if self.custom_element_state() != CustomElementState::Uncustomized {
            return false;
        }
        if self.custom_element_is_name().is_some() {
            return self.set_custom_element_state(CustomElementState::Undefined);
        }
        if initial_custom_element_state_for_identity(
            &self.namespace,
            &self.local_name,
            &self.attributes,
        ) != CustomElementState::Undefined
        {
            return false;
        }
        self.set_custom_element_state(CustomElementState::Undefined)
    }

    pub fn is_html_element(&self, local_name: &str) -> bool {
        self.namespace() == "http://www.w3.org/1999/xhtml" && self.local_name() == local_name
    }

    pub fn is_svg_element(&self, local_name: &str) -> bool {
        self.namespace() == "http://www.w3.org/2000/svg" && self.local_name() == local_name
    }

    pub fn is_image_resource_element(&self) -> bool {
        self.is_html_element("img") || self.is_svg_element("image")
    }

    /// Whether this element implements the SVG text-content-element role.
    ///
    /// This inheritance fact is needed even for SVG element interfaces that
    /// currently share the generic wrapper prototype.
    pub fn is_svg_text_content_element(&self) -> bool {
        self.namespace() == "http://www.w3.org/2000/svg"
            && matches!(self.local_name(), "text" | "tspan" | "textPath")
    }

    pub fn is_inline_style_element(&self) -> bool {
        self.local_name() == "style"
            && matches!(
                self.namespace(),
                "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
            )
    }

    pub fn is_html_input(&self) -> bool {
        self.is_html_element("input")
    }

    pub fn is_html_button(&self) -> bool {
        self.is_html_element("button")
    }

    pub fn is_html_option(&self) -> bool {
        self.is_html_element("option")
    }

    pub fn is_html_select(&self) -> bool {
        self.is_html_element("select")
    }

    pub fn is_html_fieldset(&self) -> bool {
        self.is_html_element("fieldset")
    }

    pub fn is_html_label(&self) -> bool {
        self.is_html_element("label")
    }

    pub fn is_html_textarea(&self) -> bool {
        self.is_html_element("textarea")
    }

    pub fn is_html_form(&self) -> bool {
        self.is_html_element("form")
    }

    pub fn is_html_audio(&self) -> bool {
        self.is_html_element("audio")
    }

    pub fn is_html_video(&self) -> bool {
        self.is_html_element("video")
    }

    pub fn is_html_media(&self) -> bool {
        self.is_html_audio() || self.is_html_video()
    }

    pub fn is_html_script(&self) -> bool {
        self.is_html_element("script")
    }

    pub fn is_script_element(&self) -> bool {
        self.local_name() == "script"
            && matches!(
                self.namespace.as_ref(),
                "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
            )
    }

    pub fn script_source_attribute(&self) -> Option<&str> {
        match self.namespace.as_ref() {
            "http://www.w3.org/1999/xhtml" => self.attribute_ns("", "src"),
            "http://www.w3.org/2000/svg" => self
                .attribute_ns("", "href")
                .or_else(|| self.attribute_ns("http://www.w3.org/1999/xlink", "href")),
            _ => None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.attribute("id").filter(|value| !value.is_empty())
    }

    pub fn name_attribute(&self) -> Option<&str> {
        self.attribute("name").filter(|value| !value.is_empty())
    }

    pub fn matches_named_item_key(&self, key: &str) -> bool {
        self.id() == Some(key)
            || (self.namespace() == "http://www.w3.org/1999/xhtml"
                && self.name_attribute() == Some(key))
    }

    pub fn has_attributes(&self) -> bool {
        !self.attributes.is_empty()
    }

    pub fn wrapper_prototype_name(&self) -> &'static str {
        match (self.namespace.as_ref(), self.local_name.as_ref()) {
            ("http://www.w3.org/2000/svg", local_name) => svg_element_interface_name(local_name),
            ("http://www.w3.org/1999/xhtml", local_name) => html_element_interface_name(local_name),
            (namespace, local_name) if is_mathml_namespace(namespace) => {
                mathml_element_interface_name(local_name)
            }
            _ => "Element",
        }
    }

    pub fn attribute_namespace(&self) -> &str {
        if self.namespace() == "http://www.w3.org/1999/xhtml" {
            ""
        } else {
            &self.namespace
        }
    }

    pub fn normalized_attribute_name(&self, name: &str) -> String {
        if self.namespace() == "http://www.w3.org/1999/xhtml" {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        }
    }

    pub fn node_name(&self) -> String {
        let local_name = if self.namespace() == "http://www.w3.org/1999/xhtml" {
            self.local_name().to_ascii_uppercase()
        } else {
            self.local_name.to_string()
        };
        match self.prefix() {
            Some(prefix) if !prefix.is_empty() => {
                let prefix = if self.namespace() == "http://www.w3.org/1999/xhtml" {
                    prefix.to_ascii_uppercase()
                } else {
                    prefix.to_owned()
                };
                format!("{prefix}:{local_name}")
            }
            _ => local_name,
        }
    }

    pub fn attribute_names(&self) -> Vec<String> {
        self.attributes.iter().map(Attribute::name).collect()
    }

    pub fn matches_tag_name(&self, tag_name: &str) -> bool {
        self.matches_tag_name_in_html_document(tag_name, true)
    }

    pub fn matches_tag_name_in_html_document(
        &self,
        tag_name: &str,
        is_html_document: bool,
    ) -> bool {
        if tag_name == "*" {
            return true;
        }

        let qualified_name = self.qualified_name();
        if is_html_document && self.namespace() == "http://www.w3.org/1999/xhtml" {
            qualified_name == tag_name.to_ascii_lowercase()
        } else {
            qualified_name == tag_name
        }
    }

    pub fn matches_tag_name_ns(&self, namespace: Option<&str>, local_name: &str) -> bool {
        let namespace_matches = match namespace {
            Some("*") => true,
            Some(expected) => self.namespace() == expected,
            None => self.namespace().is_empty(),
        };
        let local_name_matches = local_name == "*" || self.local_name() == local_name;
        namespace_matches && local_name_matches
    }

    pub fn matches_class_names(&self, class_name: &str) -> bool {
        let expected_classes = split_class_names(class_name);
        if expected_classes.is_empty() {
            return false;
        }

        let Some(actual) = self.attribute("class") else {
            return false;
        };
        let actual_classes = split_class_names(actual);
        expected_classes
            .iter()
            .all(|expected| actual_classes.iter().any(|actual| actual == expected))
    }

    pub fn matches_name(&self, name: &str) -> bool {
        self.namespace() == "http://www.w3.org/1999/xhtml" && self.name_attribute() == Some(name)
    }

    pub fn input_type(&self) -> String {
        canonical_input_type(self.attribute("type").unwrap_or("text")).to_owned()
    }

    pub fn input_value(&self) -> String {
        if self.is_html_input()
            && matches!(self.input_type().as_str(), "checkbox" | "radio")
            && !self.input_value_dirty()
            && self.attribute("value").is_none()
        {
            return "on".to_owned();
        }
        self.control_state()
            .input_value()
            .unwrap_or_default()
            .to_owned()
    }

    pub fn input_value_dirty(&self) -> bool {
        self.control_state().input_value_dirty()
    }

    pub fn input_value_user_edited(&self) -> bool {
        self.control_state().input_value_user_edited()
    }

    pub fn input_bad_input(&self) -> bool {
        self.control_state().input_bad_input()
    }

    pub fn datalist_text_decoration_initial_value_dirty(&self) -> Option<bool> {
        self.control_state()
            .datalist_text_decoration_initial_value_dirty()
    }

    pub fn autofilled(&self) -> bool {
        self.control_state().autofilled()
    }

    pub fn selected_files(&self) -> &[SelectedFile] {
        self.control_state().selected_files()
    }

    pub fn checked(&self) -> bool {
        self.control_state().checked().unwrap_or(false)
    }

    pub fn checked_dirty(&self) -> bool {
        self.control_state().checked_dirty()
    }

    pub fn selected(&self) -> bool {
        self.control_state().selected().unwrap_or(false)
    }

    pub fn output_default_value(&self) -> Option<&str> {
        self.control_state().output_default_value()
    }

    pub fn select_explicit_none(&self) -> bool {
        self.control_state().select_explicit_none()
    }

    pub fn indeterminate(&self) -> bool {
        self.control_state().indeterminate()
    }

    pub fn script_async(&self) -> bool {
        self.has_attribute("async") || self.control_state().script_force_async()
    }

    pub fn script_already_started(&self) -> bool {
        self.control_state().script_already_started()
    }

    pub fn script_parser_inserted_for_prepare(&self) -> bool {
        self.control_state().script_parser_inserted_for_prepare()
    }

    pub fn script_text_internal_slot(&self) -> &str {
        self.control_state().script_text_internal_slot()
    }

    pub fn script_children_changed_by_api(&self) -> bool {
        self.control_state().script_children_changed_by_api()
    }

    pub fn link_explicitly_enabled(&self) -> bool {
        self.control_state().link_explicitly_enabled()
    }

    pub fn link_created_by_parser(&self) -> bool {
        self.is_html_element("link") && self.control_state().link_created_by_parser()
    }

    pub fn set_input_value(&mut self, value: &str) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        if self.is_html_input() && self.input_type() == "file" {
            if !value.is_empty() {
                return false;
            }
            return self.set_selected_files(Vec::new());
        }
        let value = if self.is_html_input() {
            sanitize_input_value_for_type(&self.input_type(), value)
        } else {
            value.to_owned()
        };
        self.control_state_mut().set_input_value(&value)
    }

    pub fn set_input_value_with_dirty(&mut self, value: &str, dirty: bool) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        if self.is_html_input() && self.input_type() == "file" {
            if !value.is_empty() {
                return false;
            }
            return self.set_selected_files(Vec::new());
        }
        let value = if self.is_html_input() {
            sanitize_input_value_for_type(&self.input_type(), value)
        } else {
            value.to_owned()
        };
        self.control_state_mut()
            .set_input_value_with_dirty(&value, dirty)
    }

    pub fn set_autofilled(&mut self, autofilled: bool) -> bool {
        if !self.is_html_input() && !self.is_html_select() && !self.is_html_textarea() {
            return false;
        }
        self.control_state_mut().set_autofilled(autofilled)
    }

    pub fn set_input_value_from_user_edit(&mut self, value: &str) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        if self.is_html_input() && self.input_type() == "file" {
            return false;
        }
        let (value, bad_input) = if self.is_html_input() {
            let input_type = self.input_type();
            let bad_input =
                input_type == "number" && !value.is_empty() && !is_valid_number_input_value(value);
            let value = if bad_input {
                String::new()
            } else {
                sanitize_input_value_for_type(&input_type, value)
            };
            (value, bad_input)
        } else {
            (value.to_owned(), false)
        };
        let control_state = self.control_state_mut();
        let value_changed = control_state.set_input_value_from_user_edit(&value, bad_input);
        let autofill_changed = control_state.set_autofilled(false);
        value_changed || autofill_changed
    }

    pub fn prepare_datalist_text_decoration_for_value_mutation(
        &mut self,
        has_connected_datalist: bool,
    ) -> bool {
        if !self.is_html_input()
            || !matches!(self.input_type().as_str(), "text" | "tel" | "url" | "email")
        {
            return false;
        }
        self.control_state_mut()
            .prepare_datalist_text_decoration_for_value_mutation(has_connected_datalist)
    }

    pub fn set_selected_files(&mut self, files: Vec<SelectedFile>) -> bool {
        if !self.is_html_input() || self.input_type() != "file" {
            return false;
        }

        let fake_path = files
            .first()
            .map(|file| format!("C:\\fakepath\\{}", file.name))
            .unwrap_or_default();
        let control_state = self.control_state_mut();
        let files_changed = control_state.set_selected_files(files);
        let value_changed = control_state.set_input_value_with_dirty(&fake_path, true);
        files_changed || value_changed
    }

    pub fn set_checked(&mut self, checked: bool) -> bool {
        if !self.is_html_input() {
            return false;
        }
        self.control_state_mut().set_checked(checked)
    }

    pub fn set_checked_with_dirty(&mut self, checked: bool, dirty: bool) -> bool {
        if !self.is_html_input() {
            return false;
        }
        self.control_state_mut()
            .set_checked_with_dirty(checked, dirty)
    }

    pub fn set_selected(&mut self, selected: bool) -> bool {
        if !self.is_html_option() {
            return false;
        }
        self.control_state_mut().set_selected(selected)
    }

    pub fn selected_dirty(&self) -> bool {
        self.is_html_option() && self.control_state().selected_dirty()
    }

    pub fn set_selected_with_dirty(&mut self, selected: bool, dirty: bool) -> bool {
        if !self.is_html_option() {
            return false;
        }
        self.control_state_mut()
            .set_selected_with_dirty(selected, dirty)
    }

    pub fn set_output_default_value(&mut self, value: Option<String>) -> bool {
        if !self.is_html_element("output") {
            return false;
        }
        self.control_state_mut().set_output_default_value(value)
    }

    pub fn set_select_explicit_none(&mut self, explicit_none: bool) -> bool {
        if !self.is_html_select() {
            return false;
        }
        self.control_state_mut()
            .set_select_explicit_none(explicit_none)
    }

    pub fn set_indeterminate(&mut self, indeterminate: bool) -> bool {
        if !self.is_html_input() {
            return false;
        }
        self.control_state_mut().set_indeterminate(indeterminate)
    }

    pub fn set_script_force_async(&mut self, force_async: bool) -> bool {
        if !self.is_script_element() {
            return false;
        }
        self.control_state_mut().set_script_force_async(force_async)
    }

    pub fn set_script_already_started(&mut self, already_started: bool) -> bool {
        if !self.is_script_element() {
            return false;
        }
        self.control_state_mut()
            .set_script_already_started(already_started)
    }

    pub fn set_script_parser_inserted_for_prepare(&mut self, parser_inserted: bool) -> bool {
        if !self.is_script_element() {
            return false;
        }
        self.control_state_mut()
            .set_script_parser_inserted_for_prepare(parser_inserted)
    }

    pub fn set_script_text_internal_slot(&mut self, source: &str) -> bool {
        if !self.is_script_element() {
            return false;
        }
        self.control_state_mut()
            .set_script_text_internal_slot(source)
    }

    pub fn note_script_children_changed_by_api(&mut self) -> bool {
        if !self.is_script_element() {
            return false;
        }
        self.control_state_mut()
            .note_script_children_changed_by_api()
    }

    pub fn finish_parsing_script_children(&mut self, source: &str) -> bool {
        if !self.is_script_element() {
            return false;
        }
        self.control_state_mut()
            .finish_parsing_script_children(source)
    }

    pub fn finish_parsing_link_children(&mut self) -> bool {
        if !self.is_html_element("link") {
            return false;
        }
        self.control_state_mut().finish_parsing_link_children()
    }

    pub fn parser_associated_form_owner(&self) -> Option<NativeNodeId> {
        self.rare_data.parser_associated_form_owner()
    }

    pub fn set_parser_associated_form_owner(&mut self, owner: Option<NativeNodeId>) -> bool {
        self.rare_data.set_parser_associated_form_owner(owner)
    }

    pub fn set_link_explicitly_enabled(&mut self, explicitly_enabled: bool) -> bool {
        if !self.is_html_element("link") {
            return false;
        }
        self.control_state_mut()
            .set_link_explicitly_enabled(explicitly_enabled)
    }

    pub fn selection_start(&self) -> u32 {
        self.control_state().selection_start().unwrap_or(0)
    }

    pub fn selection_end(&self) -> u32 {
        self.control_state().selection_end().unwrap_or(0)
    }

    pub fn selection_direction(&self) -> &str {
        self.control_state().selection_direction().unwrap_or("none")
    }

    pub fn set_selection_range(&mut self, start: u32, end: u32) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        self.control_state_mut().set_selection_range(start, end)
    }

    pub fn set_selection_range_with_direction(
        &mut self,
        start: u32,
        end: u32,
        direction: &str,
    ) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        self.control_state_mut()
            .set_selection_range_with_direction(start, end, direction)
    }

    pub fn set_selection_start(&mut self, start: u32) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        self.control_state_mut().set_selection_start(start)
    }

    pub fn set_selection_end(&mut self, end: u32) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        self.control_state_mut().set_selection_end(end)
    }

    pub fn set_selection_direction(&mut self, direction: &str) -> bool {
        if !self.is_html_input() && !self.is_html_textarea() {
            return false;
        }
        self.control_state_mut().set_selection_direction(direction)
    }

    pub fn media_paused(&self) -> bool {
        self.control_state().media_paused().unwrap_or(true)
    }

    pub fn set_media_paused(&mut self, paused: bool) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_paused(paused)
    }

    pub fn media_volume(&self) -> f64 {
        self.control_state().media_volume().unwrap_or(1.0)
    }

    pub fn set_media_volume(&mut self, volume: f64) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_volume(volume)
    }

    pub fn media_muted(&self) -> bool {
        self.control_state().media_muted().unwrap_or(false)
    }

    pub fn set_media_muted(&mut self, muted: bool) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_muted(muted)
    }

    pub fn media_seeking(&self) -> bool {
        self.control_state().media_seeking().unwrap_or(false)
    }

    pub fn media_seek_token(&self) -> u64 {
        self.control_state().media_seek_token()
    }

    pub fn set_media_seeking(&mut self, seeking: bool) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_seeking(seeking)
    }

    pub fn advance_media_seek_token(&mut self) -> Option<u64> {
        self.is_html_media()
            .then(|| self.control_state_mut().advance_media_seek_token())
    }

    pub fn media_playback_rate(&self) -> f64 {
        self.control_state().media_playback_rate().unwrap_or(1.0)
    }

    pub fn set_media_playback_rate(&mut self, rate: f64) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_playback_rate(rate)
    }

    pub fn media_current_time(&self) -> f64 {
        self.control_state().media_current_time().unwrap_or(0.0)
    }

    pub fn set_media_current_time(&mut self, current_time: f64) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut()
            .set_media_current_time(current_time)
    }

    pub fn media_ready_state(&self) -> u32 {
        self.control_state().media_ready_state().unwrap_or(0)
    }

    pub fn set_media_ready_state(&mut self, ready_state: u32) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_ready_state(ready_state)
    }

    pub fn media_network_state(&self) -> u32 {
        self.control_state().media_network_state().unwrap_or(0)
    }

    pub fn media_error_code(&self) -> Option<u32> {
        self.control_state().media_error_code()
    }

    pub fn scroll_top(&self) -> f64 {
        self.control_state().scroll_top().unwrap_or(0.0)
    }

    pub fn image_load_dispatched(&self) -> bool {
        self.control_state().image_load_dispatched()
    }

    pub fn cryptographic_nonce(&self) -> Option<&str> {
        self.control_state().cryptographic_nonce()
    }

    pub fn set_image_load_dispatched(&mut self, dispatched: bool) -> bool {
        if !self.is_html_element("img") {
            return false;
        }
        self.control_state_mut()
            .set_image_load_dispatched(dispatched)
    }

    pub fn set_cryptographic_nonce(&mut self, nonce: Option<String>) -> bool {
        if self.cryptographic_nonce() == nonce.as_deref() {
            return false;
        }
        self.control_state_mut().set_cryptographic_nonce(nonce)
    }

    pub fn set_scroll_top(&mut self, scroll_top: f64) -> bool {
        if self.scroll_top() == scroll_top {
            return false;
        }
        self.control_state_mut().set_scroll_top(scroll_top)
    }

    pub fn scroll_left(&self) -> f64 {
        self.control_state().scroll_left().unwrap_or(0.0)
    }

    pub fn set_scroll_left(&mut self, scroll_left: f64) -> bool {
        if self.scroll_left() == scroll_left {
            return false;
        }
        self.control_state_mut().set_scroll_left(scroll_left)
    }

    pub fn custom_validation_message(&self) -> &str {
        self.control_state().custom_validation_message()
    }

    pub fn set_custom_validation_message(&mut self, message: &str) -> bool {
        if self.namespace() != "http://www.w3.org/1999/xhtml"
            || !matches!(
                self.local_name.as_ref(),
                "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
            )
        {
            return false;
        }
        self.control_state_mut()
            .set_custom_validation_message(message)
    }

    pub fn popover_open(&self) -> bool {
        self.control_state().popover_open()
    }

    pub fn dialog_modal(&self) -> bool {
        self.control_state().dialog_modal()
    }

    pub fn dialog_return_value(&self) -> &str {
        self.control_state().dialog_return_value()
    }

    pub fn custom_states(&self) -> &IndexSet<String> {
        self.control_state().custom_states()
    }

    pub fn has_custom_state(&self, state: &str) -> bool {
        self.control_state().has_custom_state(state)
    }

    pub fn set_popover_open(&mut self, open: bool) -> bool {
        if self.namespace() != "http://www.w3.org/1999/xhtml" {
            return false;
        }
        self.control_state_mut().set_popover_open(open)
    }

    pub fn set_dialog_modal(&mut self, modal: bool) -> bool {
        if !self.is_html_element("dialog") {
            return false;
        }
        self.control_state_mut().set_dialog_modal(modal)
    }

    pub fn set_dialog_return_value(&mut self, value: &str) -> bool {
        if !self.is_html_element("dialog") {
            return false;
        }
        self.control_state_mut().set_dialog_return_value(value)
    }

    pub fn insert_custom_state(&mut self, state: &str) -> bool {
        self.control_state_mut().insert_custom_state(state)
    }

    pub fn remove_custom_state(&mut self, state: &str) -> bool {
        self.control_state_mut().remove_custom_state(state)
    }

    pub fn clear_custom_states(&mut self) -> bool {
        self.control_state_mut().clear_custom_states()
    }

    pub fn set_media_network_state(&mut self, network_state: u32) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut()
            .set_media_network_state(network_state)
    }

    pub fn set_media_error_code(&mut self, error_code: Option<u32>) -> bool {
        if !self.is_html_media() {
            return false;
        }
        self.control_state_mut().set_media_error_code(error_code)
    }

    pub fn option_value(&self, dom: &NativeDom, handle: NativeNodeId) -> String {
        self.attribute_ns("", "value")
            .map(str::to_owned)
            .unwrap_or_else(|| self.option_text(dom, handle))
    }

    pub fn option_text(&self, dom: &NativeDom, handle: NativeNodeId) -> String {
        normalized_option_text_content(dom, handle)
    }

    pub fn option_label(&self, dom: &NativeDom, handle: NativeNodeId) -> String {
        self.attribute_ns("", "label")
            .map(str::to_owned)
            .unwrap_or_else(|| self.option_text(dom, handle))
    }

    pub fn select_value(
        &self,
        dom: &NativeDom,
        handle: NativeNodeId,
        is_selected: impl Fn(NativeNodeId) -> bool,
    ) -> String {
        if !self.is_html_select() {
            return String::new();
        }

        for descendant in dom.elements_by_tag_name(handle, "option", false) {
            let Some(option) = dom.node(descendant).and_then(Node::as_element) else {
                continue;
            };
            if !option.is_html_option() {
                continue;
            }
            if is_selected(descendant) {
                return option.option_value(dom, descendant);
            }
        }

        String::new()
    }

    pub fn template_contents(&self) -> Option<NativeNodeId> {
        self.rare_data.template_contents()
    }

    pub fn set_template_contents(&mut self, template_contents: Option<NativeNodeId>) {
        self.rare_data.set_template_contents(template_contents);
    }

    fn sync_control_state_from_attribute(
        &mut self,
        attribute_name: &str,
        attribute_value: Option<&str>,
    ) {
        let input_type = if self.namespace() == "http://www.w3.org/1999/xhtml"
            && self.local_name() == "input"
            && attribute_name == "type"
        {
            attribute_value.unwrap_or("text").to_owned()
        } else {
            self.input_type()
        };
        self.rare_data.sync_control_state_from_attribute(
            self.namespace.as_ref(),
            self.local_name.as_ref(),
            &input_type,
            attribute_name,
            attribute_value,
        );
    }

    pub fn set_attribute(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        value: String,
    ) -> bool {
        let next_value = value.clone();
        if let Some(index) = self
            .attributes
            .iter()
            .position(|attribute| attribute.name_matches(&local_name))
        {
            // `set_attribute()` models qualified-name updates (`setAttribute()` and clone/parser
            // insertion paths). Once an attribute is matched by its existing qualified name, we
            // preserve its current namespace/prefix and only update the value. Callers that need
            // namespace/local-name replacement semantics must go through `set_attribute_ns()`.
            if self.attributes[index].value() == value {
                return false;
            }
            self.attributes[index].value = value.into_boxed_str();
            let attribute_local_name = self.attributes[index].local_name.clone();
            self.sync_control_state_from_attribute(
                attribute_local_name.as_ref(),
                Some(&next_value),
            );
            return true;
        }

        let attribute_local_name = LocalName::from(local_name);
        self.attributes.push(Attribute {
            local_name: attribute_local_name.clone(),
            namespace: Namespace::from(namespace),
            prefix: prefix.map(Prefix::from),
            value: value.into_boxed_str(),
        });
        self.sync_control_state_from_attribute(attribute_local_name.as_ref(), Some(&next_value));
        true
    }

    pub fn set_attribute_ns(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
        value: String,
    ) -> bool {
        let next_value = value.clone();
        if let Some(index) = self.attributes.iter().position(|attribute| {
            attribute.local_name() == local_name && attribute.namespace() == namespace
        }) {
            if self.attributes[index].value() == value {
                return false;
            }
            self.attributes[index].value = value.into_boxed_str();
            self.sync_control_state_from_attribute(&local_name, Some(&next_value));
            return true;
        }

        let attribute_local_name = LocalName::from(local_name);
        self.attributes.push(Attribute {
            local_name: attribute_local_name.clone(),
            namespace: Namespace::from(namespace),
            prefix: prefix.map(Prefix::from),
            value: value.into_boxed_str(),
        });
        self.sync_control_state_from_attribute(attribute_local_name.as_ref(), Some(&next_value));
        true
    }

    pub fn remove_attribute(&mut self, name: &str) -> bool {
        let Some(index) = self
            .attributes
            .iter()
            .position(|attribute| attribute.name_matches(name))
        else {
            return false;
        };
        let local_name = self.attributes.remove(index).local_name;
        self.sync_control_state_from_attribute(local_name.as_ref(), None);
        true
    }

    pub fn remove_attribute_ns(&mut self, namespace: &str, local_name: &str) -> bool {
        let Some(index) = self.attributes.iter().position(|attribute| {
            attribute.namespace() == namespace && attribute.local_name() == local_name
        }) else {
            return false;
        };
        let local_name = self.attributes.remove(index).local_name;
        self.sync_control_state_from_attribute(local_name.as_ref(), None);
        true
    }

    pub fn qualified_name(&self) -> String {
        match self.prefix() {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}:{}", self.local_name),
            _ => self.local_name.to_string(),
        }
    }
}
