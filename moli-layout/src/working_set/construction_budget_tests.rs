use super::*;
use crate::{
    DocumentLayoutServices, LayoutElementCategory, LayoutError, LayoutNamespace, LayoutSource,
    LayoutSourceKind, LayoutStyleResolver, PaintColor, ResolvedLayoutElementStyles,
    builder::{build_layout_world, build_layout_world_with_retained_inputs},
};

struct Source {
    hidden_name: Arc<str>,
    children: [Vec<usize>; 3],
}

impl Source {
    fn new() -> Self {
        Self {
            hidden_name: Arc::from("hidden-construction-dependency"),
            children: [vec![1, 2], Vec::new(), Vec::new()],
        }
    }
}

impl LayoutSource for Source {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> usize {
        0
    }

    fn flat_parent(&self, node: usize) -> Option<usize> {
        (node != 0).then_some(0)
    }

    fn flat_children(&self, node: usize) -> Self::ChildIter<'_> {
        self.children[node].iter().copied()
    }

    fn node_kind(&self, node: usize) -> LayoutSourceKind {
        if node == 1 {
            LayoutSourceKind::Text
        } else {
            LayoutSourceKind::Element
        }
    }

    fn element_semantics(&self, node: usize) -> Option<LayoutElementSemantics> {
        match node {
            0 => Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "div",
                LayoutElementCategory::Generic,
                None,
            )),
            2 => Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                self.hidden_name.clone(),
                LayoutElementCategory::Generic,
                None,
            )),
            _ => None,
        }
    }

    fn text(&self, node: usize) -> Option<&str> {
        (node == 1).then_some("construction input")
    }

    fn label(&self, node: usize) -> String {
        format!("source {node}")
    }
}

struct Styles {
    root_display: LayoutDisplay,
}

impl Styles {
    fn block() -> Self {
        Self {
            root_display: LayoutDisplay::Block,
        }
    }
}

impl LayoutStyleResolver<usize> for Styles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok(Some(ResolvedLayoutElementStyles::from_primary(
            ResolvedLayoutStyle::synthetic(
                if node == 2 {
                    LayoutDisplay::None
                } else {
                    self.root_display
                },
                taffy::Style::default(),
                PaintColor::TRANSPARENT,
            ),
        )))
    }
}

fn construction_world(source: &Source) -> LayoutWorld<usize> {
    build_layout_world_with_retained_inputs(source, &mut Styles::block(), None).unwrap()
}

#[test]
fn construction_dependency_capacity_counts_towards_retention_admission() {
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut baseline = LayoutWorkingSet::default();
    let pass = construction_world(&Source::new());
    let count = pass.len();
    baseline.publish(pass, fonts.clone(), 0, 0, count);
    let baseline_stats = baseline.stats();
    assert_eq!(
        baseline_stats.admission,
        LayoutWorkingSetAdmission::Retained
    );
    assert!(baseline_stats.estimated_construction_bytes > 0);

    let source = Source::new();
    let dependency = Arc::downgrade(&source.hidden_name);
    let mut pass = construction_world(&source);
    let inputs = pass.construction_inputs.as_mut().unwrap();
    inputs.reserve_source_capacity_for_test(4096);
    let dependency_bytes = inputs.estimated_retained_bytes();
    assert!(dependency_bytes > baseline_stats.estimated_construction_bytes + 1024);
    drop(source);

    let mut work = LayoutWorkingSet::new(LayoutWorkingSetLimits {
        max_estimated_retained_bytes: baseline_stats.estimated_retained_bytes + 1024,
        ..LayoutWorkingSetLimits::default()
    });
    let count = pass.len();
    work.publish(pass, fonts, 0, 0, count);
    let stats = work.stats();
    assert_eq!(
        stats.admission,
        LayoutWorkingSetAdmission::EstimatedByteLimit
    );
    assert_eq!(stats.retained_boxes, 0);
    assert_eq!(stats.estimated_retained_bytes, 0);
    assert_eq!(stats.estimated_construction_bytes, dependency_bytes);
    assert!(stats.candidate_estimated_bytes > work.limits().max_estimated_retained_bytes);
    assert_eq!(
        stats.candidate_estimated_bytes,
        stats.estimated_graph_bytes
            + stats.estimated_inline_bytes
            + stats.estimated_font_metadata_bytes
    );
    assert!(work.retained.is_none());
    assert!(
        dependency.upgrade().is_none(),
        "rejection must release source dependencies"
    );
}

