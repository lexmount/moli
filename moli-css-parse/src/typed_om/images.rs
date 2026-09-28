use super::TypedStyleValueList;
use style::{
    properties::{ComputedValues, LonghandId, PropertyDeclaration},
    values::specified::Image,
};
use style_traits::ToCss;

/// A single native <image> iteration, including the distinct `none` keyword.
/// CSSImageValue is opaque: preserve the native AST's serialization without
/// interpreting function names or splitting commas inside image functions.
pub enum TypedImageValue {
    None,
    Image(String),
}

fn reify(image: &Image) -> TypedImageValue {
    match image {
        Image::None => TypedImageValue::None,
        _ => TypedImageValue::Image(image.to_css_string()),
    }
}

pub(super) fn from_declaration(declaration: &PropertyDeclaration) -> Option<Vec<TypedImageValue>> {
    match declaration {
        PropertyDeclaration::BackgroundImage(images) => Some(images.0.iter().map(reify).collect()),
        PropertyDeclaration::MaskImage(images) => Some(images.0.iter().map(reify).collect()),
        PropertyDeclaration::BorderImageSource(image)
        | PropertyDeclaration::ListStyleImage(image) => Some(vec![reify(image)]),
        _ => None,
    }
}

pub fn computed_typed_style_value_list(
    computed: &ComputedValues,
    id: LonghandId,
) -> Option<TypedStyleValueList> {
    if let Some(values) = computed.property_value_to_typed_value_list(id) {
        return Some(TypedStyleValueList::Native(values));
    }
    match id {
        LonghandId::BackgroundImage
        | LonghandId::MaskImage
        | LonghandId::BorderImageSource
        | LonghandId::ListStyleImage => {
            // Keep computed colors (including symbolic currentcolor), lengths
            // and absolute URLs. This converts
            // the computed AST directly; it does not parse CSSOM used values.
            let declaration = computed.computed_or_resolved_declaration(id, None);
            from_declaration(&declaration).map(TypedStyleValueList::Images)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_typed_style_value;
    use style::typed_om::TypedValue;

    #[test]
    fn native_image_iterations_preserve_nested_functions_none_and_urls() {
        let source = r#"linear-gradient(red, blue), none, image-set(url("a,b.png") 1x, radial-gradient(red, blue) 2x), url("end.png")"#;
        // mask-image requires browser prefs and is covered by the V8 fixture.
        let property = "background-image";
        let parsed = parse_typed_style_value(property, source, None)
            .unwrap_or_else(|| panic!("valid {property}: {source}"));
        let Some(TypedStyleValueList::Images(images)) = parsed.values else {
            panic!("native image projection: {property}");
        };
        assert_eq!(images.len(), 4);
        assert!(matches!(images[1], TypedImageValue::None));
        let texts: Vec<_> = images
            .iter()
            .map(|value| match value {
                TypedImageValue::None => "none",
                TypedImageValue::Image(text) => text,
            })
            .collect();
        assert_eq!(texts.join(", "), parsed.css_text);
        assert!(texts[2].contains("a,b.png"));
        assert!(texts[2].contains("radial-gradient("));
    }

    #[test]
    fn image_projection_keeps_property_grammar_and_unparsed_boundaries() {
        for property in [
            "background-image",
            "border-image-source",
            "list-style-image",
        ] {
            let parsed =
                parse_typed_style_value(property, r"\6c inear-gradient(red, blue)", None).unwrap();
            assert!(parsed.values.unwrap().is_single_image());
        }
        for property in ["background", "border-image"] {
            let parsed =
                parse_typed_style_value(property, "linear-gradient(red, blue)", None).unwrap();
            assert!(parsed.values.is_none(), "{property} must remain opaque");
        }
        for (property, source) in [
            ("background-image", "linear-gradient(var(--x), blue)"),
            ("--image", "linear-gradient(red, blue)"),
        ] {
            let parsed = parse_typed_style_value(property, source, None).unwrap();
            let values = parsed.values.unwrap();
            assert!(matches!(
                values.native().unwrap().values.as_slice(),
                [TypedValue::Unparsed(_)]
            ));
        }
        for (property, source) in [
            ("width", "linear-gradient(red, blue)"),
            ("border-image-source", "linear-gradient(red, blue), none"),
            ("background-image", "linear-gradient()"),
            ("background-image", "image-set(url(a) -1x)"),
        ] {
            assert!(
                parse_typed_style_value(property, source, None).is_none(),
                "invalid {property}: {source}"
            );
        }
    }
}
