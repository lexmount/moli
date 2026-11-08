use super::*;

#[test]
fn incremental_font_registration_retains_sfnt_table_bounds_validation() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(WebFontRegistration::new(
            "stable",
            WebFontFace::new("Stable"),
            TEST_TTF.to_vec(),
        ))
        .unwrap();
    services.parley_mut();

    let mut bytes = TEST_TTF.to_vec();
    let table_count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let record = (0..table_count)
        .map(|index| 12 + index * 16)
        .find(|offset| &bytes[*offset..*offset + 4] == b"post")
        .expect("the fixture has an optional post table");
    let outside = u32::try_from(bytes.len() + 1).unwrap();
    bytes[record + 8..record + 12].copy_from_slice(&outside.to_be_bytes());
    assert!(matches!(
        services.register_web_font(WebFontRegistration::new(
            "invalid",
            WebFontFace::new("Rejected"),
            bytes,
        )),
        Err(WebFontRegistrationError::UnsupportedPayload)
    ));
    assert_eq!(services.web_font_count(), 1);
    assert!(
        services
            .parley_mut()
            .font_context
            .collection
            .family_by_name("Rejected")
            .is_none()
    );
}

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
    let parley = services.parley_mut();
    let family = resolved_registered_family_name(parley, "Owned").unwrap();
    let font = parley
        .font_context
        .collection
        .family_by_name(&family)
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

#[test]
fn validated_font_data_keeps_its_blob_through_registration_and_layout() {
    let data = FontFaceData::from_bytes(TEST_WOFF2).unwrap();
    let original_buffer = data.bytes().as_ptr();
    let blob_id = data.bytes.id();
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(WebFontRegistration::from_font_data(
            "validated",
            WebFontFace::new("Validated"),
            data,
        ))
        .unwrap();
    assert_eq!(services.web_fonts["validated"].sfnt_bytes.id(), blob_id);
    assert_eq!(
        services.web_fonts["validated"].sfnt_bytes.as_ref().as_ptr(),
        original_buffer
    );
    let parley = services.parley_mut();
    let family = resolved_registered_family_name(parley, "Validated").unwrap();
    let font = parley
        .font_context
        .collection
        .family_by_name(&family)
        .unwrap();
    assert_eq!(font.fonts()[0].load(None).unwrap().id(), blob_id);
}

#[test]
fn stylesheet_and_font_face_set_memberships_are_retired_independently() {
    for remove_stylesheet_first in [false, true] {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        let data = FontFaceData::from_bytes(TEST_TTF).unwrap();
        services
            .register_web_font(
                WebFontRegistration::from_font_data(
                    "face",
                    WebFontFace::new("Scoped"),
                    data.clone(),
                )
                .with_stylesheet_scope(),
            )
            .unwrap();
        services
            .register_web_font(
                WebFontRegistration::from_font_data("face", WebFontFace::new("Scoped"), data)
                    .with_scope(WebFontRegistrationScope::FontFaceSetManual),
            )
            .unwrap();
        services.with_active_stylesheet_fonts(
            |_| false,
            |active| assert_eq!(active.web_font_count(), 1),
        );
        if remove_stylesheet_first {
            assert!(services.remove_stylesheet_web_font("face"));
            assert_eq!(services.web_font_count(), 1);
            assert!(!services.has_stylesheet_web_fonts());
            assert!(services.remove_font_face_set_web_font("face"));
        } else {
            assert!(services.remove_font_face_set_web_font("face"));
            assert_eq!(services.web_font_count(), 1);
            assert!(services.has_stylesheet_web_fonts());
            services.with_active_stylesheet_fonts(
                |_| false,
                |active| assert_eq!(active.web_font_count(), 0),
            );
            assert!(services.remove_stylesheet_web_font("face"));
        }
        assert_eq!(services.web_font_count(), 0);
    }
}

