// SPDX-License-Identifier: MIT OR Apache-2.0

//! A bundled last-resort font for machines without installed fonts.
//!
//! Minimal containers often ship no fonts at all. Fontique then has no family
//! for any CSS generic family and no script fallback, so shaping drops every
//! glyph and screenshots come out blank. Chromium avoids this by always having
//! a platform default; Moli bundles a small Latin font instead and only uses it
//! where the platform supplies nothing.

use std::sync::Arc;

use fontique::{Blob, Collection, FallbackKey, FamilyId, GenericFamily, Script};

/// Family name of the bundled font. It is a renamed subset of Liberation Sans,
/// which is metric-compatible with Arial; see `licenses/Liberation-Sans-OFL-1.1.txt`.
pub const LAST_RESORT_FAMILY_NAME: &str = "Moli Fallback Sans";

static LAST_RESORT_FONT: &[u8] = include_bytes!("../fonts/MoliFallbackSans-Regular.ttf");

// Emoji is left out: a Latin text font would only mask a missing emoji font.
const GENERIC_FAMILIES: [GenericFamily; 12] = [
    GenericFamily::Serif,
    GenericFamily::SansSerif,
    GenericFamily::Monospace,
    GenericFamily::Cursive,
    GenericFamily::Fantasy,
    GenericFamily::SystemUi,
    GenericFamily::UiSerif,
    GenericFamily::UiSansSerif,
    GenericFamily::UiMonospace,
    GenericFamily::UiRounded,
    GenericFamily::Math,
    GenericFamily::FangSong,
];

const LATIN: Script = Script::from_bytes(*b"Latn");

/// Registers the bundled font when `collection` cannot draw Latin text.
///
/// The font is added only when Latin script fallback is empty, which means
/// no installed font can render Latin text. It then becomes the Latin
/// fallback and fills every empty generic family. A machine with any Latin
/// font is left alone, even if some generic families such as `fantasy` are
/// empty, because script fallback already picks an installed font for them.
/// Returns whether the bundled font was added.
pub fn register_last_resort_font(collection: &mut Collection) -> bool {
    let latin_key = FallbackKey::from(LATIN);
    if collection.fallback_families(latin_key).next().is_some() {
        return false;
    }
    let missing_generics = GENERIC_FAMILIES
        .into_iter()
        .filter(|generic| collection.generic_families(*generic).next().is_none())
        .collect::<Vec<_>>();
    let Some(family) = register_bundled_family(collection) else {
        return false;
    };
    collection.append_fallbacks(latin_key, std::iter::once(family));
    for generic in missing_generics {
        collection.append_generic_families(generic, std::iter::once(family));
    }
    true
}

