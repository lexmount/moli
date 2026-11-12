// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Support for script/language based font fallback.

use super::{Language, Script, family::FamilyId};
use alloc::vec::Vec;
use hashbrown::HashMap;
use smallvec::SmallVec;

type FamilyList = SmallVec<[FamilyId; 1]>;

/// Maps script and language pairs to font families.
#[derive(Clone, Default, Debug)]
pub struct FallbackMap {
    fallbacks: HashMap<Script, PerScript>,
}

impl FallbackMap {
    /// Returns the font fallback families for the given key.
    pub fn get(&self, key: impl Into<FallbackKey>) -> Option<&[FamilyId]> {
        let key = key.into();
        let entry = self.fallbacks.get(&key.script)?;
        if key.is_default() {
            Some(entry.default.as_ref()?.as_slice())
        } else {
            for other in &entry.others {
                if key.locale() == Some(other.0) {
                    return Some(other.1.as_slice());
                }
            }
            None
        }
    }

    /// Inserts or replaces the fallback families for the given script and
    /// language.
    ///
    /// Returns `false` if we don't track that particular pair of script and
    /// language.
    pub fn set(
        &mut self,
        key: impl Into<FallbackKey>,
        families: impl Iterator<Item = FamilyId>,
    ) -> bool {
        self.set_or_append(key, families, true)
    }

    /// Inserts or appends the fallback families for the given script and
    /// language.
    ///
    /// Returns `false` if we don't track that particular pair of script and
    /// language.
    pub fn append(
        &mut self,
        key: impl Into<FallbackKey>,
        families: impl Iterator<Item = FamilyId>,
    ) -> bool {
        self.set_or_append(key, families, false)
    }

    fn set_or_append(
        &mut self,
        key: impl Into<FallbackKey>,
        families: impl Iterator<Item = FamilyId>,
        do_set: bool,
    ) -> bool {
        let key = key.into();
        let script = key.script;
        if key.is_tracked() {
            if key.is_default() {
                let existing_families = self
                    .fallbacks
                    .entry(script)
                    .or_default()
                    .default
                    .get_or_insert(SmallVec::default());
                if do_set {
                    existing_families.clear();
                }
                existing_families.extend(families);
                true
            } else {
                let locale = key
                    .locale
                    .expect("non-default fallback keys must have a locale");
                let script_fallbacks = self.fallbacks.entry(script).or_default();
                if let Some(existing_families) = script_fallbacks
                    .others
                    .iter_mut()
                    .find(|x| x.0 == locale)
                    .map(|x| &mut x.1)
                {
                    if do_set {
                        existing_families.clear();
                    }
                    existing_families.extend(families);
                } else {
                    script_fallbacks.others.push((locale, families.collect()));
                }
                true
            }
        } else {
            false
        }
    }
}

/// Describes a selector for fallback families.
///
/// This is a [`Script`] and optionally, a `locale`, represented as a [`Language`].
///
/// It can be constructed directly via [`FallbackKey::new`] or any of
/// a variety of `From` implementations to improve the ease of use.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct FallbackKey {
    script: Script,
    locale: Option<Language>,
    is_default: bool,
    is_tracked: bool,
}

impl FallbackKey {
    /// Creates a new fallback key from the given script and locale.
    pub fn new(script: impl Into<Script>, locale: Option<&Language>) -> Self {
        let script = script.into();
        let (locale, is_default, is_tracked) = match canonical_locale(script, locale) {
            Some((is_default, locale)) => (locale, is_default, true),
            None => (None, true, false),
        };
        Self {
            script,
            locale,
            is_default,
            is_tracked,
        }
    }

    /// Returns the requested script.
    pub fn script(&self) -> Script {
        self.script
    }

    /// Returns a normalized version of the requested locale.
    pub fn locale(&self) -> Option<Language> {
        self.locale
    }

    /// Returns the requested locale as a string, if present.
    pub fn locale_str(&self) -> Option<&str> {
        self.locale.as_ref().map(Language::as_str)
    }

    /// Returns `true` if the requested locale is considered the "default"
    /// language/region for the requested script.
    ///
    /// Always returns `true` when [`locale`](Self::locale) returns `None`.
    pub fn is_default(&self) -> bool {
        self.is_default
    }

