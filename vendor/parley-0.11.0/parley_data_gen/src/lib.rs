// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! See `./main.rs`.

use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{
        BidiClass, Emoji, EmojiModifier, EmojiModifierBase, EmojiPresentation,
        ExtendedPictographic, GeneralCategory, GraphemeClusterBreak, LineBreak, RegionalIndicator,
        Script, VariationSelector,
    },
};
use parley_data::Properties;
use std::fmt::Write as _;
use std::io::{BufWriter, Write};

const COPYRIGHT_HEADER: &str =
    "// Copyright 2025 the Parley Authors\n// SPDX-License-Identifier: Apache-2.0 OR MIT\n";

/// Generation configuration.
#[derive(Debug)]
pub struct Config {
    /// Compression level (1.0 = balanced, 5.0 = smaller, 9.0 = even smaller, 10.0 = smallest).
    pub compression: f64,
    /// Whether to use unsafe array access in generated code.
    pub unsafe_access: bool,
}

/// Exports ICU data as `PackTab` lookup tables + generated Rust code into the `out` directory.
pub fn generate(out: std::path::PathBuf, config: &Config) {
    let emoji_data = CodePointSetData::new::<Emoji>();
    let extended_pictographic_data = CodePointSetData::new::<ExtendedPictographic>();
    let regional_indicator_data = CodePointSetData::new::<RegionalIndicator>();

    let script_data = CodePointMapData::<Script>::new();
    let general_category_data = CodePointMapData::<GeneralCategory>::new();
    let grapheme_cluster_break_data = CodePointMapData::<GraphemeClusterBreak>::new();
    let bidi_class_data = CodePointMapData::<BidiClass>::new();

    let variation_selector_data = CodePointSetData::new::<VariationSelector>();
    let line_break_data = CodePointMapData::<LineBreak>::new();

    let emoji_presentation_data = CodePointSetData::new::<EmojiPresentation>();
    let emoji_modifier_data = CodePointSetData::new::<EmojiModifier>();
    let emoji_modifier_base_data = CodePointSetData::new::<EmojiModifierBase>();

    // Generate the data required for `CompositeProps`.
    // Dense characters table for 0..=0x10FFFF
    let mut characters = Vec::with_capacity(0x110000);

    for cp in 0_u32..=0x10FFFF {
        let is_emoji = emoji_data.contains32(cp);
        let is_extended_pictographic = extended_pictographic_data.contains32(cp);

        // This encodes in which instances ICU4X uses a dictionary to determine a word break instead
        // of applying default UAX #29 rules; if ICU4X lacks a dictionary, it emits a sequence as a
        // single word. Unicode recommends tailoring for those code points, but which exact set this
        // tracks is an ICU4X implementation detail that we try to match.
        let needs_dictionary_word_break = line_break_data.get32(cp) == LineBreak::ComplexContext
            || matches!(script_data.get32(cp), Script::Han | Script::Hiragana);

        let v = Properties::new(
            script_data.get32(cp),
            general_category_data.get32(cp),
            grapheme_cluster_break_data.get32(cp),
            bidi_class_data.get32(cp),
            is_emoji || is_extended_pictographic,
            variation_selector_data.contains32(cp),
            regional_indicator_data.contains32(cp),
            // See: https://github.com/unicode-org/icu4x/blob/ee5399a77a6b94efb5d4b60678bb458c5eedb25d/components/segmenter/src/line.rs#L338-L351
            matches!(
                line_break_data.get32(cp),
                LineBreak::MandatoryBreak
                    | LineBreak::CarriageReturn
                    | LineBreak::LineFeed
                    | LineBreak::NextLine
            ),
            needs_dictionary_word_break,
            is_emoji,
            emoji_presentation_data.contains32(cp),
            emoji_modifier_data.contains32(cp),
            emoji_modifier_base_data.contains32(cp),
        );
        characters.push(u32::from(v) as i64);
    }

    let (info, best) = packtab::pack_table(&characters, Some(0), config.compression);

    let namespace = "composite_packtab";
    let mut code = packtab::generate(
        &info,
        best,
        namespace,
        packtab::codegen::Language::Rust {
            unsafe_access: config.unsafe_access,
        },
    );

    if !code.ends_with('\n') {
        code.push('\n');
    }
    code.push('\n');
    write!(
        code,
        "#[allow(missing_docs, reason = \"packtab generated code\")]\n#[inline]\npub fn composite_get(cp: u32) -> u32 {{\n    {namespace}_get(cp as usize)\n}}\n"
    )
    .unwrap();

    let mut file = BufWriter::new(std::fs::File::create(out.join("mod.rs")).unwrap());
    writeln!(&mut file, "{COPYRIGHT_HEADER}").unwrap();
    writeln!(
        &mut file,
        "//! Backing data for composite properties (PackTab, compression={}, unsafe={})",
        config.compression, config.unsafe_access
    )
    .unwrap();
    writeln!(&mut file).unwrap();
    write!(&mut file, "{code}").unwrap();
}