fn register_bundled_family(collection: &mut Collection) -> Option<FamilyId> {
    if let Some(family) = collection.family_id(LAST_RESORT_FAMILY_NAME) {
        return Some(family);
    }
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(LAST_RESORT_FONT);
    collection
        .register_fonts(Blob::new(data), None)
        .into_iter()
        .map(|(family, _)| family)
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontique::{CollectionOptions, QueryFamily, QueryStatus, SourceCache};

    fn empty_collection() -> Collection {
        Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        })
    }

    fn resolved_family_name(
        collection: &mut Collection,
        family: QueryFamily<'_>,
    ) -> Option<String> {
        let mut source_cache = SourceCache::default();
        let mut family_id = None;
        {
            let mut query = collection.query(&mut source_cache);
            query.set_families([family]);
            query.matches_with(|font| {
                family_id = Some(font.family.0);
                QueryStatus::Stop
            });
        }
        Some(collection.family(family_id?)?.name().to_owned())
    }

    #[test]
    fn bundled_font_is_the_renamed_subset() {
        let mut collection = empty_collection();
        let families = collection.register_fonts(
            Blob::new(Arc::new(LAST_RESORT_FONT) as Arc<dyn AsRef<[u8]> + Send + Sync>),
            None,
        );
        assert_eq!(families.len(), 1);
        let name = collection
            .family(families[0].0)
            .expect("registered family")
            .name()
            .to_owned();
        // The OFL reserves the name "Liberation" for unmodified fonts.
        assert_eq!(name, LAST_RESORT_FAMILY_NAME);
    }

    #[test]
    fn empty_collection_resolves_generic_families_to_the_bundled_font() {
        let mut collection = empty_collection();
        assert_eq!(
            resolved_family_name(
                &mut collection,
                QueryFamily::Generic(GenericFamily::SansSerif)
            ),
            None,
            "without system fonts, no generic family resolves"
        );

        assert!(register_last_resort_font(&mut collection));

        for generic in GENERIC_FAMILIES {
            assert_eq!(
                resolved_family_name(&mut collection, QueryFamily::Generic(generic)).as_deref(),
                Some(LAST_RESORT_FAMILY_NAME),
                "{generic:?}"
            );
        }
        assert_eq!(
            resolved_family_name(&mut collection, QueryFamily::Generic(GenericFamily::Emoji)),
            None
        );
        let fallback = collection
            .fallback_families(FallbackKey::from(LATIN))
            .collect::<Vec<_>>();
        assert_eq!(
            fallback,
            vec![collection.family_id(LAST_RESORT_FAMILY_NAME).unwrap()]
        );
    }

    #[test]
    fn registering_twice_keeps_one_family_entry() {
        let mut collection = empty_collection();
        assert!(register_last_resort_font(&mut collection));
        assert!(!register_last_resort_font(&mut collection));
        assert_eq!(
            collection
                .generic_families(GenericFamily::SansSerif)
                .count(),
            1
        );
    }

    #[test]
    fn installed_fonts_keep_precedence() {
        let mut collection = empty_collection();
        let ahem = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/moli-ahem.ttf"
        ))
        .expect("fixture font");
        let families = collection.register_fonts(Blob::from(ahem), None);
        let installed = families[0].0;
        for generic in GENERIC_FAMILIES {
            collection.append_generic_families(generic, std::iter::once(installed));
        }
        collection.append_fallbacks(FallbackKey::from(LATIN), std::iter::once(installed));

        assert!(!register_last_resort_font(&mut collection));
        assert_eq!(collection.family_id(LAST_RESORT_FAMILY_NAME), None);
    }

    #[test]
    fn a_latin_font_leaves_empty_generic_families_to_script_fallback() {
        let mut collection = empty_collection();
        let ahem = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/moli-ahem.ttf"
        ))
        .expect("fixture font");
        let installed = collection.register_fonts(Blob::from(ahem), None)[0].0;
        collection.append_fallbacks(FallbackKey::from(LATIN), std::iter::once(installed));

        assert!(!register_last_resort_font(&mut collection));
        assert_eq!(collection.family_id(LAST_RESORT_FAMILY_NAME), None);
        assert_eq!(
            collection.generic_families(GenericFamily::Fantasy).count(),
            0
        );
    }

    #[test]
    fn without_latin_fonts_installed_generic_families_keep_their_order() {
        let mut collection = empty_collection();
        let ahem = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/moli-ahem.ttf"
        ))
        .expect("fixture font");
        let installed = collection.register_fonts(Blob::from(ahem), None)[0].0;
        collection.append_generic_families(GenericFamily::Monospace, std::iter::once(installed));

        assert!(register_last_resort_font(&mut collection));

        assert_eq!(
            collection
                .generic_families(GenericFamily::Monospace)
                .collect::<Vec<_>>(),
            vec![installed]
        );
        assert_eq!(
            resolved_family_name(&mut collection, QueryFamily::Generic(GenericFamily::Serif))
                .as_deref(),
            Some(LAST_RESORT_FAMILY_NAME)
        );
    }

    #[test]
    fn installed_system_fonts_do_not_need_the_bundled_font() {
        let mut collection = Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        assert!(
            collection.family_names().next().is_some(),
            "the system font collection should expose at least one family"
        );
        assert!(!register_last_resort_font(&mut collection));
        assert_eq!(collection.family_id(LAST_RESORT_FAMILY_NAME), None);
    }
}
