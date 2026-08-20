use moli_selector::stylo_attribute_change_can_use_retained_invalidator;

use super::StyleAttributeImpact;

pub(super) fn attribute_effect_can_use_retained_stylo_invalidator(name: &str) -> bool {
    stylo_attribute_change_can_use_retained_invalidator(
        name,
        attribute_has_non_css_runtime_side_effect(name),
    )
}

pub(super) fn attribute_has_non_css_runtime_side_effect(name: &str) -> bool {
    StyleAttributeImpact::for_attribute_name(name).has_non_css_runtime_side_effect()
}
