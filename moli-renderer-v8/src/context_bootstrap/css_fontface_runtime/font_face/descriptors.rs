use super::*;

#[derive(Clone, Copy)]
pub(super) struct FontFaceDescriptor {
    pub(super) slot: &'static str,
    css_name: &'static str,
    initial: &'static str,
}

impl FontFaceDescriptor {
    const fn new(slot: &'static str, css_name: &'static str, initial: &'static str) -> Self {
        Self {
            slot,
            css_name,
            initial,
        }
    }

    pub(super) fn parse(self, value: &str) -> Option<String> {
        if self.slot == FONT_FACE_FAMILY_SLOT {
            return Some(value.to_owned());
        }
        if self.slot == FONT_FACE_VARIANT_SLOT {
            // The legacy FontFace variant attribute uses the CSS 2.1 values;
            // Stylo no longer exposes font-variant as an @font-face descriptor.
            let mut input = cssparser::ParserInput::new(value);
            let mut parser = cssparser::Parser::new(&mut input);
            let ident = parser.expect_ident().ok()?.to_ascii_lowercase();
            parser.expect_exhausted().ok()?;
            return matches!(ident.as_str(), "normal" | "small-caps").then_some(ident);
        }
        moli_css_parse::parse_font_face_descriptor_entry_with_stylo(self.css_name, value)
            .map(|entry| entry.value)
    }
}

// Keep the order aligned with the accessor callback-data indexes.
pub(super) const FONT_FACE_WRITABLE_ATTRIBUTES: &[FontFaceDescriptor] = &[
    FontFaceDescriptor::new(FONT_FACE_FAMILY_SLOT, "font-family", ""),
    FontFaceDescriptor::new(FONT_FACE_STYLE_SLOT, "font-style", "normal"),
    FontFaceDescriptor::new(FONT_FACE_WEIGHT_SLOT, "font-weight", "normal"),
    FontFaceDescriptor::new(FONT_FACE_STRETCH_SLOT, "font-stretch", "normal"),
    FontFaceDescriptor::new(FONT_FACE_VARIANT_SLOT, "font-variant", "normal"),
    FontFaceDescriptor::new(
        FONT_FACE_FEATURE_SETTINGS_SLOT,
        "font-feature-settings",
        "normal",
    ),
    FontFaceDescriptor::new(
        FONT_FACE_VARIATION_SETTINGS_SLOT,
        "font-variation-settings",
        "normal",
    ),
    FontFaceDescriptor::new(FONT_FACE_DISPLAY_SLOT, "font-display", "auto"),
    FontFaceDescriptor::new(FONT_FACE_UNICODE_RANGE_SLOT, "unicode-range", "U+0-10FFFF"),
    FontFaceDescriptor::new(FONT_FACE_ASCENT_OVERRIDE_SLOT, "ascent-override", "normal"),
    FontFaceDescriptor::new(
        FONT_FACE_DESCENT_OVERRIDE_SLOT,
        "descent-override",
        "normal",
    ),
    FontFaceDescriptor::new(
        FONT_FACE_LINE_GAP_OVERRIDE_SLOT,
        "line-gap-override",
        "normal",
    ),
    FontFaceDescriptor::new(FONT_FACE_SIZE_ADJUST_SLOT, "size-adjust", "100%"),
];

// WebIDL reads dictionary members in lexicographic order, and must finish
// their conversion before the constructor starts parsing CSS or copying bytes.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FontFaceDescriptors")]
struct FontFaceDescriptors {
    ascent_override: Option<String>,
    descent_override: Option<String>,
    display: Option<String>,
    feature_settings: Option<String>,
    line_gap_override: Option<String>,
    size_adjust: Option<String>,
    stretch: Option<String>,
    style: Option<String>,
    unicode_range: Option<String>,
    variant: Option<String>,
    variation_settings: Option<String>,
    weight: Option<String>,
}

pub(super) fn parse_font_face_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<([String; 12], bool)> {
    let parsed = match webidl::parse_dictionary::<FontFaceDescriptors>(
        scope,
        value,
        webidl::Context::argument("FontFace", 3),
    ) {
        Ok(parsed) => parsed.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let raw = [
        parsed.style,
        parsed.weight,
        parsed.stretch,
        parsed.variant,
        parsed.feature_settings,
        parsed.variation_settings,
        parsed.display,
        parsed.unicode_range,
        parsed.ascent_override,
        parsed.descent_override,
        parsed.line_gap_override,
        parsed.size_adjust,
    ];
    let mut invalid = false;
    let values = std::array::from_fn(|index| {
        let descriptor = FONT_FACE_WRITABLE_ATTRIBUTES[index + 1];
        let Some(raw) = raw[index].as_deref() else {
            return descriptor.initial.to_owned();
        };
        descriptor.parse(raw).unwrap_or_else(|| {
            invalid = true;
            String::new()
        })
    });
    Some((values, invalid))
}
