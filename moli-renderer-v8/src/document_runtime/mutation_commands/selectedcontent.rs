use super::{
    finish_runtime_mutation_effects, finish_runtime_script_start_candidate,
    tree::{TreeInsertionPlan, TreeInsertionPostConnectionStep, TreeReactionDispatchPolicy},
};
use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::{DomMutationEffects, Node},
    mutation_coordinator::{
        ConnectedScriptMutationPolicy, RuntimeMutationOptions, ScriptStartRequest,
    },
    native_bridge::JsContextHost,
    util::context_host_ptr_from_global_bridge,
};

fn selectedcontent_update_microtask_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Ok(value) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let (index, lossless) = value.u64_value();
    if !lossless {
        return;
    }
    let select = DomHandle::new(index as usize);
    if !unsafe { &mut *host_ptr }.take_pending_selectedcontent_update(select) {
        return;
    }
    let runtime: &mut DocumentRuntime = unsafe { &mut *host_ptr };
    let _ = runtime.sync_selectedcontents_for_select_in_reaction_scope(scope, host_ptr, select);
}

impl DocumentRuntime {
    pub(super) fn apply_tree_insertion_mutation_effects_with_post_connection_steps(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        effects: DomMutationEffects,
        options: RuntimeMutationOptions,
        reaction_policy: TreeReactionDispatchPolicy,
        run_post_connection_steps: bool,
        prepublished_removals: Vec<super::devtools_mutations::DevToolsDomPrepublishedRemoval>,
    ) -> bool {
        if !run_post_connection_steps
            || !insertion_plan.requires_interleaved_post_connection_steps()
        {
            let changed = self.apply_runtime_mutation_effects_with_prepublished_removals(
                scope,
                host_ptr,
                effects,
                options,
                prepublished_removals,
            );
            if changed && run_post_connection_steps {
                self.queue_selectedcontent_updates_after_selected_option_owner_change(
                    scope,
                    host_ptr,
                    &insertion_plan.selected_option_owners_before_insert,
                );
            }
            return changed;
        }

        let run_script_steps = matches!(
            options.connected_script_policy,
            ConnectedScriptMutationPolicy::PrepareAndStart
        );
        let script_start_requests = if run_script_steps {
            self.mutations
                .plan_connected_script_start_requests(&mut self.dom_host, &effects)
        } else {
            Vec::new()
        };
        let result = self.apply_runtime_mutation_effects_before_runtime_followups(
            scope,
            host_ptr,
            effects,
            options.with_connected_script_policy(ConnectedScriptMutationPolicy::DeferToOwner),
            prepublished_removals,
        );
        if result.did_change() {
            self.queue_selectedcontent_updates_after_selected_option_owner_change(
                scope,
                host_ptr,
                &insertion_plan.selected_option_owners_before_insert,
            );
            self.run_tree_insertion_post_connection_steps(
                scope,
                host_ptr,
                insertion_plan,
                script_start_requests,
                reaction_policy,
            );
        }
        finish_runtime_mutation_effects(self, scope, host_ptr, result)
    }

    pub(super) fn run_parser_tree_insertion_post_connection_steps(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
    ) {
        if insertion_plan.requires_interleaved_post_connection_steps() {
            self.run_tree_insertion_post_connection_steps(
                scope,
                host_ptr,
                insertion_plan,
                Vec::new(),
                TreeReactionDispatchPolicy::AppendToCurrentQueue,
            );
        }
    }

