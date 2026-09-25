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

pub(super) struct TreeInsertionPlan<'a> {
    pub(super) parent: DomHandle,
    pub(super) insertion_roots: &'a [DomHandle],
    pub(super) inserting_fragment_children: bool,
    pub(super) lifecycle_connected_roots_before_insert: Vec<DomHandle>,
    pub(super) adoption: TreeAdoptionPlan,
    pub(super) focus_reset_handle_before_insert: Option<DomHandle>,
    pub(super) live_range_plan: Option<LiveRangePreInsertPlan>,
    pub(super) node_iterator_plan: Vec<NodeIteratorRemovalPlan>,
    pub(super) subtree_plan: InsertionSubtreePlan,
    pub(super) image_relevant_mutation_plan: ImageRelevantMutationPlan,
    pub(super) media_relevant_mutation_plan: MediaRelevantMutationPlan,
    pub(super) option_selectedness_before_insert: Option<Vec<(DomHandle, bool)>>,
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
    selectedness_policy: TreeInsertionSelectednessPolicy,
}

impl TreeInsertionPlan<'_> {
    pub(super) fn was_lifecycle_connected_before_insert(&self) -> bool {
        !self.lifecycle_connected_roots_before_insert.is_empty()
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
        selectedness_policy: TreeInsertionSelectednessPolicy,
    ) -> Self {
        Self {
            live_range_mode: TreeInsertionLiveRangeMode::Insert { reference_child },
            inserting_fragment_children,
            selectedness_policy,
        }
    }

    pub(super) fn replacement(old_child: DomHandle, inserting_fragment_children: bool) -> Self {
        Self {
            live_range_mode: TreeInsertionLiveRangeMode::Replace { old_child },
            inserting_fragment_children,
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
            live_range_plan,
            node_iterator_plan,
            subtree_plan,
            image_relevant_mutation_plan,
            media_relevant_mutation_plan,
            option_selectedness_before_insert,
        }
    }
}
