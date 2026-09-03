use std::{cmp::Ordering, sync::LazyLock};

use style::properties::{LonghandId, PropertyId, ShorthandId};
use style::stylesheets::{CssRuleType, CssRuleTypes};

use crate::css_style::mask_compat_property_name;

struct ComputedLonghandMetadata {
    longhands: Box<[LonghandId]>,
    first_vendor_index: usize,
}

impl ComputedLonghandMetadata {
    fn contains_name(&self, name: &str) -> bool {
        let range = if name.starts_with('-') {
            self.first_vendor_index..self.longhands.len()
        } else {
            0..self.first_vendor_index
        };
        self.longhands[range]
            .binary_search_by(|id| id.name().cmp(name))
            .is_ok()
    }
}

// Computed getPropertyValue() also accepts implemented compatibility
// shorthands and aliases that are deliberately absent from indexed names.
const COMPAT_COMPUTED_QUERY_PROPERTIES: &[&str] = &["animation-range", "mask"];

static COMPUTED_LONGHAND_METADATA: LazyLock<ComputedLonghandMetadata> = LazyLock::new(|| {
    super::ensure_stylo_browser_compat_prefs();

    // Match Servo's CSSStyleDeclaration metadata source. The `all` shorthand
    // contains every enabled longhand except direction and unicode-bidi.
    let mut longhands = ShorthandId::All
        .longhands()
        .filter(|id| stylo_property_is_enabled_for_style_rule(id.name()))
        .collect::<Vec<_>>();
    for id in [LonghandId::Direction, LonghandId::UnicodeBidi] {
        if stylo_property_is_enabled_for_style_rule(id.name()) {
            longhands.push(id);
        }
    }
    longhands.sort_unstable_by(|left, right| canonical_property_order(left.name(), right.name()));
    longhands.dedup_by(|left, right| left.name() == right.name());
    let first_vendor_index = longhands
        .iter()
        .position(|property| property.name().starts_with('-'))
        .unwrap_or(longhands.len());
    ComputedLonghandMetadata {
        longhands: longhands.into_boxed_slice(),
        first_vendor_index,
    }
});

fn canonical_property_order(left: &str, right: &str) -> Ordering {
    match (left.starts_with('-'), right.starts_with('-')) {
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
        _ => left.cmp(right),
    }
}

pub(crate) fn computed_longhand_count() -> usize {
    COMPUTED_LONGHAND_METADATA.longhands.len()
}

pub(crate) fn computed_longhand_name_at(index: usize) -> Option<&'static str> {
    COMPUTED_LONGHAND_METADATA
        .longhands
        .get(index)
        .copied()
        .map(|id| id.name())
}

pub(crate) fn computed_property_is_queryable(name: &str) -> bool {
    COMPUTED_LONGHAND_METADATA.contains_name(name)
        || stylo_property_is_enabled_for_style_rule(name)
        || mask_compat_property_name(name)
        || COMPAT_COMPUTED_QUERY_PROPERTIES.contains(&name)
}

fn stylo_property_is_enabled_for_style_rule(name: &str) -> bool {
    let Ok(PropertyId::NonCustom(id)) = PropertyId::parse_enabled_for_all_content(name) else {
        return false;
    };
    id.allowed_in_rule(CssRuleTypes::from(CssRuleType::Style))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use style::properties::PropertyId;

    use super::{
        computed_longhand_count, computed_longhand_name_at, computed_property_is_queryable,
    };

    #[test]
    fn computed_longhands_follow_stylo_metadata_and_cssom_order() {
        let names = (0..computed_longhand_count())
            .map(|index| computed_longhand_name_at(index).expect("metadata index must resolve"))
            .collect::<Vec<_>>();

        assert!(
            names.len() >= 266,
            "unexpectedly narrow metadata: {names:?}"
        );
        for representative in [
            "background-position-x",
            "direction",
            "font-size",
            "grid-auto-columns",
            "inline-size",
            "object-fit",
            "overflow-wrap",
            "pointer-events",
            "unicode-bidi",
            "white-space-collapse",
            "zoom",
        ] {
            assert!(names.contains(&representative), "missing {representative}");
        }
        for non_longhand in ["animation-range", "margin", "mask", "padding-block", "size"] {
            assert!(
                !names.contains(&non_longhand),
                "enumerated non-element longhand {non_longhand}"
            );
        }
        assert_eq!(
            names.iter().copied().collect::<HashSet<_>>().len(),
            names.len()
        );
        assert!(
            names
                .iter()
                .all(|name| computed_property_is_queryable(name)),
            "every enumerated computed longhand must be queryable"
        );

        let first_vendor = names
            .iter()
            .position(|name| name.starts_with('-'))
            .unwrap_or(names.len());
        assert!(
            first_vendor > 250,
            "ordinary longhands must precede the vendor tail: {first_vendor}"
        );
        assert!(
            names[..first_vendor]
                .iter()
                .all(|name| !name.starts_with('-'))
        );
        assert!(
            names[first_vendor..]
                .iter()
                .all(|name| name.starts_with('-'))
        );
        assert!(
            names[..first_vendor]
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(
            names[first_vendor..]
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
    }

    #[test]
    fn computed_query_gate_accepts_enabled_stylo_properties_without_opening_unknown_names() {
        crate::style_engine::ensure_stylo_browser_compat_prefs();
        for name in [
            "animation-range-end",
            "animation-range-start",
            "animation-timeline",
            "background-position-x",
            "column-span",
            "column-width",
            "font-variant-alternates",
            "font-variant-emoji",
            "font-variant-position",
            "grid-auto-columns",
            "object-fit",
            "overflow-wrap",
            "white-space-collapse",
            "zoom",
        ] {
            assert!(PropertyId::parse_enabled_for_all_content(name).is_ok());
            assert!(computed_property_is_queryable(name));
        }
        for name in [
            "-webkit-mask",
            "-webkit-mask-image",
            "animation-range",
            "zoom",
        ] {
            assert!(computed_property_is_queryable(name));
        }
        assert!(!computed_property_is_queryable("size"));
        assert!(!computed_property_is_queryable("moli-not-a-property"));
    }
}
