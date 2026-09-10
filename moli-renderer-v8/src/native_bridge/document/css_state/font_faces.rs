use super::super::{object_property_as_object, object_string_property};
use super::shared::{
    ADOPTED_STYLE_SHEETS_SLOT, FONT_FACE_SET_CONNECTED_FACES_SLOT,
    FONT_FACE_SET_CONNECTED_OWNER_FACES_SLOT, FONTS_SLOT,
};
use crate::{
    context_bootstrap::css_stylesheet_runtime::{
        private_string, private_u64, set_private_string, set_private_u64,
    },
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    style_engine::{
        StylesheetFontFaceDescriptor, StylesheetFontFaceProjection,
        StylesheetFontFaceRuleProjection,
    },
    util::{get_private_value, serialize_v8_iter_array, set_private_value, v8_string, v8str},
};
use std::convert::TryFrom;
use std::rc::Rc;

const CSS_STYLE_SHEET_FONT_FACE_WRAPPERS_SLOT: &str = "__moliCssStyleSheetFontFaceWrappers";
const FONT_FACE_STYLESHEET_RULE_IDENTITY_SLOT: &str = "__moliFontFaceStylesheetRuleIdentity";
const FONT_FACE_STYLESHEET_RULE_FINGERPRINT_SLOT: &str = "__moliFontFaceStylesheetRuleFingerprint";
const FONT_FACE_STYLESHEET_ID_SLOT: &str = "__moliFontFaceStylesheetId";

#[derive(Clone, Debug)]
pub(super) enum OwnerFontFaceProjection {
    Descriptors(std::sync::Arc<[StylesheetFontFaceDescriptor]>),
    Live {
        stylesheet_id: crate::live_stylesheet::StylesheetId,
        projection: Rc<StylesheetFontFaceProjection>,
    },
}

pub(super) fn owner_font_face_projection(
    host: &JsContextHost,
    owner: DomHandle,
) -> Option<OwnerFontFaceProjection> {
    let dom_host = host.dom_host();
    let element = dom_host.node(owner)?.as_element()?;
    if !dom_host.is_connected(owner)
        || !crate::style_engine::stylesheet_owner_type_is_supported(element)
    {
        return None;
    }
    if let Some(stylesheet) = host
        .owner_live_stylesheet(owner)
        .or_else(|| host.linked_live_stylesheet(owner))
    {
        return Some(OwnerFontFaceProjection::Live {
            stylesheet_id: stylesheet.id(),
            projection: stylesheet.font_faces(
                crate::style_engine::StyloStyleEnvironment::from_emulated_media(
                    host.emulated_media(),
                ),
                host.style_viewport(),
            ),
        });
    }
    host.stylesheet_font_faces_for_owner(owner)
        .map(OwnerFontFaceProjection::Descriptors)
}

fn construct_font_face_from_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    family: &str,
    source: &str,
    rule_css: Option<&str>,
) -> Option<v8::Local<'s, v8::Value>> {
    let ctor =
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "FontFace").ok()?;
    let family = v8_string(scope, family)?;
    let source = v8_string(scope, source)?;
    let descriptors = v8::Object::new(scope);
    let _ = descriptors.set_prototype(scope, v8::null(scope).into());
    if let Some(rule) = rule_css.and_then(moli_css_parse::parse_font_face_rule_view_with_stylo) {
        for entry in moli_css_parse::parse_declaration_list(
            &rule.style_text,
            moli_css_parse::DeclarationParseOptions {
                canonicalize_property_name: true,
                unescape_value_semicolons: false,
                preserve_empty_values: false,
            },
        ) {
            let name = match entry.name.as_str() {
                "font-style" => "style",
                "font-weight" => "weight",
                "font-stretch" => "stretch",
                "font-variant" => "variant",
                "font-feature-settings" => "featureSettings",
                "font-variation-settings" => "variationSettings",
                "font-display" => "display",
                _ => continue,
            };
            let value = v8_string(scope, &entry.value)?;
            // Define data properties rather than invoking author Object.prototype setters.
            let _ =
                descriptors.create_data_property(scope, v8str(scope, name).into(), value.into());
        }
    }
    ctor.new_instance(scope, &[family.into(), source.into(), descriptors.into()])
        .map(Into::into)
}

fn construct_font_face<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: &StylesheetFontFaceDescriptor,
) -> Option<v8::Local<'s, v8::Value>> {
    construct_font_face_from_parts(scope, descriptor.family(), descriptor.source(), None)
}

fn collect_font_face_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let mut values = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        let Some(value) = array.get_index(scope, index) else {
            continue;
        };
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        values.push(object);
    }
    values
}

fn font_face_matches_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    face: v8::Local<'_, v8::Object>,
    descriptor: &StylesheetFontFaceDescriptor,
) -> bool {
    object_string_property(scope, face, "family").as_deref() == Some(descriptor.family())
        && object_string_property(scope, face, "source").as_deref() == Some(descriptor.source())
}