#[test]
fn clear_and_drop_release_dependencies_for_sources_without_boxes() {
    for explicit_clear in [false, true] {
        let source = Source::new();
        let dependency = Arc::downgrade(&source.hidden_name);
        let pass = construction_world(&source);
        assert!(
            pass.boxes
                .iter()
                .all(|layout_box| layout_box.source != Some(2))
        );
        drop(source);
        assert!(dependency.upgrade().is_some());

        let mut work = LayoutWorkingSet::default();
        let count = pass.len();
        work.publish(
            pass,
            DocumentLayoutServices::new().font_inputs(),
            0,
            0,
            count,
        );
        assert_eq!(work.stats().admission, LayoutWorkingSetAdmission::Retained);
        assert!(
            work.retained
                .as_ref()
                .unwrap()
                .construction_inputs()
                .is_some()
        );
        assert!(dependency.upgrade().is_some());
        if explicit_clear {
            work.clear();
            assert_eq!(work.stats(), LayoutWorkingSetStats::default());
            assert!(dependency.upgrade().is_none());
        }
        drop(work);
        assert!(dependency.upgrade().is_none());
    }
}

#[test]
fn one_shot_and_unsupported_construction_do_not_retain_source_dependencies() {
    for ordinary in [false, true] {
        let source = Source::new();
        let dependency = Arc::downgrade(&source.hidden_name);
        let pass = if ordinary {
            build_layout_world(&source, &mut Styles::block()).unwrap()
        } else {
            build_layout_world_with_retained_inputs(
                &source,
                &mut Styles {
                    root_display: LayoutDisplay::Flex,
                },
                None,
            )
            .unwrap()
        };
        assert!(pass.construction_inputs.is_none());
        drop(source);
        assert!(dependency.upgrade().is_none());

        let mut work = LayoutWorkingSet::default();
        let count = pass.len();
        work.publish(
            pass,
            DocumentLayoutServices::new().font_inputs(),
            0,
            0,
            count,
        );
        assert_eq!(work.stats().admission, LayoutWorkingSetAdmission::Retained);
        assert_eq!(work.stats().estimated_construction_bytes, 0);
        assert_eq!(work.stats().reused_construction_boxes, 0);
        assert!(
            work.retained
                .as_ref()
                .unwrap()
                .construction_inputs()
                .is_none()
        );
    }
}

#[test]
fn construction_reuse_statistics_reset_after_eviction() {
    let source = Source::new();
    let fonts = DocumentLayoutServices::new().font_inputs();
    let mut work = LayoutWorkingSet::default();
    let pass = construction_world(&source);
    let count = pass.len();
    work.publish(pass, fonts.clone(), 0, 0, count);
    assert_eq!(work.stats().reused_construction_boxes, 0);

    let mut previous = work.take();
    let pass =
        build_layout_world_with_retained_inputs(&source, &mut Styles::block(), previous.as_mut())
            .unwrap();
    let count = pass.len();
    let (pass, reused) = update_working_tree(previous, pass, &fonts);
    work.publish(pass, fonts.clone(), reused, 0, count);
    assert_eq!(work.stats().reused_construction_boxes, count);

    work.clear();
    assert_eq!(work.stats(), LayoutWorkingSetStats::default());
    let pass = construction_world(&source);
    let count = pass.len();
    work.publish(pass, fonts, 0, 0, count);
    assert_eq!(work.stats().reused_construction_boxes, 0);
}
