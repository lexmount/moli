//! Browser-owned resolved Grid tracks projected from the numeric backend.

use style::Atom;

use crate::{
    LAYOUT_SUBPIXELS_PER_CSS_PIXEL, LayoutResolvedGridTrackList, LayoutResolvedGridTracks,
};

pub(crate) fn project_resolved_grid_tracks(
    style: &taffy::Style<Atom>,
    detailed: taffy::DetailedGridInfo,
) -> Option<LayoutResolvedGridTracks> {
    let row_line_names = expanded_line_names(
        &style.grid_template_rows,
        &style.grid_template_row_names,
        usize::from(detailed.rows.explicit_tracks),
        usize::from(detailed.rows.auto_repetitions),
    )?;
    let column_line_names = expanded_line_names(
        &style.grid_template_columns,
        &style.grid_template_column_names,
        usize::from(detailed.columns.explicit_tracks),
        usize::from(detailed.columns.auto_repetitions),
    )?;
    Some(LayoutResolvedGridTracks {
        rows: project_tracks(detailed.rows, row_line_names)?,
        columns: project_tracks(detailed.columns, column_line_names)?,
    })
}

fn project_tracks(
    tracks: taffy::DetailedGridTracksInfo,
    explicit_line_names: Vec<Vec<Atom>>,
) -> Option<LayoutResolvedGridTrackList> {
    let track_count = usize::from(tracks.negative_implicit_tracks)
        .checked_add(usize::from(tracks.explicit_tracks))?
        .checked_add(usize::from(tracks.positive_implicit_tracks))?;
    if tracks.sizes.len() != track_count
        || tracks.gutters.len() != track_count.saturating_add(1)
        || explicit_line_names.len() != usize::from(tracks.explicit_tracks).saturating_add(1)
    {
        return None;
    }
    let used_track_sizes = tracks
        .sizes
        .into_iter()
        .map(to_blink_layout_unit)
        .collect::<Option<Vec<_>>>()?;
    Some(LayoutResolvedGridTrackList {
        negative_implicit_track_count: usize::from(tracks.negative_implicit_tracks),
        explicit_track_count: usize::from(tracks.explicit_tracks),
        positive_implicit_track_count: usize::from(tracks.positive_implicit_tracks),
        used_track_sizes,
        explicit_line_names,
    })
}

/// Blink performs Grid sizing in a 26.6 fixed-point `LayoutUnit`. Taffy uses
/// floats, so truncate finite non-negative used track sizes at this ownership
/// boundary before they become observable through CSSOM.
fn to_blink_layout_unit(value: f32) -> Option<f32> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let raw = (f64::from(value) * f64::from(LAYOUT_SUBPIXELS_PER_CSS_PIXEL))
        .trunc()
        .min(f64::from(i32::MAX));
    Some(raw as f32 / LAYOUT_SUBPIXELS_PER_CSS_PIXEL)
}

