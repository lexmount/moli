use super::AdoptionCallbackTarget;
use super::adopted_lifecycle::call_adopted_callback;
use super::attribute_lifecycle::call_attribute_changed_callback;
use super::form_lifecycle_callbacks::{
    call_form_associated_callback, call_form_disabled_callback, call_form_reset_callback,
    call_form_state_restore_callback,
};
use super::lifecycle::call_lifecycle_callback;
use super::reaction_upgrade::invoke_upgrade_reaction;
use super::reactions::CustomElementReaction;
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) fn invoke_custom_element_reactions_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    loop {
        let reaction = unsafe { &mut *host_ptr }
            .custom_element_reactions_mut()
            .next_reaction(handle);
        let Some(reaction) = reaction else {
            break;
        };
        match reaction {
            CustomElementReaction::Upgrade => {
                invoke_upgrade_reaction(scope, host_ptr, handle);
            }
            CustomElementReaction::Connected => {
                call_lifecycle_callback(scope, host_ptr, handle, "connectedCallback");
            }
            CustomElementReaction::Disconnected => {
                call_lifecycle_callback(scope, host_ptr, handle, "disconnectedCallback");
            }
            CustomElementReaction::ConnectedMove => {
                call_lifecycle_callback(scope, host_ptr, handle, "connectedMoveCallback");
            }
            CustomElementReaction::Adopted {
                old_document,
                new_document,
            } => {
                call_adopted_callback(
                    scope,
                    host_ptr,
                    AdoptionCallbackTarget {
                        handle,
                        old_document,
                        new_document,
                    },
                );
            }
            CustomElementReaction::AttributeChanged {
                name,
                namespace,
                old_value,
                new_value,
            } => {
                call_attribute_changed_callback(
                    scope,
                    host_ptr,
                    handle,
                    &name,
                    namespace.as_deref(),
                    old_value.as_deref(),
                    new_value.as_deref(),
                );
            }
            CustomElementReaction::FormAssociated { form } => {
                call_form_associated_callback(scope, host_ptr, handle, form);
            }
            CustomElementReaction::FormDisabled { disabled } => {
                call_form_disabled_callback(scope, host_ptr, handle, disabled);
            }
            CustomElementReaction::FormReset => {
                call_form_reset_callback(scope, host_ptr, handle);
            }
            CustomElementReaction::FormStateRestore { value, mode } => {
                call_form_state_restore_callback(scope, host_ptr, handle, &value, mode);
            }
        }
    }
}
