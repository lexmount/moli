// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{borrow::Cow, collections::HashMap};

use parley::{FontFamily, FontFamilyName, TextStyle, fontique::Collection};

use crate::stylo_to_parley::TextBrush;

pub(crate) struct SystemFontFamilyResolver {
    system_families: HashMap<String, String>,
    substitutions: HashMap<String, Option<String>>,
}

impl SystemFontFamilyResolver {
    pub(crate) fn new(collection: &mut Collection) -> Self {
        let system_families = collection
            .family_names()
            .map(|name| (normalized_family_name(name), name.to_owned()))
            .collect();
        Self {
            system_families,
            substitutions: HashMap::new(),
        }
    }

    pub(crate) fn resolve_text_style(
        &mut self,
        collection: &mut Collection,
        style: &mut TextStyle<'static, 'static, TextBrush>,
    ) {
        match &mut style.font_family {
            FontFamily::Single(family) => self.resolve_family(collection, family),
            FontFamily::List(families) => {
                for family in Cow::to_mut(families) {
                    self.resolve_family(collection, family);
                }
            }
            FontFamily::Source(_) => {}
        }
    }

    fn resolve_family(
        &mut self,
        collection: &mut Collection,
        family: &mut FontFamilyName<'static>,
    ) {
        let FontFamilyName::Named(name) = family else {
            return;
        };
        if collection.family_id(name).is_some() {
            return;
        }
        let Some(substitute) = self.resolve_missing_family(name) else {
            return;
        };
        *name = Cow::Owned(substitute);
    }

    fn resolve_missing_family(&mut self, family: &str) -> Option<String> {
        let key = normalized_family_name(family);
        if let Some(cached) = self.substitutions.get(&key) {
            return cached.clone();
        }
        let substitution = platform::explicit_substitution_families(family)
            .into_iter()
            .find_map(|candidate| {
                self.system_families
                    .get(&normalized_family_name(&candidate))
                    .cloned()
            });
        self.substitutions.insert(key, substitution.clone());
        substitution
    }
}

fn normalized_family_name(name: &str) -> String {
    name.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn explicit_substitution_prefix<'a>(
    requested: &'a [String],
    default_families: &[String],
) -> &'a [String] {
    let common_suffix_len = requested
        .iter()
        .rev()
        .zip(default_families.iter().rev())
        .take_while(|(left, right)| left.eq_ignore_ascii_case(right))
        .count();
    &requested[..requested.len().saturating_sub(common_suffix_len)]
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use std::{
        ffi::{CStr, CString},
        ptr::NonNull,
        sync::OnceLock,
    };

    use fontconfig_sys::{
        FcConfigSubstitute, FcDefaultSubstitute, FcInit, FcMatchPattern, FcPattern,
        FcPatternAddString, FcPatternCreate, FcPatternDestroy, FcPatternGetString, FcResultMatch,
        constants::FC_FAMILY,
    };
    use parking_lot::Mutex;

    use super::explicit_substitution_prefix;

    const MISSING_FAMILY_SENTINEL: &str = "__moli_missing_font_family_7f41c8d2__";
    static FONTCONFIG_LOCK: Mutex<()> = Mutex::new(());
    static DEFAULT_FAMILIES: OnceLock<Option<Vec<String>>> = OnceLock::new();

    struct Pattern(NonNull<FcPattern>);

    impl Drop for Pattern {
        fn drop(&mut self) {
            unsafe { FcPatternDestroy(self.0.as_ptr()) };
        }
    }

    pub(super) fn explicit_substitution_families(family: &str) -> Vec<String> {
        let Some(default_families) = DEFAULT_FAMILIES
            .get_or_init(|| substituted_families(MISSING_FAMILY_SENTINEL))
            .as_deref()
        else {
            return Vec::new();
        };
        let Some(requested) = substituted_families(family) else {
            return Vec::new();
        };
        explicit_substitution_prefix(&requested, default_families)
            .iter()
            .filter(|candidate| !candidate.eq_ignore_ascii_case(family))
            .cloned()
            .collect()
    }

    fn substituted_families(family: &str) -> Option<Vec<String>> {
        let _guard = FONTCONFIG_LOCK.lock();
        let family = CString::new(family).ok()?;
        unsafe {
            if FcInit() == 0 {
                return None;
            }
            let pattern = Pattern(NonNull::new(FcPatternCreate())?);
            if FcPatternAddString(
                pattern.0.as_ptr(),
                FC_FAMILY.as_ptr(),
                family.as_ptr().cast(),
            ) == 0
            {
                return None;
            }
            if FcConfigSubstitute(std::ptr::null_mut(), pattern.0.as_ptr(), FcMatchPattern) == 0 {
                return None;
            }
            FcDefaultSubstitute(pattern.0.as_ptr());

            let mut families = Vec::new();
            for index in 0.. {
                let mut value = std::ptr::null_mut();
                if FcPatternGetString(pattern.0.as_ptr(), FC_FAMILY.as_ptr(), index, &mut value)
                    != FcResultMatch
                {
                    break;
                }
                if value.is_null() {
                    return None;
                }
                families.push(CStr::from_ptr(value.cast()).to_string_lossy().into_owned());
            }
            (!families.is_empty()).then_some(families)
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod platform {
    pub(super) fn explicit_substitution_families(_family: &str) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_fontconfig_defaults_are_not_treated_as_named_aliases() {
        let defaults = [
            "missing".to_owned(),
            "DejaVu Sans".to_owned(),
            "Noto Sans".to_owned(),
        ];
        let arial = [
            "Arial".to_owned(),
            "Arimo".to_owned(),
            "Liberation Sans".to_owned(),
            "DejaVu Sans".to_owned(),
            "Noto Sans".to_owned(),
        ];
        let unknown = [
            "unknown".to_owned(),
            "DejaVu Sans".to_owned(),
            "Noto Sans".to_owned(),
        ];

        assert_eq!(explicit_substitution_prefix(&arial, &defaults), &arial[..3]);
        assert_eq!(
            explicit_substitution_prefix(&unknown, &defaults),
            &unknown[..1]
        );
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn fontconfig_aliases_do_not_convert_an_unknown_family_to_default_sans() {
        let mut collection = Collection::new(parley::fontique::CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        let mut resolver = SystemFontFamilyResolver::new(&mut collection);
        assert_eq!(
            resolver.resolve_missing_family("__moli_another_missing_family_92a6__"),
            None
        );

        if resolver
            .system_families
            .contains_key(&normalized_family_name("Liberation Sans"))
        {
            assert_eq!(
                resolver.resolve_missing_family("Arial").as_deref(),
                Some("Liberation Sans")
            );
        }
    }
}
