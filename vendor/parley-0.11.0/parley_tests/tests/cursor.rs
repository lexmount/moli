// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cursor navigation tests.

// TODO: these should use the create_font_context helper to avoid introducing
// accidental dependencies on system fonts

use core::{iter::successors, ops::Range};

use crate::{
    test_name,
    util::{ColorBrush, CursorTest, TestEnv, env::create_font_context},
};
use parley::{Affinity, Cluster, ClusterPath, Cursor, LayoutContext, Selection};

#[test]
fn cursor_previous_visual() {
    let (mut lcx, mut fcx) = (LayoutContext::new(), create_font_context());
    let text = "Lorem ipsum dolor sit amet";
    let layout = CursorTest::single_line(text, &mut lcx, &mut fcx);

    let mut cursor: Cursor = layout.cursor_after("ipsum");
    layout.print_cursor(cursor);
    cursor = cursor.previous_visual(layout.layout());

    layout.assert_cursor_is_before("m dolor", cursor);
}

#[test]
fn cursor_next_visual() {
    let (mut lcx, mut fcx) = (LayoutContext::new(), create_font_context());
    let text = "Lorem ipsum dolor sit amet";
    let layout = CursorTest::single_line(text, &mut lcx, &mut fcx);

    let mut cursor: Cursor = layout.cursor_before("dolor");
    layout.print_cursor(cursor);
    cursor = cursor.next_visual(layout.layout());

    layout.assert_cursor_is_after("ipsum d", cursor);
}

#[test]
fn cursor_ligature_selection() {
    let mut env = TestEnv::new(test_name!(), None);
    // Test with ligature text "fi" using a font which has that ligature
    let text = "fi";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    // Make sure there's actually only one glyph (the ligature)
    let line = layout.lines().next().unwrap();
    let run = line.runs().next().unwrap();
    let cluster = run.clusters().next().unwrap();
    let glyphs: Vec<_> = cluster.glyphs().collect();
    assert_eq!(glyphs.len(), 1);

    // Test cursor positioning at the end of the text (byte index 2)
    // This should position the cursor at the end, not at the start of the cluster
    let cursor_end = Cursor::from_byte_index(&layout, 2, Affinity::Upstream);

    let selection: Selection = cursor_end.into();

    let focus = selection.focus();

    let clusters = focus.logical_clusters(&layout);

    assert_eq!(clusters[0].as_ref().map(|c| c.text_range()), Some(1..2));
}

/// Check consistency of cluster paths, navigation, and lookups.
fn check_cluster_navigation(text: &str, max_advance: Option<f32>) {
    let mut env = TestEnv::new(test_name!(), None);
    let mut layout = env.ranged_builder(text).build(text);
    layout.break_all_lines(max_advance);

    fn key(cluster: &Cluster<'_, ColorBrush>) -> (ClusterPath, Range<usize>) {
        (cluster.path(), cluster.text_range())
    }

    /// Checks that stepping from the first of `expected` visits exactly `expected`, in order.
    fn walk<'a>(
        expected: &[Cluster<'a, ColorBrush>],
        step: impl FnMut(&Cluster<'a, ColorBrush>) -> Option<Cluster<'a, ColorBrush>>,
        name: &str,
    ) {
        let walked = successors(expected.first().cloned(), step);
        let walked: Vec<_> = walked.map(|cluster| key(&cluster)).collect();
        let expected: Vec<_> = expected.iter().map(key).collect();
        assert_eq!(walked, expected, "{name}");
    }

    // Every cluster in the layout, in logical and in visual order.
    let mut logical = Vec::new();
    let mut visual = Vec::new();
    for line in layout.lines() {
        let on_line: Vec<_> = line.runs().flat_map(|run| run.visual_clusters()).collect();
        for (i, cluster) in on_line.iter().enumerate() {
            assert_eq!(cluster.is_start_of_line(), i == 0, "line start");
            assert_eq!(cluster.is_end_of_line(), i + 1 == on_line.len(), "line end");
        }
        for run in line.runs() {
            let run_logical_clusters: Vec<_> = run.clusters().collect();
            for cluster in &run_logical_clusters {
                let expected = key(cluster);
                let by_path = cluster.path().cluster(&layout);
                assert_eq!(
                    by_path.as_ref().map(key).unwrap(),
                    expected,
                    "path round trip"
                );
                for byte in cluster.text_range() {
                    let by_byte = Cluster::from_byte_index(&layout, byte);
                    assert_eq!(by_byte.as_ref().map(key).unwrap(), expected, "byte {byte}");
                }
            }
            let run_visual_clusters: Vec<_> = run.visual_clusters().collect();
            logical.extend(run_logical_clusters);
            visual.extend(run_visual_clusters);
        }
    }
    logical.sort_by_key(|cluster| cluster.text_range().start);

    walk(&logical, Cluster::next_logical, "forward logical");
    walk(&visual, Cluster::next_visual, "forward visual");
    logical.reverse();
    visual.reverse();
    walk(&logical, Cluster::previous_logical, "backward logical");
    walk(&visual, Cluster::previous_visual, "backward visual");
}

#[test]
fn cluster_navigation_mixed_directions_and_ligatures() {
    check_cluster_navigation("Hello Ligature: fi, Arabic: حداً", Some(60.0));
}

#[test]
fn cluster_navigation_hard_breaks_and_trailing_newline() {
    check_cluster_navigation("AAA \nااااا\nfi\n", None);
}

#[test]
fn cluster_navigation_multi_character_graphemes() {
    check_cluster_navigation("e\u{0301}x 🇳🇱 a\r\nb", None);
}
