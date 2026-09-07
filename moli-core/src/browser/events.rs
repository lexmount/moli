use tokio::sync::broadcast;

use super::{BrowserContextId, BrowserSequence, WebContentsHandle};

/// A committed Browser lifetime change, with no protocol or session identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserEvent {
    ContextCreated(BrowserContextId),
    ContextDisposed(BrowserContextId),
    WebContentsCreated(WebContentsHandle),
    WebContentsClosed {
        web_contents: WebContentsHandle,
        activated: Option<WebContentsHandle>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserEventRecord {
    pub sequence: BrowserSequence,
    pub event: BrowserEvent,
}

/// Physical membership at one Browser owner boundary. A lagged observer must
/// resubscribe with this snapshot rather than guessing which events it lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserSnapshot {
    pub sequence: BrowserSequence,
    pub contexts: Vec<BrowserContextId>,
    pub web_contents: Vec<WebContentsHandle>,
    pub selected_web_contents: Vec<WebContentsHandle>,
}

pub type BrowserEventReceiver = broadcast::Receiver<BrowserEventRecord>;

pub(super) struct BrowserEventStream {
    sender: broadcast::Sender<BrowserEventRecord>,
    sequence: BrowserSequence,
}

impl Default for BrowserEventStream {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self {
            sender,
            sequence: BrowserSequence::allocate(),
        }
    }
}

impl BrowserEventStream {
    pub(super) fn publish(&mut self, event: BrowserEvent) {
        self.sequence = BrowserSequence::allocate();
        let _ = self.sender.send(BrowserEventRecord {
            sequence: self.sequence,
            event,
        });
    }

