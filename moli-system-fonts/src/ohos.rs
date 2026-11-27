// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fs,
    sync::{Arc, LazyLock},
};

use fontique::{Blob, Collection, CollectionOptions, GenericFamily};

type FontData = Arc<dyn AsRef<[u8]> + Send + Sync>;

// OHOS shell processes can read system fonts through ordinary file I/O.
// Share the bytes between Fontique layout and the SVG font database.
static SYSTEM_FONT_DATA: LazyLock<Vec<FontData>> = LazyLock::new(|| {
    let Ok(entries) = fs::read_dir("/system/fonts") else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("ttf" | "ttc" | "otf" | "otc")
            )
        })
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
        .into_iter()
        .filter_map(|path| fs::read(path).ok())
        .map(|bytes| Arc::new(bytes) as FontData)
        .collect()
});

/// Returns shared bytes for the readable system font files on OHOS.
pub fn ohos_system_font_data() -> impl Iterator<Item = FontData> {
    SYSTEM_FONT_DATA.iter().cloned()
}

pub(super) static SYSTEM_FONTS: LazyLock<Collection> = LazyLock::new(|| {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    for data in ohos_system_font_data() {
        collection.register_fonts(Blob::new(data), None);
    }

    let mut sans = [
        "HarmonyOS Sans",
        "HarmonyOS Sans SC",
        "HarmonyOS Sans TC",
        "Noto Sans",
        "Roboto",
    ]
    .into_iter()
    .filter_map(|name| collection.family_id(name))
    .collect::<Vec<_>>();
    if sans.is_empty() {
        sans.extend(collection.family_ids().take(1));
    }
    for &generic in GenericFamily::all() {
        let preferred: &[&str] = match generic {
            GenericFamily::Serif | GenericFamily::UiSerif => &["Noto Serif"],
            GenericFamily::Monospace | GenericFamily::UiMonospace => &["Noto Sans Mono"],
            GenericFamily::Emoji => &["HMOS Color Emoji", "Noto Color Emoji"],
            _ => &[],
        };
        let family = preferred.iter().find_map(|name| collection.family_id(name));
        collection.set_generic_families(generic, family.into_iter().chain(sans.iter().copied()));
    }
    collection
});
