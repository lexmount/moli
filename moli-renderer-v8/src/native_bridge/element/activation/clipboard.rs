//! Trusted editing shortcuts share the BrowserContext clipboard with the
//! asynchronous Clipboard API. Event payloads are native, operation-scoped views.
use crate::context_bootstrap::{
    DragDataStoreMode, create_drag_data_transfer_view, create_external_drag_data_store,
    finish_drag_data_transfer_view, native_data_transfer_get_data, read_string_drag_data,
};
use crate::native_bridge::element::forms::{
    is_text_control_user_editable, replace_text_control_selection,
};
use crate::native_bridge::element::{
    TextEditInputType, construct_clipboard_event, contenteditable_editing_host,
    dispatch_public_event, is_text_control, text_control_value,
};
use crate::runtime::ClipboardSnapshot;
use crate::util::utf16_units;
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

fn selected_text(
    scope: &mut v8::PinScope<'_, '_>,
    host: &PageScriptHost,
    target: DomHandle,
    text_control: bool,
) -> String {
    if text_control {
        let Some(element) = host
            .dom_host()
            .node(target)
            .and_then(|node| node.as_element())
        else {
            return String::new();
        };
        if element
            .attribute("type")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
        {
            String::new()
        } else {
            let units = utf16_units(&text_control_value(host, target));
            let start = (element.selection_start() as usize).min(units.len());
            let end = (element.selection_end() as usize)
                .min(units.len())
                .max(start);
            String::from_utf16_lossy(&units[start..end])
        }
    } else {
        crate::context_bootstrap::native_editing_selected_text(scope)
    }
}

pub(crate) fn perform_clipboard_shortcut(
    scope: &mut v8::PinScope<'_, '_>,
    ptr: *mut PageScriptHost,
    target: DomHandle,
    key: &str,
) -> bool {
    let Some(context) =
        crate::native_bridge::node_owner_document_relevant_context(scope, ptr, target)
    else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let host = unsafe { &*ptr };
    let Some(document) = host.dom_host().owner_document_handle(target) else {
        return false;
    };
    let browser = host.browser_context_runtime();
    let paste = key == "v";
    let kind = if paste {
        "paste"
    } else if key == "x" {
        "cut"
    } else {
        "copy"
    };
    let snapshot = if paste {
        browser.clipboard_snapshot()
    } else {
        ClipboardSnapshot::default()
    };
    let items = snapshot
        .representations
        .iter()
        .map(|(mime_type, data)| crate::RendererDragDataItem {
            mime_type: mime_type.clone(),
            data: String::from_utf8_lossy(data).into_owned(),
            title: None,
            base_url: None,
        })
        .collect();
    let data = crate::RendererDragData {
        items,
        files: Vec::new(),
        directories: Vec::new(),
        drag_operations_mask: 0,
    };
    let Some(store) = create_external_drag_data_store(scope, &data) else {
        return false;
    };
    let Some(transfer) = create_drag_data_transfer_view(
        scope,
        &store,
        if paste {
            DragDataStoreMode::ReadOnly
        } else {
            DragDataStoreMode::ReadWrite
        },
        "none",
    ) else {
        return false;
    };
    let Some(event) = construct_clipboard_event(scope, kind, transfer) else {
        return false;
    };
    let allowed = dispatch_public_event(scope, ptr, target, event).allows_default();
    let still_valid = unsafe { &*ptr }.dom_host().is_connected(target)
        && unsafe { &*ptr }.dom_host().owner_document_handle(target) == Some(document);
    let host = unsafe { &*ptr };
    let text_control = is_text_control(host, target);
    let editable = if text_control {
        is_text_control_user_editable(host, target)
    } else {
        contenteditable_editing_host(host, target) == Some(target)
    };
    let result = if paste {
        if allowed && editable && still_valid {
            let text = native_data_transfer_get_data(scope, transfer, "text/plain");
            // An absent or empty plain-text representation has no rich-text
            // paste default. Text controls still replace their selection.
            if !text_control && text.is_empty() {
                finish_drag_data_transfer_view(scope, transfer);
                return false;
            }
            super::text_input::perform_text_input_with_transfer(
                scope,
                ptr,
                target,
                &text,
                TextEditInputType::InsertFromPaste,
                super::text_input::TextInputSource::Keyboard,
                (!text_control).then_some(transfer),
            )
        } else {
            false
        }
    } else {
        // Clipboard listeners can change both the selection and editability.
        // Read the same live selection that the cutting default will delete.
        let selected = if allowed && still_valid && (key != "x" || editable) {
            selected_text(scope, host, target, text_control)
        } else {
            String::new()
        };
        if !allowed {
            browser.set_clipboard_snapshot(ClipboardSnapshot {
                representations: read_string_drag_data(scope, &store)
                    .0
                    .into_iter()
                    .map(|item| (item.mime_type, item.data.into_bytes()))
                    .collect(),
                ..Default::default()
            });
        } else if !selected.is_empty() {
            browser.set_clipboard_snapshot(ClipboardSnapshot {
                representations: vec![("text/plain".into(), selected.as_bytes().to_vec())],
                ..Default::default()
            });
        }
        if key == "x" && allowed && editable && still_valid && !selected.is_empty() {
            if text_control {
                replace_text_control_selection(
                    scope,
                    ptr,
                    target,
                    "",
                    TextEditInputType::DeleteByCut,
                )
            } else {
                super::default_action::replace_contenteditable_selection(
                    scope,
                    ptr,
                    target,
                    "",
                    TextEditInputType::DeleteByCut,
                )
            }
        } else {
            true
        }
    };
    finish_drag_data_transfer_view(scope, transfer);
    result
}
