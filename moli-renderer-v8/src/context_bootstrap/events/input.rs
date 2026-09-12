use super::*;

/// The non-composition edits currently performed by the native input pipeline.
/// Carry the operation, not just the replacement text: deletion and a line
/// break have null event data, even though both still replace a selection.
#[derive(Clone, Copy, strum::IntoStaticStr)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum TextInputType {
    InsertText,
    InsertLineBreak,
    InsertFromDrop,
    DeleteContentBackward,
    DeleteContentForward,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct NativeInputEventInit<'scope> {
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    data: v8::Local<'scope, v8::Value>,
    data_transfer: v8::Local<'scope, v8::Value>,
    input_type: v8::Local<'scope, v8::String>,
    is_composing: bool,
}

pub(crate) fn construct_original_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    input_type: TextInputType,
    text: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = match input_type {
        TextInputType::InsertText | TextInputType::InsertFromDrop => v8_string(scope, text)?.into(),
        _ => v8::null(scope).into(),
    };
    construct_input_event(scope, event_type, input_type, data, v8::null(scope).into())
}

/// Rich editing carries a readable DataTransfer instead of a string. The caller
/// retains the same transfer for both beforeinput and input.
pub(crate) fn construct_original_drop_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data_transfer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_input_event(
        scope,
        event_type,
        TextInputType::InsertFromDrop,
        v8::null(scope).into(),
        data_transfer.into(),
    )
}

fn construct_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    input_type: TextInputType,
    data: v8::Local<'s, v8::Value>,
    data_transfer: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let type_name: &'static str = input_type.into();
    let init = NativeInputEventInit::new(
        true,
        event_type == "beforeinput",
        true,
        data,
        data_transfer,
        v8_string(scope, type_name)?,
        false,
    )
    .bind(scope)
    .ok()?;
    // Native editing must not call a page-replaced window.InputEvent.
    let ctor = super::super::exposed_interfaces::ensure_intrinsic_interface_constructor(
        scope,
        "InputEvent",
    )
    .ok()?;
    let event_type = v8_string(scope, event_type)?;
    let event = ctor.new_instance(scope, &[event_type.into(), init.into()])?;
    mark_event_trusted(scope, event);
    Some(event)
}
