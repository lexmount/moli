use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;

use crate::{MessagePortId, MessagePortWake};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagePortCloseKind {
    PeerClosed,
    ConnectionError,
}

/// One atomic observation of an endpoint and its enabled receiving queue.
#[derive(Debug, Eq, PartialEq)]
pub enum MessagePortPoll<P> {
    Removed,
    Idle,
    Message(P),
    Close(MessagePortCloseKind),
}

/// Closing an endpoint cancels its queued messages. The embedding releases
/// their native attachments after the registry lock has been released.
#[derive(Debug)]
pub struct MessagePortCloseResult<P, O> {
    pub peer_wake: Option<MessagePortWake<O>>,
    pub discarded_messages: VecDeque<P>,
}

/// A removed internal endpoint, including work and owner state to dispose of
/// outside the registry lock.
#[derive(Debug)]
pub struct DiscardedMessagePort<P, O> {
    pub port_id: MessagePortId,
    pub owner: Option<O>,
    pub discarded_messages: VecDeque<P>,
}

#[derive(Debug, Default)]
struct MessagePortState<P, O> {
    peer_id: Option<MessagePortId>,
    owner: Option<O>,
    pending_messages: VecDeque<P>,
    pending_close: Option<MessagePortCloseKind>,
}

/// Registry for MessagePort endpoint state.
#[derive(Debug)]
pub struct MessagePortRegistry<P, O> {
    ports: Mutex<HashMap<MessagePortId, MessagePortState<P, O>>>,
    next_port_id: AtomicU64,
}

impl<P, O> Default for MessagePortRegistry<P, O> {
    fn default() -> Self {
        Self {
            ports: Mutex::default(),
            next_port_id: AtomicU64::new(1),
        }
    }
}

