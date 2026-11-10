use style::{properties::PropertyDeclaration, values::specified::easing::TimingFunction};
use style_traits::ToCss;

/// One easing iteration. Functional values are opaque CSSStyleValues, while
/// specified keywords retain their type even when CSSOM serializes them as steps().
pub enum TypedEasingValue {
    Keyword(String),
    Function(String),
}

impl TypedEasingValue {
    pub fn declaration_text(&self) -> &str {
        match self {
            Self::Keyword(text) | Self::Function(text) => text,
        }
    }
}

fn reify(value: &TimingFunction) -> TypedEasingValue {
    match value {
        TimingFunction::StepStart => TypedEasingValue::Keyword("step-start".into()),
        TimingFunction::StepEnd => TypedEasingValue::Keyword("step-end".into()),
        TimingFunction::Keyword(keyword) => TypedEasingValue::Keyword(keyword.to_css_string()),
        _ => TypedEasingValue::Function(value.to_css_string()),
    }
}

pub(super) fn from_declaration(declaration: &PropertyDeclaration) -> Option<Vec<TypedEasingValue>> {
    match declaration {
        PropertyDeclaration::AnimationTimingFunction(values) => {
            Some(values.0.iter().map(reify).collect())
        }
        PropertyDeclaration::TransitionTimingFunction(values) => {
            Some(values.0.iter().map(reify).collect())
        }
        _ => None,
    }
}

pub(crate) fn from_block(
    block: &style::properties::PropertyDeclarationBlock,
    name: &str,
) -> Option<Vec<TypedEasingValue>> {
    let id = style::properties::PropertyId::parse_enabled_for_all_content(name).ok()?;
    from_declaration(block.get(id.as_shorthand().err()?)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_declaration_block, parse_typed_style_value};
    use style::{
        typed_om::{ToTyped, TypedValue},
        values::generics::easing::BeforeFlag,
    };

    #[test]
    fn step_keywords_keep_their_native_type_until_computation() {
        for property in ["animation-timing-function", "transition-timing-function"] {
            for (keyword, serialized) in
                [("step-start", "steps(1, start)"), ("step-end", "steps(1)")]
            {
                let parsed = parse_typed_style_value(property, keyword, None).unwrap();
                assert_eq!(parsed.css_text, serialized);
                assert!(
                    matches!(parsed.values.unwrap().native().unwrap().values.as_slice(), [TypedValue::Keyword(value)] if value.0 == keyword)
                );

                let mut block =
                    parse_declaration_block(&format!("{property}: {keyword}; color: red"));
                block.set_property("color", "blue", false);
                let values = block.typed_easing_values(property).unwrap();
                assert!(matches!(&values[0], TypedEasingValue::Keyword(value) if value == keyword));
                assert_eq!(block.property_value(property).as_deref(), Some(serialized));
                // Replacing the declaration with a function must not retain
                // its former keyword type merely because CSSOM text is equal.
                block.set_property(property, serialized, false);
                assert!(
                    matches!(&block.typed_easing_values(property).unwrap()[0], TypedEasingValue::Function(value) if value == serialized)
                );
            }
        }
        for (keyword, specified) in [
            ("step-start", TimingFunction::StepStart),
            ("step-end", TimingFunction::StepEnd),
        ] {
            let computed = specified.to_computed_value_without_context().unwrap();
            assert!(computed.to_typed_value_list().is_none(), "{keyword}");
            assert_eq!(specified.to_css_string(), computed.to_css_string());
            let reconstructed =
                <TimingFunction as style::values::computed::ToComputedValue>::from_computed_value(
                    &computed,
                );
            assert!(matches!(reconstructed, TimingFunction::Steps(..)));
            for progress in [0.0, 0.25, 0.5, 0.99, 1.0] {
                let expected = if keyword == "step-start" || progress == 1.0 {
                    1.0
                } else {
                    0.0
                };
                assert_eq!(
                    computed.calculate_output(progress, BeforeFlag::Unset, 1e-7),
                    expected
                );
            }
            assert_eq!(computed.calculate_output(0.0, BeforeFlag::Set, 1e-7), 0.0);
        }
    }

    #[test]
    fn easing_iterations_keep_keyword_and_function_boundaries() {
        for property in ["animation-timing-function", "transition-timing-function"] {
            let parsed = parse_typed_style_value(
                property,
                "step-start, steps(4, end), step-end, ease",
                None,
            )
            .unwrap();
            let Some(super::super::TypedStyleValueList::Easings(values)) = parsed.values else {
                panic!("mixed easing list");
            };
            assert_eq!(values.len(), 4);
            assert!(
                matches!(&values[0], TypedEasingValue::Keyword(value) if value == "step-start")
            );
            assert!(matches!(&values[1], TypedEasingValue::Function(value) if value == "steps(4)"));
            assert!(matches!(&values[2], TypedEasingValue::Keyword(value) if value == "step-end"));
            assert!(matches!(&values[3], TypedEasingValue::Keyword(value) if value == "ease"));
            for invalid in [
                "step-middle",
                "step-start()",
                "steps(1, jump-none)",
                "step-end,",
            ] {
                assert!(
                    parse_typed_style_value(property, invalid, None).is_none(),
                    "{property}: {invalid}"
                );
            }
        }
    }
}
