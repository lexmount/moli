use super::super::util::{v8_string, v8str};
use super::definition::{CustomElementCallbacks, CustomElementDefineError};
use anyhow::Result;

pub(super) fn custom_element_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
) -> Result<v8::Local<'s, v8::Object>, CustomElementDefineError> {
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .ok_or(CustomElementDefineError::PendingException)?;
    v8::Local::<v8::Object>::try_from(prototype)
        .map_err(|_| CustomElementDefineError::InvalidPrototype)
}

pub(super) fn callbacks_for_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<CustomElementCallbacks, CustomElementDefineError> {
    let connected = callback_for_prototype(scope, prototype, "connectedCallback")?;
    let disconnected = callback_for_prototype(scope, prototype, "disconnectedCallback")?;
    let connected_move = callback_for_prototype(scope, prototype, "connectedMoveCallback")?;
    let adopted = callback_for_prototype(scope, prototype, "adoptedCallback")?;
    let attribute_changed = callback_for_prototype(scope, prototype, "attributeChangedCallback")?;
    Ok(CustomElementCallbacks {
        connected,
        disconnected,
        connected_move,
        adopted,
        attribute_changed,
        form_associated: None,
        form_reset: None,
        form_disabled: None,
        form_state_restore: None,
    })
}

pub(super) fn form_callbacks_for_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    callbacks: &mut CustomElementCallbacks,
) -> Result<(), CustomElementDefineError> {
    callbacks.form_associated = callback_for_prototype(scope, prototype, "formAssociatedCallback")?;
    callbacks.form_reset = callback_for_prototype(scope, prototype, "formResetCallback")?;
    callbacks.form_disabled = callback_for_prototype(scope, prototype, "formDisabledCallback")?;
    callbacks.form_state_restore =
        callback_for_prototype(scope, prototype, "formStateRestoreCallback")?;
    Ok(())
}

fn callback_for_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    callback_name: &'static str,
) -> Result<Option<v8::Global<v8::Function>>, CustomElementDefineError> {
    let callback_key =
        v8_string(scope, callback_name).ok_or(CustomElementDefineError::PendingException)?;
    let callback = prototype
        .get(scope, callback_key.into())
        .ok_or(CustomElementDefineError::PendingException)?;
    if callback.is_undefined() {
        return Ok(None);
    }
    let callback = v8::Local::<v8::Function>::try_from(callback)
        .map_err(|_| CustomElementDefineError::InvalidCallback(callback_name))?;
    Ok(Some(v8::Global::new(scope, callback)))
}
