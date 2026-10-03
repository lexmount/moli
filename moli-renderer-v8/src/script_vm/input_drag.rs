use anyhow::Result;
use moli_layout::{LayoutPoint, LayoutTransform2D};

use super::input_dispatch::{
    dispatch_native_pointer_event, release_pointer_capture_after_pointer_end,
};
use super::{ActiveDragSession, ScriptVm, input_dispatch_outcome};
use crate::context_bootstrap::{
    DragDataStore, allowed_drag_drop_effect, prepare_drag_drop_effect, set_drag_drop_effect,
};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::native_bridge::element::{
    InputHit, construct_drag_event_with_related_target, construct_pointer_event_with_modifiers,
    dispatch_public_event, perform_drop_default_action,
};
use crate::runtime::{RendererInputDispatchOutcome, RendererPointerEventProperties};
use crate::util::node_wrapper_from_handle;

#[derive(Clone, Copy)]
pub(super) struct NativeDragSource {
    pub handle: DomHandle,
    pub pointer_id: i32,
    pub root_to_frame: LayoutTransform2D,
    pub position: LayoutPoint,
}

pub(super) fn suppress_drag_pointer_stream(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: *mut JsContextHost,
    target: DomHandle,
    x: f64,
    y: f64,
    pointer: &RendererPointerEventProperties,
    modifiers: u8,
) {
    // Canceling the stream ends its active-buttons state, but preserves the
    // last pointer's geometry. Capture is released before subsequent boundaries.
    unsafe { &mut *runtime }.update_pointer_input(pointer.pointer_id, 0);
    if let Some(event) = construct_pointer_event_with_modifiers(
        scope,
        "pointercancel",
        x,
        y,
        0,
        0,
        pointer,
        modifiers,
    ) {
        dispatch_native_pointer_event(scope, runtime, target, event, pointer.pointer_id);
    }
    release_pointer_capture_after_pointer_end(
        scope,
        runtime,
        pointer.pointer_id,
        x,
        y,
        0,
        0,
        pointer,
        modifiers,
    );
    for event_name in ["pointerout", "pointerleave"] {
        if let Some(event) = construct_pointer_event_with_modifiers(
            scope, event_name, x, y, 0, 0, pointer, modifiers,
        ) {
            dispatch_native_pointer_event(scope, runtime, target, event, pointer.pointer_id);
        }
    }
}

fn dispatch_drag(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: *mut JsContextHost,
    target: InputHit,
    event_name: &str,
    position: LayoutPoint,
    buttons: i32,
    modifiers: u8,
    store: &DragDataStore,
    related: Option<DomHandle>,
) -> bool {
    let point = target.root_to_frame.map_point(position);
    fire_drag_event(
        scope,
        runtime,
        target.handle,
        event_name,
        point,
        buttons,
        modifiers,
        store,
        related,
    )
}

/// A native drag event owns one temporary view of the session's data store.
/// Construct it in the target Document's realm, then retire it after dispatch.
pub(super) fn fire_drag_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: *mut JsContextHost,
    target: DomHandle,
    event_name: &str,
    position: LayoutPoint,
    buttons: i32,
    modifiers: u8,
    store: &DragDataStore,
    related: Option<DomHandle>,
) -> bool {
    let host = unsafe { &mut *runtime };
    let Some(context) = host
        .owner_dispatch_scope_for_node(target)
        .and_then(|target| {
            let owner = host.current_window_execution_context_owner(target)?;
            host.window_execution_context(scope, owner, target)
                .map(|(_, context)| context)
        })
    else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(view) = store.open_event(scope, event_name) else {
        return false;
    };
    let related = related
        .and_then(|handle| node_wrapper_from_handle(scope, handle))
        .map(Into::into);
    let allows_default = construct_drag_event_with_related_target(
        scope,
        event_name,
        f64::from(position.x),
        f64::from(position.y),
        buttons,
        view.object().into(),
        modifiers,
        related,
    )
    .is_some_and(|event| dispatch_public_event(scope, runtime, target, event).allows_default());
    view.finish(scope);
    allows_default
}

fn end_native_drag(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: *mut JsContextHost,
    session: &ActiveDragSession,
    position: LayoutPoint,
    buttons: i32,
    modifiers: u8,
    allow_drop: bool,
) {
    let Some(source) = session.native else {
        return;
    };
    let transfer = session.data_store.backing_transfer(scope);
    let mut effect = if allow_drop && session.drop_allowed {
        allowed_drag_drop_effect(scope, transfer)
    } else {
        "none".to_owned()
    };
    if effect != "none"
        && let Some(target) = session.target
    {
        let allows_default = dispatch_drag(
            scope,
            runtime,
            target,
            "drop",
            position,
            buttons,
            modifiers,
            &session.data_store,
            None,
        );
        if allows_default && !perform_drop_default_action(scope, runtime, target.handle, transfer) {
            effect = "none".to_owned();
        } else {
            effect = allowed_drag_drop_effect(scope, transfer);
        }
    } else if let Some(target) = session.target {
        set_drag_drop_effect(scope, transfer, "none");
        dispatch_drag(
            scope,
            runtime,
            target,
            "dragleave",
            position,
            buttons,
            modifiers,
            &session.data_store,
            None,
        );
    }
    set_drag_drop_effect(scope, transfer, &effect);
    dispatch_drag(
        scope,
        runtime,
        InputHit {
            handle: source.handle,
            root_to_frame: source.root_to_frame,
        },
        "dragend",
        position,
        0,
        modifiers,
        &session.data_store,
        None,
    );
}

