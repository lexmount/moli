// SPDX-License-Identifier: MIT OR Apache-2.0

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
