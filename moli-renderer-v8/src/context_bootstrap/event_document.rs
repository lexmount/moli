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
    TouchEvent,
}

impl DocumentCreateEventKind {
    fn constructor_name(self) -> &'static str {
        match self {
            DocumentCreateEventKind::BeforeUnloadEvent => "BeforeUnloadEvent",
            DocumentCreateEventKind::Event => "Event",
            DocumentCreateEventKind::CustomEvent => "CustomEvent",
            DocumentCreateEventKind::DeviceMotionEvent => "DeviceMotionEvent",
            DocumentCreateEventKind::DeviceOrientationEvent => "DeviceOrientationEvent",
            DocumentCreateEventKind::DragEvent => "DragEvent",
            DocumentCreateEventKind::UiEvent => "UIEvent",
            DocumentCreateEventKind::TextEvent => "TextEvent",
            DocumentCreateEventKind::CompositionEvent => "CompositionEvent",
            DocumentCreateEventKind::FocusEvent => "FocusEvent",
            DocumentCreateEventKind::HashChangeEvent => "HashChangeEvent",
            DocumentCreateEventKind::MouseEvent => "MouseEvent",
            DocumentCreateEventKind::KeyboardEvent => "KeyboardEvent",
            DocumentCreateEventKind::MessageEvent => "MessageEvent",
            DocumentCreateEventKind::StorageEvent => "StorageEvent",
            DocumentCreateEventKind::TouchEvent => "TouchEvent",
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
    let name = kind.constructor_name();
    let constructor =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, name).ok()?;
    match kind {
        DocumentCreateEventKind::BeforeUnloadEvent => {
            return new_uninitialized_before_unload_event(scope);
        }
        DocumentCreateEventKind::TextEvent => return new_uninitialized_text_event(scope),
        _ => {}
    }
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
    // Exposure follows the Document's realm, while an operation's exception
    // belongs to the callee realm even when its receiver is from another one.
    let exposed = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        super::exposed_interfaces::is_realm_interface_exposed(target_scope, kind.constructor_name())
    };
    if !exposed {
        throw_not_supported_dom_exception(
            scope,
            "The provided event type is not exposed in this realm.",
        );
        return;
    }
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match new_uninitialized_document_event(target_scope, kind) {
        Some(event) => rv.set(event.into()),
        None => rv.set_undefined(),
    }
}

pub(super) fn document_has_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((host_ptr, document)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        webidl::throw_type_error(scope, "Document.hasFocus called on incompatible receiver.");
        return;
    };
    let host = unsafe { &*host_ptr };
    if !host
        .dom_host()
        .node(document)
        .is_some_and(crate::dom::native::Node::is_document)
    {
        webidl::throw_type_error(scope, "Document.hasFocus called on incompatible receiver.");
        return;
    }
    rv.set_bool(host.document_has_focus(document));
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
        for name in ["DragEvent", "HashChangeEvent", "TouchEvent"] {
            assert_eq!(
                DocumentCreateEventKind::from_str(name)
                    .map(DocumentCreateEventKind::constructor_name),
                Ok(name)
            );
        }
        assert!(DocumentCreateEventKind::from_str("SubmitEvent").is_err());
        assert!(DocumentCreateEventKind::from_str("PointerEvent").is_err());
    }
}
