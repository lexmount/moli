use super::*;
use crate::{
    DocumentLayoutServices,
    inline::{InlineContentWidthsMemo, InlineFragments, InlineNumericInputs, InlineSourceFragment},
};

fn numeric_world(
    fragment_capacity: usize,
    accepted: bool,
    numeric_style: taffy::Style<style::Atom>,
) -> LayoutWorld<usize> {
    let mut world = super::tests::world(None);
    let owner = world.root;
    world.boxes[owner.index()].inline_formatting_context = true;
    let style = &world.boxes[owner.index()].style;
    let inputs = InlineNumericInputs {
        layout: taffy::LayoutInput {
            run_mode: taffy::RunMode::PerformLayout,
            ..taffy::LayoutInput::HIDDEN
        },
        writing_mode: style.writing_mode(),
        resolved_aspect_ratio: None,
        scrollbar_insets: taffy::Rect {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
        display: style.display(),
        alignment: style.text_align(),
        direction: style.direction(),
        parent_is_flex_or_grid: false,
    };
    let mut numeric = InlineNumericReuse::new(world.len());
    numeric.store(owner, inputs, numeric_style, taffy::LayoutOutput::DEFAULT);
    world.inline_numeric_reuse = Some(Box::new(numeric));
    world.prepare_inline_storage();
    world.inline_layouts[owner.index()] = Some(InlineFormattingContext {
        text: String::new(),
        root_style: owner,
        measurement_layout: Some(parley::Layout::default()),
        laid_out: accepted.then(parley::Layout::default),
        content_widths: InlineContentWidthsMemo::default(),
        text_units: Vec::new(),
        source_map: Vec::new(),
        selection: None,
        objects: Vec::new(),
        font_metrics: Vec::new(),
        parent_strut: None,
        uses_quirks_line_height: false,
        root_includes_used_font_metrics: false,
        style_parents: Vec::new(),
        structural_boxes: Vec::new(),
        line_placements: Vec::new(),
        fragments: InlineFragments {
            text: Vec::with_capacity(fragment_capacity),
            ..InlineFragments::default()
        },
    });
    world
}

#[test]
fn retained_processed_text_capacity_counts_towards_admission() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(
        numeric_world(0, true, taffy::Style::default()),
        fonts.clone(),
        0,
        0,
        1,
    );
    let baseline_stats = baseline.stats();
    let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: baseline_stats.estimated_retained_bytes + 1024,
        ..LayoutWorkingSetLimits::default()
    });
    let mut pass = numeric_world(0, true, taffy::Style::default());
    let text = &mut pass.inline_layouts[0].as_mut().unwrap().text;
    *text = String::with_capacity(8192);
    let owned_text_bytes = text.capacity();
    assert!(text.is_empty());
    work.publish(pass, fonts, 0, 0, 1);
    let stats = work.stats();
    assert_eq!(
        stats.admission,
        LayoutWorkingSetAdmission::EstimatedByteLimit
    );
    assert_eq!(stats.retained_boxes, 0);
    assert_eq!(stats.estimated_retained_bytes, 0);
    assert_eq!(
        stats.estimated_inline_bytes,
        baseline_stats.estimated_inline_bytes + owned_text_bytes
    );
    assert!(work.retained.is_none());
}

#[test]
fn retained_numeric_output_capacity_counts_towards_admission() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(
        numeric_world(0, true, taffy::Style::default()),
        fonts.clone(),
        0,
        0,
        1,
    );
    let baseline_stats = baseline.stats();
    assert!(baseline_stats.estimated_inline_numeric_bytes > 0);
    let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: baseline_stats.estimated_retained_bytes + 1024,
        ..LayoutWorkingSetLimits::default()
    });
    let pass = numeric_world(1024, true, taffy::Style::default());
    let output_bytes = pass.inline_layouts[0]
        .as_ref()
        .unwrap()
        .fragments
        .text
        .capacity()
        * std::mem::size_of::<InlineSourceFragment>();
    work.publish(pass, fonts, 0, 0, 1);
    let stats = work.stats();
    assert_eq!(
        stats.admission,
        LayoutWorkingSetAdmission::EstimatedByteLimit
    );
    assert_eq!(stats.retained_boxes, 0);
    assert_eq!(stats.estimated_retained_bytes, 0);
    assert_eq!(
        stats.estimated_inline_numeric_bytes,
        baseline_stats.estimated_inline_numeric_bytes + output_bytes
    );
    assert_eq!(
        stats.candidate_estimated_bytes,
        stats.estimated_graph_bytes
            + stats.estimated_inline_bytes
            + stats.estimated_font_metadata_bytes
    );
    assert!(work.retained.is_none());
}

