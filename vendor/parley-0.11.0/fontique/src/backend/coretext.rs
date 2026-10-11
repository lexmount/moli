// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{
    FallbackKey, FamilyId, FamilyInfo, FamilyNameMap, GenericFamily, GenericFamilyMap, ScriptExt,
    scan,
};
use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr::{null, null_mut};
use hashbrown::{HashMap, HashSet};
use objc2_core_foundation::{
    CFArray, CFDictionary, CFRange, CFRetained, CFString, CFType, CFURL, CFURLPathStyle,
};
use objc2_core_text::{
    CTFont, CTFontCollection, CTFontDescriptor, CTFontUIFontType, kCTFontURLAttribute,
};
use objc2_foundation::{
    NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains,
};
use parlance::Script;
use std::path::{Path, PathBuf};

const DEFAULT_GENERIC_FAMILIES: &[(GenericFamily, &[&str])] = &[
    (GenericFamily::Serif, &["Times", "Times New Roman"]),
    (GenericFamily::SansSerif, &["Helvetica"]),
    (GenericFamily::Monospace, &["Courier", "Courier New"]),
    (GenericFamily::Cursive, &["Apple Chancery"]),
    (GenericFamily::Fantasy, &["Papyrus"]),
    (GenericFamily::SystemUi, &["System Font", ".SF NS"]),
    (GenericFamily::Emoji, &["Apple Color Emoji"]),
    (GenericFamily::Math, &["STIX Two Math"]),
];

pub(crate) struct SystemFonts {
    pub(crate) name_map: Arc<FamilyNameMap>,
    pub(crate) generic_families: Arc<GenericFamilyMap>,
    family_map: HashMap<FamilyId, FamilyInfo>,
}

impl SystemFonts {
    pub(crate) fn new() -> Self {
        let scanned = scan_system_fonts().unwrap_or_default();
        let name_map = scanned.family_names;
        let mut generic_families = GenericFamilyMap::default();
        for (family, names) in DEFAULT_GENERIC_FAMILIES {
            generic_families.set(
                *family,
                names
                    .iter()
                    .filter_map(|name| name_map.get(name))
                    .map(|name| name.id()),
            );
        }
        Self {
            name_map: Arc::new(name_map),
            generic_families: Arc::new(generic_families),
            family_map: scanned.families,
        }
    }

    pub(crate) fn family(&mut self, id: FamilyId) -> Option<FamilyInfo> {
        self.family_map.get(&id).cloned()
    }

    pub(crate) fn fallback(&mut self, key: impl Into<FallbackKey>) -> Option<FamilyId> {
        const HANI: Script = Script::from_bytes(*b"Hani");
        const HANT: Script = Script::from_bytes(*b"Hant");
        const HANS: Script = Script::from_bytes(*b"Hans");
        let key = key.into();
        let script = key.script();
        let font = create_fallback_font_for_text(script.sample()?, key.locale_str(), false)?;
        let family_name = unsafe { font.family_name() };
        if let Some(family) = self.name_map.get(&family_name.to_string()) {
            return Some(family.id());
        }
        // HACK: if we don't have a usable PingFangUI due to our inability
        // to render hvgl outlines then try another font
        let name = match script {
            HANI | HANS => "Heiti SC",
            HANT => "Heiti TC",
            _ => return None,
        };
        self.name_map.get(name).map(|family| family.id())
    }
}

/// Discover system fonts by combining CoreText enumeration with a directory scan of all
/// Library/Fonts paths, then index them through the shared scan pipeline.
fn scan_system_fonts() -> Option<scan::ScannedCollection> {
    // SAFETY: Calls into CoreText. If anything fails we return None and use the fallback scan.
    let collection = unsafe { CTFontCollection::from_available_fonts(None) };
    let descriptors = unsafe { collection.matching_font_descriptors()? };
    let descriptors: CFRetained<CFArray<CTFontDescriptor>> =
        unsafe { CFRetained::cast_unchecked(descriptors) };

    // Collect unique font file paths to avoid redundant scanning.
    let mut paths: HashSet<PathBuf> = HashSet::new();
    for index in 0..descriptors.len() {
        let Some(descriptor) = descriptors.get(index) else {
            continue;
        };

        let Some(url_cf): Option<CFRetained<CFType>> =
            (unsafe { descriptor.attribute(kCTFontURLAttribute) })
        else {
            continue;
        };

        // The attribute is typed as CFType; attempt to downcast to CFURL.
        let Ok(url_cf): Result<CFRetained<CFURL>, _> = url_cf.downcast::<CFURL>() else {
            continue;
        };

        // Convert the file URL into a POSIX path.
        let Some(path_cf): Option<CFRetained<CFString>> =
            url_cf.file_system_path(CFURLPathStyle::CFURLPOSIXPathStyle)
        else {
            continue;
        };

        let path = PathBuf::from(path_cf.to_string());
        if path.exists() {
            paths.insert(path);
        }
    }

    // Apple hides certain fonts from CTFontCollection (notably SFNS.ttf, the San Francisco
    // system UI font). Scanning Library/Fonts directories catches what CoreText omits.
    paths.extend(library_font_files());

    if paths.is_empty() {
        return None;
    }

    Some(scan::ScannedCollection::from_paths(paths.iter(), 0))
}

fn library_font_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in NSSearchPathForDirectoriesInDomains(
        NSSearchPathDirectory::LibraryDirectory,
        NSSearchPathDomainMask::AllDomainsMask,
        true,
    ) {
        let font_dir = PathBuf::from(format!("{dir}/Fonts"));
        if font_dir.is_dir() {
            collect_files(&font_dir, 8, 0, &mut files);
        }
    }
    files
}

fn collect_files(dir: &Path, max_depth: u32, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if depth < max_depth {
                collect_files(&path, max_depth, depth + 1, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn create_base_font(prefer_ui_font: bool) -> CFRetained<CTFont> {
    if prefer_ui_font
        && let Some(font) =
            unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, 0.0, None) }
    {
        return font;
    }
    unsafe {
        let attrs = CFDictionary::new(None, null_mut(), null_mut(), 0, null(), null());
        let desc = CTFontDescriptor::with_attributes(&attrs.unwrap());
        CTFont::with_font_descriptor(&desc, 0.0, null())
    }
}

fn create_fallback_font_for_text(
    text: &str,
    locale: Option<&str>,
    prefer_ui_font: bool,
) -> Option<CFRetained<CTFont>> {
    let text = CFString::from_str(text);
    let text_range = CFRange {
        location: 0,
        length: text.length(),
    };
    let locale = locale.map(CFString::from_str);
    let base_font = create_base_font(prefer_ui_font);
    let font = unsafe {
        if let Some(locale) = locale {
            CTFont::for_string_with_language(&base_font, &text, text_range, Some(&locale))
        } else {
            CTFont::for_string(&base_font, &text, text_range)
        }
    };
    Some(font)
}
