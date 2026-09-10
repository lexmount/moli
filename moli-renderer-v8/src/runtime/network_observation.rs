use std::sync::Arc;

use moli_page_types::ScriptNetworkOutputItem;
use parking_lot::Mutex;
use tokio::sync::watch;

use super::{RendererBrowserContextRuntimeId, RendererDocumentLifecycleIdentity};

/// One physical producer occurrence, shared by native input and source FIFO.
/// A complete-only diagnostic remains complete-only; it does not invent a start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererNetworkOccurrence {
    pub runtime: RendererBrowserContextRuntimeId,
    pub owner_local_host_id: super::RendererOwnerLocalHostId,
    pub document: RendererDocumentLifecycleIdentity,
    pub item: Arc<ScriptNetworkOutputItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_network_receipts_require_exact_commit_and_source_close_follows_the_fifo() {
        let runtime = RendererBrowserContextRuntimeId::new_for_testing(19);
        let reporter = RendererNetworkReporter::new(runtime);
        let owner = crate::RendererOwnerLocalHostId::new_for_testing(11);
        let document = crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(
            crate::PageId::new_for_testing(7),
        )
        .identity();
        let item = ScriptNetworkOutputItem::SubresourceBodyFinished(std::sync::Arc::new(
            moli_page_types::SubresourceBodyFinished::failed(
                moli_page_types::SubresourceNetworkRequestHandle::new(1),
                "failed".into(),
            ),
        ));
        assert!(
            reporter
                .report(owner, document, item.clone())
                .committed()
                .await
                .is_none()
        );
        let inputs = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let pending = inputs.clone();
        reporter.install_handler(move |input| pending.lock().push_back(input));
        let rejected = reporter.report(owner, document, item.clone());
        let accepted = reporter.report(owner, document, item.clone());
        let clone = accepted.clone();
        reporter.close_source(owner, document.document.page_id);
        drop(inputs.lock().pop_front().unwrap());
        assert!(rejected.committed().await.is_none());
        let RendererNetworkInput::Observation(input) = inputs.lock().pop_front().unwrap() else {
            panic!("receipt must precede closure");
        };
        input.commit(47, document);
        for observation in [accepted, clone] {
            let committed = observation.committed().await.unwrap();
            assert_eq!(committed.browser_sequence(), 47);
            assert_eq!(committed.occurrence().item.as_ref(), &item);
        }
        assert!(
            matches!(inputs.lock().pop_front(), Some(RendererNetworkInput::SourceClosed { runtime: actual, owner_local_host_id, page }) if actual == runtime && owner_local_host_id == owner && page == document.document.page_id)
        );
        assert!(inputs.lock().is_empty());
    }
}

#[derive(Clone, Debug)]
pub struct RendererNetworkObservation {
    occurrence: Arc<RendererNetworkOccurrence>,
    committed: watch::Receiver<Option<(u64, RendererDocumentLifecycleIdentity)>>,
}

impl PartialEq for RendererNetworkObservation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.occurrence, &other.occurrence)
    }
}
impl Eq for RendererNetworkObservation {}

#[derive(Clone, Debug)]
pub struct RendererCommittedNetworkObservation {
    occurrence: Arc<RendererNetworkOccurrence>,
    browser_sequence: u64,
}

impl RendererCommittedNetworkObservation {
    pub fn occurrence(&self) -> &RendererNetworkOccurrence {
        &self.occurrence
    }
    pub fn browser_sequence(&self) -> u64 {
        self.browser_sequence
    }
}

impl RendererNetworkObservation {
    pub async fn committed(mut self) -> Option<RendererCommittedNetworkObservation> {
        loop {
            if let Some((browser_sequence, document)) = *self.committed.borrow_and_update() {
                if self.occurrence.document != document {
                    Arc::make_mut(&mut self.occurrence).document = document;
                }
                return Some(RendererCommittedNetworkObservation {
                    occurrence: self.occurrence,
                    browser_sequence,
                });
            }
            self.committed.changed().await.ok()?;
        }
    }

    pub(crate) fn item(&self) -> &ScriptNetworkOutputItem {
        &self.occurrence.item
    }
}

/// Only the native owner may acknowledge input. Dropping it rejects the FIFO
/// observation, including shutdown and input from a revoked reservation.
pub enum RendererNetworkInput {
    Observation(RendererNetworkCommit),
    SourceClosed {
        runtime: RendererBrowserContextRuntimeId,
        owner_local_host_id: super::RendererOwnerLocalHostId,
        page: super::PageId,
    },
}

pub struct RendererNetworkCommit {
    pub occurrence: Arc<RendererNetworkOccurrence>,
    committed: watch::Sender<Option<(u64, RendererDocumentLifecycleIdentity)>>,
}

impl RendererNetworkCommit {
    pub fn commit(self, browser_sequence: u64, document: RendererDocumentLifecycleIdentity) {
        assert_ne!(browser_sequence, 0, "native occurrence needs a sequence");
        self.committed
            .send_replace(Some((browser_sequence, document)));
    }
}

type NetworkHandler = Box<dyn Fn(RendererNetworkInput) + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RendererNetworkReporter {
    runtime: RendererBrowserContextRuntimeId,
    handler: Arc<Mutex<Option<NetworkHandler>>>,
}

impl std::fmt::Debug for RendererNetworkReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererNetworkReporter")
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl RendererNetworkReporter {
    pub(crate) fn new(runtime: RendererBrowserContextRuntimeId) -> Self {
        Self {
            runtime,
            handler: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn install_handler(
        &self,
        handler: impl Fn(RendererNetworkInput) + Send + Sync + 'static,
    ) {
        let mut slot = self.handler.lock();
        assert!(slot.is_none(), "one runtime has one native Network owner");
        *slot = Some(Box::new(handler));
    }

    pub(crate) fn report(
        &self,
        owner_local_host_id: super::RendererOwnerLocalHostId,
        document: RendererDocumentLifecycleIdentity,
        item: ScriptNetworkOutputItem,
    ) -> RendererNetworkObservation {
        let occurrence = Arc::new(RendererNetworkOccurrence {
            runtime: self.runtime,
            owner_local_host_id,
            document,
            item: Arc::new(item),
        });
        let (committed, observation) = watch::channel(None);
        let input = RendererNetworkInput::Observation(RendererNetworkCommit {
            occurrence: occurrence.clone(),
            committed,
        });
        if let Some(handler) = self.handler.lock().as_ref() {
            handler(input);
        }
        RendererNetworkObservation {
            occurrence,
            committed: observation,
        }
    }

    pub(crate) fn close_source(
        &self,
        owner_local_host_id: super::RendererOwnerLocalHostId,
        page: super::PageId,
    ) {
        if let Some(handler) = self.handler.lock().as_ref() {
            handler(RendererNetworkInput::SourceClosed {
                runtime: self.runtime,
                owner_local_host_id,
                page,
            });
        }
    }
}
