use crate::context_bootstrap::{
    build_clipboard_data_transfer, clipboard_data_transfer_contents,
    disable_clipboard_data_transfer,
};
use crate::dom::{forms::InputType, native::Node};
use crate::native_bridge::element::*;
use crate::runtime::ClipboardSnapshot;
use crate::util::utf16_units;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipboardAction {
    Copy,
    Cut,
    Paste,
}

impl ClipboardAction {
    fn for_key(key: &str, modifiers: u8) -> Option<Self> {
        let accelerator = if cfg!(target_os = "macos") { 4 } else { 2 };
        if modifiers & !8 != accelerator {
            return None;
        }
        match (key, modifiers & 8 != 0) {
            ("c", false) => Some(Self::Copy),
            ("x", false) => Some(Self::Cut),
            ("v", _) => Some(Self::Paste),
            _ => None,
        }
    }

    fn event_type(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Cut => "cut",
            Self::Paste => "paste",
        }
    }
}

fn active_target(runtime: &JsContextHost) -> Option<DomHandle> {
    runtime
        .active_element_handle()
        .or_else(|| runtime.document_focus_fallback_handle())
}

fn is_password(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            element.is_html_input() && element.input_type() == InputType::Password
        })
}

fn is_clipboard_text_control(runtime: &JsContextHost, handle: DomHandle, writing: bool) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    (element.is_html_textarea()
        || (element.is_html_input()
            && matches!(
                element.input_type(),
                InputType::Text
                    | InputType::Search
                    | InputType::Tel
                    | InputType::Url
                    | InputType::Email
                    | InputType::Password
                    | InputType::Number
            )))
        && !form_control_is_effectively_disabled(runtime, handle)
        && (!writing || !element.has_attribute("readonly"))
}

fn selected_control_text(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    if !is_clipboard_text_control(runtime, handle, false) || is_password(runtime, handle) {
        return None;
    }
    let element = runtime.dom_host().node(handle)?.as_element()?;
    let units = utf16_units(&text_control_value(runtime, handle));
    let start = (element.selection_start() as usize).min(units.len());
    let end = (element.selection_end() as usize).min(units.len());
    (start < end).then(|| String::from_utf16_lossy(&units[start..end]))
}

pub(crate) fn perform_clipboard_key_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    key: &str,
    modifiers: u8,
) -> Option<bool> {
    let action = ClipboardAction::for_key(key, modifiers)?;
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(handle) = active_target(runtime) else {
        return Some(false);
    };
    if action != ClipboardAction::Paste && is_password(runtime, handle) {
        return Some(false);
    }
    let context = runtime
        .owner_dispatch_scope_for_node(handle)
        .and_then(|target| {
            runtime
                .current_window_execution_context_owner(target)
                .and_then(|owner| runtime.window_execution_context(scope, owner, target))
        })
        .map(|(_, context)| context);
    let Some(context) = context else {
        return Some(false);
    };
    let context_scope = &mut v8::ContextScope::new(scope, context);
    Some(perform_clipboard_action(
        context_scope,
        runtime_ptr,
        handle,
        action,
    ))
}

fn perform_clipboard_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    action: ClipboardAction,
) -> bool {
    let clipboard = unsafe { &*runtime_ptr }.browser_context_runtime();
    let contents = if action == ClipboardAction::Paste {
        clipboard.clipboard_snapshot().representations
    } else {
        Vec::new()
    };
    let Some(allows_default) = dispatch_clipboard_action_event(
        scope,
        runtime_ptr,
        handle,
        action.event_type(),
        &contents,
        action == ClipboardAction::Paste,
    ) else {
        return false;
    };
    if !allows_default {
        return true;
    }

    // Clipboard listeners can move focus or change the selection. Resolve both
    // again before the default action; beforeinput gets its own later edit check.
    let runtime = unsafe { &*runtime_ptr };
    let Some(handle) = active_target(runtime) else {
        return true;
    };
    match action {
        ClipboardAction::Copy | ClipboardAction::Cut => {
            if action == ClipboardAction::Cut && !is_clipboard_text_control(runtime, handle, true) {
                return true;
            }
            if let Some(text) = selected_control_text(runtime, handle) {
                clipboard.set_clipboard_snapshot(ClipboardSnapshot {
                    representations: vec![(
                        "text/plain".to_owned(),
                        native_clipboard_text_bytes(&text),
                    )],
                    ..ClipboardSnapshot::default()
                });
                if action == ClipboardAction::Cut {
                    replace_text_control_selection(
                        scope,
                        runtime_ptr,
                        handle,
                        "",
                        TextEditInputType::DeleteByCut,
                    );
                }
            }
        }
        ClipboardAction::Paste => {
            if is_clipboard_text_control(runtime, handle, true) {
                let text = contents
                    .iter()
                    .find(|(mime_type, _)| mime_type == "text/plain")
                    .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
                    .unwrap_or_default();
                replace_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    &text,
                    TextEditInputType::InsertFromPaste,
                );
            }
        }
    }
    true
}

pub(super) fn dispatch_clipboard_action_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    event_type: &str,
    contents: &[(String, Vec<u8>)],
    read_only: bool,
) -> Option<bool> {
    let clipboard = unsafe { &*runtime_ptr }.browser_context_runtime();
    let transfer = build_clipboard_data_transfer(scope, contents, read_only)?;
    let Some(event) = construct_clipboard_event(scope, event_type, transfer) else {
        disable_clipboard_data_transfer(scope, transfer);
        return None;
    };
    let outcome = dispatch_public_event(scope, runtime_ptr, handle, event);
    let authored_contents = if !outcome.allows_default() && !read_only {
        clipboard_data_transfer_contents(scope, transfer)
    } else {
        Vec::new()
    };
    disable_clipboard_data_transfer(scope, transfer);
    if !outcome.allows_default() {
        if !authored_contents.is_empty() {
            clipboard.set_clipboard_snapshot(ClipboardSnapshot {
                representations: authored_contents,
                ..ClipboardSnapshot::default()
            });
        }
        return Some(false);
    }

    Some(true)
}

pub(super) fn native_clipboard_text_bytes(text: &str) -> Vec<u8> {
    // Like Blob's native endings, the virtual clipboard uses the browser's
    // platform profile. Clipboard writes preserve lone CR characters.
    if !moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE
        .platform
        .starts_with("Win")
    {
        return text.as_bytes().to_vec();
    }
    let mut bytes = Vec::with_capacity(text.len());
    let mut previous = None;
    for byte in text.bytes() {
        if byte == b'\n' && previous != Some(b'\r') {
            bytes.push(b'\r');
        }
        bytes.push(byte);
        previous = Some(byte);
    }
    bytes
}
