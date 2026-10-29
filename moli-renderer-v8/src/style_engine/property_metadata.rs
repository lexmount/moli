use std::{borrow::Cow, cmp::Ordering, sync::LazyLock};

use style::properties::{LonghandId, PropertyId, ShorthandId};
use style::stylesheets::{CssRuleType, CssRuleTypes};

use crate::css_style::{mask_compat_property_name, stylo_property_is_chromium_exposed};

struct ComputedLonghandMetadata {
    longhands: Box<[&'static str]>,
    first_vendor_index: usize,
}

impl ComputedLonghandMetadata {
    fn contains_name(&self, name: &str) -> bool {
        let range = if name.starts_with('-') {
            self.first_vendor_index..self.longhands.len()
        } else {
            0..self.first_vendor_index
        };
        self.longhands[range].binary_search(&name).is_ok()
    }
}

// Computed getPropertyValue() also accepts implemented compatibility
// shorthands and aliases that are deliberately absent from indexed names.
const COMPAT_COMPUTED_QUERY_PROPERTIES: &[&str] = &["animation-range", "mask"];

static COMPUTED_LONGHAND_METADATA: LazyLock<ComputedLonghandMetadata> = LazyLock::new(|| {
    super::ensure_stylo_browser_compat_prefs();

    // Use Stylo's longhand metadata with the same public-property policy as the
    // CSSStyleDeclaration accessors. The `all` shorthand contains every enabled
    // longhand except direction and unicode-bidi.
    let mut longhands = ShorthandId::All
        .longhands()
        .filter(|id| {
            stylo_property_is_enabled_for_style_rule(id.name())
                && stylo_property_is_chromium_exposed(id.name())
        })
        .map(|id| id.name())
        .collect::<Vec<_>>();
    for id in [LonghandId::Direction, LonghandId::UnicodeBidi] {
        if stylo_property_is_enabled_for_style_rule(id.name()) {
            longhands.push(id.name());
        }
    }
    // CSS Masking exposes one mask-position longhand. Stylo represents it with
    // private x/y longhands and a shorthand, so expose its public name here.
    if stylo_property_is_enabled_for_style_rule("mask-position") {
        longhands.push("mask-position");
    }
    longhands.sort_unstable_by(|left, right| canonical_property_order(left, right));
    longhands.dedup();
    let first_vendor_index = longhands
        .iter()
        .position(|property| property.starts_with('-'))
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
    COMPUTED_LONGHAND_METADATA.longhands.get(index).copied()
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

static TRANSITION_ALL_LONGHANDS: LazyLock<Box<[&'static str]>> = LazyLock::new(|| {
    super::ensure_stylo_browser_compat_prefs();
    let mut names = ShorthandId::All
        .longhands()
        .filter(|id| {
            !id.is_logical() && id.is_animatable() && computed_property_is_queryable(id.name())
        })
        .map(|id| id.name())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    names.into_boxed_slice()
});

/// Expand each transition declaration into physical longhands before choosing
/// the last matching list index. `all` and shorthands use the same Stylo metadata.
pub(crate) fn transition_property_longhands(
    declarations: &[&str],
    writing_mode: style::logical_geometry::WritingMode,
) -> Vec<(Cow<'static, str>, usize)> {
    super::ensure_stylo_browser_compat_prefs();
    if declarations.len() == 1 && declarations[0].eq_ignore_ascii_case("all") {
        return TRANSITION_ALL_LONGHANDS
            .iter()
            .map(|&name| (Cow::Borrowed(name), 0))
            .collect();
    }
    let mut matched = std::collections::BTreeMap::new();
    for (index, name) in declarations.iter().enumerate() {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            continue;
        };
        match property {
            PropertyId::Custom(_) => {
                matched.insert(Cow::Owned((*name).to_owned()), index);
            }
            PropertyId::NonCustom(id) => {
                let mut admit = |id: LonghandId| {
                    let id = id.to_physical(writing_mode);
                    if id.is_animatable() && computed_property_is_queryable(id.name()) {
                        matched.insert(Cow::Borrowed(id.name()), index);
                    }
                };
                match id.longhand_or_shorthand() {
                    Ok(id) => admit(id),
                    Err(ShorthandId::All) => {
                        for &name in TRANSITION_ALL_LONGHANDS.iter() {
                            matched.insert(Cow::Borrowed(name), index);
                        }
                    }
                    Err(shorthand) => {
                        for id in shorthand.longhands() {
                            admit(id);
                        }
                    }
                }
            }
        }
    }
    matched.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use style::properties::PropertyId;

    use super::{
        computed_longhand_count, computed_longhand_name_at, computed_property_is_queryable,
    };

    #[test]
    fn transition_all_cache_matches_physical_longhand_metadata() {
        use style::properties::ShorthandId;
        crate::style_engine::ensure_stylo_browser_compat_prefs();
        let mode = style::logical_geometry::WritingMode::horizontal_tb();
        let expected = ShorthandId::All
            .longhands()
            .map(|id| id.to_physical(mode))
            .filter(|id| id.is_animatable() && super::computed_property_is_queryable(id.name()))
            .map(|id| id.name())
            .collect::<std::collections::BTreeSet<_>>();
        let cached = super::transition_property_longhands(&["all"], mode);
        assert_eq!(
            cached
                .iter()
                .map(|(name, _)| name.as_ref())
                .collect::<Vec<_>>(),
            expected.into_iter().collect::<Vec<_>>()
        );
        assert!(
            cached
                .iter()
                .all(|(name, index)| matches!(name, std::borrow::Cow::Borrowed(_)) && *index == 0)
        );
    }

    #[test]
    fn transition_matching_expands_shorthands_and_preserves_last_list_index() {
        let writing_mode = style::logical_geometry::WritingMode::horizontal_tb();
        let matched = super::transition_property_longhands(
            &["all", "margin", "margin-left", "font-size", "--x"],
            writing_mode,
        );
        let index = |name: &str| matched.iter().find(|(p, _)| p == name).map(|(_, i)| *i);
        assert_eq!(index("font-size"), Some(3));
        assert_eq!(index("margin-left"), Some(2));
        assert_eq!(index("margin-right"), Some(1));
        assert_eq!(index("--x"), Some(4));
        assert_eq!(index("margin"), None);
        assert_eq!(index("all"), None);
        let logical =
            super::transition_property_longhands(&["margin-inline", "margin-left"], writing_mode);
        assert_eq!(
            logical,
            vec![("margin-left".into(), 1), ("margin-right".into(), 0)]
        );
    }

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
            "mask-position",
            "object-fit",
            "overflow-wrap",
            "pointer-events",
            "unicode-bidi",
            "white-space-collapse",
            "zoom",
        ] {
            assert!(names.contains(&representative), "missing {representative}");
        }
        for excluded in [
            "animation-range",
            "margin",
            "mask",
            "padding-block",
            "size",
            "mask-position-x",
            "mask-position-y",
            "-moz-default-appearance",
            "-moz-min-font-size-ratio",
            "-x-text-scale",
        ] {
            assert!(
                !names.contains(&excluded),
                "enumerated non-public longhand {excluded}"
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
