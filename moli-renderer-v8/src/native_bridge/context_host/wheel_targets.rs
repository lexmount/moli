use super::mouse_click::in_shadow_including_subtree;
use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};
use moli_layout::{FrozenLayoutTree, LayoutDisplay};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

#[derive(PartialEq, Eq)]
struct ScrollBoxShape {
    source: DomHandle,
    display: LayoutDisplay,
    marker_inside: bool,
}

pub(super) struct WheelTargetWatch {
    document: DomHandle,
    source: DomHandle,
    box_path: Vec<ScrollBoxShape>,
    alive: Weak<AtomicBool>,
}

impl PageScriptHost {
    pub(crate) fn watch_wheel_scroll_target(
        &self,
        document: DomHandle,
        source: DomHandle,
    ) -> Option<Arc<AtomicBool>> {
        let box_path = self
            .with_latest_layout_tree_for_document(document, |tree| {
                scroll_box_path(self, tree, source)
            })
            .flatten()?;
        let alive = Arc::new(AtomicBool::new(true));
        let mut watches = self.wheel_scroll_targets.borrow_mut();
        watches.retain(|watch| {
            watch
                .alive
                .upgrade()
                .is_some_and(|alive| alive.load(Ordering::Relaxed))
        });
        watches.push(WheelTargetWatch {
            document,
            source,
            box_path,
            alive: Arc::downgrade(&alive),
        });
        Some(alive)
    }

    pub(super) fn note_wheel_scroll_target_layout(
        &self,
        document: DomHandle,
        tree: &FrozenLayoutTree<DomHandle>,
    ) {
        self.wheel_scroll_targets.borrow_mut().retain(|watch| {
            let Some(alive) = watch.alive.upgrade() else {
                return false;
            };
            let member = self
                .dom_host()
                .dom()
                .document_element_handle_for_document(watch.document)
                .and_then(|root| tree.tree_for_root(root));
            let survives = match member {
                Some(member) => {
                    scroll_box_path(self, member, watch.source).as_ref() == Some(&watch.box_path)
                }
                None => document != watch.document && document != self.document_handle(),
            };
            if !survives {
                alive.store(false, Ordering::Relaxed);
            }
            survives && alive.load(Ordering::Relaxed)
        });
    }

    pub(crate) fn note_wheel_scroll_target_removals(
        &mut self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        self.wheel_scroll_targets.get_mut().retain(|watch| {
            let Some(alive) = watch.alive.upgrade() else {
                return false;
            };
            if !alive.load(Ordering::Relaxed) {
                return false;
            }
            // Mutation facts preserve actual removals even when the same node
            // is reinserted before the next wheel, with no observer installed.
            for mutation in effects.style().child_list_mutations() {
                for &root in mutation.removed_nodes() {
                    if in_shadow_including_subtree(dom, root, watch.source) {
                        alive.store(false, Ordering::Relaxed);
                        return false;
                    }
                }
            }
            true
        });
    }
}

fn scroll_box_path(
    runtime: &PageScriptHost,
    tree: &FrozenLayoutTree<DomHandle>,
    source: DomHandle,
) -> Option<Vec<ScrollBoxShape>> {
    let scroller = tree
        .boxes
        .iter()
        .find(|layout_box| layout_box.principal_source == Some(source))?;
    if source != tree.source_root() && !scroller.scroll_extent.is_scroll_container {
        return None;
    }
    // Each publication numbers its boxes again. Compare the source-backed
    // boxes that establish this scroller's structure and their display types.
    let document = runtime.dom_host().owner_document_handle(source)?;
    runtime.with_style_resolution_pass(document, None, |pass| {
        std::iter::successors(Some(scroller), |layout_box| {
            tree.boxes.get(layout_box.structural_parent?.index())
        })
        .filter_map(|layout_box| {
            layout_box
                .principal_source
                .map(|source| (source, layout_box.display))
        })
        .map(|(source, display)| {
            let style = pass.element_styles(source)?.computed_values();
            Some(ScrollBoxShape {
                source,
                display,
                marker_inside: style.clone_list_style_position()
                    == style::computed_values::list_style_position::T::Inside,
            })
        })
        .collect()
    })
}
