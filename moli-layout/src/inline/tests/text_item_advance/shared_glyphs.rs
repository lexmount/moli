use super::*;

fn shared(text: &str) -> Layout<TextBrush> {
    shaped_with_font(text, &[], 0.0, "\"Shared Glyph Fixture\"")
}

fn allocated(text: &str, ends: &[usize]) -> Layout<TextBrush> {
    let mut layout = shared(text);
    layout.set_text_item_quantization(1.0 / 64.0, ends);
    layout.break_all_lines(None);
    layout
}

fn items(layout: &Layout<TextBrush>) -> Vec<(std::ops::Range<usize>, f32, usize)> {
    layout
        .lines()
        .flat_map(|line| {
            line.runs()
                .map(|run| {
                    let glyph_count = run
                        .visual_clusters()
                        .flat_map(|cluster| cluster.glyphs())
                        .count();
                    (run.text_range(), run.advance(), glyph_count)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn shared_glyph_allocation_assigns_ligature_to_its_starting_text_item() {
    let mut raw = shared("ffi");
    raw.break_all_lines(None);
    let raw_glyphs = glyphs(&raw);
    assert_eq!(
        raw_glyphs.len(),
        1,
        "the fixed font must form the ffi ligature"
    );
    for ends in [&[1, 2, 3][..], &[1, 3], &[2, 3]] {
        let layout = allocated("ffi", ends);
        assert_eq!(
            layout.width(),
            18.0625,
            "item boundaries must not multiply a shared glyph's rounding"
        );
        assert_eq!(layout.calculate_content_widths().max, 18.0625);
        let parts = items(&layout);
        assert_eq!(parts[0].1, 18.0625);
        assert_eq!(parts[0].2, 1);
        assert!(parts[1..].iter().all(|part| part.1 == 0.0 && part.2 == 0));
        assert_eq!(
            glyphs(&layout),
            raw_glyphs,
            "shaping and glyph placement must remain intact"
        );
    }
}

#[test]
fn shared_glyph_allocation_preserves_prefix_and_suffix_positions() {
    let layout = allocated("xffiy", &[2, 5]);
    assert_eq!(
        items(&layout),
        vec![(0..2, 25.859375, 2), (2..5, 7.8125, 1)]
    );
    let positioned = glyphs(&layout);
    assert_eq!(positioned.len(), 3);
    assert_eq!(positioned[2].x, 25.859375);
    assert_eq!(layout.width(), 33.671875);
    assert_eq!(layout.calculate_content_widths().max, layout.width());
}

#[test]
fn shared_glyph_allocation_moves_combining_glyph_width_to_the_base_item() {
    let layout = allocated("a\u{301}", &[1, 3]);
    assert_eq!(items(&layout), vec![(0..1, 8.03125, 1), (1..3, 0.0, 0)]);
    assert_eq!(layout.width(), 8.03125);
}

#[test]
fn shared_glyph_allocation_uses_logical_ownership_for_rtl_glyphs() {
    let layout = allocated("لا", &[2, 4]);
    // Visual item order is reversed. The alef continuation has no glyph;
    // the lam node owns the ligature even though it appears later visually.
    assert_eq!(items(&layout), vec![(2..4, 0.0, 0), (0..2, 11.234375, 1)]);
    assert_eq!(glyphs(&layout).len(), 1);
    assert_eq!(glyphs(&layout)[0].x, 0.0);
    assert_eq!(layout.calculate_content_widths().max, 11.234375);
}

#[test]
fn shared_glyph_allocation_handles_bidi_runs_after_a_latin_prefix() {
    let layout = allocated("xلاy", &[3, 6]);
    assert_eq!(layout.width(), 26.859375);
    assert_eq!(layout.calculate_content_widths().max, layout.width());
    assert_eq!(
        items(&layout),
        vec![
            (0..1, 7.8125, 1),
            (3..5, 0.0, 0),
            (1..3, 11.234375, 1),
            (5..6, 7.8125, 1),
        ]
    );
}

#[test]
fn shared_glyph_allocation_reconfiguration_restores_shaping_before_regrouping() {
    for (text, split) in [("ffi", &[1, 2, 3][..]), ("لا", &[2, 4][..])] {
        let mut layout = shared(text);
        for ends in [split, &[text.len()][..], split] {
            layout.set_text_item_quantization(1.0 / 64.0, ends);
            let fresh = allocated(text, ends);
            let mut measured = layout.clone();
            measured.break_all_lines(None);
            assert_eq!(items(&measured), items(&fresh));
            assert_eq!(glyphs(&measured), glyphs(&fresh));
            let measured_widths = measured.calculate_content_widths();
            let fresh_widths = fresh.calculate_content_widths();
            assert_eq!(
                (measured_widths.min, measured_widths.max),
                (fresh_widths.min, fresh_widths.max)
            );
        }
    }
}

#[test]
fn shared_glyph_allocation_line_breaks_use_the_same_item_widths_as_measurement() {
    let text = "x ffi x";
    let mut layout = shared(text);
    layout.set_text_item_quantization(1.0 / 64.0, &[2, 3, 5, 7]);
    assert_eq!(layout.calculate_content_widths().max, 41.46875);
    for (width, lines) in [(29.75, 3), (29.765625, 2), (41.46875, 1), (29.75, 3)] {
        layout.break_all_lines(Some(width));
        assert_eq!(layout.len(), lines, "available width {width}");
        let mut fresh = shared(text);
        fresh.set_text_item_quantization(1.0 / 64.0, &[2, 3, 5, 7]);
        fresh.break_all_lines(Some(width));
        assert_eq!(items(&layout), items(&fresh));
        assert_eq!(glyphs(&layout), glyphs(&fresh));
    }
}

#[test]
fn shared_glyph_allocation_retains_line_breaks_between_rtl_words() {
    // Real Chromium at 13px wraps these three lam-alef ligatures into two
    // lines at 34px, whether the middle ligature crosses a text-item boundary.
    let text = "لا لا لا";
    for ends in [&[14][..], &[5, 7, 9, 14]] {
        let mut layout = shared(text);
        layout.set_text_item_quantization(1.0 / 64.0, ends);
        for width in [34.0, 11.234375, 34.0] {
            layout.break_all_lines(Some(width));
            let expected = if width == 34.0 { 2 } else { 3 };
            assert_eq!(
                layout.len(),
                expected,
                "RTL word breaks at {width}px, text items {ends:?}: {:?}",
                items(&layout),
            );
            assert_eq!(glyphs(&layout).len(), 5, "three ligatures and two spaces");
        }
    }
}

#[test]
fn rtl_shaping_cluster_metadata_stays_with_its_source_character() {
    let mut layout = shared("لا لا لا");
    layout.break_all_lines(None);
    let boundaries = layout
        .lines()
        .flat_map(|line| line.runs())
        .flat_map(|run| {
            run.clusters()
                .map(|cluster| (cluster.text_range().start, cluster.is_word_boundary()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        boundaries,
        vec![
            (0, true),
            (2, false),
            (4, true),
            (5, true),
            (7, false),
            (9, true),
            (10, true),
            (12, false),
        ],
        "lam, not the second logical character alef, owns each word boundary"
    );
    layout.break_all_lines(Some(34.0));
    assert_eq!(layout.len(), 2, "unquantized RTL text must still wrap");
}

#[test]
fn shared_glyph_allocation_preserves_shaping_for_every_text_partition() {
    let text = "ffiffia\u{301}لاx";
    let source = shared(text);
    let mut raw = source.clone();
    raw.break_all_lines(None);
    let signature = |layout: &Layout<TextBrush>| {
        glyphs(layout)
            .into_iter()
            .map(|glyph| (glyph.id, glyph.advance.to_bits()))
            .collect::<Vec<_>>()
    };
    let expected = signature(&raw);
    let boundaries = text
        .char_indices()
        .skip(1)
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    for partition in 0..(1 << boundaries.len()) {
        let mut ends = boundaries
            .iter()
            .enumerate()
            .filter_map(|(bit, &offset)| (partition & (1 << bit) != 0).then_some(offset))
            .collect::<Vec<_>>();
        ends.push(text.len());
        let mut layout = source.clone();
        layout.set_text_item_quantization(1.0 / 64.0, &ends);
        let intrinsic = layout.calculate_content_widths().max;
        layout.break_all_lines(None);
        assert_eq!(
            signature(&layout),
            expected,
            "partition {ends:?} must not drop, duplicate, reorder or reshape glyphs"
        );
        assert_eq!(
            layout.width(),
            intrinsic,
            "partition {ends:?} must allocate the same width in measurement and layout"
        );
    }
}
