use crate::dom::native::Node;

use super::*;

mod data;
mod edits;
mod helpers;
mod prototype;
mod text;

pub(in crate::native_bridge) use data::node_character_data_length_from_object;
pub(in crate::native_bridge) use edits::{
    node_append_data_callback, node_delete_data_callback, node_insert_data_callback,
    node_replace_data_callback, node_substring_data_callback,
};
pub(in crate::native_bridge) use helpers::{
    dom_string_value_or_throw, require_argument_count, utf16_count_value,
    utf16_index_value_or_throw,
};
pub(crate) use prototype::install_character_data_template_bindings;
pub(in crate::native_bridge) use text::{
    node_split_text_callback, node_whole_text_utf16_units_from_object,
};
