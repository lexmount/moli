// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! # Parley Bench
//!
//! This crate provides benchmarks for the Parley library.

use std::{
    borrow::Cow,
    cell::RefCell,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use parley::{
    FontContext, FontFamilyName, LayoutContext,
    fontique::{Blob, Collection, CollectionOptions, SourceCache},
};

pub mod benches;
pub mod fontique_benches;
pub mod query_benches;

/// A color brush.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorBrush {}

std::thread_local! {
    static FONT_CX_TL: RefCell<FontContext> = RefCell::new(create_font_context());
    static LAYOUT_CX_TL: RefCell<LayoutContext<ColorBrush>> = RefCell::new(LayoutContext::new());
}

/// Runs the provided closure with mutable access to the thread-local font and layout contexts.
pub fn with_contexts<R>(
    f: impl FnOnce(&mut FontContext, &mut LayoutContext<ColorBrush>) -> R,
) -> R {
    FONT_CX_TL.with(|font_cell| {
        LAYOUT_CX_TL.with(|layout_cell| {
            let mut font_cx = font_cell.borrow_mut();
            let mut layout_cx = layout_cell.borrow_mut();
            f(&mut font_cx, &mut layout_cx)
        })
    })
}

pub(crate) fn create_font_context() -> FontContext {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    load_fonts(&mut collection, parley_dev::font_dirs()).unwrap();
    for font in FONT_FAMILY_LIST {
        if let FontFamilyName::Named(font_name) = font {
            collection
                .family_id(font_name)
                .unwrap_or_else(|| panic!("{font_name} font not found"));
        }
    }
    FontContext {
        collection,
        source_cache: SourceCache::default(),
    }
}

pub(crate) const FONT_FAMILY_LIST: &[FontFamilyName<'_>] = &[
    FontFamilyName::Named(Cow::Borrowed("Roboto")),
    FontFamilyName::Named(Cow::Borrowed("Noto Kufi Arabic")),
];

pub(crate) fn load_fonts(
    collection: &mut Collection,
    font_dirs: impl Iterator<Item = PathBuf>,
) -> std::io::Result<()> {
    for dir in font_dirs {
        let paths = std::fs::read_dir(dir)?;
        for entry in paths {
            let entry = entry?;
            if !entry.metadata()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_none_or(|ext| !["ttf", "otf", "ttc", "otc"].contains(&ext))
            {
                continue;
            }
            let font_data = std::fs::read(&path)?;
            collection.register_fonts(Blob::new(Arc::new(font_data)), None);
        }
    }
    Ok(())
}

/// A sample to be used for benchmarking.
#[derive(Debug)]
pub struct Sample {
    /// The name of the sample.
    pub name: &'static str,
    /// The modification of the sample.
    pub modification: &'static str,
    /// The text of the sample.
    pub text: String,
}

static SAMPLES: OnceLock<Vec<Sample>> = OnceLock::new();

/// Take `chars` characters of `text`.
///
/// A paragraph that crosses the budget is truncated. Paragraphs are joined by a single `\n`.
fn take_chars(text: &str, chars: usize) -> String {
    let mut out = String::new();
    let mut remaining = chars;
    for paragraph in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        if remaining == 0 {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
            remaining -= 1;
        }
        let taken = paragraph.chars().take(remaining).collect::<String>();
        remaining -= taken.chars().count();
        out.push_str(&taken);
    }
    out
}

/// Returns a list of samples to be used for benchmarking.
pub fn get_samples() -> &'static [Sample] {
    let samples = parley_dev::TextSamples::new();

    SAMPLES.get_or_init(|| {
        vec![
            Sample {
                name: samples.arabic.name,
                modification: "20 characters",
                text: take_chars(samples.arabic.text, 20),
            },
            Sample {
                name: samples.latin.name,
                modification: "20 characters",
                text: take_chars(samples.latin.text, 20),
            },
            Sample {
                name: samples.japanese.name,
                modification: "20 characters",
                text: take_chars(samples.japanese.text, 20),
            },
            Sample {
                name: samples.arabic.name,
                modification: "400 characters",
                text: take_chars(samples.arabic.text, 400),
            },
            Sample {
                name: samples.latin.name,
                modification: "400 characters",
                text: take_chars(samples.latin.text, 400),
            },
            Sample {
                name: samples.japanese.name,
                modification: "400 characters",
                text: take_chars(samples.japanese.text, 400),
            },
            Sample {
                name: samples.arabic.name,
                modification: "8000 characters",
                text: take_chars(samples.arabic.text, 8000),
            },
            Sample {
                name: samples.latin.name,
                modification: "8000 characters",
                text: take_chars(samples.latin.text, 8000),
            },
            Sample {
                name: samples.japanese.name,
                modification: "8000 characters",
                text: take_chars(samples.japanese.text, 8000),
            },
        ]
    })
}
