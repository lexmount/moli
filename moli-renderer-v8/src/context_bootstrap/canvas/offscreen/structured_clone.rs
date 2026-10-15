use super::*;
use crate::context_bootstrap::throw_dom_exception_value;

/// Only context-mode none is transferable: its receiver starts with a blank
/// bitmap. No source pixels, V8 handles or author constructors cross agents.
#[derive(Clone, Debug)]
pub(crate) struct OffscreenCanvasTransferPayload {
    width: u32,
    height: u32,
}

impl OffscreenCanvasTransferPayload {
    pub(crate) fn receive<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        build_offscreen_canvas_object(scope, self.width, self.height)
    }
}

/// Run after message serialization, in transfer-list order. Observable getters
/// may have resized, acquired a context, or already transferred this canvas.
pub(crate) fn transfer_offscreen_canvas<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<OffscreenCanvasTransferPayload> {
    if offscreen_canvas_is_detached(scope, canvas) {
        throw_dom_exception_value(
            scope,
            "The transferred OffscreenCanvas is detached.",
            "DataCloneError",
        );
        return None;
    }
    if get_private_value(scope, canvas, OFFSCREEN_CANVAS_CONTEXT_SLOT).is_some() {
        throw_dom_exception_value(
            scope,
            "An OffscreenCanvas with a rendering context cannot be transferred.",
            "InvalidStateError",
        );
        return None;
    }
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    set_private_value(
        scope,
        canvas,
        OFFSCREEN_CANVAS_DETACHED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    for slot in OFFSCREEN_CANVAS_ATTRIBUTE_SLOTS {
        set_private_value(scope, canvas, slot, v8::Number::new(scope, 0.0).into());
    }
    reset_canvas_like_backing_store(scope, canvas);
    Some(OffscreenCanvasTransferPayload { width, height })
}