    fn run_tree_insertion_post_connection_steps(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        script_start_requests: Vec<ScriptStartRequest>,
        reaction_policy: TreeReactionDispatchPolicy,
    ) {
        match reaction_policy {
            TreeReactionDispatchPolicy::DispatchNow => {
                custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
                    self.run_tree_insertion_post_connection_steps_appending_to_current_reaction_queue(
                        scope,
                        host_ptr,
                        insertion_plan,
                        script_start_requests,
                    );
                });
            }
            TreeReactionDispatchPolicy::AppendToCurrentQueue => {
                self.run_tree_insertion_post_connection_steps_appending_to_current_reaction_queue(
                    scope,
                    host_ptr,
                    insertion_plan,
                    script_start_requests,
                );
            }
        }
    }

    fn run_tree_insertion_post_connection_steps_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        script_start_requests: Vec<ScriptStartRequest>,
    ) {
        let script_step_handles = insertion_plan
            .post_connection_steps
            .iter()
            .filter_map(|step| match step {
                TreeInsertionPostConnectionStep::Script(handle) => Some(*handle),
                _ => None,
            })
            .collect::<Vec<_>>();
        let (extra_script_requests, mut subtree_script_requests): (Vec<_>, Vec<_>) =
            script_start_requests
                .into_iter()
                .partition(|request| !script_step_handles.contains(&request.handle()));

        // A connected script parent can be prepared by a child insertion even
        // though it is outside the inserted subtree. It precedes every
        // descendant post-connection step in tree order.
        for request in extra_script_requests {
            self.run_script_start_request(scope, host_ptr, request);
        }

        for step in insertion_plan.post_connection_steps.iter().copied() {
            let handle = step.handle();
            if !self.dom_host.is_connected(handle) {
                continue;
            }
            match step {
                TreeInsertionPostConnectionStep::Script(script) => {
                    let Some(index) = subtree_script_requests
                        .iter()
                        .position(|request| request.handle() == script)
                    else {
                        continue;
                    };
                    let request = subtree_script_requests.remove(index);
                    self.run_script_start_request(scope, host_ptr, request);
                }
                TreeInsertionPostConnectionStep::SelectedContent(selectedcontent) => {
                    let _ = self.sync_selectedcontent_after_post_connection(
                        scope,
                        host_ptr,
                        selectedcontent,
                    );
                }
                TreeInsertionPostConnectionStep::SelectedOption(option) => {
                    let _ = self.sync_selectedcontents_for_selected_option(scope, host_ptr, option);
                }
            }
        }
    }

    fn run_script_start_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        request: ScriptStartRequest,
    ) {
        if request.clears_force_async() {
            let _ = self
                .dom_host
                .set_script_force_async(request.handle(), false);
        }
        let candidate = self.mutations.collect_connected_script_start_candidate(
            scope,
            host_ptr,
            &mut self.dom_host,
            request.handle(),
            &self.document,
        );
        if let Some(candidate) = candidate {
            finish_runtime_script_start_candidate(self, scope, host_ptr, candidate);
        }
    }

    pub(crate) fn sync_selectedcontents_for_select_in_reaction_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        select: DomHandle,
    ) -> bool {
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            self.sync_selectedcontents_for_select_appending_to_current_reaction_queue(
                scope, host_ptr, select,
            )
        })
    }

    pub(crate) fn sync_selectedcontents_after_parser_option_finished(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        option: DomHandle,
    ) -> bool {
        self.sync_selectedcontents_for_selected_option(scope, host_ptr, option)
    }

    fn sync_selectedcontents_for_selected_option(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        option: DomHandle,
    ) -> bool {
        let Some(select) = self.dom_host.option_nearest_ancestor_select(option) else {
            return false;
        };
        if self
            .dom_host
            .select_selectedcontent_elements(select)
            .is_empty()
        {
            return false;
        }
        if self
            .dom_host
            .select_selected_option_elements(select)
            .first()
            .copied()
            != Some(option)
        {
            return false;
        }
        self.sync_selectedcontents_for_select_appending_to_current_reaction_queue(
            scope, host_ptr, select,
        )
    }

    fn sync_selectedcontent_after_post_connection(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        selectedcontent: DomHandle,
    ) -> bool {
        let Some(select) = self
            .dom_host
            .selectedcontent_nearest_ancestor_select(selectedcontent)
        else {
            return false;
        };
        if self
            .dom_host
            .node(select)
            .and_then(Node::as_element)
            .is_some_and(|element| element.has_attribute("multiple"))
        {
            return false;
        }
        let selected_option = self
            .dom_host
            .select_selected_option_elements(select)
            .first()
            .copied();
        self.clone_selected_option_contents_into_selectedcontent(
            scope,
            host_ptr,
            selectedcontent,
            selected_option,
        )
    }

    pub(super) fn queue_selectedcontent_update_microtask(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        select: DomHandle,
    ) {
        if !unsafe { &mut *host_ptr }.mark_pending_selectedcontent_update(select) {
            return;
        }
        let data = v8::BigInt::new_from_u64(scope, select.index() as u64).into();
        let Some(callback) = v8::Function::builder(selectedcontent_update_microtask_callback)
            .data(data)
            .build(scope)
        else {
            let _ = unsafe { &mut *host_ptr }.take_pending_selectedcontent_update(select);
            return;
        };
        scope.enqueue_microtask(callback);
    }

    pub(super) fn queue_selectedcontent_updates_after_tree_removal(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        selected_option_owners_before_remove: &[(DomHandle, DomHandle)],
    ) {
        self.queue_selectedcontent_updates_after_selected_option_owner_change(
            scope,
            host_ptr,
            selected_option_owners_before_remove,
        );
    }

    fn queue_selectedcontent_updates_after_selected_option_owner_change(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        selected_option_owners_before_change: &[(DomHandle, DomHandle)],
    ) {
        for &(option, previous_select) in selected_option_owners_before_change {
            if self.dom_host.option_nearest_ancestor_select(option) != Some(previous_select) {
                self.queue_selectedcontent_update_microtask(scope, host_ptr, previous_select);
            }
        }
    }

    pub(super) fn selected_option_owners_in_subtrees(
        &self,
        roots: &[DomHandle],
    ) -> Vec<(DomHandle, DomHandle)> {
        let options = roots
            .iter()
            .flat_map(|root| {
                self.dom_host
                    .collect_matching_elements(*root, true, |handle| {
                        self.dom_host.is_html_element_named(handle, "option")
                    })
            })
            .collect::<Vec<_>>();
        let mut selected_option_by_select = Vec::new();
        let mut owners = Vec::new();
        for option in options {
            let Some(select) = self.dom_host.option_nearest_ancestor_select(option) else {
                continue;
            };
            let selected_option = if let Some((_, selected_option)) = selected_option_by_select
                .iter()
                .find(|(cached_select, _)| *cached_select == select)
            {
                *selected_option
            } else {
                let selected_option = self
                    .dom_host
                    .select_selected_option_elements(select)
                    .first()
                    .copied();
                selected_option_by_select.push((select, selected_option));
                selected_option
            };
            if selected_option == Some(option) {
                owners.push((option, select));
            }
        }
        owners
    }

    pub(crate) fn sync_selectedcontents_for_select_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        select: DomHandle,
    ) -> bool {
        let Some(select_element) = self.dom_host.node(select).and_then(Node::as_element) else {
            return false;
        };
        if !select_element.is_html_select() || select_element.has_attribute("multiple") {
            return false;
        }

        let targets = self.dom_host.select_selectedcontent_elements(select);
        if targets.is_empty() {
            return false;
        }
        let selected_option = self
            .dom_host
            .select_selected_option_elements(select)
            .first()
            .copied();
        let mut changed = false;
        for target in targets {
            changed |= self.clone_selected_option_contents_into_selectedcontent(
                scope,
                host_ptr,
                target,
                selected_option,
            );
        }
        changed
    }

    fn clone_selected_option_contents_into_selectedcontent(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        selectedcontent: DomHandle,
        selected_option: Option<DomHandle>,
    ) -> bool {
        let Some(document) = self.dom_host.owner_document_handle(selectedcontent) else {
            return false;
        };
        let fragment = self.create_document_fragment_for_document(document);
        if let Some(option) = selected_option {
            let source_children = self.dom_host.child_handles(option).collect::<Vec<_>>();
            for source_child in source_children {
                let Some(clone) = self.clone_node(scope, host_ptr, source_child, true) else {
                    return false;
                };
                if !self
                    .dom_host
                    .append_child_without_mutation_effects(fragment, clone)
                {
                    return false;
                }
            }
        }

        let existing_children = self
            .dom_host
            .child_handles(selectedcontent)
            .collect::<Vec<_>>();
        self.replace_all_children_with_fragment_appending_to_current_reaction_queue(
            scope,
            host_ptr,
            selectedcontent,
            fragment,
            &existing_children,
        )
    }
}
