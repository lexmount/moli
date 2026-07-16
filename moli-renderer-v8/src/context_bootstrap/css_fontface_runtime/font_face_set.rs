use super::font_face::{font_face_load_failed, start_font_face_load};
use super::query::{
    font_face_set_matching_faces_array, font_load_query_contains_css_wide_keyword,
    make_rejected_dom_exception_promise,
};
use super::storage::{
    array_contains_value, font_face_set_faces_array, font_face_set_manual_faces_array,
    initialize_font_face_set_object, is_font_face_value, rebuild_font_face_set_faces,
    set_font_face_set_slot_value,
};
use super::*;

mod collection;
mod constructor;
mod event_target;
mod iteration;
mod loading;

pub(in crate::context_bootstrap) use collection::{
    font_face_set_add_callback, font_face_set_clear_callback, font_face_set_delete_callback,
    font_face_set_has_callback,
};
pub(in crate::context_bootstrap) use constructor::font_face_set_constructor_callback;
pub(in crate::context_bootstrap) use event_target::{
    font_face_set_add_event_listener_callback, font_face_set_dispatch_event_callback,
    font_face_set_remove_event_listener_callback,
};
pub(in crate::context_bootstrap) use iteration::{
    font_face_set_entries_callback, font_face_set_for_each_callback, font_face_set_keys_callback,
    font_face_set_values_callback,
};
pub(in crate::context_bootstrap) use loading::{
    font_face_set_check_callback, font_face_set_load_callback,
};