fn source_id(services: &mut ParleyDocumentServices, family: &str) -> parley::fontique::SourceId {
    let family = resolved_registered_family_name(services, family).unwrap();
    services
        .font_context
        .collection
        .family_by_name(&family)
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
    assert!(services.parley_mut().inline_font_metrics_cache.is_empty());
    assert_eq!(
        shape_one_character(services.parley_mut(), style.clone(), 'R'),
        TEST_TTF
    );
    let mut resolved = style;
    services
        .parley_mut()
        .resolve_font_families(&mut resolved, None);
    assert!(
        services
            .parley_mut()
            .inline_font_metrics(&resolved, None)
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
        WebFontFace::new("Shared").with_css_descriptors("100 900", "normal", "normal"),
        WebFontFace::new("Shared").with_css_descriptors("400", "75% 125%", "normal"),
        WebFontFace::new("Shared").with_css_descriptors("400", "normal", "oblique 10deg 30deg"),
    ];
    for index in [4, 8, 3, 7, 2, 10, 6, 0, 9, 5, 1] {
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
        let mut fresh = build_parley_services(
            SystemFontPolicy::Disabled,
            &services.web_fonts,
            &services.web_font_order,
        );
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
fn incremental_ranges_refresh_priorities_after_unloaded_members_are_removed() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    let narrow = include_bytes!("../../../tests/fixtures/moli-ahem-narrow.ttf");
    let wide = include_bytes!("../../../tests/fixtures/moli-ahem-wide.ttf");
    services.set_web_font_order(&[], &["unloaded".into(), "z-wide".into()]);
    services
        .register_web_font(WebFontRegistration::new(
            "z-wide",
            WebFontFace::new("Shifted").with_css_descriptors("100 900", "normal", "normal"),
            wide.to_vec(),
        ))
        .unwrap();
    assert_eq!(
        shape_one_character(services.parley_mut(), web_font_style("Shifted", 700.0), 'A'),
        wide
    );
    let context_address = std::ptr::from_ref(&services.parley_mut().font_context);
    assert!(!services.set_web_font_order(&[], &["z-wide".into(), "a-narrow".into()]));
    services
        .register_web_font(WebFontRegistration::new(
            "a-narrow",
            WebFontFace::new("Shifted").with_css_descriptors("500 800", "normal", "normal"),
            narrow.to_vec(),
        ))
        .unwrap();
    assert_eq!(
        std::ptr::from_ref(&services.parley_mut().font_context),
        context_address
    );
    assert_eq!(
        shape_one_character(services.parley_mut(), web_font_style("Shifted", 700.0), 'A'),
        narrow
    );
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
fn unchanged_bytes_merge_and_retire_stylesheet_scope_independently() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    services
        .register_web_font(registration("same", "Same", TEST_TTF))
        .unwrap();
    assert_eq!(
        services.register_web_font(registration("same", "Same", TEST_TTF).with_stylesheet_scope()),
        Ok(WebFontRegistrationOutcome::Replaced)
    );
    services.with_active_stylesheet_fonts(
        |_| false,
        |selected| assert_eq!(selected.web_font_count(), 1),
    );
    assert!(services.remove_font_face_set_web_font("same"));
    services.with_active_stylesheet_fonts(
        |_| false,
        |selected| assert_eq!(selected.web_font_count(), 0),
    );
    assert!(services.remove_stylesheet_web_font("same"));
    assert_eq!(services.web_font_count(), 0);
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

#[test]
fn reordered_overlapping_fonts_keep_sources_contexts_and_invalidate_selection() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    let narrow = include_bytes!("../../../tests/fixtures/moli-ahem-narrow.ttf");
    let wide = include_bytes!("../../../tests/fixtures/moli-ahem-wide.ttf");
    for (slot, bytes, weight) in [
        ("wide", wide.as_slice(), "100 900"),
        ("narrow", narrow.as_slice(), "500 800"),
    ] {
        services
            .register_web_font(WebFontRegistration::new(
                slot,
                WebFontFace::new("Ordered").with_css_descriptors(weight, "normal", "normal"),
                bytes.to_vec(),
            ))
            .unwrap();
    }
    services
        .register_web_font(registration("seed", "Seed", TEST_TTF))
        .unwrap();
    let original_source = source_id(services.parley_mut(), "Seed");
    let context = std::ptr::from_ref(&services.parley_mut().font_context);
    let layout = std::ptr::from_ref(&services.parley_mut().layout_context);
    for (order, bytes) in [
        (["wide", "narrow"], narrow.as_slice()),
        (["narrow", "wide"], wide.as_slice()),
        (["wide", "narrow"], narrow.as_slice()),
    ] {
        services.set_web_font_order(&[], &order.map(str::to_owned));
        assert_eq!(
            shape_one_character(services.parley_mut(), web_font_style("Ordered", 700.0), 'A'),
            bytes
        );
        assert_eq!(source_id(services.parley_mut(), "Seed"), original_source);
        assert_eq!(
            std::ptr::from_ref(&services.parley_mut().font_context),
            context
        );
        assert_eq!(
            std::ptr::from_ref(&services.parley_mut().layout_context),
            layout
        );
        assert_eq!(services.font_collection_rebuilds, 0);
    }
}

#[test]
fn batch_membership_retirement_rebuilds_once_and_preserves_other_owners() {
    for scope in [
        WebFontRegistrationScope::Stylesheet,
        WebFontRegistrationScope::FontFaceSetManual,
    ] {
        let mut services =
            DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
        for index in 0..64 {
            services
                .register_web_font(
                    registration(&format!("slot-{index}"), "Shared", TEST_TTF).with_scope(scope),
                )
                .unwrap();
        }
        services
            .register_web_font(registration("retained", "Shared", TEST_TTF).with_stylesheet_scope())
            .unwrap();
        services
            .register_web_font(registration("retained", "Shared", TEST_TTF))
            .unwrap();
        services.parley_mut();
        let mut slots = (0..64)
            .map(|index| format!("slot-{index}"))
            .collect::<Vec<_>>();
        slots.extend(["retained".into(), "missing".into(), "slot-0".into()]);
        let retire = |services: &mut DocumentLayoutServices| match scope {
            WebFontRegistrationScope::Stylesheet => {
                services.remove_stylesheet_web_fonts(slots.iter().map(String::as_str))
            }
            _ => services.remove_font_face_set_web_fonts(slots.iter().map(String::as_str)),
        };
        assert!(retire(&mut services));
        assert_eq!(services.font_collection_rebuilds, 1);
        assert_eq!(services.web_font_count(), 1);
        assert_eq!(
            shape_one_character(services.parley_mut(), web_font_style("Shared", 625.0), 'R'),
            TEST_TTF
        );
        assert!(!retire(&mut services));
        assert_eq!(services.font_collection_rebuilds, 1);
        let source = source_id(services.parley_mut(), "Shared");
        // Adding and retiring an independent relation must retain the source.
        services
            .register_web_font(registration("retained", "Shared", TEST_TTF).with_scope(scope))
            .unwrap();
        assert!(retire(&mut services));
        assert_eq!(source_id(services.parley_mut(), "Shared"), source);
        assert_eq!(services.font_collection_rebuilds, 1);
        assert!(services.remove_web_font("retained"));
        assert_eq!(services.font_collection_rebuilds, 2);
        assert_eq!(services.web_font_count(), 0);
    }
}

#[test]
fn temporary_css_order_keeps_sources_and_restores_selection_after_an_error() {
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    let narrow = include_bytes!("../../../tests/fixtures/moli-ahem-narrow.ttf");
    let wide = include_bytes!("../../../tests/fixtures/moli-ahem-wide.ttf");
    let face = WebFontFace::new("ScopedOrder").with_css_descriptors("100 900", "normal", "normal");
    for (slot, bytes) in [("wide", wide.as_slice()), ("narrow", narrow.as_slice())] {
        services
            .register_web_font(
                WebFontRegistration::new(slot, face.clone(), bytes.to_vec())
                    .with_stylesheet_scope(),
            )
            .unwrap();
    }
    services
        .register_web_font(registration("seed", "Seed", TEST_TTF))
        .unwrap();
    services.set_stylesheet_web_font_order(&["wide".into(), "narrow".into()]);
    let source = source_id(services.parley_mut(), "Seed");
    let context = std::ptr::from_ref(&services.parley_mut().font_context);
    let result: Result<(), ()> = services.with_stylesheet_font_declarations(
        ["narrow", "wide"].map(|slot| WebFontDeclaration {
            slot,
            source_key: "fixture",
            face: &face,
        }),
        |temporary| {
            assert_eq!(
                shape_one_character(
                    temporary.parley_mut(),
                    web_font_style("ScopedOrder", 700.0),
                    'A'
                ),
                wide
            );
            assert_eq!(source_id(temporary.parley_mut(), "Seed"), source);
            assert_eq!(
                std::ptr::from_ref(&temporary.parley_mut().font_context),
                context
            );
            Err(())
        },
    );
    assert!(result.is_err());
    assert_eq!(
        shape_one_character(
            services.parley_mut(),
            web_font_style("ScopedOrder", 700.0),
            'A'
        ),
        narrow
    );
    assert_eq!(source_id(services.parley_mut(), "Seed"), source);
    assert_eq!(
        std::ptr::from_ref(&services.parley_mut().font_context),
        context
    );
    assert_eq!(services.font_collection_rebuilds, 0);
}