    /// Returns `true` if the requested script and locale pair are actually
    /// tracked for fallback.
    pub fn is_tracked(&self) -> bool {
        self.is_tracked
    }
}

impl<S> From<(S, &str)> for FallbackKey
where
    S: Into<Script>,
{
    fn from(value: (S, &str)) -> Self {
        let locale = Language::parse(value.1).ok();
        Self::new(value.0, locale.as_ref())
    }
}

impl<S> From<(S, &Language)> for FallbackKey
where
    S: Into<Script>,
{
    fn from(value: (S, &Language)) -> Self {
        Self::new(value.0, Some(value.1))
    }
}

impl From<Script> for FallbackKey {
    fn from(value: Script) -> Self {
        Self::new(value, None)
    }
}

#[derive(Clone, Default, Debug)]
struct PerScript {
    default: Option<FamilyList>,
    others: Vec<(Language, FamilyList)>,
}

fn canonical_locale(script: Script, locale: Option<&Language>) -> Option<(bool, Option<Language>)> {
    let Some(locale) = locale else {
        return Some((true, None));
    };
    let lang = locale.language();
    let region = locale.region().unwrap_or_default();
    let (is_default, token) = match &script.to_bytes() {
        b"Arab" => match (lang, region) {
            ("ar", "") => (true, "ar"),
            ("ar", "IR") => (false, "ar-IR"),
            ("fa", "") => (false, "fa"),
            ("ks", "") => (false, "ks"),
            ("ku", "IQ") => (false, "ku-IQ"),
            ("ku", "IR") => (false, "ku-IR"),
            ("la", "") => (false, "la"),
            ("ota", "") => (false, "ota"),
            ("pa", "PK") => (false, "pa-PK"),
            ("ps", "AF") => (false, "ps-AF"),
            ("ps", "PK") => (false, "ps-PK"),
            ("sd", "") => (false, "sd"),
            ("ug", "") => (false, "ug"),
            ("ur", "") => (false, "ur"),
            _ => return None,
        },
        b"Beng" => match (lang, region) {
            ("bn", "") => (true, "bn"),
            ("as", "") => (false, "as"),
            ("mni", "") => (false, "mni"),
            _ => return None,
        },
        b"Deva" => match (lang, region) {
            ("hi", "") => (true, "hi"),
            ("bh", "") => (false, "bh"),
            ("bho", "") => (false, "bho"),
            ("brx", "") => (false, "brx"),
            ("doi", "") => (false, "doi"),
            ("hne", "") => (false, "hne"),
            ("kok", "") => (false, "kok"),
            ("mai", "") => (false, "mai"),
            ("mr", "") => (false, "mr"),
            ("bne", "") => (false, "bne"),
            ("sa", "") => (false, "sa"),
            ("sat", "") => (false, "sat"),
            _ => return None,
        },
        b"Ethi" => match (lang, region) {
            ("gez", "") => (true, "gez"),
            ("am", "") => (false, "am"),
            ("byn", "") => (false, "byn"),
            ("sid", "") => (false, "sid"),
            ("ti", "ER") => (false, "ti-ER"),
            ("ti", "ET") => (false, "ti-ET"),
            ("tig", "") => (false, "tig"),
            ("wal", "") => (false, "wal"),
            _ => return None,
        },
        b"Hani" => match lang {
            "ja" => (false, "ja"),
            "ko" => (false, "ko"),
            "zh" => {
                match region {
                    "HK" => (false, "zh-HK"),
                    "TW" => (false, "zh-TW"),
                    "MO" => (false, "zh-MO"),
                    "SG" => (false, "zh-SG"),
                    _ => {
                        if locale.script() == Some("Hant") {
                            (false, "zh-TW")
                        } else {
                            // Default to simplified Chinese
                            (true, "zh-CN")
                        }
                    }
                }
            }
            _ => return None,
        },
        b"Hebr" => match (lang, region) {
            ("he", "") => (true, "he"),
            ("yi", "") => (false, "yi"),
            _ => return None,
        },
        b"Tibt" => match (lang, region) {
            ("bo", "") => (true, "bo"),
            ("dz", "") => (false, "dz"),
            _ => return None,
        },
        _ => return None,
    };
    Some((
        is_default,
        Some(Language::parse(token).expect("valid canonical locale")),
    ))
}
