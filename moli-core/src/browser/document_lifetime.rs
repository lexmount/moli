use tokio::sync::watch;

/// Why an exact browser Document is no longer current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentRetirement {
    Superseded,
    Unavailable,
}

/// Move-owned retirement signal for one DocumentHost.
///
/// Observation is lazy and does not extend the Document's lifetime. Explicit
/// removal publishes `Superseded`; dropping the owner without that transition
/// reports `Unavailable`. The channel itself names the exact incarnation, so
/// it does not need another copy of the DocumentId or a registry lookup.
#[derive(Debug, Default)]
pub struct DocumentLifetime {
    retired: Option<watch::Sender<bool>>,
}

impl DocumentLifetime {
    pub fn observe(&mut self) -> DocumentLifetimeObserver {
        let sender = self.retired.get_or_insert_with(|| watch::channel(false).0);
        DocumentLifetimeObserver {
            receiver: sender.subscribe(),
        }
    }

    pub fn supersede(self) {
        if let Some(sender) = self.retired {
            sender.send_replace(true);
        }
    }
}

/// Move-only wait for the retirement of one exact DocumentHost.
#[derive(Debug)]
pub struct DocumentLifetimeObserver {
    receiver: watch::Receiver<bool>,
}

impl DocumentLifetimeObserver {
    pub async fn wait(mut self) -> DocumentRetirement {
        loop {
            if *self.receiver.borrow_and_update() {
                return DocumentRetirement::Superseded;
            }
            if self.receiver.changed().await.is_err() {
                return DocumentRetirement::Unavailable;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    #[tokio::test]
    async fn moving_the_owner_preserves_all_observers_until_retirement() {
        let mut lifetime = DocumentLifetime::default();
        assert!(
            lifetime.retired.is_none(),
            "unobserved Documents allocate no channel"
        );
        let first = lifetime.observe();
        let second = lifetime.observe();
        let moved = lifetime;

        let mut first_wait = Box::pin(first.wait());
        let mut context = Context::from_waker(Waker::noop());
        assert_eq!(first_wait.as_mut().poll(&mut context), Poll::Pending);
        moved.supersede();
        assert_eq!(first_wait.await, DocumentRetirement::Superseded);
        assert_eq!(second.wait().await, DocumentRetirement::Superseded);
    }

    #[tokio::test]
    async fn owner_loss_reports_unavailable() {
        let mut lifetime = DocumentLifetime::default();
        let observer = lifetime.observe();
        drop(lifetime);
        assert_eq!(observer.wait().await, DocumentRetirement::Unavailable);
    }
}