    pub(super) fn subscribe(
        &self,
        contexts: impl Iterator<Item = BrowserContextId>,
        web_contents: impl Iterator<Item = WebContentsHandle>,
        selected_web_contents: impl Iterator<Item = WebContentsHandle>,
    ) -> (BrowserSnapshot, BrowserEventReceiver) {
        (
            BrowserSnapshot {
                sequence: self.sequence,
                contexts: contexts.collect(),
                web_contents: web_contents.collect(),
                selected_web_contents: selected_web_contents.collect(),
            },
            self.sender.subscribe(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{
        BrowserContextStoragePartitionHandles, BrowserService, StoragePartitionKind,
    };
    use broadcast::error::TryRecvError;

    #[tokio::test]
    async fn web_contents_close_publishes_exact_membership_and_native_selection() {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let context = browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap();
        let (first, _) = context.create_web_contents(Default::default()).unwrap();
        let (second, _) = context.create_web_contents(Default::default()).unwrap();
        assert!(context.select_web_contents(first.id()));
        let (snapshot, mut events) = browser.subscribe().unwrap();
        assert_eq!(snapshot.web_contents, [first, second]);
        assert_eq!(snapshot.selected_web_contents, [first]);
        let close = browser.close_web_contents(first).unwrap();
        assert_eq!(close.activated, Some(second));
        assert_eq!(context.selected_web_contents_handle(), Some(second));
        let closed = events.try_recv().unwrap();
        assert_eq!(
            closed.event,
            BrowserEvent::WebContentsClosed {
                web_contents: first,
                activated: Some(second)
            }
        );
        assert!(closed.sequence > snapshot.sequence);
        assert!(browser.close_web_contents(first).is_err());
        assert_eq!(events.try_recv(), Err(TryRecvError::Empty));
        let (current, _) = browser.subscribe().unwrap();
        assert_eq!(current.web_contents, [second]);
        assert_eq!(current.selected_web_contents, [second]);
        close.close_async().await;
        let (replacement, _) = context.create_web_contents(Default::default()).unwrap();
        assert_ne!(replacement, first);
        assert_eq!(
            events.try_recv().unwrap().event,
            BrowserEvent::WebContentsCreated(replacement)
        );
        for close in context.close_all_web_contents() {
            close.close_async().await;
        }
        for handle in [second, replacement] {
            assert_eq!(
                events.try_recv().unwrap().event,
                BrowserEvent::WebContentsClosed {
                    web_contents: handle,
                    activated: None
                }
            );
        }
        assert!(context.selected_web_contents_handle().is_none());
        assert!(browser.subscribe().unwrap().0.web_contents.is_empty());
        service.shutdown();
    }

    #[test]
    fn context_events_and_snapshot_share_the_committed_owner_boundary() {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (initial, mut first) = browser.subscribe().unwrap();
        assert!(initial.contexts.is_empty());
        let context = browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap();
        let created = first.try_recv().unwrap();
        assert_eq!(created.event, BrowserEvent::ContextCreated(context.id()));
        assert!(created.sequence > initial.sequence);
        let (snapshot, mut second) = browser.subscribe().unwrap();
        assert_eq!(snapshot.sequence, created.sequence);
        assert_eq!(snapshot.contexts, vec![context.id()]);
        assert_eq!(second.try_recv(), Err(TryRecvError::Empty));

        assert!(context.remove().unwrap());
        let disposed = first.try_recv().unwrap();
        assert_eq!(disposed.event, BrowserEvent::ContextDisposed(context.id()));
        assert_eq!(second.try_recv().unwrap(), disposed);
        assert!(disposed.sequence > created.sequence);
        assert!(!context.is_live());
        assert!(context.selected_web_contents_id().is_none());
        assert!(context.selected_web_contents_handle().is_none());
        assert!(!context.has_pending_javascript_dialog());
        context.set_service_worker_pause_on_start(false);
        context.set_service_worker_related_pause_on_start_policies(Vec::new());
        context.set_dedicated_worker_pause_on_start(false);
        assert!(!context.remove().unwrap());
        assert_eq!(first.try_recv(), Err(TryRecvError::Empty));
        let (snapshot, observer) = browser.subscribe().unwrap();
        assert_eq!(snapshot.sequence, disposed.sequence);
        assert!(snapshot.contexts.is_empty());
        drop(observer);
        assert!(
            browser.subscribe().is_ok(),
            "an observer must not own the Browser"
        );
        service.shutdown();
        assert_eq!(first.try_recv(), Err(TryRecvError::Closed));
        assert_eq!(second.try_recv(), Err(TryRecvError::Closed));
        assert!(browser.subscribe().is_err());
    }

    #[test]
    fn lagged_browser_events_require_an_atomic_snapshot_and_new_subscription() {
        let mut stream = BrowserEventStream::default();
        let context = BrowserContextId::allocate();
        let (_, mut slow) =
            stream.subscribe(std::iter::empty(), std::iter::empty(), std::iter::empty());
        for _ in 0..257 {
            stream.publish(BrowserEvent::ContextCreated(context));
        }
        assert_eq!(slow.try_recv(), Err(TryRecvError::Lagged(1)));
        let (snapshot, mut recovered) = stream.subscribe(
            std::iter::once(context),
            std::iter::empty(),
            std::iter::empty(),
        );
        assert_eq!(snapshot.contexts, vec![context]);
        assert_eq!(recovered.try_recv(), Err(TryRecvError::Empty));
        stream.publish(BrowserEvent::ContextDisposed(context));
        let disposed = recovered.try_recv().unwrap();
        assert!(disposed.sequence > snapshot.sequence);
        assert_eq!(disposed.event, BrowserEvent::ContextDisposed(context));
        assert_eq!(recovered.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn browser_shutdown_publishes_each_context_disposal_before_stream_closure() {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let contexts = (0..2)
            .map(|_| {
                browser
                    .create_context(
                        BrowserContextStoragePartitionHandles::memory(),
                        StoragePartitionKind::Ephemeral,
                        None,
                        None,
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let (snapshot, mut events) = browser.subscribe().unwrap();
        service.shutdown();
        let mut previous = snapshot.sequence;
        for context in contexts {
            assert!(!context.is_live());
            let event = events.try_recv().unwrap();
            assert_eq!(event.event, BrowserEvent::ContextDisposed(context.id()));
            assert!(event.sequence > previous);
            previous = event.sequence;
        }
        assert_eq!(events.try_recv(), Err(TryRecvError::Closed));
    }
}
