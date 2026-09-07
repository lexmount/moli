//! Wake-route storage shared by deferred and live browser-context workers.
//!
//! This collection owns only channel senders, not renderer lifecycle authority.
//! Callers provide synchronization: the lazy runtime's state lock while deferred,
//! and the service's wake-route lock while live.

use tokio::sync::mpsc;

pub(crate) struct WorkerOwnerWakeRoutes<W> {
    senders: Vec<mpsc::UnboundedSender<W>>,
}

impl<W> Default for WorkerOwnerWakeRoutes<W> {
    fn default() -> Self {
        Self {
            senders: Vec::new(),
        }
    }
}

impl<W> WorkerOwnerWakeRoutes<W> {
    pub(crate) fn register(&mut self, sender: mpsc::UnboundedSender<W>) {
        // Deferred or idle services may never send another wake. Prune on
        // registration too, so sequential target churn cannot accumulate closed
        // channels. This is not immediate unregistration: the last closed routes
        // remain until the next register/broadcast, and Vec capacity is retained.
        // Tokio's receiver-drop waker cleanup independently releases the retired
        // renderer's I/O driver even while these senders remain registered.
        self.senders.retain(|registered| !registered.is_closed());
        if !sender.is_closed() {
            self.senders.push(sender);
        }
    }

    /// Transfer deferred routes through the live service's registration entry
    /// point, which also wakes the new owner if service-lane work is pending.
    pub(crate) fn into_senders(self) -> impl Iterator<Item = mpsc::UnboundedSender<W>> {
        self.senders.into_iter()
    }

    #[cfg(test)]
    pub(crate) fn len_for_test(&self) -> usize {
        self.senders.len()
    }
}

impl<W: Copy> WorkerOwnerWakeRoutes<W> {
    /// Returns whether at least one receiver accepted the wake.
    pub(crate) fn broadcast(&mut self, wake: W) -> bool {
        self.senders.retain(|sender| sender.send(wake).is_ok());
        !self.senders.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_prunes_closed_routes_without_affecting_live_peer() {
        let mut routes = WorkerOwnerWakeRoutes::default();
        let (peer, mut peer_rx) = mpsc::unbounded_channel();
        routes.register(peer);
        for _ in 0..64 {
            let (sender, receiver) = mpsc::unbounded_channel();
            routes.register(sender);
            assert_eq!(routes.senders.len(), 2);
            drop(receiver);
        }
        let (closed, mut receiver) = mpsc::unbounded_channel();
        receiver.close();
        routes.register(closed);
        assert_eq!(routes.senders.len(), 1);
        assert!(routes.broadcast(7));
        assert_eq!(peer_rx.try_recv(), Ok(7));
        assert!(matches!(
            peer_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn broadcast_wakes_every_live_receiver_and_prunes_retired_routes() {
        let mut routes = WorkerOwnerWakeRoutes::default();
        assert!(!routes.broadcast(1));
        let (first, mut first_rx) = mpsc::unbounded_channel();
        let (retired, retired_rx) = mpsc::unbounded_channel();
        let (second, mut second_rx) = mpsc::unbounded_channel();
        routes.register(first);
        routes.register(retired);
        routes.register(second);
        drop(retired_rx);
        assert!(routes.broadcast(7));
        assert_eq!(routes.senders.len(), 2);
        assert!(routes.broadcast(9));
        for receiver in [&mut first_rx, &mut second_rx] {
            assert_eq!(receiver.try_recv(), Ok(7));
            assert_eq!(receiver.try_recv(), Ok(9));
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
        drop(first_rx);
        drop(second_rx);
        assert!(!routes.broadcast(11));
        assert!(routes.senders.is_empty());
    }

    #[test]
    fn transfer_preserves_live_routes_without_reviving_closed_receivers() {
        let mut deferred = WorkerOwnerWakeRoutes::default();
        let (peer, mut peer_rx) = mpsc::unbounded_channel();
        let (retired, retired_rx) = mpsc::unbounded_channel();
        deferred.register(peer);
        deferred.register(retired);
        drop(retired_rx);

        let mut live = WorkerOwnerWakeRoutes::default();
        for sender in deferred.into_senders() {
            live.register(sender);
        }
        assert_eq!(live.senders.len(), 1);
        assert!(live.broadcast(7));
        assert_eq!(peer_rx.try_recv(), Ok(7));
    }
}