fn font_face_matches_rule_projection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    stylesheet_id: crate::live_stylesheet::StylesheetId,
    projection: &StylesheetFontFaceRuleProjection,
) -> bool {
    private_u64(scope, face, FONT_FACE_STYLESHEET_ID_SLOT) == Some(stylesheet_id.get())
        && private_u64(scope, face, FONT_FACE_STYLESHEET_RULE_IDENTITY_SLOT)
            == Some(projection.rule_identity)
        && private_string(scope, face, FONT_FACE_STYLESHEET_RULE_FINGERPRINT_SLOT)
            == projection.rule_fingerprint
}

pub(super) fn sync_document_fonts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    host: &JsContextHost,
    document: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let fonts = object_property_as_object(scope, holder, FONTS_SLOT)?;
    let candidates = host
        .dom_host()
        .stylesheet_candidate_handles_for_tree_scope(document);
    for owner in candidates.iter().copied() {
        let projection = owner_font_face_projection(host, owner);
        set_owner_font_face_contribution(scope, fonts, owner, projection.as_ref());
    }
    rebuild_connected_font_faces(scope, fonts, host, document);
    Some(fonts)
}

pub(super) fn apply_font_face_owner_projection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: DomHandle,
    owner: DomHandle,
    projection: Option<&OwnerFontFaceProjection>,
) -> bool {
    let Some(holder) = crate::util::node_wrapper_from_handle(scope, document) else {
        return false;
    };
    let Some(fonts) = object_property_as_object(scope, holder, FONTS_SLOT) else {
        return false;
    };
    set_owner_font_face_contribution(scope, fonts, owner, projection);
    true
}

pub(super) fn finish_font_face_owner_projections<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    document: DomHandle,
) {
    let Some(holder) = crate::util::node_wrapper_from_handle(scope, document) else {
        return;
    };
    let Some(fonts) = object_property_as_object(scope, holder, FONTS_SLOT) else {
        return;
    };
    rebuild_connected_font_faces(scope, fonts, host, document);
}

fn set_owner_font_face_contribution<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fonts: v8::Local<'s, v8::Object>,
    owner: DomHandle,
    projection: Option<&OwnerFontFaceProjection>,
) {
    let contributions = connected_owner_contributions(scope, fonts);
    let index = owner.index_u32();
    let Some(projection) = projection else {
        let undefined = v8::undefined(scope);
        let _ = contributions.set_index(scope, index, undefined.into());
        return;
    };
    let existing = contributions
        .get_index(scope, index)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .map(|array| collect_font_face_objects(scope, array))
        .unwrap_or_default();
    let faces = match projection {
        OwnerFontFaceProjection::Descriptors(descriptors) => {
            let mut used = vec![false; existing.len()];
            let mut faces = Vec::with_capacity(descriptors.len());
            for descriptor in descriptors.iter() {
                let mut face = None;
                for (existing_index, existing_face) in existing.iter().enumerate() {
                    if used[existing_index]
                        || !font_face_matches_descriptor(scope, *existing_face, descriptor)
                    {
                        continue;
                    }
                    used[existing_index] = true;
                    face = Some((*existing_face).into());
                    break;
                }
                if let Some(face) = face.or_else(|| construct_font_face(scope, descriptor)) {
                    faces.push(face);
                }
            }
            faces
        }
        OwnerFontFaceProjection::Live {
            stylesheet_id,
            projection,
        } => sync_stylesheet_font_face_wrappers(scope, existing, *stylesheet_id, projection, true)
            .into_iter()
            .map(|(_, face)| face.into())
            .collect(),
    };
    let faces = serialize_v8_iter_array(scope, faces).unwrap_or_else(|| v8::Array::new(scope, 0));
    let _ = contributions.set_index(scope, index, faces.into());
}

fn sync_stylesheet_font_face_wrappers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    existing: Vec<v8::Local<'s, v8::Object>>,
    stylesheet_id: crate::live_stylesheet::StylesheetId,
    projection: &StylesheetFontFaceProjection,
    effective_only: bool,
) -> Vec<(u64, v8::Local<'s, v8::Object>)> {
    let mut used = vec![false; existing.len()];
    let mut all_faces = Vec::with_capacity(projection.all_rules.len());
    for rule in &projection.all_rules {
        if effective_only
            && !projection
                .effective_rule_identities
                .contains(&rule.rule_identity)
        {
            continue;
        }
        let mut face = None;
        for (existing_index, existing_face) in existing.iter().enumerate() {
            if used[existing_index]
                || !font_face_matches_rule_projection(scope, *existing_face, stylesheet_id, rule)
            {
                continue;
            }
            used[existing_index] = true;
            face = Some(*existing_face);
            break;
        }
        let face = face.or_else(|| {
            let face = construct_font_face_from_parts(
                scope,
                &rule.descriptor.family,
                &rule.descriptor.source,
                Some(&rule.rule_fingerprint),
            )?;
            let face = v8::Local::<v8::Object>::try_from(face).ok()?;
            set_private_u64(
                scope,
                face,
                FONT_FACE_STYLESHEET_ID_SLOT,
                stylesheet_id.get(),
            );
            set_private_u64(
                scope,
                face,
                FONT_FACE_STYLESHEET_RULE_IDENTITY_SLOT,
                rule.rule_identity,
            );
            set_private_string(
                scope,
                face,
                FONT_FACE_STYLESHEET_RULE_FINGERPRINT_SLOT,
                &rule.rule_fingerprint,
            );
            Some(face)
        });
        if let Some(face) = face {
            if let Some(font) = rule
                .resource
                .as_ref()
                .and_then(|resource| resource.web_font())
                && crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(
                    scope,
                )
                .is_none()
                && let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope)
            {
                // The resource metadata retains the stylesheet parser's URLs
                // and the same slot used by CSS resource admission.
                let load = unsafe { &*host_ptr }.observe_document_font_face(font);
                if !crate::context_bootstrap::bind_stylesheet_font_face(scope, face, load) {
                    continue;
                }
            }
            all_faces.push((rule.rule_identity, face));
        }
    }
    all_faces
}

