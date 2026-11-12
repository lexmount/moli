// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{Brush, LayoutContext, WhiteSpaceCollapse};

use parley_engine::break_overrides::LineBreakOverrideFn;

use parley_engine::{AnalysisOptions, BidiObject, LineBreakConfig};

use parlance::BaseDirection;

pub(crate) fn analyze_text<B: Brush>(
    lcx: &mut LayoutContext<B>,
    text: &str,
    base_direction: BaseDirection,
    line_break_override: Option<&LineBreakOverrideFn>,
) {
    let text = if text.is_empty() && lcx.inline_boxes.is_empty() {
        " "
    } else {
        text
    };

    // Collect the style runs' line break configurations. Gaps use the default configuration, so
    // only non-default configurations need an entry, and adjacent equal configurations are merged.
    //
    // Separately, collect the `break-spaces` runs, which allow wrapping after each preserved space
    // or tab. Adjacent runs are merged, so that an opportunity is not created before the first
    // space of a sequence spanning a style boundary.
    lcx.line_break.clear();
    lcx.break_spaces.clear();
    for style_run in lcx.style_runs.iter() {
        let style = &lcx.style_table[style_run.style_index as usize];
        let line_break = LineBreakConfig {
            word_break: style.word_break,
            line_break: style.line_break,
            language: style.locale,
        };
        if line_break != LineBreakConfig::default() {
            match lcx.line_break.last_mut() {
                Some((range, last))
                    if range.end == style_run.range.start && *last == line_break =>
                {
                    range.end = style_run.range.end;
                }
                _ => lcx.line_break.push((style_run.range.clone(), line_break)),
            }
        }
        if style.white_space_collapse == WhiteSpaceCollapse::BreakSpaces {
            match lcx.break_spaces.last_mut() {
                Some(last) if last.end == style_run.range.start => last.end = style_run.range.end,
                _ => lcx.break_spaces.push(style_run.range.clone()),
            }
        }
    }

    let options = AnalysisOptions {
        base_direction,
        line_break: &lcx.line_break,
        break_spaces: &lcx.break_spaces,
        line_break_override,
    };
    lcx.bidi_objects.clear();
    if lcx.inline_boxes.is_empty() {
        lcx.analyzer.analyze(text, &options, &mut lcx.analysis);
        return;
    }
    lcx.bidi_objects.extend(
        lcx.inline_boxes
            .iter()
            .map(|input| BidiObject::with_participation(input.inline_box.index, input.bidi)),
    );
    lcx.analyzer
        .analyze_with_objects(text, &options, &mut lcx.bidi_objects, &mut lcx.analysis);
    for (input, object) in lcx.inline_boxes.iter_mut().zip(&lcx.bidi_objects) {
        input.bidi_level = object.level();
    }
}
