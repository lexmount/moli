// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::{Arc, LazyLock};

use fontique::{Blob, Collection, Script};
use objc2_core_foundation::CFString;
use objc2_core_text::{CTFont, CTFontTableOptions};
use write_fonts::{FontBuilder, types::Tag};

struct SystemFace {
    name: &'static str,
    metrics: Blob<u8>,
}

static PINGFANG: LazyLock<Vec<SystemFace>> = LazyLock::new(|| {
    [
        "PingFangSC-Regular",
        "PingFangSC-Semibold",
        "PingFangTC-Regular",
        "PingFangTC-Semibold",
        "PingFangHK-Regular",
        "PingFangHK-Semibold",
    ]
    .into_iter()
    .filter_map(pingfang_metrics)
    .collect()
});

fn pingfang_metrics(name: &'static str) -> Option<SystemFace> {
    // Core Text may silently substitute a different face for an unknown name.
    let font = unsafe { CTFont::with_name(&CFString::from_str(name), 1.0, std::ptr::null()) };
    if unsafe { font.post_script_name() }.to_string() != name {
        return None;
    }
    let table =
        |tag: &[u8; 4]| unsafe { font.table(u32::from_be_bytes(*tag), CTFontTableOptions(0)) };
    table(b"hvgl")?;

    // The shaper needs glyph IDs, metrics and OpenType shaping tables. Apple's
    // outlines stay in Core Text and are resolved by the paint backend.
    let mut builder = FontBuilder::default();
    for tag in [b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name"] {
        builder.add_raw(Tag::new(tag), table(tag)?.to_vec());
    }
    for tag in [
        b"OS/2", b"post", b"GDEF", b"GPOS", b"GSUB", b"kern", b"vhea", b"vmtx", b"VORG",
    ] {
        if let Some(data) = table(tag) {
            builder.add_raw(Tag::new(tag), data.to_vec());
        }
    }
    Some(SystemFace {
        name,
        metrics: Blob::new(Arc::new(builder.build())),
    })
}

pub fn native_outline_font_name(blob_id: u64) -> Option<&'static str> {
    PINGFANG
        .iter()
        .find(|face| face.metrics.id() == blob_id)
        .map(|face| face.name)
}

pub(super) fn prefer_pingfang(collection: &mut Collection) {
    for face in PINGFANG.iter() {
        collection.register_fonts(face.metrics.clone(), None);
    }

    let han = Script::from_bytes(*b"Hani");
    for (locale, name) in [
        ("", "PingFang SC"),
        ("zh-CN", "PingFang SC"),
        ("zh-Hans", "PingFang SC"),
        ("zh-Hant", "PingFang TC"),
        ("zh-TW", "PingFang TC"),
        ("zh-HK", "PingFang HK"),
    ] {
        let Some(family) = collection.family_by_name(name) else {
            continue;
        };
        let preferred = family.id();
        let key = (han, locale);
        let system = collection.fallback_families(key).collect::<Vec<_>>();
        collection.set_fallbacks(
            key,
            std::iter::once(preferred).chain(system.into_iter().filter(|id| *id != preferred)),
        );
    }
}
