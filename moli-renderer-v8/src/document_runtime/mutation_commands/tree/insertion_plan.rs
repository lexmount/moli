use super::{
    adoption::TreeAdoptionPlan,
    live_ranges::LiveRangePreInsertPlan,
    node_iterators::NodeIteratorRemovalPlan,
    resources::{ImageRelevantMutationPlan, InsertionSubtreePlan, MediaRelevantMutationPlan},
};
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::document_runtime::mutation_commands) enum TreeInsertionPostConnectionStep {
    Script(DomHandle),
    SelectedContent(DomHandle),
    SelectedOption(DomHandle),
}

impl TreeInsertionPostConnectionStep {
    pub(in crate::document_runtime::mutation_commands) fn handle(self) -> DomHandle {
        match self {
            Self::Script(handle) | Self::SelectedContent(handle) | Self::SelectedOption(handle) => {
                handle
            }
        }
    }
}

pub(in crate::document_runtime::mutation_commands) struct TreeInsertionPlan<'a> {
    pub(super) parent: DomHandle,
    pub(super) insertion_roots: &'a [DomHandle],
    pub(super) inserting_fragment_children: bool,
    pub(super) lifecycle_connected_roots_before_insert: Vec<DomHandle>,
    pub(super) adoption: TreeAdoptionPlan,
    pub(super) focus_reset_handle_before_insert: Option<DomHandle>,
    pub(super) scroll_anchor_adjustment: Option<(f64, f64)>,
    pub(super) live_range_plan: Option<LiveRangePreInsertPlan>,
    pub(super) node_iterator_plan: Vec<NodeIteratorRemovalPlan>,
    pub(super) subtree_plan: InsertionSubtreePlan,
    pub(super) image_relevant_mutation_plan: ImageRelevantMutationPlan,
    pub(super) media_relevant_mutation_plan: MediaRelevantMutationPlan,
    pub(super) option_selectedness_before_insert: Option<Vec<(DomHandle, bool)>>,
    pub(in crate::document_runtime::mutation_commands) selected_option_owners_before_insert:
        Vec<(DomHandle, DomHandle)>,
    pub(in crate::document_runtime::mutation_commands) post_connection_steps:
        Vec<TreeInsertionPostConnectionStep>,
}

#[derive(Clone, Copy)]
enum TreeInsertionLiveRangeMode {
    Insert { reference_child: Option<DomHandle> },
    Replace { old_child: DomHandle },
}

#[derive(Clone, Copy)]
pub(super) enum TreeInsertionSelectednessPolicy {
    Skip,
    CaptureAndRestore,
}

pub(super) struct TreeInsertionPlanOptions {
    live_range_mode: TreeInsertionLiveRangeMode,
    inserting_fragment_children: bool,
    scroll_anchor_adjustment: Option<(f64, f64)>,
    selectedness_policy: TreeInsertionSelectednessPolicy,
}

impl TreeInsertionPlan<'_> {
    pub(super) fn was_lifecycle_connected_before_insert(&self) -> bool {
        !self.lifecycle_connected_roots_before_insert.is_empty()
    }
    pub(in crate::document_runtime::mutation_commands) fn requires_interleaved_post_connection_steps(
        &self,
    ) -> bool {
        self.post_connection_steps
            .iter()
            .any(|step| !matches!(step, TreeInsertionPostConnectionStep::Script(_)))
    }
}

impl TreeInsertionLiveRangeMode {
    fn reference_child(self) -> Option<DomHandle> {
        match self {
            Self::Insert { reference_child } => reference_child,
            Self::Replace { old_child } => Some(old_child),
        }
    }
}

impl TreeInsertionSelectednessPolicy {
    fn captures(self) -> bool {
        matches!(self, Self::CaptureAndRestore)
    }
}

impl TreeInsertionPlanOptions {
    pub(super) fn insert(
        reference_child: Option<DomHandle>,
        inserting_fragment_children: bool,
        scroll_anchor_adjustment: Option<(f64, f64)>,
        selectedness_policy: TreeInsertionSelectednessPolicy,
    ) -> Self {
        Self {
            live_range_mode: TreeInsertionLiveRangeMode::Insert { reference_child },
            inserting_fragment_children,
            scroll_anchor_adjustment,
            selectedness_policy,
        }
    }