impl<P, O> MessagePortRegistry<P, O>
where
    O: Clone,
{
    fn next_message_port_id(&self) -> MessagePortId {
        self.next_port_id.fetch_add(1, Ordering::Relaxed).max(1)
    }

    /// Register a single unentangled MessagePort endpoint.
    pub fn create_message_port(&self, owner: O) -> MessagePortId {
        let port_id = self.next_message_port_id();
        self.ports.lock().insert(
            port_id,
            MessagePortState {
                peer_id: None,
                owner: Some(owner),
                pending_messages: VecDeque::new(),
                pending_close: None,
            },
        );
        port_id
    }

    /// Register the dangling connection reconstructed from a closed port.
    /// Its connection error is consumed when the receiver starts its queue.
    pub fn create_disconnected_message_port(&self, owner: O) -> MessagePortId {
        let port_id = self.next_message_port_id();
        self.ports.lock().insert(
            port_id,
            MessagePortState {
                peer_id: None,
                owner: Some(owner),
                pending_messages: VecDeque::new(),
                pending_close: Some(MessagePortCloseKind::ConnectionError),
            },
        );
        port_id
    }

    /// Register an entangled pair of MessagePort endpoints.
    pub fn create_entangled_message_port_pair(&self, owner: O) -> (MessagePortId, MessagePortId) {
        let port1 = self.next_message_port_id();
        let port2 = self.next_message_port_id();
        let mut ports = self.ports.lock();
        ports.insert(
            port1,
            MessagePortState {
                peer_id: Some(port2),
                owner: Some(owner.clone()),
                pending_messages: VecDeque::new(),
                pending_close: None,
            },
        );
        ports.insert(
            port2,
            MessagePortState {
                peer_id: Some(port1),
                owner: Some(owner),
                pending_messages: VecDeque::new(),
                pending_close: None,
            },
        );
        (port1, port2)
    }

    /// Attach an embedding owner after a port is transferred or materialized.
    ///
    /// Returns a wake target when the port already has queued messages or a
    /// pending close event.
    pub fn attach_message_port_owner(
        &self,
        port_id: MessagePortId,
        owner: O,
    ) -> Option<MessagePortWake<O>> {
        let mut ports = self.ports.lock();
        let state = ports.get_mut(&port_id)?;
        state.owner = Some(owner.clone());
        (state.pending_close.is_some() || !state.pending_messages.is_empty())
            .then_some(MessagePortWake { port_id, owner })
    }

    /// Detach owner state before transferring a port to another context.
    pub fn detach_message_port_owner_for_transfer(&self, port_id: MessagePortId) {
        if let Some(state) = self.ports.lock().get_mut(&port_id) {
            state.owner = None;
        }
    }

    /// Clear a stale owner after the embedding layer failed to schedule it.
    pub fn clear_message_port_owner(&self, port_id: MessagePortId) {
        if let Some(state) = self.ports.lock().get_mut(&port_id) {
            state.owner = None;
        }
    }

    /// Return the owner that should be woken for already queued work.
    pub fn wake_message_port_if_pending(
        &self,
        port_id: MessagePortId,
    ) -> Option<MessagePortWake<O>> {
        let ports = self.ports.lock();
        let state = ports.get(&port_id)?;
        if state.pending_close.is_none() && state.pending_messages.is_empty() {
            return None;
        }
        Some(MessagePortWake {
            port_id,
            owner: state.owner.clone()?,
        })
    }

    /// Capture the destination before an embedding runs author code.
    pub fn message_port_peer_id(&self, port_id: MessagePortId) -> Option<MessagePortId> {
        self.ports.lock().get(&port_id)?.peer_id
    }

    /// Enqueue to a previously captured destination, even if its source has
    /// since been disentangled. Port ids survive transfer to another owner.
    pub fn enqueue_message_to_endpoint(&self, port_id: MessagePortId, payload: P) -> Result<(), P> {
        let mut ports = self.ports.lock();
        let Some(state) = ports.get_mut(&port_id) else {
            return Err(payload);
        };
        state.pending_messages.push_back(payload);
        Ok(())
    }

    /// Enqueue a message to the peer endpoint and return the target peer id.
    pub fn enqueue_message_to_message_port(
        &self,
        port_id: MessagePortId,
        payload: P,
    ) -> Result<MessagePortId, P> {
        let mut ports = self.ports.lock();
        let Some(peer_id) = ports.get(&port_id).and_then(|state| state.peer_id) else {
            return Err(payload);
        };
        let Some(peer_state) = ports.get_mut(&peer_id) else {
            return Err(payload);
        };
        peer_state.pending_messages.push_back(payload);
        Ok(peer_id)
    }

    /// Observe removal even before start(), without consuming a disabled
    /// queue. Enabled queues deliver messages before their close notification.
    pub fn poll_message_port(&self, port_id: MessagePortId, started: bool) -> MessagePortPoll<P> {
        let mut ports = self.ports.lock();
        let Some(state) = ports.get_mut(&port_id) else {
            return MessagePortPoll::Removed;
        };
        if !started {
            return MessagePortPoll::Idle;
        }
        if let Some(message) = state.pending_messages.pop_front() {
            MessagePortPoll::Message(message)
        } else if let Some(close) = state.pending_close.take() {
            MessagePortPoll::Close(close)
        } else {
            MessagePortPoll::Idle
        }
    }

    /// Pop one pending message for a port.
    pub fn take_pending_message_port_message(&self, port_id: MessagePortId) -> Option<P> {
        self.ports
            .lock()
            .get_mut(&port_id)?
            .pending_messages
            .pop_front()
    }

    /// Whether the endpoint still has registry state.
    pub fn contains_message_port(&self, port_id: MessagePortId) -> bool {
        self.ports.lock().contains_key(&port_id)
    }

    /// Number of live endpoint records retained by the registry.
    ///
    /// Embedders use this for lifecycle diagnostics and to prove that transfer
    /// failure or an unclaimed transfer entry did not orphan either endpoint.
    pub fn endpoint_count(&self) -> usize {
        self.ports.lock().len()
    }

    /// Discard an entire entangled channel without publishing a peer close
    /// event.
    ///
    /// This is a transaction-rollback operation for internal channels that
    /// were never handed to author code. Ordinary `MessagePort.close()` must
    /// use [`Self::close_message_port`] so the peer can observe its normal
    /// close lifecycle.
    pub fn discard_message_port_channel(
        &self,
        port_id: MessagePortId,
    ) -> Vec<DiscardedMessagePort<P, O>> {
        let mut ports = self.ports.lock();
        let Some(state) = ports.remove(&port_id) else {
            return Vec::new();
        };
        let peer = state
            .peer_id
            .and_then(|peer_id| ports.remove(&peer_id).map(|state| (peer_id, state)));
        drop(ports);
        let mut discarded = vec![DiscardedMessagePort {
            port_id,
            owner: state.owner,
            discarded_messages: state.pending_messages,
        }];
        if let Some((port_id, state)) = peer {
            discarded.push(DiscardedMessagePort {
                port_id,
                owner: state.owner,
                discarded_messages: state.pending_messages,
            });
        }
        discarded
    }

    /// Take a pending close event for a port.
    pub fn take_pending_message_port_close(
        &self,
        port_id: MessagePortId,
    ) -> Option<MessagePortCloseKind> {
        let mut ports = self.ports.lock();
        ports.get_mut(&port_id)?.pending_close.take()
    }

    /// Close a port endpoint and queue a close event for its peer.
    pub fn close_message_port(&self, port_id: MessagePortId) -> MessagePortCloseResult<P, O> {
        let mut ports = self.ports.lock();
        let Some(state) = ports.remove(&port_id) else {
            return MessagePortCloseResult {
                peer_wake: None,
                discarded_messages: VecDeque::new(),
            };
        };
        let peer_wake = state.peer_id.and_then(|peer_id| {
            let peer_state = ports.get_mut(&peer_id)?;
            peer_state.peer_id = None;
            peer_state.pending_close = Some(MessagePortCloseKind::PeerClosed);
            Some(MessagePortWake {
                port_id: peer_id,
                owner: peer_state.owner.clone()?,
            })
        });
        drop(ports);
        MessagePortCloseResult {
            peer_wake,
            discarded_messages: state.pending_messages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_preserves_unstarted_queues_and_delivers_messages_before_close() {
        let registry = MessagePortRegistry::<u8, u8>::default();
        let (sender, receiver) = registry.create_entangled_message_port_pair(7);
        assert_eq!(
            registry.poll_message_port(receiver, true),
            MessagePortPoll::Idle
        );
        registry.enqueue_message_to_message_port(sender, 1).unwrap();
        registry.enqueue_message_to_message_port(sender, 2).unwrap();
        registry.close_message_port(sender);
        assert_eq!(
            registry.poll_message_port(receiver, false),
            MessagePortPoll::Idle
        );
        assert_eq!(
            registry.poll_message_port(receiver, true),
            MessagePortPoll::Message(1)
        );
        assert_eq!(
            registry.poll_message_port(receiver, true),
            MessagePortPoll::Message(2)
        );
        assert_eq!(
            registry.poll_message_port(receiver, false),
            MessagePortPoll::Idle
        );
        assert_eq!(
            registry.poll_message_port(receiver, true),
            MessagePortPoll::Close(MessagePortCloseKind::PeerClosed)
        );
        assert_eq!(
            registry.poll_message_port(receiver, true),
            MessagePortPoll::Idle
        );
        for started in [false, true] {
            assert_eq!(
                registry.poll_message_port(sender, started),
                MessagePortPoll::Removed
            );
        }
        registry.discard_message_port_channel(receiver);
        assert_eq!(
            registry.poll_message_port(receiver, false),
            MessagePortPoll::Removed
        );
        let disconnected = registry.create_disconnected_message_port(7);
        assert_eq!(
            registry.poll_message_port(disconnected, false),
            MessagePortPoll::Idle
        );
        assert_eq!(
            registry.poll_message_port(disconnected, true),
            MessagePortPoll::Close(MessagePortCloseKind::ConnectionError)
        );
    }

    #[test]
    fn disconnected_transfer_keeps_its_connection_error_through_owner_changes() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let port = registry.create_disconnected_message_port(7);
        assert_eq!(registry.message_port_peer_id(port), None);
        assert_eq!(registry.endpoint_count(), 1);
        registry.detach_message_port_owner_for_transfer(port);
        assert!(registry.wake_message_port_if_pending(port).is_none());
        let wake = registry
            .attach_message_port_owner(port, 8)
            .expect("pending connection error");
        assert_eq!((wake.port_id, wake.owner), (port, 8));
        assert_eq!(
            registry.take_pending_message_port_close(port),
            Some(MessagePortCloseKind::ConnectionError)
        );
        assert_eq!(registry.take_pending_message_port_close(port), None);
        assert!(registry.close_message_port(port).peer_wake.is_none());
        assert_eq!(registry.endpoint_count(), 0);
    }

    #[test]
    fn close_message_port_drops_closed_side_and_wakes_peer() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (closed_port, peer_port) = registry.create_entangled_message_port_pair(7);

        assert_eq!(
            registry.enqueue_message_to_message_port(peer_port, Vec::new()),
            Ok(closed_port)
        );

        let wake = registry
            .close_message_port(closed_port)
            .peer_wake
            .expect("peer should be woken");
        assert_eq!(wake.port_id, peer_port);
        assert_eq!(wake.owner, 7);

        assert!(
            registry
                .take_pending_message_port_message(closed_port)
                .is_none()
        );
        assert_eq!(
            registry.take_pending_message_port_close(peer_port),
            Some(MessagePortCloseKind::PeerClosed)
        );
        assert_eq!(registry.take_pending_message_port_close(peer_port), None);

        registry.close_message_port(peer_port);
        assert_eq!(registry.endpoint_count(), 0);
        assert!(
            registry
                .take_pending_message_port_message(peer_port)
                .is_none()
        );
    }

    #[test]
    fn discard_message_port_channel_removes_both_endpoints_without_a_close_wake() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (first, second) = registry.create_entangled_message_port_pair(7);
        assert_eq!(
            registry.enqueue_message_to_message_port(first, b"queued".to_vec()),
            Ok(second)
        );

        let removed = registry.discard_message_port_channel(second);
        assert_eq!(
            removed[0].discarded_messages,
            VecDeque::from([b"queued".to_vec()])
        );
        assert!(removed.iter().all(|port| port.owner == Some(7)));
        let mut discarded = removed
            .into_iter()
            .map(|port| port.port_id)
            .collect::<Vec<_>>();
        discarded.sort_unstable();
        let mut expected = vec![first, second];
        expected.sort_unstable();
        assert_eq!(discarded, expected);
        assert_eq!(registry.endpoint_count(), 0);
        assert!(registry.wake_message_port_if_pending(first).is_none());
        assert!(registry.wake_message_port_if_pending(second).is_none());
    }

    #[test]
    fn attach_owner_returns_wake_for_pending_message() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (source, target) = registry.create_entangled_message_port_pair(1);
        registry.detach_message_port_owner_for_transfer(target);

        assert_eq!(
            registry.enqueue_message_to_message_port(source, b"hello".to_vec()),
            Ok(target)
        );
        assert!(
            registry.wake_message_port_if_pending(target).is_none(),
            "detached ports have no owner to wake"
        );

        let wake = registry
            .attach_message_port_owner(target, 2)
            .expect("reattached owner should be woken");
        assert_eq!(wake.port_id, target);
        assert_eq!(wake.owner, 2);
        assert_eq!(
            registry.take_pending_message_port_message(target),
            Some(b"hello".to_vec())
        );
    }

    #[test]
    fn attach_owner_returns_wake_for_pending_close() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (source, target) = registry.create_entangled_message_port_pair(1);
        registry.detach_message_port_owner_for_transfer(target);

        assert!(
            registry.close_message_port(source).peer_wake.is_none(),
            "detached target has no owner to wake for close"
        );
        assert!(
            registry.wake_message_port_if_pending(target).is_none(),
            "detached target still has no owner before reattach"
        );

        let wake = registry
            .attach_message_port_owner(target, 2)
            .expect("reattached owner should be woken for close");
        assert_eq!(wake.port_id, target);
        assert_eq!(wake.owner, 2);
        assert_eq!(
            registry.take_pending_message_port_close(target),
            Some(MessagePortCloseKind::PeerClosed)
        );
        assert_eq!(registry.take_pending_message_port_close(target), None);
    }

    #[test]
    fn close_during_delivery_cancels_the_remaining_receive_queue() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (receiver, sender) = registry.create_entangled_message_port_pair(1);
        registry
            .enqueue_message_to_message_port(sender, b"first".to_vec())
            .unwrap();
        registry
            .enqueue_message_to_message_port(sender, b"second".to_vec())
            .unwrap();
        assert_eq!(
            registry.take_pending_message_port_message(receiver),
            Some(b"first".to_vec())
        );

        let closed = registry.close_message_port(receiver);
        assert_eq!(
            closed.discarded_messages,
            VecDeque::from([b"second".to_vec()])
        );
        assert!(!registry.contains_message_port(receiver));
        assert_eq!(registry.take_pending_message_port_message(receiver), None);
        assert!(
            registry
                .close_message_port(receiver)
                .discarded_messages
                .is_empty()
        );
        assert_eq!(registry.endpoint_count(), 1);
        assert_eq!(
            registry.take_pending_message_port_close(sender),
            Some(MessagePortCloseKind::PeerClosed)
        );
    }

    #[test]
    fn closing_the_sender_preserves_messages_already_accepted_by_the_receiver() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (receiver, sender) = registry.create_entangled_message_port_pair(1);
        registry
            .enqueue_message_to_message_port(sender, b"first".to_vec())
            .unwrap();
        registry
            .enqueue_message_to_message_port(sender, b"second".to_vec())
            .unwrap();
        let closed = registry.close_message_port(sender);
        assert!(closed.discarded_messages.is_empty());
        assert_eq!(
            registry.take_pending_message_port_message(receiver),
            Some(b"first".to_vec())
        );
        assert_eq!(
            registry.take_pending_message_port_message(receiver),
            Some(b"second".to_vec())
        );
        assert_eq!(
            registry.take_pending_message_port_close(receiver),
            Some(MessagePortCloseKind::PeerClosed)
        );
    }

    #[test]
    fn failed_enqueue_returns_the_payload_for_native_attachment_disposal() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (receiver, sender) = registry.create_entangled_message_port_pair(1);
        registry.close_message_port(receiver);
        assert_eq!(
            registry.enqueue_message_to_endpoint(receiver, b"captured destination".to_vec()),
            Err(b"captured destination".to_vec())
        );
        assert_eq!(
            registry.enqueue_message_to_message_port(sender, b"disconnected source".to_vec()),
            Err(b"disconnected source".to_vec())
        );
    }

    #[test]
    fn canceled_payloads_are_dropped_after_the_registry_lock_is_released() {
        use std::sync::{Arc, Weak, atomic::AtomicUsize};
        struct Payload {
            registry: Weak<MessagePortRegistry<Payload, u8>>,
            dropped: Arc<AtomicUsize>,
        }
        impl Drop for Payload {
            fn drop(&mut self) {
                let registry = self.registry.upgrade().unwrap();
                assert!(
                    registry.ports.try_lock().is_some(),
                    "payload disposal reenters the native registry"
                );
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        let registry = Arc::new(MessagePortRegistry::default());
        let dropped = Arc::new(AtomicUsize::new(0));
        let (receiver, sender) = registry.create_entangled_message_port_pair(1);
        assert!(
            registry
                .enqueue_message_to_message_port(
                    sender,
                    Payload {
                        registry: Arc::downgrade(&registry),
                        dropped: dropped.clone()
                    }
                )
                .is_ok()
        );
        let closed = registry.close_message_port(receiver);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        drop(closed);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }
}
