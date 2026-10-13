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
    world.boxes[owner.index()].inline_layout = Some(InlineFormattingContext {
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
    let output_bytes = pass.boxes[0]
        .inline_layout
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
fn numeric_entries_without_accepted_outputs_release_their_retained_buffers() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    for absent_ifc in [false, true] {
        let mut pass = numeric_world(1024, false, taffy::Style::default());
        if absent_ifc {
            pass.boxes[0].inline_layout = None;
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
