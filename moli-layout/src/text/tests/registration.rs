use super::*;

#[test]
fn owned_sfnt_payload_is_retained_without_copying_its_buffer() {
    // Spare capacity must not force a shrink/copy either.
    let mut bytes = Vec::with_capacity(TEST_TTF.len() * 2);
    bytes.extend_from_slice(TEST_TTF);
    let original_buffer = bytes.as_ptr();
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(WebFontRegistration::new(
            "owned",
            WebFontFace::new("Owned"),
            bytes,
        ))
        .unwrap();
    let retained = &services.web_fonts["owned"].sfnt_bytes;
    assert_eq!(retained.as_ref().as_ptr(), original_buffer);
    let blob_id = retained.id();
    assert_eq!(
        shape_one_character(services.parley_mut(), web_font_style("Owned", 400.0), 'R'),
        TEST_TTF
    );
    let font = services
        .parley_mut()
        .font_context
        .collection
        .family_by_name("Owned")
        .unwrap();
    assert_eq!(font.fonts()[0].load(None).unwrap().id(), blob_id);
}

#[test]
fn owned_and_borrowed_validation_have_the_same_acceptance() {
    for bytes in [
        TEST_TTF,
        TEST_WOFF,
        TEST_WOFF2,
        TEST_CJK_TTF,
        b"garbage",
        b"wOFFbad",
        b"wOF2bad",
        b"",
    ] {
        assert_eq!(
            validate_web_font_bytes(bytes),
            validate_web_font_bytes(bytes.to_vec())
        );
    }
}

fn source_id(services: &mut ParleyDocumentServices, family: &str) -> parley::fontique::SourceId {
    services
        .font_context
        .collection
        .family_by_name(family)
        .unwrap()
        .fonts()[0]
        .source()
        .id
}

#[test]
fn incremental_arrivals_do_not_reregister_old_fonts_or_replace_contexts() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services.parley_mut();
    services
        .register_web_font(registration("seed", "Seed", TEST_TTF))
        .unwrap();
    let context_address = std::ptr::from_ref(&services.parley_mut().font_context);
    let layout_address = std::ptr::from_ref(&services.parley_mut().layout_context);
    let original_source = source_id(services.parley_mut(), "Seed");
    for index in (0..128).rev() {
        let slot = format!("{index:03}");
        let family = format!("Family {index}");
        services
            .register_web_font(registration(&slot, &family, TEST_TTF))
            .unwrap();
        let parley = services.parley_mut();
        assert_eq!(
            source_id(parley, "Seed"),
            original_source,
            "Fontique source must not be re-registered"
        );
        assert_eq!(context_address, std::ptr::from_ref(&parley.font_context));
        assert_eq!(layout_address, std::ptr::from_ref(&parley.layout_context));
    }
    assert_eq!(services.web_font_count(), 129);
}

#[test]
fn invalid_incremental_arrival_leaves_fonts_and_selection_caches_untouched() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(registration("seed", "Seed", TEST_TTF))
        .unwrap();
    let original_source = source_id(services.parley_mut(), "Seed");
    assert_eq!(
        shape_one_character(services.parley_mut(), web_font_style("Seed", 625.0), 'R'),
        TEST_TTF
    );
    let plan_count = services.parley_mut().font_family_resolution_plans.len();
    for bytes in [b"garbage".as_slice(), b"wOF2bad", &TEST_TTF[..32]] {
        assert!(
            services
                .register_web_font(registration("bad", "Invalid Arrival", bytes))
                .is_err()
        );
        assert_eq!(services.web_font_count(), 1);
        assert_eq!(source_id(services.parley_mut(), "Seed"), original_source);
        assert_eq!(
            services.parley_mut().font_family_resolution_plans.len(),
            plan_count
        );
        assert!(!has_family(&mut services, "Invalid Arrival"));
    }
}

#[test]
fn arriving_font_invalidates_a_previously_cached_missing_font_and_metrics() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    let style = web_font_style("Arriving", 400.0);
    services
        .parley_mut()
        .resolve_font_families(&mut style.clone(), Some('R'));
    assert!(
        services
            .parley_mut()
            .inline_font_metrics(&style, None)
            .is_none()
    );
    services
        .register_web_font(WebFontRegistration::new(
            "arrival",
            WebFontFace::new("Arriving"),
            TEST_TTF.to_vec(),
        ))
        .unwrap();
    assert_eq!(
        shape_one_character(services.parley_mut(), style.clone(), 'R'),
        TEST_TTF
    );
    assert!(
        services
            .parley_mut()
            .inline_font_metrics(&style, None)
            .is_some()
    );
}

