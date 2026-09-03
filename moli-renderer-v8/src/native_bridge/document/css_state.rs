mod accessors;
mod font_faces;
mod projection;
mod shared;
mod style_sheets;

pub(super) use accessors::detached_document_fonts_getter;
pub(in crate::native_bridge) use accessors::{
    AdoptedStyleSheetsArrayOwner, document_fonts_getter_function,
    install_adopted_style_sheets_array_mutation_methods, normalize_adopted_style_sheets_assignment,
};
pub(crate) use accessors::{
    install_adopted_style_sheets_array_primordials,
    node_document_adopted_style_sheets_getter_function,
    node_document_adopted_style_sheets_setter_function, node_document_style_sheets_getter_function,
};
pub(crate) use font_faces::load_font_faces_used_by_subtrees;
pub(crate) use projection::{
    apply_stylesheet_owner_css_projections, apply_stylesheet_source_css_projection,
};

pub(crate) fn sync_document_fonts_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
    document: crate::document_runtime::DomHandle,
) {
    let Some(holder) = crate::util::node_wrapper_from_handle(scope, document) else {
        return;
    };
    let _ = font_faces::sync_document_fonts(scope, holder, host, document);
}

pub(crate) fn clear_adopted_stylesheet_font_face_wrappers(
    scope: &mut v8::PinScope<'_, '_>,
    sheet: v8::Local<'_, v8::Object>,
) {
    font_faces::clear_adopted_stylesheet_font_face_wrappers(scope, sheet);
}
