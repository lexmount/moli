use crate::css_style::value_dependencies::{
    ValueDependency, is_container_unit, value_has_dependency,
};

use super::*;

/// Dependencies of the renderer's existing numeric/custom-function CSSOM
/// projections. This only observes source/style values; it never asks layout
/// for a used value. Its caller must drop this query before a freshness flush.
pub(in crate::native_bridge::element::styles) fn property_uses_container_geometry(
    runtime: &PageScriptHost,
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
    runtime: &'a PageScriptHost,
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
            None => self.value(handle, &value, &mut Vec::new(), depth + 1),
        });
        self.visiting.pop();
        result
    }

    fn raw_property(&self, handle: DomHandle, property: &str) -> Option<String> {
        ComputedStyleRead::new(self.runtime, handle).raw_primary_property(property)
    }

    fn value(
        &self,
        handle: DomHandle,
        value: &str,
        visiting: &mut Vec<String>,
        depth: usize,
    ) -> bool {
        if depth >= 64 {
            return true;
        }
        value_has_dependency(value, &mut |dependency| match dependency {
            ValueDependency::Variable(name) => {
                if visiting.iter().any(|candidate| candidate == name) {
                    return None;
                }
                let value = self.raw_property(handle, name)?;
                visiting.push(name.to_owned());
                let result = self.value(handle, &value, visiting, depth + 1);
                visiting.pop();
                Some(result)
            }
            ValueDependency::Dimension(unit) => Some(is_container_unit(unit)),
            ValueDependency::Function(name) => Some(name.starts_with("--")),
            _ => None,
        })
    }
}