fn expanded_line_names(
    template: &[taffy::GridTemplateComponent<Atom>],
    template_line_names: &[Vec<Atom>],
    explicit_track_count: usize,
    auto_repetitions: usize,
) -> Option<Vec<Vec<Atom>>> {
    if template.is_empty() {
        return template_line_names
            .is_empty()
            .then(|| vec![Vec::new(); explicit_track_count.saturating_add(1)]);
    }
    if template_line_names.len() != template.len().saturating_add(1) {
        return None;
    }

    let append_names = |line: &mut Vec<Atom>, names: &[Atom]| {
        line.extend(names.iter().cloned());
    };
    let mut lines = vec![Vec::new()];
    let mut expanded_track_count = 0usize;
    let mut saw_auto_repeat = false;
    for (index, component) in template.iter().enumerate() {
        append_names(lines.last_mut()?, template_line_names.get(index)?);
        match component {
            taffy::GridTemplateComponent::Single(_) => {
                expanded_track_count = expanded_track_count.checked_add(1)?;
                lines.push(Vec::new());
            }
            taffy::GridTemplateComponent::Repeat(repeat) => {
                if repeat.tracks.is_empty()
                    || repeat.line_names.len() != repeat.tracks.len().saturating_add(1)
                {
                    return None;
                }
                let repeat_count = match repeat.count {
                    taffy::RepetitionCount::Count(count) => usize::from(count),
                    taffy::RepetitionCount::AutoFill | taffy::RepetitionCount::AutoFit => {
                        if saw_auto_repeat {
                            return None;
                        }
                        saw_auto_repeat = true;
                        auto_repetitions
                    }
                };
                for _ in 0..repeat_count {
                    append_names(lines.last_mut()?, repeat.line_names.first()?);
                    for track_index in 0..repeat.tracks.len() {
                        expanded_track_count = expanded_track_count.checked_add(1)?;
                        lines.push(Vec::new());
                        append_names(lines.last_mut()?, repeat.line_names.get(track_index + 1)?);
                    }
                }
            }
        }
    }
    if !saw_auto_repeat && auto_repetitions != 0 {
        return None;
    }
    append_names(lines.last_mut()?, template_line_names.get(template.len())?);
    if expanded_track_count > explicit_track_count {
        return None;
    }

    // `grid-template-areas` can extend the explicit grid beyond the authored
    // track list. Those extra tracks have no authored line names; generated
    // area names do not serialize into the resolved track listing.
    lines.resize_with(explicit_track_count.saturating_add(1), Vec::new);
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::style_helpers::TaffyAuto;

    fn names(values: &[&str]) -> Vec<Atom> {
        values.iter().map(|value| Atom::from(*value)).collect()
    }

    fn track() -> taffy::TrackSizingFunction {
        taffy::TrackSizingFunction::AUTO
    }

    #[test]
    fn fixed_repeat_merges_names_at_each_expanded_boundary() {
        let template = vec![
            taffy::GridTemplateComponent::Single(track()),
            taffy::GridTemplateComponent::Repeat(taffy::GridTemplateRepetition {
                count: taffy::RepetitionCount::Count(2),
                tracks: vec![track(), track()],
                line_names: vec![names(&["c"]), names(&["d"]), names(&["e"])],
            }),
            taffy::GridTemplateComponent::Single(track()),
        ];
        let line_names = vec![names(&["a"]), names(&["b"]), names(&["f"]), names(&["g"])];

        assert_eq!(
            expanded_line_names(&template, &line_names, 6, 0),
            Some(vec![
                vec!["a".into()],
                vec!["b".into(), "c".into()],
                vec!["d".into()],
                vec!["e".into(), "c".into()],
                vec!["d".into()],
                vec!["e".into(), "f".into()],
                vec!["g".into()],
            ]),
        );
    }

    #[test]
    fn auto_repeat_uses_the_layout_result_count() {
        let template = vec![
            taffy::GridTemplateComponent::Single(track()),
            taffy::GridTemplateComponent::Repeat(taffy::GridTemplateRepetition {
                count: taffy::RepetitionCount::AutoFill,
                tracks: vec![track(), track()],
                line_names: vec![names(&["c"]), names(&["d"]), names(&["e"])],
            }),
            taffy::GridTemplateComponent::Single(track()),
        ];
        let line_names = vec![names(&["a"]), names(&["b"]), names(&["f"]), names(&["g"])];
        let expanded = expanded_line_names(&template, &line_names, 12, 5).expect("valid expansion");

        assert_eq!(expanded.len(), 13);
        assert_eq!(expanded[1], names(&["b", "c"]));
        assert_eq!(expanded[3], names(&["e", "c"]));
        assert_eq!(expanded[11], names(&["e", "f"]));
        assert_eq!(expanded[12], names(&["g"]));
    }

    #[test]
    fn area_expanded_explicit_grid_does_not_change_auto_repeat_count() {
        let template = vec![taffy::GridTemplateComponent::Repeat(
            taffy::GridTemplateRepetition {
                count: taffy::RepetitionCount::AutoFill,
                tracks: vec![track()],
                line_names: vec![names(&["a"]), names(&["b"])],
            },
        )];
        let line_names = vec![Vec::new(), Vec::new()];

        let expanded = expanded_line_names(&template, &line_names, 8, 5)
            .expect("grid-template-areas may extend the explicit grid after auto-repeat");

        assert_eq!(expanded.len(), 9);
        assert_eq!(expanded[0], names(&["a"]));
        assert_eq!(expanded[4], names(&["b", "a"]));
        assert_eq!(expanded[5], names(&["b"]));
        assert!(expanded[6..].iter().all(Vec::is_empty));
    }

    #[test]
    fn used_track_sizes_are_truncated_to_blink_layout_units() {
        assert_eq!(to_blink_layout_unit(100.0 / 3.0), Some(33.328_125));
        assert_eq!(to_blink_layout_unit(0.015_625), Some(0.015_625));
        assert_eq!(to_blink_layout_unit(-1.0), None);
        assert_eq!(to_blink_layout_unit(f32::NAN), None);
    }
}
