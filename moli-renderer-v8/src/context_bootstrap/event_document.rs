use super::events::{
    new_uninitialized_before_unload_event, new_uninitialized_text_event, set_event_initialized,
};
use super::*;
use crate::webidl;
use std::str::FromStr;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createEvent")]
struct DocumentCreateEventArgs {
    #[webidl(required)]
    interface: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumString)]
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
            DocumentCreateEventKind::BeforeUnloadEvent => "BeforeUnloadEvent",
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

fn new_uninitialized_document_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: DocumentCreateEventKind,
) -> Option<v8::Local<'s, v8::Object>> {
    if matches!(
        kind,
        DocumentCreateEventKind::BeforeUnloadEvent | DocumentCreateEventKind::TextEvent
    ) {
        return match kind {
            DocumentCreateEventKind::BeforeUnloadEvent => {
                new_uninitialized_before_unload_event(scope)
            }
            DocumentCreateEventKind::TextEvent => new_uninitialized_text_event(scope),
            _ => unreachable!(),
        };
    }
    let global = scope.get_current_context().global(scope);
    let constructor_value = global.get(scope, v8str(scope, kind.constructor_name()).into())?;
    let constructor = v8::Local::<v8::Function>::try_from(constructor_value).ok()?;
    let empty_type = v8str(scope, "");
    let event = constructor.new_instance(scope, &[empty_type.into()])?;
    set_event_initialized(scope, event, false);
    Some(event)
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
    let relevant_context = crate::native_bridge::node_relevant_context(scope, args.this())
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match new_uninitialized_document_event(target_scope, kind) {
        Some(event) => rv.set(event.into()),
        None => rv.set_undefined(),
    }
}

pub(super) fn document_has_focus_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let document = args.this();
    let focused = document
        .get(scope, v8str(scope, "defaultView").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|default_view| {
            default_view
                .get(scope, v8str(scope, "frameElement").into())
                .filter(|value| !value.is_null_or_undefined())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .map(|frame_element| {
                    frame_element
                        .get(scope, v8str(scope, "ownerDocument").into())
                        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                        .and_then(|owner_document| {
                            owner_document.get(scope, v8str(scope, "activeElement").into())
                        })
                        .is_some_and(|active_element| {
                            active_element.strict_equals(frame_element.into())
                        })
                })
                .unwrap_or(true)
        })
        .unwrap_or(false);
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
