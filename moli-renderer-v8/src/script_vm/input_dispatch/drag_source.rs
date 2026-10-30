use super::*;

pub(super) fn dispatch_drag_source_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    session: &ActiveDragSession,
    coordinates: InputEventCoordinates,
    modifiers: u8,
) {
    let Some(source) = session.source else {
        return;
    };
    let Some(source_transfer) = session.source_transfer.as_ref() else {
        return;
    };
    // Blink reuses the source transfer for dragstart, drag and dragend. Its
    // original metadata stays protected while target event views come and go.
    let transfer = v8::Local::new(scope, source_transfer);
    // Blink dispatches source drag before destination updates and ignores its
    // canceled result. It does not run an autonomous timer while intercepted.
    let _ = dispatch_native_drag_event(
        scope,
        runtime_ptr,
        "drag",
        source,
        None,
        coordinates,
        transfer,
        modifiers,
    );
}