    pub(super) fn replacement(old_child: DomHandle, inserting_fragment_children: bool) -> Self {
        Self {
            live_range_mode: TreeInsertionLiveRangeMode::Replace { old_child },
            inserting_fragment_children,
            scroll_anchor_adjustment: None,
            selectedness_policy: TreeInsertionSelectednessPolicy::CaptureAndRestore,
        }
    }
}

impl DocumentRuntime {
    pub(super) fn tree_insertion_plan<'a>(
        &self,
        parent: DomHandle,
        insertion_roots: &'a [DomHandle],
        host_ptr: *mut JsContextHost,
        options: TreeInsertionPlanOptions,
    ) -> TreeInsertionPlan<'a> {
        let live_range_plan = if unsafe { &mut *host_ptr }.live_ranges_is_empty() {
            None
        } else {
            match options.live_range_mode {
                TreeInsertionLiveRangeMode::Insert { reference_child } => {
                    Some(self.live_range_pre_insert_plan(parent, insertion_roots, reference_child))
                }
                TreeInsertionLiveRangeMode::Replace { old_child } => {
                    self.live_range_replace_plan(parent, insertion_roots, old_child)
                }
            }
        };
        let reference_child = options.live_range_mode.reference_child();
        let node_iterator_plan = if unsafe { &*host_ptr }.node_iterators_is_empty() {
            Vec::new()
        } else {
            self.node_iterator_pre_insert_remove_plan(insertion_roots, reference_child)
        };
        let subtree_plan = self.insertion_subtree_plan(insertion_roots);
        let image_relevant_mutation_plan = if subtree_plan.may_have_image_relevant_picture_source {
            self.image_relevant_mutation_plan_before_insert(parent, insertion_roots)
        } else {
            Default::default()
        };
        let media_relevant_mutation_plan = if subtree_plan.may_have_media_sources {
            self.media_relevant_mutation_plan_before_insert(parent, insertion_roots)
        } else {
            Default::default()
        };
        let option_selectedness_before_insert = (options.selectedness_policy.captures()
            && subtree_plan.may_have_options)
            .then(|| self.option_selectedness_before_insert(insertion_roots));
        let selected_option_owners_before_insert = if subtree_plan.may_have_options {
            self.selected_option_owners_in_subtrees(insertion_roots)
        } else {
            Vec::new()
        };
        let post_connection_steps = self.tree_insertion_post_connection_steps(insertion_roots);
        let adoption = self.tree_adoption_plan_before_insert(host_ptr, insertion_roots, parent);
        let lifecycle_connected_roots_before_insert: Vec<DomHandle> = insertion_roots
            .iter()
            .copied()
            .filter(|handle| self.is_custom_element_lifecycle_connected(*handle))
            .collect();
        let focus_reset_handle_before_insert = self.focus_reset_handle_before_tree_change(
            insertion_roots,
            &lifecycle_connected_roots_before_insert,
        );
        TreeInsertionPlan {
            parent,
            insertion_roots,
            inserting_fragment_children: options.inserting_fragment_children,
            lifecycle_connected_roots_before_insert,
            adoption,
            focus_reset_handle_before_insert,
            scroll_anchor_adjustment: options.scroll_anchor_adjustment,
            live_range_plan,
            node_iterator_plan,
            subtree_plan,
            image_relevant_mutation_plan,
            media_relevant_mutation_plan,
            option_selectedness_before_insert,
            selected_option_owners_before_insert,
            post_connection_steps,
        }
    }

    fn tree_insertion_post_connection_steps(
        &self,
        roots: &[DomHandle],
    ) -> Vec<TreeInsertionPostConnectionStep> {
        let mut stack = roots.iter().rev().copied().collect::<Vec<_>>();
        let mut steps = Vec::new();
        while let Some(handle) = stack.pop() {
            let Some(node) = self.dom_host.node(handle) else {
                continue;
            };
            if node.is_script_element() {
                steps.push(TreeInsertionPostConnectionStep::Script(handle));
            } else if let Some(element) = node.as_element() {
                if element.is_html_element("selectedcontent") {
                    steps.push(TreeInsertionPostConnectionStep::SelectedContent(handle));
                } else if element.is_html_option() {
                    steps.push(TreeInsertionPostConnectionStep::SelectedOption(handle));
                }
            }

            stack.extend(self.dom_host.child_handles_reversed(handle));
            if let Some(shadow_root) = self.dom_host.shadow_root_handle(handle) {
                stack.push(shadow_root);
            }
        }
        steps
    }
}
