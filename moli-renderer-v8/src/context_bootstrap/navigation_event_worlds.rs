//! Event views for one Navigation dispatch. User callbacks never receive the
//! internal control object, so page expandos cannot cross into another world.

use super::{events, history_runtime::native, shared_event_targets, world_wrappers};
use crate::util::v8str;

pub(super) fn event_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !shared_event_targets::is_shared_target(scope, target) {
        return Some(event);
    }
    let backing = events::event_backing(scope, event);
    // Register an author's original object in its world; foreign listeners
    // receive their own wrapper, even when the event is untrusted. A retained
    // UA view already belongs to the backing object's wrapper cache.
    if backing.strict_equals(event.into()) && !events::event_trusted(scope, event) {
        let creation = event.get_creation_context(scope)?;
        if world_wrappers::get(scope, backing, creation).is_none() {
            world_wrappers::insert(scope, backing, event);
        }
    }
    let target = shared_event_targets::target_in_realm(scope, target, context);
    // A callback in another same-world Document still receives the owning
    // Window's wrapper. Its callback realm is entered separately by the invoker.
    let context = target.get_creation_context(scope).unwrap_or(context);
    if let Some(wrapper) = world_wrappers::get(scope, backing, context) {
        return Some(wrapper);
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let init = crate::util::new_null_prototype_object(scope);
    // Native interface identity survives author prototype/constructor changes
    // and initEvent(). An event named "navigate" can still be an ordinary Event.
    let interface = moli_webapi_declare::web_api_object_type(scope, backing)?.name();
    let properties = init_properties(interface);
    for property in properties
        .iter()
        .copied()
        .chain(["bubbles", "cancelable", "composed"])
    {
        let value = backing.get(scope, v8str(scope, property).into())?;
        let value = attribute_in_realm(scope, property, value, context)?;
        let _ = init.set(scope, v8str(scope, property).into(), value);
    }
    let constructor =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, interface).ok()?;
    let event_type = crate::util::v8_string(scope, event_type)?;
    let wrapper = constructor.new_instance(scope, &[event_type.into(), init.into()])?;
    for property in ["target", "srcElement"] {
        let _ = wrapper.set(scope, v8str(scope, property).into(), target.into());
    }
    events::bind_event_backing(scope, wrapper, backing);
    for property in properties {
        events::bind_event_attribute(scope, wrapper, property);
    }
    world_wrappers::insert(scope, backing, wrapper);
    Some(wrapper)
}

pub(super) fn attribute_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    property: &str,
    value: v8::Local<'s, v8::Value>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Value>> {
    match property {
        "from" => Some(native::entry_value_in_realm(scope, value, context)),
        "destination" => {
            let destination = v8::Local::<v8::Object>::try_from(value).ok()?;
            let scope = &mut v8::ContextScope::new(scope, context);
            super::navigation_events::navigation_destination_for_realm(scope, destination)
                .map(Into::into)
        }
        "signal" | "formData" | "sourceElement" | "relatedTarget" | "submitter" | "source" => {
            super::platform_object_worlds::in_realm(scope, value, context)
        }
        // info, error, detail, data, reason, and state can contain arbitrary JS
        // values. They are not platform wrappers or storage-serialized payloads.
        _ => Some(value),
    }
}

fn init_properties(interface: &str) -> Vec<&'static str> {
    let mut properties = Vec::new();
    if matches!(
        interface,
        "UIEvent"
            | "FocusEvent"
            | "TextEvent"
            | "CompositionEvent"
            | "MouseEvent"
            | "KeyboardEvent"
            | "WheelEvent"
            | "PointerEvent"
            | "TouchEvent"
            | "DragEvent"
            | "InputEvent"
            | "CapturedMouseEvent"
    ) {
        properties.extend(["view", "detail"]);
    }
    if matches!(
        interface,
        "MouseEvent" | "WheelEvent" | "PointerEvent" | "DragEvent" | "CapturedMouseEvent"
    ) {
        properties.extend([
            "screenX",
            "screenY",
            "clientX",
            "clientY",
            "ctrlKey",
            "shiftKey",
            "altKey",
            "metaKey",
            "button",
            "buttons",
            "relatedTarget",
            "movementX",
            "movementY",
        ]);
    }
    let own: &[&str] = match interface {
        "CustomEvent" => &["detail"],
        "FocusEvent" => &["relatedTarget"],
        "TextEvent" | "CompositionEvent" => &["data"],
        "KeyboardEvent" => &[
            "key",
            "code",
            "location",
            "ctrlKey",
            "shiftKey",
            "altKey",
            "metaKey",
            "repeat",
            "isComposing",
            "charCode",
            "keyCode",
            "which",
        ],
        "WheelEvent" => &["deltaX", "deltaY", "deltaZ", "deltaMode"],
        "PointerEvent" => &[
            "pointerId",
            "width",
            "height",
            "pressure",
            "tangentialPressure",
            "tiltX",
            "tiltY",
            "twist",
            "altitudeAngle",
            "azimuthAngle",
            "pointerType",
            "isPrimary",
            "persistentDeviceId",
        ],
        "TouchEvent" => &[
            "touches",
            "targetTouches",
            "changedTouches",
            "ctrlKey",
            "shiftKey",
            "altKey",
            "metaKey",
        ],
        "MessageEvent" => &["data", "origin", "lastEventId", "source", "ports"],
        "ErrorEvent" => &["message", "filename", "lineno", "colno", "error"],
        "PromiseRejectionEvent" => &["promise", "reason"],
        "NavigationCurrentEntryChangeEvent" => &["from", "navigationType"],
        "NavigateEvent" => &[
            "navigationType",
            "destination",
            "canIntercept",
            "userInitiated",
            "hashChange",
            "signal",
            "formData",
            "downloadRequest",
            "info",
            "sourceElement",
            "hasUAVisualTransition",
        ],
        "CloseEvent" => &["wasClean", "code", "reason"],
        "DragEvent" => &["dataTransfer"],
        "ClipboardEvent" => &["clipboardData"],
        "CapturedMouseEvent" => &["surfaceX", "surfaceY"],
        "SubmitEvent" => &["submitter"],
        "FormDataEvent" => &["formData"],
        "InputEvent" => &[
            "data",
            "isComposing",
            "inputType",
            "dataTransfer",
            "targetRanges",
        ],
        "PopStateEvent" => &["state", "hasUAVisualTransition"],
        "PageTransitionEvent" => &["persisted"],
        "CommandEvent" => &["command", "source"],
        "ToggleEvent" => &["oldState", "newState", "source"],
        "TrackEvent" => &["track"],
        "InterestEvent" => &["source"],
        "StorageEvent" => &["key", "oldValue", "newValue", "url", "storageArea"],
        "SecurityPolicyViolationEvent" => &[
            "documentURI",
            "referrer",
            "blockedURI",
            "violatedDirective",
            "effectiveDirective",
            "originalPolicy",
            "disposition",
            "sourceFile",
            "sample",
            "statusCode",
            "lineNumber",
            "columnNumber",
        ],
        "FontFaceSetLoadEvent" => &["fontfaces"],
        "ProgressEvent" => &["lengthComputable", "loaded", "total"],
        "HashChangeEvent" => &["oldURL", "newURL"],
        "AnimationEvent" => &["animationName", "elapsedTime", "pseudoElement"],
        "TransitionEvent" => &["propertyName", "elapsedTime", "pseudoElement"],
        _ => &[],
    };
    properties.extend_from_slice(own);
    properties
}
