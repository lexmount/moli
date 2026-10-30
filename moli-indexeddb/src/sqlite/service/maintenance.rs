//! One service task proposes reclamation through each database's own FIFO.

use std::sync::Weak;
use tokio::sync::oneshot;

use super::*;

pub(super) const IDLE_GRACE: Duration = Duration::from_secs(2);
const ROUND_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct Maintenance {
    stop: oneshot::Sender<()>,
    finished: oneshot::Receiver<()>,
}

impl Maintenance {
    pub(super) fn start(
        runtime: RuntimeHandle,
        registry: Weak<Mutex<Registry>>,
    ) -> Result<Self, DispatchError> {
        let (stop, stopping) = oneshot::channel();
        let (completed, finished) = oneshot::channel();
        runtime.spawn(async move {
            run(registry, stopping).await;
            let _ = completed.send(());
        })?;
        Ok(Self { stop, finished })
    }

    pub(super) fn stop(self, runtime: &RuntimeHandle) -> Result<(), IndexedDbError> {
        let _ = self.stop.send(());
        runtime
            .wait(async move {
                self.finished.await.map_err(|_| {
                    IndexedDbError::Io("SQLite maintenance stopped without finishing".into())
                })
            })
            .map_err(|error| IndexedDbError::Io(error.to_string()))?
    }
}

pub(super) type PendingChecks = BTreeMap<DatabaseId, oneshot::Receiver<Result<(), IndexedDbError>>>;

pub(super) fn submit_round(registry: &Mutex<Registry>, now: Instant, pending: &mut PendingChecks) {
    pending.retain(|_, receipt| {
        matches!(receipt.try_recv(), Err(oneshot::error::TryRecvError::Empty))
    });
    let owners = registry
        .lock()
        .entries
        .values()
        .filter(|client| matches!(client.location, Location::Disk { .. }) && !client.is_closed())
        .map(|client| (client.id, client.owner.clone()))
        .collect::<Vec<_>>();
    for (id, owner) in owners {
        if let std::collections::btree_map::Entry::Vacant(slot) = pending.entry(id)
            && let Ok(receipt) = owner.maintain(now)
        {
            slot.insert(receipt);
        }
    }
}

async fn run(registry: Weak<Mutex<Registry>>, mut stopping: oneshot::Receiver<()>) {
    let mut pending = PendingChecks::new();
    loop {
        tokio::select! {
            _ = &mut stopping => return,
            _ = tokio::time::sleep(ROUND_INTERVAL) => (),
        }
        let Some(registry) = registry.upgrade() else {
            return;
        };
        // Keep at most one outstanding check for each owner. A slow database
        // cannot delay new rounds for databases that became idle afterward.
        submit_round(&registry, Instant::now(), &mut pending);
    }
}
