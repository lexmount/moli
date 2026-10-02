//! Browser input uses the same event state as WebIDL constructors.

use super::super::{SubclassInit, construct_trusted_subclass, native_event_interface};
use super::*;
use crate::events::init::EventInit;
use crate::events::ui::{UiEventInit, is_window_receiver};

/// Screen and target-frame client coordinates supplied by the embedder.
#[derive(Clone, Copy, Default)]
pub struct InputEventCoordinates {
    pub screen_x: f64,
    pub screen_y: f64,
    pub client_x: f64,
    pub client_y: f64,
}

impl InputEventCoordinates {
    pub fn from_root(x: f64, y: f64) -> Self {
        Self {
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        }
    }

    pub fn with_client(self, x: f64, y: f64) -> Self {
        Self {
            client_x: x,
            client_y: y,
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointerEventProperties {
    pub pointer_id: i32,
    pub pointer_type: String,
    pub is_primary: bool,
    pub pressure: f64,
    pub tangential_pressure: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
    pub twist: f64,
}

impl Default for PointerEventProperties {
    fn default() -> Self {
        Self {
            pointer_id: 1,
            pointer_type: "mouse".to_owned(),
            is_primary: true,
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
        }
    }
}

impl<'s> MouseEventInit<'s> {
    pub fn from_native(
        scope: &mut v8::PinScope<'s, '_>,
        base: EventInit,
        coordinates: InputEventCoordinates,
        detail: i32,
        button: i32,
        buttons: i32,
        modifiers: u8,
        related_target: Option<v8::Local<'s, v8::Value>>,
        view: Option<v8::Local<'s, v8::Object>>,
    ) -> Option<Self> {
        let view = match view {
            Some(view) => Some(native_event_interface(
                scope,
                view.into(),
                "UIEventInit",
                "view",
                webidl::InterfaceOptions {
                    name: "Window",
                    brand_check: is_window_receiver,
                },
            )?),
            None => None,
        };
        let InputEventCoordinates {
            client_x,
            client_y,
            screen_x,
            screen_y,
        } = coordinates;
        finite_doubles(
            scope,
            "MouseEventInit",
            [
                ("clientX", client_x),
                ("clientY", client_y),
                ("screenX", screen_x),
                ("screenY", screen_y),
            ],
        )?;
        let related_target = match related_target.filter(|value| !value.is_null_or_undefined()) {
            Some(target) => Some(native_event_interface(
                scope,
                target,
                "MouseEventInit",
                "relatedTarget",
                webidl::InterfaceOptions {
                    name: "EventTarget",
                    brand_check: web_api_interfaces::EventTarget::is_instance,
                },
            )?),
            None => None,
        };
        let ui = UiEventInit::from_native(base, detail, view);
        Some(Self {
            modifiers: EventModifierInit::from_native(
                ui,
                modifiers & 1 != 0,
                modifiers & 2 != 0,
                modifiers & 4 != 0,
                modifiers & 8 != 0,
            ),
            button: button as i16,
            buttons: buttons as u16,
            client_x,
            client_y,
            screen_x,
            screen_y,
            related_target,
            ..Self::default()
        })
    }
}

impl<'s> PointerEventInit<'s> {
    fn from_native_mouse(mouse: MouseEventInit<'s>, pointer_id: i32, pointer_type: &str) -> Self {
        Self {
            mouse,
            altitude_angle: None,
            azimuth_angle: None,
            coalesced_events: Vec::new(),
            height: 1.0,
            is_primary: false,
            persistent_device_id: 0,
            pointer_id,
            pointer_type: webidl::DomString16(pointer_type.encode_utf16().collect()),
            predicted_events: Vec::new(),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: None,
            tilt_y: None,
            twist: 0,
            width: 1.0,
        }
    }
}

pub fn construct_native_mouse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    init: MouseEventInit<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    construct_trusted_subclass(
        scope,
        EventSubclassKind::MouseEvent,
        event_type,
        SubclassInit::Pointing(PointerEventInitPayload::Mouse(init)),
    )
}

pub fn construct_native_wheel_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    mouse: MouseEventInit<'s>,
    delta_x: f64,
    delta_y: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    finite_doubles(
        scope,
        "WheelEventInit",
        [("deltaX", delta_x), ("deltaY", delta_y)],
    )?;
    let init = WheelEventInit {
        mouse,
        delta_mode: 0,
        delta_x,
        delta_y,
        delta_z: 0.0,
        wheel_delta_x: 0,
        wheel_delta_y: 0,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::WheelEvent,
        event_type,
        SubclassInit::Pointing(PointerEventInitPayload::Wheel(init)),
    )
}

pub fn construct_native_pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    mouse: MouseEventInit<'s>,
    pointer: &PointerEventProperties,
) -> Option<v8::Local<'s, v8::Object>> {
    let pressure = pointer.pressure as f32;
    let tangential_pressure = pointer.tangential_pressure as f32;
    for (name, value) in [
        ("pressure", pressure),
        ("tangentialPressure", tangential_pressure),
    ] {
        if !value.is_finite() {
            let context = webidl::Context::member("PointerEventInit", name);
            webidl::throw_error(
                scope,
                &webidl::WebIdlError::cannot_convert(context, "float"),
            );
            return None;
        }
    }
    let mut init =
        PointerEventInit::from_native_mouse(mouse, pointer.pointer_id, &pointer.pointer_type);
    init.is_primary = pointer.is_primary;
    init.pressure = pressure;
    init.tangential_pressure = tangential_pressure;
    init.tilt_x = v8::Number::new(scope, pointer.tilt_x).int32_value(scope);
    init.tilt_y = v8::Number::new(scope, pointer.tilt_y).int32_value(scope);
    init.twist = v8::Number::new(scope, pointer.twist).int32_value(scope)?;
    construct_trusted_subclass(
        scope,
        EventSubclassKind::PointerEvent,
        event_type,
        SubclassInit::Pointing(PointerEventInitPayload::Pointer(init)),
    )
}

pub fn construct_native_activation_pointer_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    mouse: MouseEventInit<'s>,
    pointer_id: i32,
    pointer_type: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = PointerEventInit::from_native_mouse(mouse, pointer_id, pointer_type);
    construct_trusted_subclass(
        scope,
        EventSubclassKind::PointerEvent,
        event_type,
        SubclassInit::Pointing(PointerEventInitPayload::Pointer(init)),
    )
}

pub fn construct_native_drag_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    mouse: MouseEventInit<'s>,
    data_transfer: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data_transfer = if data_transfer.is_null_or_undefined() {
        None
    } else {
        Some(native_event_interface(
            scope,
            data_transfer,
            "DragEventInit",
            "dataTransfer",
            webidl::InterfaceOptions {
                name: "DataTransfer",
                brand_check: web_api_interfaces::DataTransfer::is_instance,
            },
        )?)
    };
    let init = DragEventInit {
        mouse,
        data_transfer,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::DragEvent,
        event_type,
        SubclassInit::Pointing(PointerEventInitPayload::Drag(init)),
    )
}

fn finite_doubles(
    scope: &mut v8::PinScope<'_, '_>,
    prefix: &'static str,
    members: impl IntoIterator<Item = (&'static str, f64)>,
) -> Option<()> {
    for (name, value) in members {
        if !value.is_finite() {
            let context = webidl::Context::member(prefix, name);
            webidl::throw_error(
                scope,
                &webidl::WebIdlError::cannot_convert(context, "double"),
            );
            return None;
        }
    }
    Some(())
}
