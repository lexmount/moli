use super::*;

/// Data captured from one successful Web Storage mutation.
///
/// Storage-area matching and source exclusion happen synchronously at the
/// producer boundary. A copy of this immutable event data is then queued for
/// each exact recipient LocalDOMWindow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebStorageEventData {
    url: String,
    is_session: bool,
    key: Option<Vec<u16>>,
    old_value: Option<Vec<u16>>,
    new_value: Option<Vec<u16>>,
}

impl WebStorageEventData {
    pub fn new(
        url: String,
        is_session: bool,
        key: Option<Vec<u16>>,
        old_value: Option<Vec<u16>>,
        new_value: Option<Vec<u16>>,
    ) -> Self {
        Self {
            url,
            is_session,
            key,
            old_value,
            new_value,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub const fn is_session(&self) -> bool {
        self.is_session
    }

    pub fn key(&self) -> Option<&[u16]> {
        self.key.as_deref()
    }

    pub fn old_value(&self) -> Option<&[u16]> {
        self.old_value.as_deref()
    }

    pub fn new_value(&self) -> Option<&[u16]> {
        self.new_value.as_deref()
    }
}

/// An existing producer route's shared identity, without exposing its task types.
#[derive(Clone)]
pub struct WebStorageEventSource(Arc<dyn Send + Sync>);

impl WebStorageEventSource {
    pub fn new(identity: Arc<dyn Send + Sync>) -> Self {
        Self(identity)
    }
}

impl PartialEq for WebStorageEventSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WebStorageEventSource {}

impl fmt::Debug for WebStorageEventSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebStorageEventSource")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebStorageEventDelivery {
    Skipped,
    Queued,
    Closed,
}

/// The renderer supplies a nonblocking sender bound to one exact recipient Window.
pub trait WebStorageEventSink: Send + Sync {
    fn queue_storage_event(
        &self,
        source: &WebStorageEventSource,
        origin: &str,
        area_key: &str,
        data: &WebStorageEventData,
    ) -> WebStorageEventDelivery;
}

impl WebStorageStore {
    pub fn register_event_recipient(&self, recipient: Arc<dyn WebStorageEventSink>) {
        let mut entries = self.event_recipients.lock();
        entries.retain(|entry| entry.strong_count() != 0);
        entries.push(Arc::downgrade(&recipient));
    }

    pub fn queue_remote_storage_events(
        &self,
        source: &WebStorageEventSource,
        origin: &str,
        area_key: &str,
        data: &WebStorageEventData,
    ) -> usize {
        debug_assert!(!data.is_session());
        let recipients = {
            let mut entries = self.event_recipients.lock();
            let mut recipients = Vec::with_capacity(entries.len());
            entries.retain(|entry| {
                let Some(recipient) = entry.upgrade() else {
                    return false;
                };
                recipients.push(recipient);
                true
            });
            recipients
        };
        // Call senders and drop their strong references outside the registry lock.
        let mut queued = 0;
        let mut closed = Vec::new();
        for recipient in &recipients {
            match recipient.queue_storage_event(source, origin, area_key, data) {
                WebStorageEventDelivery::Skipped => {}
                WebStorageEventDelivery::Queued => queued += 1,
                WebStorageEventDelivery::Closed => closed.push(Arc::downgrade(recipient)),
            }
        }
        if !closed.is_empty() {
            self.event_recipients
                .lock()
                .retain(|entry| !closed.iter().any(|closed| Weak::ptr_eq(entry, closed)));
        }
        queued
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recipient {
        events: Mutex<Vec<WebStorageEventData>>,
        register: Mutex<Option<(Weak<WebStorageStore>, Arc<Recipient>)>>,
        closed: bool,
    }

    impl WebStorageEventSink for Recipient {
        fn queue_storage_event(
            &self,
            _source: &WebStorageEventSource,
            _origin: &str,
            _area_key: &str,
            data: &WebStorageEventData,
        ) -> WebStorageEventDelivery {
            self.events.lock().push(data.clone());
            if self.closed {
                return WebStorageEventDelivery::Closed;
            }
            if let Some((store, recipient)) = self.register.lock().take() {
                store.upgrade().unwrap().register_event_recipient(recipient);
            }
            WebStorageEventDelivery::Queued
        }
    }

    #[test]
    fn event_recipients_are_weak_and_delivery_can_register_a_future_recipient() {
        let store = new_shared_web_storage_store();
        let future = Arc::new(Recipient::default());
        let first = Arc::new(Recipient {
            register: Mutex::new(Some((Arc::downgrade(&store), future.clone()))),
            ..Default::default()
        });
        let closed = Arc::new(Recipient {
            closed: true,
            ..Default::default()
        });
        let expired = Arc::new(Recipient::default());
        let weak = Arc::downgrade(&expired);
        store.register_event_recipient(first.clone());
        store.register_event_recipient(closed.clone());
        store.register_event_recipient(expired.clone());
        drop(expired);
        assert!(weak.upgrade().is_none());
        let identity = Arc::new(());
        let source = WebStorageEventSource::new(identity.clone());
        assert_eq!(source, WebStorageEventSource::new(identity));
        let data = WebStorageEventData::new(
            "https://source.test/".into(),
            false,
            Some(vec![0xd800]),
            None,
            Some(vec![0xdc00]),
        );
        assert_eq!(
            store.queue_remote_storage_events(&source, "https://source.test", "area", &data),
            1
        );
        assert!(future.events.lock().is_empty());
        assert_eq!(
            store.queue_remote_storage_events(&source, "https://source.test", "area", &data),
            2
        );
        assert_eq!(*first.events.lock(), vec![data.clone(), data.clone()]);
        assert_eq!(*future.events.lock(), vec![data.clone()]);
        assert_eq!(*closed.events.lock(), vec![data]);
        drop(first);
        drop(future);
        assert_eq!(store.event_recipients.lock().len(), 2);
        let data = WebStorageEventData::new(String::new(), false, None, None, None);
        assert_eq!(
            store.queue_remote_storage_events(&source, "https://source.test", "area", &data),
            0
        );
        assert!(store.event_recipients.lock().is_empty());
    }
}
