use super::css_runtime::resolved_promise;
use super::*;

fn apply_pending_stylesheet_source_css_projections(scope: &mut v8::PinScope<'_, '_>) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
}

const FONT_FACE_SET_FACES_SLOT: &str = "__moliFontFaceSetFaces";
pub(super) const FONT_FACE_FAMILY_SLOT: &str = "__moliFontFaceFamily";
pub(super) const FONT_FACE_FAMILY_CSS_TEXT_SLOT: &str = "__moliFontFaceFamilyCssText";
pub(super) const FONT_FACE_SOURCE_SLOT: &str = "__moliFontFaceSource";
pub(super) const FONT_FACE_STYLE_SLOT: &str = "__moliFontFaceStyle";
pub(super) const FONT_FACE_WEIGHT_SLOT: &str = "__moliFontFaceWeight";
pub(super) const FONT_FACE_STRETCH_SLOT: &str = "__moliFontFaceStretch";
pub(super) const FONT_FACE_VARIANT_SLOT: &str = "__moliFontFaceVariant";
pub(super) const FONT_FACE_FEATURE_SETTINGS_SLOT: &str = "__moliFontFaceFeatureSettings";
pub(super) const FONT_FACE_VARIATION_SETTINGS_SLOT: &str = "__moliFontFaceVariationSettings";
pub(super) const FONT_FACE_DISPLAY_SLOT: &str = "__moliFontFaceDisplay";
pub(super) const FONT_FACE_UNICODE_RANGE_SLOT: &str = "__moliFontFaceUnicodeRange";
pub(super) const FONT_FACE_ASCENT_OVERRIDE_SLOT: &str = "__moliFontFaceAscentOverride";
pub(super) const FONT_FACE_DESCENT_OVERRIDE_SLOT: &str = "__moliFontFaceDescentOverride";
pub(super) const FONT_FACE_LINE_GAP_OVERRIDE_SLOT: &str = "__moliFontFaceLineGapOverride";
pub(super) const FONT_FACE_SIZE_ADJUST_SLOT: &str = "__moliFontFaceSizeAdjust";
pub(super) const FONT_FACE_STATUS_SLOT: &str = "__moliFontFaceStatus";
pub(super) const FONT_FACE_LOADED_SLOT: &str = "__moliFontFaceLoaded";
pub(super) const FONT_FACE_SET_OWNERS_SLOT: &str = "__moliFontFaceSetOwners";
pub(super) const FONT_FACE_LOAD_NOTIFICATION_SENT_SLOT: &str = "__moliFontFaceLoadNotificationSent";
pub(super) const FONT_FACE_SET_MANUAL_FACES_SLOT: &str = "__moliFontFaceSetManualFaces";
pub(super) const FONT_FACE_SET_CONNECTED_FACES_SLOT: &str = "__moliFontFaceSetConnectedFaces";
pub(super) const FONT_FACE_SET_STATUS_SLOT: &str = "__moliFontFaceSetStatus";
const FONT_FACE_SET_READY_SLOT: &str = "__moliFontFaceSetReady";

mod automatic;
mod events;
mod font_face;
mod font_face_set;
mod loading;
mod query;
mod state;
mod storage;

pub(crate) use automatic::discover_requests as discover_font_requests;
#[cfg(test)]
pub(crate) use automatic::discovery_count as font_request_discovery_count;
pub(in crate::context_bootstrap) use events::{
    FontFaceSetLoadEventInit, install_font_face_set_event_handler_accessors,
    install_font_face_set_load_event_template_accessors,
};
pub(super) use font_face::{
    font_face_constructor_callback, font_face_load_callback, install_font_face_template_accessors,
};
pub(crate) use font_face::{font_face_raw_family, mark_font_face_css_connected};
pub(super) use font_face_set::{
    font_face_set_add_callback, font_face_set_check_callback, font_face_set_clear_callback,
    font_face_set_delete_callback, font_face_set_entries_callback, font_face_set_for_each_callback,
    font_face_set_has_callback, font_face_set_keys_callback, font_face_set_load_callback,
    font_face_set_values_callback,
};
pub(crate) use loading::{
    bind_stylesheet_font_face, finish_font_face_url_load, publish_font_face_load_changes,
};
pub(super) use storage::install_font_face_set_template_accessors;
pub(crate) use storage::{
    new_font_face_set, rebuild_font_face_set_faces, settle_document_font_face_set_ready,
};