#[test]
fn out_of_order_incremental_selection_matches_a_fresh_ordered_collection() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services.parley_mut();
    services
        .register_web_font(registration("unrelated", "Unrelated", TEST_TTF))
        .unwrap();
    let unrelated_source = source_id(services.parley_mut(), "Unrelated");
    let context_address = std::ptr::from_ref(&services.parley_mut().font_context);
    let faces = [
        WebFontFace::new("Shared").with_weight(400.0),
        WebFontFace::new("Shared").with_weight(700.0),
        WebFontFace::new("Shared")
            .with_weight(400.0)
            .with_style(WebFontStyle::Italic),
        WebFontFace::new("Shared")
            .with_weight(400.0)
            .with_style(WebFontStyle::Oblique(None)),
        WebFontFace::new("Shared")
            .with_weight(400.0)
            .with_style(WebFontStyle::Oblique(Some(14.0))),
        WebFontFace::new("Shared")
            .with_weight(400.0)
            .with_stretch(75.0),
        WebFontFace::new("Shared")
            .with_weight(400.0)
            .with_unicode_ranges([WebFontUnicodeRange::new(0x4e00, 0x9fff)]),
        WebFontFace::new("Shared")
            .with_weight(400.0)
            .with_unicode_ranges([WebFontUnicodeRange::new(0, 0xff)]),
    ];
    for index in [4, 3, 7, 2, 6, 0, 5, 1] {
        services
            .register_web_font(WebFontRegistration::new(
                format!("slot-{index}"),
                faces[index].clone(),
                TEST_TTF.to_vec(),
            ))
            .unwrap();
        assert_eq!(
            source_id(services.parley_mut(), "Unrelated"),
            unrelated_source
        );
        assert_eq!(
            std::ptr::from_ref(&services.parley_mut().font_context),
            context_address
        );
        let mut fresh = build_parley_services(SystemFontPolicy::Disabled, &services.web_fonts);
        for weight in [100.0, 400.0, 500.0, 625.0, 700.0, 900.0] {
            for font_style in [
                FontStyle::Normal,
                FontStyle::Italic,
                FontStyle::Oblique(None),
                FontStyle::Oblique(Some(14.0)),
                FontStyle::Oblique(Some(-10.0)),
            ] {
                for width in [75.0, 100.0, 125.0] {
                    for character in [None, Some('R'), Some('中')] {
                        let mut incremental_style = web_font_style("Shared", weight);
                        incremental_style.font_style = font_style;
                        incremental_style.font_width = FontWidth::from_percentage(width);
                        let mut fresh_style = incremental_style.clone();
                        services
                            .parley_mut()
                            .resolve_font_families(&mut incremental_style, character);
                        fresh.resolve_font_families(&mut fresh_style, character);
                        assert_eq!(
                            incremental_style.font_family, fresh_style.font_family,
                            "after slot-{index}, weight={weight}, style={font_style:?}, width={width}, char={character:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn replacement_and_removal_preserve_other_same_capability_slots() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services.parley_mut();
    for slot in ["a", "b"] {
        services
            .register_web_font(registration(slot, "Shared", TEST_TTF))
            .unwrap();
    }
    services
        .register_web_font(registration("a", "Replacement", TEST_CJK_TTF))
        .unwrap();
    assert!(has_family(&mut services, "Shared"));
    assert!(has_family(&mut services, "Replacement"));
    assert_eq!(
        shape_one_character(services.parley_mut(), web_font_style("Shared", 625.0), 'R'),
        TEST_TTF
    );
    assert!(services.remove_web_font("a"));
    assert!(!has_family(&mut services, "Replacement"));
    assert_eq!(
        shape_one_character(services.parley_mut(), web_font_style("Shared", 625.0), 'R'),
        TEST_TTF
    );
    assert!(services.remove_web_font("b"));
    assert!(!has_family(&mut services, "Shared"));
}

#[test]
fn unchanged_bytes_do_not_depend_on_blob_allocation_identity() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(registration("same", "Same", TEST_TTF))
        .unwrap();
    let source = source_id(services.parley_mut(), "Same");
    for _ in 0..4 {
        assert_eq!(
            services.register_web_font(registration("same", "Same", TEST_TTF)),
            Ok(WebFontRegistrationOutcome::Unchanged)
        );
        assert_eq!(source_id(services.parley_mut(), "Same"), source);
    }
}
