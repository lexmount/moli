use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use parking_lot::Mutex;

use super::store::{
    SharedWebStorageStore, WebStorageAreaKind, WebStorageMutation, WebStorageMutationError,
};
use crate::{
    native_bridge::WindowTaskTarget,
    page_task_queue::{RendererPageStorageEventData, RendererPageStorageEventDeliverySender},
};

trait WebStorageEventDelivery: std::fmt::Debug + Send + Sync {
    fn enqueue(&self, target: WindowTaskTarget, data: RendererPageStorageEventData);
}

impl WebStorageEventDelivery for RendererPageStorageEventDeliverySender {
    fn enqueue(&self, target: WindowTaskTarget, data: RendererPageStorageEventData) {
        let _ = self.send(target, data);
    }
}

/// The shared store retains an event delivery interface. Concrete Page queues
/// and their other task payloads remain behind the renderer's delivery route.
#[derive(Debug, Default)]
pub(crate) struct WebStorageEventRoute {
    sender: Mutex<Option<Box<dyn WebStorageEventDelivery>>>,
}

impl WebStorageEventRoute {
    pub(crate) fn bind(&self, sender: RendererPageStorageEventDeliverySender) {
        *self.sender.lock() = Some(Box::new(sender));
    }

    pub(crate) fn close(&self) {
        *self.sender.lock() = None;
    }

    fn enqueue(&self, target: WindowTaskTarget, data: RendererPageStorageEventData) {
        if let Some(sender) = self.sender.lock().as_ref() {
            sender.enqueue(target, data);
        }
    }
}

/// One Storage area as seen by one exact LocalWindow. The backing store keeps
/// weak Sources; the Window owns the subscription and its source identity.
#[derive(Clone, Debug)]
pub(crate) struct WebStorageArea {
    store: SharedWebStorageStore,
    source: Arc<WebStorageEventSource>,
}

#[derive(Debug)]
pub(super) struct WebStorageEventSource {
    area_key: String,
    kind: WebStorageAreaKind,
    target: WindowTaskTarget,
    route: Arc<WebStorageEventRoute>,
}

pub(super) type WebStorageMutationSource<'a> = (&'a Arc<WebStorageEventSource>, &'a str);

#[derive(Default)]
pub(super) struct StorageAreaEventRecipients {
    areas: HashMap<String, Vec<Weak<WebStorageEventSource>>>,
}

impl StorageAreaEventRecipients {
    fn subscribe(&mut self, source: &Arc<WebStorageEventSource>) {
        self.areas.retain(|_, sources| {
            sources.retain(|source| source.strong_count() != 0);
            !sources.is_empty()
        });
        self.areas
            .entry(source.area_key.clone())
            .or_default()
            .push(Arc::downgrade(source));
    }

    pub(super) fn publish(
        &mut self,
        mutation: &WebStorageMutation,
        source: Option<WebStorageMutationSource<'_>>,
    ) {
        let (area_key, key, old_value, new_value) = match mutation {
            WebStorageMutation::ItemAdded {
                area_key,
                key,
                value,
            } => (area_key, Some(key.as_units()), None, Some(value.as_units())),
            WebStorageMutation::ItemUpdated {
                area_key,
                key,
                old_value,
                new_value,
            } => (
                area_key,
                Some(key.as_units()),
                Some(old_value.as_units()),
                Some(new_value.as_units()),
            ),
            WebStorageMutation::ItemRemoved {
                area_key,
                key,
                old_value,
            } => (
                area_key,
                Some(key.as_units()),
                Some(old_value.as_units()),
                None,
            ),
            WebStorageMutation::ItemsCleared { area_key } => (area_key, None, None, None),
        };
        let Some(recipients) = self.areas.get_mut(area_key) else {
            return;
        };
        recipients.retain(|recipient| {
            let Some(recipient) = recipient.upgrade() else {
                return false;
            };
            if source.is_some_and(|(source, _)| {
                Arc::ptr_eq(source, &recipient) || source.kind != recipient.kind
            }) {
                return true;
            }
            let data = RendererPageStorageEventData::new(
                source.map_or("", |(_, url)| url).to_owned(),
                recipient.kind == WebStorageAreaKind::Session,
                key.map(<[u16]>::to_vec),
                old_value.map(<[u16]>::to_vec),
                new_value.map(<[u16]>::to_vec),
            );
            recipient.route.enqueue(recipient.target, data);
            true
        });
    }
}

impl WebStorageArea {
    pub(crate) fn new(
        store: SharedWebStorageStore,
        area_key: String,
        kind: WebStorageAreaKind,
        target: WindowTaskTarget,
        route: Arc<WebStorageEventRoute>,
    ) -> Self {
        let source = Arc::new(WebStorageEventSource {
            area_key,
            kind,
            target,
            route,
        });
        store.lock().event_recipients.subscribe(&source);
        Self { store, source }
    }

    pub(crate) fn try_set_item(
        &self,
        key: &[u16],
        value: &[u16],
        url: &str,
    ) -> Result<bool, WebStorageMutationError> {
        self.store.lock().try_set_item_utf16_from_source(
            &self.source.area_key,
            key,
            value,
            Some((&self.source, url)),
        )
    }

    pub(crate) fn try_remove_item(
        &self,
        key: &[u16],
        url: &str,
    ) -> Result<bool, WebStorageMutationError> {
        self.store.lock().try_remove_item_utf16_from_source(
            &self.source.area_key,
            key,
            Some((&self.source, url)),
        )
    }

    pub(crate) fn try_clear(&self, url: &str) -> Result<bool, WebStorageMutationError> {
        self.store
            .lock()
            .try_clear_from_source(&self.source.area_key, Some((&self.source, url)))
    }
}
