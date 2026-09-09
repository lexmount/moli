//! Initial browser font preferences, before the author cascade is applied.

use style::properties::style_structs::Font;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(super) fn initial_font() -> Font {
    use style::values::computed::font::{
        FamilyName, FontFamilyList, FontFamilyNameSyntax, SingleFontFamily,
    };

    let mut font = Font::initial_values();
    font.font_family.families = FontFamilyList {
        list: style::ArcSlice::from_iter(std::iter::once(SingleFontFamily::FamilyName(
            FamilyName {
                name: moli_layout::DEFAULT_STANDARD_FONT_FAMILY.into(),
                syntax: FontFamilyNameSyntax::Quoted,
            },
        ))),
    };
    font.compute_font_hash();
    font
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub(super) fn initial_font() -> Font {
    Font::initial_values()
}
