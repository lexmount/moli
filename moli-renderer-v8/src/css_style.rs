use crate::webidl;
use moli_css_parse::{CssFontFace, parse_font_faces};

pub(crate) use moli_css_parse::{
    camel_case_style_property_name, canonical_style_property_identifier,
    canonical_style_property_name,
};
pub(crate) use moli_dom::native::inline_style::{
    CssInlineStyleDeclarationState, CssStyleEntry, CssomStyleAttribute, InlineStyleCompatibility,
    InlineStyleParseContext, background_shorthand_color, border_shorthand_color,
    border_shorthand_style, border_shorthand_width, box_shorthand_value_components,
    compress_box_components, css_declaration_list_property_value, css_wide_keyword,
    ident_is_system_color, mask_compat_property_name, mask_compat_value_is_supported,
    normalize_cssom_component_value_serialization_with_spaced_slash,
    normalize_cssom_flex_basis_value, normalize_cssom_flex_shorthand_value,
    parse_css_declaration_list, pdb_mutation_value_for_style_entry, scan_inline_css_text,
    serialize_css_style_entries, serialize_css_style_entries_with_pdb_block,
    serialize_text_decoration_shorthand_value, stylo_declaration_block_property_names,
    stylo_mask_property_name, system_color_rgb, top_level_comma_separated_component_values,
    webkit_transform_origin_compat_property_name,
    webkit_transform_origin_compat_value_is_supported,
};

/// Filter Stylo's names to the public CSS property surface. Accessor installation
/// and computed declaration enumeration must use the same exposure policy.
pub(crate) fn stylo_property_is_chromium_exposed(name: &str) -> bool {
    !name.starts_with("-moz-")
        && !name.starts_with("-x-")
        && !matches!(name, "mask-position-x" | "mask-position-y")
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration.item")]
pub(crate) struct CssStyleDeclarationItemArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'item' on 'CSSStyleDeclaration'"
    )]
    pub(crate) index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration.cssText")]
pub(crate) struct CssStyleDeclarationCssTextArgs {
    // An attribute setter converts undefined even when called with no argument.
    #[webidl(default = "undefined", treat_null_as_empty_string)]
    pub(crate) value: String,
}

/// Named interceptors receive a value rather than function arguments. Use the
/// same WebIDL string conversion as the declaration's generated arguments.
pub(crate) fn css_style_declaration_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    match webidl::convert_with_options::<webidl::DomString>(
        scope,
        value,
        webidl::Context::argument("CSSStyleDeclaration", 1),
        &webidl::StringOptions {
            treat_null_as_empty_string: true,
        },
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration.setProperty")]
pub(crate) struct CssStyleDeclarationSetPropertyArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'setProperty' on 'CSSStyleDeclaration'"
    )]
    pub(crate) property: String,
    #[webidl(
        required,
        missing_message = "Failed to execute 'setProperty' on 'CSSStyleDeclaration'",
        treat_null_as_empty_string
    )]
    pub(crate) value: String,
    #[webidl(default = "", treat_null_as_empty_string)]
    pub(crate) priority: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration property")]
pub(crate) struct CssStyleDeclarationPropertyArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute CSS property method on 'CSSStyleDeclaration'"
    )]
    pub(crate) property: String,
}

#[derive(Clone)]
pub(crate) struct CssFontFaceEntry {
    pub(crate) family: String,
    pub(crate) source: String,
}

impl From<CssFontFace> for CssFontFaceEntry {
    fn from(face: CssFontFace) -> Self {
        Self {
            family: face.family,
            source: face.source,
        }
    }
}

pub(crate) fn parse_css_font_faces(css_text: &str) -> Vec<CssFontFaceEntry> {
    parse_font_faces(css_text)
        .into_iter()
        .map(CssFontFaceEntry::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_font_face_parser_uses_cssparser_rule_boundaries() {
        let entries = parse_css_font_faces(
            r#"
            .ignored { content: "@font-face { font-family: Bad; src: url(bad.woff2); }"; }
            @font-face {
                font-family: "A; B";
                src: url("data:font/woff2;base64;a;b");
            }
            @FONT-FACE {
                font-family: CaseFace;
                src: local("Case Face");
            }
            "#,
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].family, "A; B");
        assert_eq!(entries[0].source, r#"url("data:font/woff2;base64;a;b")"#);
        assert_eq!(entries[1].family, "CaseFace");
        assert_eq!(entries[1].source, r#"local("Case Face")"#);
    }

    #[test]
    fn css_font_face_parser_filters_invalid_and_incomplete_faces() {
        let entries = parse_css_font_faces(
            r#"
            @font-face { font-family: serif; src: url(generic.woff2); }
            @font-face { font-family: MissingSource; }
            @font-face { src: url(missing-family.woff2); }
            @font-face { font-family: Valid; src: url(valid.woff2); }
            "#,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].family, "Valid");
        assert_eq!(entries[0].source, r#"url("valid.woff2")"#);
    }
}
