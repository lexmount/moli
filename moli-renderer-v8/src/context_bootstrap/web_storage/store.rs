pub use moli_web_storage::*;

use crate::{
    native_bridge::WindowTaskTarget,
    page_task_queue::{RendererPageStorageEventData, RendererPageStorageEventDeliverySender},
};

/// A live LocalWindow owns this registration; the shared store retains it weakly.
pub(crate) struct WebStorageEventRecipient {
    origin: String,
    area_key: String,
    target: WindowTaskTarget,
    sender: RendererPageStorageEventDeliverySender,
}

impl WebStorageEventRecipient {
    pub(crate) fn new(
        origin: String,
        area_key: String,
        target: WindowTaskTarget,
        sender: RendererPageStorageEventDeliverySender,
    ) -> Self {
        Self {
            origin,
            area_key,
            target,
            sender,
        }
    }

    pub(crate) fn matches(&self, origin: &str, area_key: &str, target: WindowTaskTarget) -> bool {
        self.origin == origin && self.area_key == area_key && self.target == target
    }
}

impl WebStorageEventSink for WebStorageEventRecipient {
    fn queue_storage_event(
        &self,
        source: &WebStorageEventSource,
        origin: &str,
        area_key: &str,
        data: &RendererPageStorageEventData,
    ) -> WebStorageEventDelivery {
        // The source Page already captured its local, potentially unmaterialized Windows.
        if self.sender.event_source().eq(source)
            || self.origin != origin
            || self.area_key != area_key
        {
            return WebStorageEventDelivery::Skipped;
        }
        if self.sender.send(self.target, data.clone()).is_ok() {
            WebStorageEventDelivery::Queued
        } else {
            WebStorageEventDelivery::Closed
        }
    }
}
