use crate::{
    custom_elements, dom::native::DomMutationEffects, mutation_coordinator::notify_dom_mutation,
    native_bridge::JsContextHost,
};

/// Document-specific resource and lifecycle work surrounding the shared parser
/// notifications. Owners keep their runtime state; they do not choose which
/// observer or custom-element notifications a mutation produces.
pub(crate) trait ParserMutationEffectsOwner {
    type Prepared;

    fn prepare_parser_mutation_effects(&mut self, effects: &DomMutationEffects) -> Self::Prepared;

    fn ensure_parser_reaction_queue(&mut self, host_ptr: *mut JsContextHost);

    fn finish_parser_mutation_effects(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        prepared: Self::Prepared,
    );
}

pub(crate) fn apply_parser_mutation_effects(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    owner: &mut impl ParserMutationEffectsOwner,
    effects: &DomMutationEffects,
) {
    if !effects.did_change() {
        return;
    }
    let prepared = owner.prepare_parser_mutation_effects(effects);
    let number_lists =
        crate::context_bootstrap::svg_runtime::collect_number_list_attribute_projections(
            scope,
            host_ptr,
            effects.style().attribute_mutations(),
        );
    crate::context_bootstrap::svg_runtime::apply_number_list_attribute_projections(
        scope,
        number_lists,
    );
    notify_dom_mutation(scope, host_ptr, unsafe { &*host_ptr }.dom_host(), effects);

    let form_owner_changed = custom_elements::form_owner_mutation_effects_touch_html_form(
        unsafe { &*host_ptr }.dom_host(),
        effects,
    );
    let tree = effects.tree();
    if !tree.disconnected_roots().is_empty()
        || !tree.connected_roots().is_empty()
        || form_owner_changed
    {
        owner.ensure_parser_reaction_queue(host_ptr);
        for &root in tree.disconnected_roots() {
            custom_elements::enqueue_disconnected_callbacks_for_subtree(scope, host_ptr, root);
        }
        custom_elements::enqueue_connected_and_form_callbacks_for_already_upgraded_subtrees(
            scope,
            host_ptr,
            tree.connected_roots(),
        );
        if form_owner_changed {
            custom_elements::enqueue_form_association_callbacks_for_all(scope, host_ptr);
        }
    }
    owner.finish_parser_mutation_effects(scope, host_ptr, prepared);
}