fn connected_owner_contributions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fonts: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    if let Some(existing) =
        get_private_value(scope, fonts, FONT_FACE_SET_CONNECTED_OWNER_FACES_SLOT)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return existing;
    }
    let contributions = v8::Array::new(scope, 0);
    set_private_value(
        scope,
        fonts,
        FONT_FACE_SET_CONNECTED_OWNER_FACES_SLOT,
        contributions.into(),
    );
    contributions
}

fn rebuild_connected_font_faces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fonts: v8::Local<'s, v8::Object>,
    host: &JsContextHost,
    document: DomHandle,
) {
    let contributions = connected_owner_contributions(scope, fonts);
    let mut connected = Vec::new();
    let candidates = host
        .dom_host()
        .stylesheet_candidate_handles_for_tree_scope(document);
    for owner in candidates.iter().copied() {
        let Some(owner_faces) = contributions
            .get_index(scope, owner.index_u32())
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        else {
            continue;
        };
        for index in 0..owner_faces.length() {
            if let Some(face) = owner_faces.get_index(scope, index) {
                connected.push(face);
            }
        }
    }
    connected.extend(document_adopted_font_faces(scope, host, document));
    let connected =
        serialize_v8_iter_array(scope, connected).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        fonts,
        FONT_FACE_SET_CONNECTED_FACES_SLOT,
        connected.into(),
    );
    crate::context_bootstrap::rebuild_font_face_set_faces(scope, fonts);
}

fn document_adopted_font_faces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    document: DomHandle,
) -> Vec<v8::Local<'s, v8::Value>> {
    let Some(holder) = crate::util::node_wrapper_from_handle(scope, document) else {
        return Vec::new();
    };
    let Some(sheets) = get_private_value(scope, holder, ADOPTED_STYLE_SHEETS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut faces = Vec::new();
    for index in 0..sheets.length() {
        let Some(sheet) = sheets
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if seen
            .iter()
            .any(|seen_sheet: &v8::Local<'s, v8::Object>| seen_sheet.strict_equals(sheet.into()))
        {
            continue;
        }
        seen.push(sheet);
        let Some(stylesheet) =
            crate::context_bootstrap::css_stylesheet_runtime::css_style_sheet_live_stylesheet(
                scope, sheet,
            )
        else {
            continue;
        };
        let descriptors = stylesheet.font_faces(
            crate::style_engine::StyloStyleEnvironment::from_emulated_media(host.emulated_media()),
            host.style_viewport(),
        );
        faces.extend(sync_adopted_stylesheet_font_faces(
            scope,
            sheet,
            &descriptors,
        ));
    }
    faces
}

fn sync_adopted_stylesheet_font_faces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    projection: &StylesheetFontFaceProjection,
) -> Vec<v8::Local<'s, v8::Value>> {
    let existing = get_private_value(scope, sheet, CSS_STYLE_SHEET_FONT_FACE_WRAPPERS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .map(|array| collect_font_face_objects(scope, array))
        .unwrap_or_default();
    let Some(stylesheet_id) =
        crate::context_bootstrap::css_stylesheet_runtime::css_style_sheet_id(scope, sheet)
    else {
        return Vec::new();
    };
    let all_faces =
        sync_stylesheet_font_face_wrappers(scope, existing, stylesheet_id, projection, false);
    let stored = serialize_v8_iter_array(scope, all_faces.iter().map(|(_, face)| *face))
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_FONT_FACE_WRAPPERS_SLOT,
        stored.into(),
    );
    all_faces
        .into_iter()
        .filter_map(|(rule_identity, face)| {
            projection
                .effective_rule_identities
                .contains(&rule_identity)
                .then_some(face.into())
        })
        .collect()
}

pub(super) fn clear_adopted_stylesheet_font_face_wrappers(
    scope: &mut v8::PinScope<'_, '_>,
    sheet: v8::Local<'_, v8::Object>,
) {
    let empty = v8::Array::new(scope, 0);
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_FONT_FACE_WRAPPERS_SLOT,
        empty.into(),
    );
}