#[test]
fn retained_numeric_style_key_capacity_counts_towards_admission() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(
        numeric_world(0, true, taffy::Style::default()),
        fonts.clone(),
        0,
        0,
        1,
    );
    let baseline_stats = baseline.stats();
    let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: baseline_stats.estimated_retained_bytes + 1024,
        ..LayoutWorkingSetLimits::default()
    });
    let names = Vec::<style::Atom>::with_capacity(1024);
    let owned_key_bytes = names.capacity() * std::mem::size_of::<style::Atom>();
    let numeric_style = taffy::Style {
        grid_template_row_names: vec![names],
        ..taffy::Style::default()
    };
    work.publish(numeric_world(0, true, numeric_style), fonts, 0, 0, 1);
    let stats = work.stats();
    assert_eq!(
        stats.admission,
        LayoutWorkingSetAdmission::EstimatedByteLimit
    );
    assert!(
        stats.estimated_inline_numeric_bytes
            >= baseline_stats.estimated_inline_numeric_bytes + owned_key_bytes
    );
    assert!(work.retained.is_none());
}

#[test]
fn measurement_capacity_is_charged_and_evicted_before_valid_final_results() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(
        numeric_world(0, true, taffy::Style::default()),
        fonts.clone(),
        0,
        0,
        1,
    );
    let base_bytes = baseline.stats().estimated_retained_bytes;
    let mut pass = numeric_world(0, true, taffy::Style::default());
    let owner = pass.root;
    let style = &pass.boxes[0].style;
    let inputs = crate::inline::InlineMeasurementInputs {
        numeric: InlineNumericInputs {
            layout: taffy::LayoutInput {
                run_mode: taffy::RunMode::ComputeSize,
                ..taffy::LayoutInput::HIDDEN
            },
            writing_mode: style.writing_mode(),
            resolved_aspect_ratio: None,
            scrollbar_insets: taffy::Rect::zero(),
            display: style.display(),
            alignment: style.text_align(),
            direction: style.direction(),
            parent_is_flex_or_grid: false,
        },
        measure_baselines: true,
    };
    let numeric_style = taffy::Style {
        grid_template_row_names: vec![Vec::with_capacity(2048)],
        ..taffy::Style::default()
    };
    pass.inline_numeric_reuse
        .as_mut()
        .unwrap()
        .store_measurement(owner, inputs, numeric_style, taffy::LayoutOutput::DEFAULT);
    let measured = pass
        .inline_numeric_reuse
        .as_ref()
        .unwrap()
        .estimated_retained_bytes();
    assert!(measured > base_bytes + 1024);
    let mut roomy = LayoutWorkingSet::default();
    roomy.publish(pass, fonts.clone(), 0, 0, 1);
    assert!(roomy.stats().estimated_retained_bytes > base_bytes + 1024);
    let mut restored = roomy.take().unwrap();
    restored.discard_inline_measurements();
    assert_eq!(restored.estimated_retention().total(), base_bytes);

    let mut pass = numeric_world(0, true, taffy::Style::default());
    let numeric_style = taffy::Style {
        grid_template_row_names: vec![Vec::with_capacity(2048)],
        ..taffy::Style::default()
    };
    pass.inline_numeric_reuse
        .as_mut()
        .unwrap()
        .store_measurement(owner, inputs, numeric_style, taffy::LayoutOutput::DEFAULT);
    let mut limited = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: base_bytes + 1024,
        ..Default::default()
    });
    limited.publish(pass, fonts, 0, 0, 1);
    assert_eq!(
        limited.stats().admission,
        LayoutWorkingSetAdmission::Retained
    );
    assert_eq!(limited.stats().estimated_retained_bytes, base_bytes);
    let retained = limited.retained.as_mut().unwrap();
    let numeric = retained.inline_numeric_reuse.as_mut().unwrap();
    assert!(
        numeric.contains(owner),
        "the accepted final result must survive"
    );
    assert!(
        numeric
            .lookup_measurement(owner, inputs, &taffy::Style::default())
            .is_none()
    );
    assert!(retained.inline_contexts[0].1.laid_out.is_some());
    limited.clear();
    assert_eq!(limited.stats(), LayoutWorkingSetStats::default());
}

#[test]
fn numeric_entries_without_accepted_outputs_release_their_retained_buffers() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    for absent_ifc in [false, true] {
        let mut pass = numeric_world(1024, false, taffy::Style::default());
        if absent_ifc {
            pass.inline_layouts[0] = None;
        }
        let mut work = LayoutWorkingSet::default();
        work.publish(pass, fonts.clone(), 0, 0, 1);
        assert_eq!(work.stats().admission, LayoutWorkingSetAdmission::Retained);
        assert_eq!(work.stats().estimated_inline_numeric_bytes, 0);
        let retained = work.retained.as_ref().unwrap();
        assert!(retained.inline_numeric_reuse.is_none());
        if let Some((_, inline)) = retained.inline_contexts.first() {
            assert!(inline.measurement_layout.is_some());
            assert!(inline.laid_out.is_none());
            assert_eq!(inline.fragments.text.capacity(), 0);
        }
    }
}

