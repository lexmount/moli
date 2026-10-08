use super::*;
use crate::{
    context_bootstrap::SharedWebStorageStore,
    native_bridge::WindowTaskTarget,
    page_task_queue::{RendererPageStorageEventData, RendererPageStorageEventDeliverySender},
};

#[derive(Clone, Debug)]
struct StorageEventRecipient {
    identity: Arc<()>,
    store: SharedWebStorageStore,
    origin: String,
    area_key: String,
    sender: RendererPageStorageEventDeliverySender,
}

#[derive(Debug, Default)]
pub(super) struct BrowserStorageEventRecipients {
    windows: HashMap<
        (super::super::RendererOutputStreamIdentity, WindowTaskTarget),
        StorageEventRecipient,
    >,
}

#[derive(Debug)]
pub(crate) struct BrowserStorageEventRegistration {
    registry: Arc<Mutex<BrowserStorageEventRecipients>>,
    target: (super::super::RendererOutputStreamIdentity, WindowTaskTarget),
    identity: Arc<()>,
}

impl Drop for BrowserStorageEventRegistration {
    fn drop(&mut self) {
        let mut registry = self.registry.lock();
        if registry
            .windows
            .get(&self.target)
            .is_some_and(|recipient| Arc::ptr_eq(&recipient.identity, &self.identity))
        {
            registry.windows.remove(&self.target);
        }
    }
}

impl RendererBrowserContextRuntime {
    pub(crate) fn register_storage_event_recipient(
        &self,
        page: super::super::RendererOutputStreamIdentity,
        target: WindowTaskTarget,
        store: SharedWebStorageStore,
        origin: String,
        area_key: String,
        sender: RendererPageStorageEventDeliverySender,
    ) -> BrowserStorageEventRegistration {
        let target = (page, target);
        let identity = Arc::new(());
        self.inner.storage_event_recipients.lock().windows.insert(
            target,
            StorageEventRecipient {
                identity: identity.clone(),
                store,
                origin,
                area_key,
                sender,
            },
        );
        BrowserStorageEventRegistration {
            registry: self.inner.storage_event_recipients.clone(),
            target,
            identity,
        }
    }

    pub(crate) fn queue_other_page_storage_events(
        &self,
        source_page: super::super::RendererOutputStreamIdentity,
        store: &SharedWebStorageStore,
        origin: &str,
        area_key: &str,
        data: &RendererPageStorageEventData,
    ) -> usize {
        if data.is_session() {
            return 0;
        }
        let recipients = self
            .inner
            .storage_event_recipients
            .lock()
            .windows
            .iter()
            .filter(|(target, recipient)| {
                target.0 != source_page
                    && Arc::ptr_eq(&recipient.store, store)
                    && recipient.origin == origin
                    && recipient.area_key == area_key
            })
            .map(|(target, recipient)| (target.1, recipient.sender.clone()))
            .collect::<Vec<_>>();
        recipients
            .into_iter()
            .filter(|(target, sender)| sender.send(*target, data.clone()).is_ok())
            .count()
    }
}
