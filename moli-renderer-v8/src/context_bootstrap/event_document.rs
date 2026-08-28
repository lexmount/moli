use super::events::{new_uninitialized_text_event, set_event_initialized};
use super::*;
use crate::webidl;
use std::str::FromStr;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createEvent")]
struct DocumentCreateEventArgs {
    #[webidl(required)]
    interface: String,
}

#[derive(Debug, PartialEq, Eq, strum::EnumString)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
enum DocumentCreateEventKind {
    BeforeUnloadEvent,
    #[strum(
        serialize = "event",
        serialize = "events",
        serialize = "htmlevents",
        serialize = "svgevents"
    )]
    Event,
    CustomEvent,
    DeviceMotionEvent,
    DeviceOrientationEvent,
    DragEvent,
    #[strum(serialize = "uievent", serialize = "uievents")]
    UiEvent,
    TextEvent,
    CompositionEvent,
    FocusEvent,
    HashChangeEvent,
    #[strum(serialize = "mouseevent", serialize = "mouseevents")]
    MouseEvent,
    KeyboardEvent,
    MessageEvent,
    StorageEvent,
    SubmitEvent,
}

impl DocumentCreateEventKind {
    fn constructor_name(self) -> &'static str {
        match self {
            DocumentCreateEventKind::BeforeUnloadEvent => "Event",
            DocumentCreateEventKind::Event => "Event",
            DocumentCreateEventKind::CustomEvent => "CustomEvent",
            DocumentCreateEventKind::DeviceMotionEvent => "Event",
            DocumentCreateEventKind::DeviceOrientationEvent => "Event",
            DocumentCreateEventKind::DragEvent => "Event",
            DocumentCreateEventKind::UiEvent => "UIEvent",
            DocumentCreateEventKind::TextEvent => "TextEvent",
            DocumentCreateEventKind::CompositionEvent => "CompositionEvent",
            DocumentCreateEventKind::FocusEvent => "FocusEvent",
            DocumentCreateEventKind::HashChangeEvent => "Event",
            DocumentCreateEventKind::MouseEvent => "MouseEvent",
            DocumentCreateEventKind::KeyboardEvent => "KeyboardEvent",
            DocumentCreateEventKind::MessageEvent => "MessageEvent",
            DocumentCreateEventKind::StorageEvent => "StorageEvent",
            DocumentCreateEventKind::SubmitEvent => "SubmitEvent",
        }
    }
}

fn throw_not_supported_dom_exception(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    crate::context_bootstrap::throw_dom_exception_value(scope, message, "NotSupportedError");
}

pub(super) fn document_create_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateEventArgs>(scope, &args) else {
        return;
    };
    let Ok(kind) = DocumentCreateEventKind::from_str(&parsed.interface) else {
        throw_not_supported_dom_exception(scope, "The provided event type is not supported.");
        return;
    };
    if kind == DocumentCreateEventKind::TextEvent {
        match new_uninitialized_text_event(scope) {
            Some(event) => rv.set(event.into()),
            None => rv.set_undefined(),
        }
        return;
    }
    let ctor_name = kind.constructor_name();

    let global = scope.get_current_context().global(scope);
    let Some(constructor_value) = global.get(scope, v8str(scope, ctor_name).into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(constructor) = v8::Local::<v8::Function>::try_from(constructor_value) else {
        rv.set_undefined();
        return;
    };
    let empty_type = v8str(scope, "");
    match constructor.new_instance(scope, &[empty_type.into()]) {
        Some(event) => {
            set_event_initialized(scope, event, false);
            rv.set(event.into());
        }
        None => rv.set_undefined(),
    }
}

pub(super) fn document_has_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let focused =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, args.this())
            .is_ok_and(|(runtime_ptr, document_handle)| {
                unsafe { &*runtime_ptr }.document_has_focus(document_handle)
            });
    rv.set_bool(focused);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_create_event_kind_parses_legacy_aliases() {
        assert_eq!(
            DocumentCreateEventKind::from_str("Events"),
            Ok(DocumentCreateEventKind::Event)
        );
        assert_eq!(
            DocumentCreateEventKind::from_str("UIEvents")
                .map(DocumentCreateEventKind::constructor_name),
            Ok("UIEvent")
        );
        assert_eq!(
            DocumentCreateEventKind::from_str("MouseEvents")
                .map(DocumentCreateEventKind::constructor_name),
            Ok("MouseEvent")
        );
        assert_eq!(
            DocumentCreateEventKind::from_str("SubmitEvent")
                .map(DocumentCreateEventKind::constructor_name),
            Ok("SubmitEvent")
        );
        assert!(DocumentCreateEventKind::from_str("PointerEvent").is_err());
    }
}