#[test]
fn retained_numeric_output_preserves_one_paragraph_until_explicit_eviction() {
    let source_text: Arc<str> = Arc::from("owner input");
    let source_weak = Arc::downgrade(&source_text);
    let mut pass = numeric_world(64, true, taffy::Style::default());
    pass.boxes[0].text = Some(source_text);
    let mut work = LayoutWorkingSet::default();
    work.publish(pass, DocumentLayoutServices::new().font_inputs(), 0, 0, 1);
    let retained = work.retained.as_ref().unwrap();
    assert!(retained.inline_numeric_reuse.is_some());
    let (_, inline) = &retained.inline_contexts[0];
    assert!(inline.measurement_layout.is_none());
    assert!(inline.laid_out.is_some());
    assert!(inline.fragments.text.capacity() >= 64);
    assert!(work.stats().estimated_inline_numeric_bytes > 0);

    work.clear();
    assert!(work.retained.is_none());
    assert_eq!(work.stats(), LayoutWorkingSetStats::default());
    assert!(source_weak.upgrade().is_none());
}

#[test]
fn partial_numeric_owner_index_grows_shrinks_and_rejects_removed_entries() {
    let mut world = numeric_world(0, true, taffy::Style::default());
    let style = &world.boxes[0].style;
    let inputs = InlineNumericInputs {
        layout: taffy::LayoutInput {
            run_mode: taffy::RunMode::PerformLayout,
            ..taffy::LayoutInput::HIDDEN
        },
        writing_mode: style.writing_mode(),
        resolved_aspect_ratio: None,
        scrollbar_insets: taffy::Rect {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        },
        display: style.display(),
        alignment: style.text_align(),
        direction: style.direction(),
        parent_is_flex_or_grid: false,
    };
    let mut numeric = world.inline_numeric_reuse.take().unwrap();
    numeric.retain_owners_for_box_count(257, |_| true);
    assert!(
        numeric.estimated_retained_bytes()
            >= std::mem::size_of::<InlineNumericReuse>()
                + 257 * std::mem::size_of::<Option<usize>>()
    );
    let last = LayoutBoxId::from_index(256);
    let mut old_output = taffy::LayoutOutput::DEFAULT;
    old_output.size.width = 17.0;
    numeric.store(last, inputs, taffy::Style::default(), old_output);
    assert_eq!(
        numeric
            .lookup(last, inputs, &taffy::Style::default())
            .unwrap()
            .size
            .width,
        17.0
    );

    // The caller's predicate deliberately accepts everything: the new graph
    // bound itself must retire an out-of-range owner before rebuilding slots.
    numeric.retain_owners_for_box_count(1, |_| true);
    assert!(!numeric.contains(last));
    assert!(
        numeric
            .lookup(last, inputs, &taffy::Style::default())
            .is_none()
    );
    numeric.retain_owners_for_box_count(257, |_| true);
    assert!(
        !numeric.contains(last),
        "regrowth must not revive a removed dense slot"
    );
    let mut new_output = taffy::LayoutOutput::DEFAULT;
    new_output.size.width = 29.0;
    numeric.store(last, inputs, taffy::Style::default(), new_output);
    assert_eq!(
        numeric
            .lookup(last, inputs, &taffy::Style::default())
            .unwrap()
            .size
            .width,
        29.0
    );
    assert_eq!(
        numeric
            .lookup(LayoutBoxId::from_index(0), inputs, &taffy::Style::default())
            .unwrap()
            .size
            .width,
        0.0
    );

    // Shrinking the logical graph keeps this allocation available for the
    // next append. Its remaining capacity must still enter admission charge.
    numeric.retain_owners_for_box_count(1, |_| true);
    let fonts = DocumentLayoutServices::new().font_inputs();
    let fresh = numeric_world(0, true, taffy::Style::default());
    let fresh_numeric_bytes = fresh
        .inline_numeric_reuse
        .as_ref()
        .unwrap()
        .estimated_retained_bytes();
    let excess = numeric.estimated_retained_bytes() - fresh_numeric_bytes;
    assert!(excess >= 256 * std::mem::size_of::<Option<usize>>());
    let mut baseline = LayoutWorkingSet::default();
    baseline.publish(fresh, fonts.clone(), 0, 0, 1);
    let baseline_stats = baseline.stats();
    world.inline_numeric_reuse = Some(numeric);
    let mut bounded = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: baseline_stats.estimated_retained_bytes + excess - 1,
        ..LayoutWorkingSetLimits::default()
    });
    bounded.publish(world, fonts, 0, 0, 1);
    assert_eq!(
        bounded.stats().admission,
        LayoutWorkingSetAdmission::EstimatedByteLimit
    );
    assert_eq!(
        bounded.stats().candidate_estimated_bytes,
        baseline_stats.estimated_retained_bytes + excess
    );
    assert_eq!(
        bounded.stats().estimated_inline_numeric_bytes,
        baseline_stats.estimated_inline_numeric_bytes + excess
    );
    assert!(bounded.retained.is_none());
}