impl ScriptVm {
    pub(super) fn has_native_drag_session(&self) -> bool {
        self.active_drag_session
            .as_ref()
            .is_some_and(|session| session.native.is_some())
    }

    pub(super) fn dispatch_native_drag_mouse_input(
        &mut self,
        position: LayoutPoint,
        event_name: &str,
        buttons: i32,
        pointer_id: i32,
        modifiers: u8,
        hit: Option<InputHit>,
    ) -> Result<Option<RendererInputDispatchOutcome>> {
        if !self.has_native_drag_session() {
            if self.suppressed_drag_pointer == Some(pointer_id) {
                if event_name == "mouseup" && buttons == 0 {
                    self.suppressed_drag_pointer = None;
                } else if event_name == "mousedown" {
                    // A new press starts a new pointer stream even if no
                    // release command arrived for the previous canceled one.
                    self.suppressed_drag_pointer = None;
                    return Ok(None);
                }
                return Ok(Some(input_dispatch_outcome(true)));
            }
            return Ok(None);
        }
        let mut session = self
            .active_drag_session
            .take()
            .expect("native drag session exists");
        let source = session.native.as_mut().expect("native drag has a source");
        if source.pointer_id != pointer_id {
            self.active_drag_session = Some(session);
            return Ok(Some(input_dispatch_outcome(true)));
        }
        source.position = position;
        let source = *source;
        let mut finished = false;
        let result = self.with_default_context_scope(|scope, runtime| {
            if matches!(event_name, "mousemove" | "mouseup") && buttons & 1 == 0 {
                end_native_drag(
                    scope,
                    runtime,
                    &session,
                    position,
                    buttons,
                    modifiers,
                    event_name == "mouseup"
                        && hit.map(|h| h.handle) == session.target.map(|h| h.handle),
                );
                finished = true;
            } else if event_name == "mousemove" {
                let transfer = session.data_store.backing_transfer(scope);
                set_drag_drop_effect(scope, transfer, "none");
                if !dispatch_drag(
                    scope,
                    runtime,
                    InputHit {
                        handle: source.handle,
                        root_to_frame: source.root_to_frame,
                    },
                    "drag",
                    position,
                    buttons,
                    modifiers,
                    &session.data_store,
                    None,
                ) {
                    end_native_drag(scope, runtime, &session, position, 0, modifiers, false);
                    finished = true;
                    return Ok(());
                }
                let previous = session.target;
                if previous.map(|h| h.handle) != hit.map(|h| h.handle) {
                    session.drop_allowed = false;
                    if let Some(target) = hit {
                        prepare_drag_drop_effect(scope, transfer, modifiers);
                        dispatch_drag(
                            scope,
                            runtime,
                            target,
                            "dragenter",
                            position,
                            buttons,
                            modifiers,
                            &session.data_store,
                            previous.map(|h| h.handle),
                        );
                    }
                    if let Some(previous) = previous {
                        set_drag_drop_effect(scope, transfer, "none");
                        dispatch_drag(
                            scope,
                            runtime,
                            previous,
                            "dragleave",
                            position,
                            buttons,
                            modifiers,
                            &session.data_store,
                            hit.map(|h| h.handle),
                        );
                    }
                    session.target = hit;
                }
                session.drop_allowed = if let Some(target) = session.target {
                    prepare_drag_drop_effect(scope, transfer, modifiers);
                    !dispatch_drag(
                        scope,
                        runtime,
                        target,
                        "dragover",
                        position,
                        buttons,
                        modifiers,
                        &session.data_store,
                        None,
                    ) && allowed_drag_drop_effect(scope, transfer) != "none"
                } else {
                    false
                };
            }
            Ok(())
        });
        if !finished {
            self.active_drag_session = Some(session);
        }
        if finished && buttons == 0 {
            self.suppressed_drag_pointer = None;
        }
        result.map(|()| Some(input_dispatch_outcome(true)))
    }

    pub(super) fn cancel_native_drag(&mut self, modifiers: u8) -> Result<()> {
        let Some(session) = self.active_drag_session.take() else {
            return Ok(());
        };
        let Some(source) = session.native else {
            return Ok(());
        };
        self.with_default_context_scope(|scope, runtime| {
            end_native_drag(
                scope,
                runtime,
                &session,
                source.position,
                0,
                modifiers,
                false,
            );
            Ok(())
        })
    }
}
