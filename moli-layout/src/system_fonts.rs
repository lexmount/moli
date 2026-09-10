// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{borrow::Cow, collections::HashMap};

use parley::{FontFamily, FontFamilyName, TextStyle, fontique::Collection};

use crate::stylo_to_parley::TextBrush;

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
mod matching;

pub(crate) struct SystemFontFamilyResolver {
    system_families: HashMap<String, String>,
    substitutions: HashMap<String, Option<String>>,
    #[cfg(test)]
    family_lookup_count: usize,
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
            #[cfg(test)]
            family_lookup_count: 0,
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
        let key = normalized_family_name(name);
        if let Some(cached) = self.substitutions.get(&key) {
            if let Some(substitute) = cached {
                *name = Cow::Owned(substitute.clone());
            }
            return;
        }
        #[cfg(test)]
        {
            self.family_lookup_count = self.family_lookup_count.saturating_add(1);
        }
        if collection.family_id(name).is_some() {
            self.substitutions.insert(key, None);
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
        let substitution = platform::match_family(family, &self.system_families);
        self.substitutions.insert(key, substitution.clone());
        substitution
    }
}

fn normalized_family_name(name: &str) -> String {
    name.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use std::{
        collections::HashMap,
        ffi::{CStr, CString},
        ptr::NonNull,
    };

    use fontconfig_sys::{
        FcConfig, FcConfigSubstitute, FcDefaultSubstitute, FcFontSet, FcFontSetDestroy, FcFontSort,
        FcInit, FcMatchPattern, FcPattern, FcPatternAddBool, FcPatternAddString, FcPatternCreate,
        FcPatternDestroy, FcPatternGetString, FcResultMatch,
        constants::{FC_FAMILY, FC_SCALABLE},
    };
    use parking_lot::Mutex;

    use super::{matching::accepts_substitution, normalized_family_name};

    static FONTCONFIG_LOCK: Mutex<()> = Mutex::new(());

    struct Pattern(NonNull<FcPattern>);

    impl Drop for Pattern {
        fn drop(&mut self) {
            unsafe { FcPatternDestroy(self.0.as_ptr()) };
        }
    }

    struct FontSet(NonNull<FcFontSet>);

    impl Drop for FontSet {
        fn drop(&mut self) {
            unsafe { FcFontSetDestroy(self.0.as_ptr()) };
        }
    }

    pub(super) fn match_family(
        family: &str,
        available_families: &HashMap<String, String>,
    ) -> Option<String> {
        if family.len() > 2048 {
            return None;
        }
        let _guard = FONTCONFIG_LOCK.lock();
        let requested = CString::new(family).ok()?;
        unsafe {
            if FcInit() == 0 {
                return None;
            }
            let pattern = Pattern(NonNull::new(FcPatternCreate())?);
            if FcPatternAddString(
                pattern.0.as_ptr(),
                FC_FAMILY.as_ptr(),
                requested.as_ptr().cast(),
            ) == 0
            {
                return None;
            }
            if FcPatternAddBool(pattern.0.as_ptr(), FC_SCALABLE.as_ptr(), 1) == 0 {
                return None;
            }
            if FcConfigSubstitute(std::ptr::null_mut(), pattern.0.as_ptr(), FcMatchPattern) == 0 {
                return None;
            }
            FcDefaultSubstitute(pattern.0.as_ptr());

            match_pattern(std::ptr::null_mut(), family, &pattern, available_families)
        }
    }

    // The config (or global config for null) must outlive the query. Taking a
    // configured pattern keeps native substitution separate from accepting the
    // resulting best match; tests can supply an isolated application font set.
    unsafe fn match_pattern(
        config: *mut FcConfig,
        family: &str,
        pattern: &Pattern,
        available_families: &HashMap<String, String>,
    ) -> Option<String> {
        unsafe {
            let configured = family_names(pattern.0.as_ptr()).into_iter().next()?;
            let mut result = FcResultMatch;
            let matches = FontSet(NonNull::new(FcFontSort(
                config,
                pattern.0.as_ptr(),
                0,
                std::ptr::null_mut(),
                &mut result,
            ))?);
            let font_set = matches.0.as_ref();
            for index in 0..font_set.nfont {
                let names = family_names(*font_set.fonts.add(index as usize));
                let Some(available) = names
                    .iter()
                    .find_map(|name| available_families.get(&normalized_family_name(name)))
                else {
                    continue;
                };
                // Fontconfig always finds a best-effort font. Like Chromium's
                // SkFontConfigInterfaceDirect, reject an unrelated first match
                // so the next CSS family can be tried instead. Do not search
                // later matches for an arbitrary acceptable alias.
                return names
                    .iter()
                    .any(|name| accepts_substitution(family, &configured, name))
                    .then(|| available.clone());
            }
            None
        }
    }

    // The caller retains the owning Pattern or FontSet for the entire read.
    unsafe fn family_names(pattern: *mut FcPattern) -> Vec<String> {
        let mut families = Vec::new();
        for index in 0..255 {
            let mut value = std::ptr::null_mut();
            if unsafe { FcPatternGetString(pattern, FC_FAMILY.as_ptr(), index, &mut value) }
                != FcResultMatch
                || value.is_null()
            {
                break;
            }
            families.push(
                unsafe { CStr::from_ptr(value.cast()) }
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        families
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use fontconfig_sys::{FcConfigAppFontAddFile, FcConfigCreate, FcConfigDestroy};

        struct Config(NonNull<FcConfig>);

        impl Drop for Config {
            fn drop(&mut self) {
                unsafe { FcConfigDestroy(self.0.as_ptr()) };
            }
        }

        #[test]
        fn native_font_match_rejects_appended_alias_but_accepts_configured_rename() {
            let _guard = FONTCONFIG_LOCK.lock();
            unsafe {
                assert_ne!(FcInit(), 0);
                let config = Config(NonNull::new(FcConfigCreate()).expect("fontconfig config"));
                let file = CString::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/moli-ahem.ttf"
                ))
                .unwrap();
                assert_ne!(
                    FcConfigAppFontAddFile(config.0.as_ptr(), file.as_ptr().cast()),
                    0
                );
                let families = HashMap::from([("moli ahem".to_owned(), "Moli Ahem".to_owned())]);

                for (requested, configured, expected) in [
                    ("Missing Font", ["Missing Font", "Moli Ahem"], None),
                    (
                        "Renamed Font",
                        ["Moli Ahem", "Renamed Font"],
                        Some("Moli Ahem"),
                    ),
                    (
                        "Moli Ahem",
                        ["Different Override", "Moli Ahem"],
                        Some("Moli Ahem"),
                    ),
                ] {
                    let pattern = Pattern(NonNull::new(FcPatternCreate()).expect("font pattern"));
                    for name in configured {
                        let name = CString::new(name).unwrap();
                        assert_ne!(
                            FcPatternAddString(
                                pattern.0.as_ptr(),
                                FC_FAMILY.as_ptr(),
                                name.as_ptr().cast()
                            ),
                            0
                        );
                    }
                    FcDefaultSubstitute(pattern.0.as_ptr());
                    assert_eq!(
                        match_pattern(config.0.as_ptr(), requested, &pattern, &families).as_deref(),
                        expected,
                        "native best-match acceptance for {requested}"
                    );
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod platform {
    pub(super) fn match_family(
        _family: &str,
        _available_families: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_family_lookup_is_cached() {
        let mut collection = Collection::new(parley::fontique::CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        let family = collection
            .family_names()
            .next()
            .expect("the system font collection should expose at least one family")
            .to_owned();
        let mut resolver = SystemFontFamilyResolver::new(&mut collection);

        for _ in 0..2 {
            let mut style = TextStyle {
                font_family: FontFamily::Single(FontFamilyName::Named(Cow::Owned(family.clone()))),
                ..TextStyle::default()
            };
            resolver.resolve_text_style(&mut collection, &mut style);
        }

        assert_eq!(
            resolver.family_lookup_count, 1,
            "a known family must not rebuild Fontique's normalized lookup key"
        );
    }

    #[test]
    fn missing_family_lookup_is_cached_without_replacing_the_css_fallback() {
        let mut collection = Collection::new(parley::fontique::CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        let mut resolver = SystemFontFamilyResolver::new(&mut collection);
        let family = "__moli_missing_font_family_cache_3bb2__";
        for _ in 0..2 {
            let mut style = TextStyle {
                font_family: FontFamily::Single(FontFamilyName::Named(Cow::Borrowed(family))),
                ..TextStyle::default()
            };
            resolver.resolve_text_style(&mut collection, &mut style);
            assert!(matches!(
                style.font_family,
                FontFamily::Single(FontFamilyName::Named(name)) if name == family
            ));
        }
        assert_eq!(resolver.family_lookup_count, 1);
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
