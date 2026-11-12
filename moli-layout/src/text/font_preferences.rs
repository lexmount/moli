// SPDX-License-Identifier: MIT OR Apache-2.0

//! Browser font preferences and CSS fallback ordering remain in the layout layer.
//! Native family matching is supplied by `moli-system-fonts`.

use moli_system_fonts::SystemFontFamilyResolver;
use parley::fontique::Collection;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use parley::{FontFamily, FontFamilyName, GenericFamily};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::borrow::Cow;

/// Standard and serif preference for the Fontconfig-backed desktop profile.
/// Platform matching still resolves this name to an actually installed font.
pub const DEFAULT_STANDARD_FONT_FAMILY: &str = "Times New Roman";

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(super) fn install_browser_generic_preferences(
    resolver: &mut SystemFontFamilyResolver,
    collection: &mut Collection,
) {
    // Install preferences before downloadable fonts, so a web face cannot
    // replace a system generic. Leave monospace/system-ui at platform defaults.
    for (generic, preferred) in [
        (GenericFamily::Serif, DEFAULT_STANDARD_FONT_FAMILY),
        (GenericFamily::SansSerif, "Arial"),
    ] {
        let substitute = resolver.substitute_family(collection, preferred);
        if let Some(id) = collection.family_id(substitute.as_deref().unwrap_or(preferred)) {
            collection.set_generic_families(generic, std::iter::once(id));
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub(super) fn install_browser_generic_preferences(
    _resolver: &mut SystemFontFamilyResolver,
    _collection: &mut Collection,
) {
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(super) fn append_standard_fallback(family: &mut FontFamily<'static>) {
    // Blink's FontFallbackList tries the user's standard font after the CSS
    // families are exhausted, before script-specific platform last resorts.
    // This profile's standard and serif preferences are the same. Using the
    // generic keeps the fallback bound to platform fonts, not a downloadable
    // face that happens to use the preferred system family's name.
    let standard = FontFamilyName::Generic(GenericFamily::Serif);
    match family {
        FontFamily::Single(first) if *first != standard => {
            *family = FontFamily::List(Cow::Owned(vec![first.clone(), standard]));
        }
        FontFamily::List(families) if !families.contains(&standard) => {
            Cow::to_mut(families).push(standard);
        }
        _ => {}
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "freebsd")))]
mod tests {
    use super::*;

    #[test]
    fn standard_fallback_preserves_author_family_order_and_is_not_duplicated() {
        let named = FontFamilyName::Named(Cow::Borrowed("Author Font"));
        let sans = FontFamilyName::Generic(GenericFamily::SansSerif);
        let serif = FontFamilyName::Generic(GenericFamily::Serif);
        let mut family = FontFamily::List(Cow::Owned(vec![named.clone(), sans.clone()]));
        append_standard_fallback(&mut family);
        append_standard_fallback(&mut family);
        assert_eq!(
            family,
            FontFamily::List(Cow::Owned(vec![named, sans, serif.clone()]))
        );
        let mut single = FontFamily::Single(serif.clone());
        append_standard_fallback(&mut single);
        assert_eq!(single, FontFamily::Single(serif));
    }

    #[test]
    fn browser_preferences_select_real_families_without_changing_system_ui_or_monospace() {
        use parley::fontique::{Blob, CollectionOptions, FontInfoOverride};
        use std::sync::Arc;

        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        let ids = [DEFAULT_STANDARD_FONT_FAMILY, "Arial", "Platform UI"].map(|family_name| {
            collection.register_fonts(
                Blob::new(Arc::new(
                    include_bytes!("../../tests/fixtures/moli-ahem.ttf").to_vec(),
                )),
                Some(FontInfoOverride {
                    family_name: Some(family_name),
                    ..Default::default()
                }),
            )[0]
            .0
        });
        for generic in [
            GenericFamily::Serif,
            GenericFamily::SansSerif,
            GenericFamily::Monospace,
            GenericFamily::SystemUi,
        ] {
            collection.set_generic_families(generic, std::iter::once(ids[2]));
        }
        let mut resolver = SystemFontFamilyResolver::new(&mut collection);
        install_browser_generic_preferences(&mut resolver, &mut collection);
        for (generic, expected) in [
            (GenericFamily::Serif, ids[0]),
            (GenericFamily::SansSerif, ids[1]),
            (GenericFamily::Monospace, ids[2]),
            (GenericFamily::SystemUi, ids[2]),
        ] {
            assert_eq!(
                collection.generic_families(generic).collect::<Vec<_>>(),
                vec![expected]
            );
        }
    }
}
