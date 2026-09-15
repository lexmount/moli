use super::super::document_runtime::DomHandle;
use super::PendingCustomElementConstruction;
use super::definition::CustomElementStore;

impl CustomElementStore {
    /// Returns true when there is nothing for the upgrade / lifecycle pipeline
    /// to do for newly inserted subtrees: no constructors are registered, no
    /// elements have been upgraded. The vast majority of pages never call
    /// `customElements.define(...)` at all, in which case every appendChild /
    /// insertBefore can skip the recursive subtree scan entirely.
    pub(crate) fn is_subtree_lifecycle_quiescent(&self) -> bool {
        self.definitions.is_empty() && self.upgraded_handles.is_empty()
    }

    pub(crate) fn begin_construction(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        constructor: v8::Local<'_, v8::Function>,
        wrapper: v8::Local<'_, v8::Object>,
        handle: DomHandle,
    ) {
        self.construction_stack
            .begin_existing_element_upgrade(scope, constructor, wrapper, handle);
    }

    pub(crate) fn begin_create_element_construction(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        constructor: v8::Local<'_, v8::Function>,
        wrapper: v8::Local<'_, v8::Object>,
        handle: DomHandle,
    ) {
        self.construction_stack.begin_synchronous_create_element(
            scope,
            constructor,
            wrapper,
            handle,
        );
    }

    pub(crate) fn finish_construction(&mut self, handle: DomHandle) {
        self.upgraded_handles.insert(handle);
        self.discard_pending_construction(handle);
    }

    pub(crate) fn discard_pending_construction(&mut self, handle: DomHandle) {
        self.construction_stack.discard(handle);
    }

    pub(crate) fn take_pending_wrapper_for<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> Option<PendingCustomElementConstruction<'s>> {
        self.construction_stack
            .take_pending_wrapper_for(scope, new_target)
    }

    pub(crate) fn has_pending_wrapper_for(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> bool {
        self.construction_stack
            .has_pending_wrapper_for(scope, new_target)
    }

    pub(crate) fn is_upgraded_handle(&self, handle: DomHandle) -> bool {
        self.upgraded_handles.contains(&handle)
    }

    pub(crate) fn is_pending_construction_handle(&self, handle: DomHandle) -> bool {
        self.construction_stack.is_pending_handle(handle)
    }

    pub(super) fn pending_construction_is_already_constructed(&self, handle: DomHandle) -> bool {
        self.construction_stack.is_already_constructed(handle)
    }

    pub(crate) fn mark_failed_construction_handle(&mut self, handle: DomHandle) {
        self.construction_stack.mark_failed(handle);
    }

    pub(crate) fn is_failed_construction_handle(&self, handle: DomHandle) -> bool {
        self.construction_stack.is_failed(handle)
    }

    pub(crate) fn owns_custom_element_handle(&self, handle: DomHandle) -> bool {
        self.upgraded_handles.contains(&handle)
            || self.upgraded_definition_names.contains_key(&handle)
            || self.form_association_states.contains_key(&handle)
            || self.form_disabled_states.contains_key(&handle)
            || self.construction_stack.owns_handle(handle)
    }

    pub(super) fn form_association_state(&self, handle: DomHandle) -> Option<Option<DomHandle>> {
        self.form_association_states.get(&handle).copied()
    }

    pub(super) fn set_form_association_state(
        &mut self,
        handle: DomHandle,
        form: Option<DomHandle>,
    ) {
        self.form_association_states.insert(handle, form);
    }

    pub(super) fn form_disabled_state(&self, handle: DomHandle) -> Option<bool> {
        self.form_disabled_states.get(&handle).copied()
    }

    pub(super) fn set_form_disabled_state(&mut self, handle: DomHandle, disabled: bool) {
        self.form_disabled_states.insert(handle, disabled);
    }

    pub(super) fn mark_upgraded_handle(&mut self, handle: DomHandle, definition_name: &str) {
        self.construction_stack.clear_failed(handle);
        self.upgraded_handles.insert(handle);
        self.upgraded_definition_names
            .insert(handle, definition_name.to_owned());
    }
}
