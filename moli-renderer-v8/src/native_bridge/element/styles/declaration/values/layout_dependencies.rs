use super::*;

/// Dependencies of the renderer's existing numeric/custom-function CSSOM
/// projections. This only observes source/style values; it never asks layout
/// for a used value. Its caller must drop this query before a freshness flush.
///
/// Native Stylo container-query support is a separate boundary: an element-wide
/// USES_CONTAINER_UNITS flag would incorrectly turn an unrelated color read
/// into a layout demand, and does not describe supplemental numeric entries.
pub(in crate::native_bridge::element::styles) fn property_uses_container_geometry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> bool {
    let Some(property) = canonical_computed_cssom_query_property_name(property) else {
        return false;
    };
    let mut query = ContainerGeometryDependencies {
        runtime,
        visiting: Vec::new(),
    };
    query.property(handle, &property, 0)
}

struct ContainerGeometryDependencies<'a> {
    runtime: &'a JsContextHost,
    visiting: Vec<(DomHandle, String)>,
}

impl ContainerGeometryDependencies<'_> {
    fn property(&mut self, handle: DomHandle, property: &str, depth: usize) -> bool {
        if matches!(property, "animation" | "transition" | "animation-range") {
            return shorthand_longhands(property).is_some_and(|longhands| {
                longhands
                    .iter()
                    .any(|longhand| self.property(handle, longhand, depth + 1))
            });
        }
        if property.starts_with("--") {
            // The only renderer custom-property projection which samples a
            // container is the existing no-argument custom-function adapter.
            // A literal unregistered value such as --size: 1cqw stays a token
            // stream and has no public used-geometry requirement by itself.
            let read = ComputedStyleRead::new(self.runtime, handle);
            return computed_property_requires_stylesheet_sources(
                property,
                read.stylo_style.as_ref(),
            );
        }
        if css_numeric_computed_property_rule(property).is_none()
            && !matches!(
                property,
                "animation-timing-function"
                    | "transition-timing-function"
                    | "animation-range-start"
                    | "animation-range-end"
                    | "zoom"
                    | "text-size-adjust"
            )
        {
            return false;
        }
        if depth >= 64 {
            return true;
        }
        if self
            .visiting
            .iter()
            .any(|(node, name)| *node == handle && name == property)
        {
            return false;
        }
        self.visiting.push((handle, property.to_owned()));
        // Use the same inline/PDB source as the supplemental serializers. Raw
        // Stylo values cover values they receive after cascade/substitution;
        // unlike property(), raw_primary_property does not project used sizes.
        let value = inline_style_entry_for_inline_style(self.runtime, handle, property)
            .map(|entry| entry.value)
            .or_else(|| self.raw_property(handle, property));
        let result = value.is_some_and(|value| match css_wide_keyword(&value).as_deref() {
            Some("inherit") => inherited_style_parent(self.runtime, handle)
                .is_some_and(|parent| self.property(parent, property, depth + 1)),
            Some("unset") if property == "text-size-adjust" => {
                inherited_style_parent(self.runtime, handle)
                    .is_some_and(|parent| self.property(parent, property, depth + 1))
            }
            Some(_) => false,
            None => self.value(handle, &value, depth + 1),
        });
        self.visiting.pop();
        result
    }

    fn raw_property(&self, handle: DomHandle, property: &str) -> Option<String> {
        ComputedStyleRead::new(self.runtime, handle).raw_primary_property(property)
    }

    fn value(&mut self, handle: DomHandle, value: &str, depth: usize) -> bool {
        self.tokens(handle, &mut Parser::new(value), depth)
    }

    fn tokens(&mut self, handle: DomHandle, input: &mut Parser<'_>, depth: usize) -> bool {
        if depth >= 64 {
            return true;
        }
        let mut depends = false;
        while let Ok(token) = input.next().cloned() {
            match token {
                Token::Dimension { unit, .. } => {
                    depends |= ["cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax"]
                        .iter()
                        .any(|known| unit.eq_ignore_ascii_case(known));
                }
                Token::Function(name) if name.eq_ignore_ascii_case("var") => {
                    depends |= input
                        .parse_nested_block(|arguments| {
                            let name = arguments.expect_ident_cloned()?;
                            let key = (handle, name.to_string());
                            let cyclic = self.visiting.contains(&key);
                            let value = (!cyclic)
                                .then(|| self.raw_property(handle, &name))
                                .flatten();
                            if let Some(value) = value {
                                self.visiting.push(key);
                                let result = self.value(handle, &value, depth + 1);
                                self.visiting.pop();
                                // The fallback is not consumed by a present value.
                                // Still drain its tokens so parse_nested_block has
                                // the normal exhausted-input postcondition.
                                while arguments.next().is_ok() {}
                                return Ok::<_, cssparser::ParseError<()>>(result);
                            }
                            if arguments.try_parse(Parser::expect_comma).is_ok() {
                                return Ok(self.tokens(handle, arguments, depth + 1));
                            }
                            Ok(false)
                        })
                        .unwrap_or(true);
                }
                Token::Function(name) => {
                    let custom_function = name.starts_with("--");
                    depends |= input
                        .parse_nested_block(|nested| {
                            let nested = self.tokens(handle, nested, depth + 1);
                            Ok::<_, cssparser::ParseError<()>>(custom_function || nested)
                        })
                        .unwrap_or(true);
                }
                Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                    depends |= input
                        .parse_nested_block(|nested| {
                            Ok::<_, cssparser::ParseError<()>>(self.tokens(
                                handle,
                                nested,
                                depth + 1,
                            ))
                        })
                        .unwrap_or(true);
                }
                _ => {}
            }
        }
        depends
    }
}
