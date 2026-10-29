use super::*;

pub(super) fn selection_drag_is_delayed(elapsed: std::time::Duration, macos: bool) -> bool {
    // MouseEventManager::kTextDragDelay is zero on other platforms.
    macos && elapsed < std::time::Duration::from_millis(150)
}

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

#[cfg(test)]
mod tests {
    use super::selection_drag_is_delayed;
    use std::time::Duration;

    #[test]
    fn selection_drag_delay_is_macos_only_and_expires_at_150ms() {
        for millis in [0, 1, 149, 150, 250] {
            let elapsed = Duration::from_millis(millis);
            assert_eq!(selection_drag_is_delayed(elapsed, true), millis < 150);
            assert!(!selection_drag_is_delayed(elapsed, false));
        }
    }
}
