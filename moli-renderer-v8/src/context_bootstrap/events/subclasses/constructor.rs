use super::*;

fn event_subclass_kind_from_callback_data(
    data: v8::Local<'_, v8::Value>,
) -> Option<EventSubclassKind> {
    let value = i32::try_from(v8::Local::<v8::Integer>::try_from(data).ok()?.value()).ok()?;
    EventSubclassKind::from_i32(value)
}

fn event_subclass_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(kind) = event_subclass_kind_from_callback_data(args.data()) else {
        throw_type_error(scope, "Invalid event subclass constructor.");
        return;
    };

    if !args.is_construct_call() {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{}': Please use the 'new' operator.",
                kind.constructor_name()
            ),
        );
        return;
    }

    let event = args.this();
    let Some(event_type) = event_type_argument(scope, &args, kind.constructor_name()) else {
        return;
    };
    let init = {
        let init_arg = args.get(1);
        if init_arg.is_null_or_undefined() || !init_arg.is_object() {
            None
        } else {
            init_arg.to_object(scope)
        }
    };
    let (bubbles, cancelable, composed) = read_event_init(scope, &args);

    initialize_event_object(scope, event, &event_type, bubbles, cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );

    match kind {
        EventSubclassKind::UiEvent => {
            if !basic::initialize_ui_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::FocusEvent => {
            if !basic::initialize_focus_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::TextEvent => {
            if !basic::initialize_text_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::CompositionEvent => {
            if !basic::initialize_composition_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::CustomEvent => basic::initialize_custom_event(scope, event, init),
        EventSubclassKind::MouseEvent => {
            if !pointer::initialize_mouse_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::DragEvent => {
            if !data::initialize_drag_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::ClipboardEvent => {
            data::initialize_clipboard_event(scope, event, init);
        }
        EventSubclassKind::CapturedMouseEvent => {
            if !data::initialize_captured_mouse_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::KeyboardEvent => {
            if !keyboard::initialize_keyboard_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::InputEvent => data::initialize_input_event(scope, event, init),
        EventSubclassKind::WheelEvent => {
            if !pointer::initialize_wheel_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::PointerEvent => {
            if !pointer::initialize_pointer_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::TouchEvent => {
            crate::context_bootstrap::touch_runtime::initialize_touch_event(scope, event, init);
        }
        EventSubclassKind::MessageEvent => data::initialize_message_event(scope, event, init),
        EventSubclassKind::StorageEvent => {
            if !data::initialize_storage_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::ErrorEvent => data::initialize_error_event(scope, event, init),
        EventSubclassKind::PromiseRejectionEvent => {
            if !data::initialize_promise_rejection_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::SecurityPolicyViolationEvent => {
            if !data::initialize_security_policy_violation_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::NavigationCurrentEntryChangeEvent => {
            if !data::initialize_navigation_current_entry_change_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::NavigateEvent => {
            if !data::initialize_navigate_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::CloseEvent => data::initialize_close_event(scope, event, init),
        EventSubclassKind::SubmitEvent => {
            if !data::initialize_submit_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::FormDataEvent => {
            if !data::initialize_form_data_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::CommandEvent => data::initialize_command_event(scope, event, init),
        EventSubclassKind::ToggleEvent => {
            if !data::initialize_toggle_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::InterestEvent => data::initialize_interest_event(scope, event, init),
        EventSubclassKind::PopStateEvent => {
            if !data::initialize_pop_state_event(scope, event, init) {
                return;
            }
        }
        EventSubclassKind::PageTransitionEvent => {
            data::initialize_page_transition_event(scope, event, init);
        }
        EventSubclassKind::TrackEvent => data::initialize_track_event(scope, event, init),
        EventSubclassKind::FontFaceSetLoadEvent => {
            if !crate::context_bootstrap::css_fontface_runtime::initialize_font_face_set_load_event(
                scope, event, init,
            ) {
                return;
            }
        }
    }

    set_private_value(
        scope,
        event,
        EVENT_SUBCLASS_KIND_SLOT,
        v8::Integer::new(scope, kind as i32).into(),
    );
    rv.set(event.into());
}

pub(in crate::context_bootstrap) fn build_event_subclass_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let data: v8::Local<'s, v8::Value> = v8::Integer::new(scope, kind as i32).into();
    let length = if matches!(kind, EventSubclassKind::NavigateEvent) {
        2
    } else {
        1
    };
    v8::FunctionTemplate::builder(event_subclass_constructor_callback)
        .data(data)
        .length(length)
        .build(scope)
}
