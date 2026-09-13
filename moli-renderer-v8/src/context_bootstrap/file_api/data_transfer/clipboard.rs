use super::*;
use crate::runtime::RendererDragDataItem;

pub(crate) fn build_clipboard_data_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    contents: &[(String, Vec<u8>)],
    read_only: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut data = RendererDragData {
        items: Vec::new(),
        files: Vec::new(),
        directories: Vec::new(),
        drag_operations_mask: 0,
    };
    for (mime_type, bytes) in contents {
        if bytes.is_empty() {
            continue;
        }
        if mime_type == "image/png" {
            data.files.push(RendererDraggedFile {
                bytes: bytes.clone(),
                mime_type: mime_type.clone(),
                name: "image.png".to_owned(),
                last_modified: 0.0,
            });
        } else {
            data.items.push(RendererDragDataItem {
                mime_type: mime_type.clone(),
                data: String::from_utf8_lossy(bytes).into_owned(),
                title: None,
                base_url: None,
            });
        }
    }
    let transfer = build_data_transfer_object(scope, &data)?;
    set_private_value(
        scope,
        transfer,
        DATA_TRANSFER_CLIPBOARD_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_string(
        scope,
        transfer,
        DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        "uninitialized",
    );
    set_private_number(
        scope,
        transfer,
        DATA_TRANSFER_MODE_SLOT,
        if read_only { 1.0 } else { 0.0 },
    );
    Some(transfer)
}

pub(crate) fn clipboard_data_transfer_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transfer: v8::Local<'s, v8::Object>,
) -> Vec<(String, Vec<u8>)> {
    let Some(store) = DataTransferItemStore::for_owner(scope, transfer) else {
        return Vec::new();
    };
    let Some(items) = item_list_array(scope, store.item_list) else {
        return Vec::new();
    };
    let mut contents = Vec::new();
    for index in 0..items.length() {
        let Some(item) = items
            .get_index(scope, index)
            .and_then(|item| v8::Local::<v8::Object>::try_from(item).ok())
        else {
            continue;
        };
        if item_kind(scope, item).as_deref() == Some("string") {
            if let (Some(mime_type), Some(text)) =
                (item_type(scope, item), item_string_value(scope, item))
            {
                contents.push((mime_type, text.into_bytes()));
            }
        } else if let Some(file) =
            item_file_object(scope, item).and_then(|file| selected_file_from_object(scope, file))
        {
            contents.push((file.mime_type, file.bytes));
        }
    }
    contents
}

pub(crate) fn disable_clipboard_data_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transfer: v8::Local<'s, v8::Object>,
) {
    if let Some(store) = DataTransferItemStore::for_owner(scope, transfer) {
        store.clear(scope);
    }
    set_private_number(scope, transfer, DATA_TRANSFER_MODE_SLOT, 2.0);
}
