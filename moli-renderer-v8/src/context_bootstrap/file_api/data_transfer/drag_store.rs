use super::*;

/// The session's backing transfer is never exposed to script. Native events
/// publish separate DataTransfer/item views with event-local access rights.
pub(crate) struct DragDataStore {
    backing: v8::Global<v8::Object>,
}

impl DragDataStore {
    pub(crate) fn new(scope: &mut v8::PinScope<'_, '_>, data: &RendererDragData) -> Option<Self> {
        let backing = build_data_transfer_object(scope, data)?;
        Some(Self {
            backing: v8::Global::new(scope, backing),
        })
    }

    pub(crate) fn backing_transfer<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Object> {
        v8::Local::new(scope, &self.backing)
    }

    pub(crate) fn open_event<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event_name: &str,
    ) -> Option<DragDataTransferEvent<'s>> {
        let backing = self.backing_transfer(scope);
        let transfer = DataTransferShellDeclaration::default().bind(scope).ok()?;
        let items = initialize_data_transfer_object(scope, transfer)?;
        let backing_items = DataTransferItemStore::for_owner(scope, backing)?;
        let array = item_list_array(scope, backing_items.item_list)?;
        set_private_value(
            scope,
            items,
            DATA_TRANSFER_ITEM_LIST_ARRAY_SLOT,
            array.into(),
        );
        set_private_value(scope, transfer, DATA_TRANSFER_STORE_SLOT, backing.into());
        let mode = match event_name {
            "dragstart" => DataTransferMode::ReadWrite,
            "drop" => DataTransferMode::ReadOnly,
            _ => DataTransferMode::Protected,
        };
        set_private_number(
            scope,
            transfer,
            DATA_TRANSFER_MODE_SLOT,
            f64::from(mode as u8),
        );
        for slot in [
            DATA_TRANSFER_DROP_EFFECT_SLOT,
            DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        ] {
            if let Some(value) = get_private_value(scope, backing, slot) {
                set_private_value(scope, transfer, slot, value);
            }
        }
        data_transfer_item_list_did_change(scope, items);
        Some(DragDataTransferEvent { backing, transfer })
    }
}

pub(crate) struct DragDataTransferEvent<'s> {
    backing: v8::Local<'s, v8::Object>,
    transfer: v8::Local<'s, v8::Object>,
}

impl<'s> DragDataTransferEvent<'s> {
    pub(crate) fn object(&self) -> v8::Local<'s, v8::Object> {
        self.transfer
    }

    /// Preserve the negotiated effects before invalidating every live view.
    /// Payloads stay with the backing store for later events and UA defaults.
    pub(crate) fn finish(self, scope: &mut v8::PinScope<'s, '_>) {
        for slot in [
            DATA_TRANSFER_DROP_EFFECT_SLOT,
            DATA_TRANSFER_EFFECT_ALLOWED_SLOT,
        ] {
            if let Some(value) = get_private_value(scope, self.transfer, slot) {
                set_private_value(scope, self.backing, slot, value);
            }
        }
        set_private_number(
            scope,
            self.transfer,
            DATA_TRANSFER_MODE_SLOT,
            f64::from(DataTransferMode::Disabled as u8),
        );
        let null = v8::null(scope);
        set_private_value(scope, self.transfer, DATA_TRANSFER_STORE_SLOT, null.into());
        if let Some(items) = DataTransferItemStore::for_owner(scope, self.transfer) {
            data_transfer_item_list_did_change(scope, items.item_list);
        }
    }
}
