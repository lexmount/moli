use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;

use crate::{MessagePortId, MessagePortWake};

#[derive(Debug, Default)]
struct MessagePortState<P, O> {
    peer_id: Option<MessagePortId>,
    owner: Option<O>,
    pending_messages: VecDeque<P>,
    pending_close: bool,
    close_after_delivery: bool,
    active_delivery_depth: usize,
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
                pending_close: false,
                close_after_delivery: false,
                active_delivery_depth: 0,
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
                pending_close: false,
                close_after_delivery: false,
                active_delivery_depth: 0,
            },
        );
        ports.insert(
            port2,
            MessagePortState {
                peer_id: Some(port1),
                owner: Some(owner),
                pending_messages: VecDeque::new(),
                pending_close: false,
                close_after_delivery: false,
                active_delivery_depth: 0,
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
        (state.pending_close || !state.pending_messages.is_empty())
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
        if !state.pending_close && state.pending_messages.is_empty() {
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
    pub fn enqueue_message_to_endpoint(&self, port_id: MessagePortId, payload: P) -> bool {
        let mut ports = self.ports.lock();
        let Some(state) = ports.get_mut(&port_id) else {
            return false;
        };
        state.pending_messages.push_back(payload);
        true
    }

    /// Enqueue a message to the peer endpoint and return the target peer id.
    pub fn enqueue_message_to_message_port(
        &self,
        port_id: MessagePortId,
        payload: P,
    ) -> Option<MessagePortId> {
        let mut ports = self.ports.lock();
        let peer_id = ports.get(&port_id).and_then(|state| state.peer_id)?;
        let peer_state = ports.get_mut(&peer_id)?;
        peer_state.pending_messages.push_back(payload);
        Some(peer_id)
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
    ///
    /// A close performed from inside a message callback retains the closed
    /// endpoint until every message accepted before that callback has been
    /// delivered. Embedders use this to keep the detached wrapper alive for
    /// those already-queued delivery tasks without keeping ordinary closed
    /// wrappers alive indefinitely.
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
    pub fn discard_message_port_channel(&self, port_id: MessagePortId) -> Vec<MessagePortId> {
        let mut ports = self.ports.lock();
        let Some(state) = ports.remove(&port_id) else {
            return Vec::new();
        };
        let mut discarded = vec![port_id];
        if let Some(peer_id) = state.peer_id
            && ports.remove(&peer_id).is_some()
        {
            discarded.push(peer_id);
        }
        discarded
    }

    /// Mark the start of message callback delivery for close cleanup ordering.
    pub fn begin_message_port_message_delivery(&self, port_id: MessagePortId) {
        if let Some(state) = self.ports.lock().get_mut(&port_id) {
            state.active_delivery_depth = state.active_delivery_depth.saturating_add(1);
        }
    }

    /// Mark the end of message callback delivery and remove deferred closed
    /// endpoints when they no longer have queued work.
    pub fn finish_message_port_message_delivery(&self, port_id: MessagePortId) {
        let mut ports = self.ports.lock();
        let remove = {
            let Some(state) = ports.get_mut(&port_id) else {
                return;
            };
            state.active_delivery_depth = state.active_delivery_depth.saturating_sub(1);
            state.close_after_delivery
                && state.active_delivery_depth == 0
                && state.pending_messages.is_empty()
                && !state.pending_close
        };
        if remove {
            ports.remove(&port_id);
        }
    }

    /// Take a pending close event for a port.
    pub fn take_pending_message_port_close(&self, port_id: MessagePortId) -> bool {
        let mut ports = self.ports.lock();
        let Some(state) = ports.get_mut(&port_id) else {
            return false;
        };
        if !state.pending_close {
            return false;
        }
        state.pending_close = false;
        true
    }

    /// Close a port endpoint and queue a close event for its peer.
    pub fn close_message_port(&self, port_id: MessagePortId) -> Option<MessagePortWake<O>> {
        let mut ports = self.ports.lock();
        let (peer_id, remove_closed_side) = {
            let state = ports.get_mut(&port_id)?;
            state.owner = None;
            let peer_id = state.peer_id.take();
            let remove_closed_side =
                state.active_delivery_depth == 0 || state.pending_messages.is_empty();
            if !remove_closed_side {
                state.close_after_delivery = true;
            }
            (peer_id, remove_closed_side)
        };
        if remove_closed_side {
            ports.remove(&port_id);
        }
        let peer_id = peer_id?;
        let peer_state = ports.get_mut(&peer_id)?;
        peer_state.peer_id = None;
        peer_state.pending_close = true;
        Some(MessagePortWake {
            port_id: peer_id,
            owner: peer_state.owner.clone()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_message_port_drops_closed_side_and_wakes_peer() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (closed_port, peer_port) = registry.create_entangled_message_port_pair(7);

        assert_eq!(
            registry.enqueue_message_to_message_port(peer_port, Vec::new()),
            Some(closed_port)
        );

        let wake = registry
            .close_message_port(closed_port)
            .expect("peer should be woken");
        assert_eq!(wake.port_id, peer_port);
        assert_eq!(wake.owner, 7);

        assert!(
            registry
                .take_pending_message_port_message(closed_port)
                .is_none()
        );
        assert!(registry.take_pending_message_port_close(peer_port));
        assert!(!registry.take_pending_message_port_close(peer_port));

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
            Some(second)
        );

        let mut discarded = registry.discard_message_port_channel(second);
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
            Some(target)
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
            registry.close_message_port(source).is_none(),
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
        assert!(registry.take_pending_message_port_close(target));
        assert!(!registry.take_pending_message_port_close(target));
    }

    #[test]
    fn close_during_delivery_retains_endpoint_until_queued_messages_are_consumed() {
        let registry = MessagePortRegistry::<Vec<u8>, u8>::default();
        let (closed_port, peer_port) = registry.create_entangled_message_port_pair(1);
        registry.enqueue_message_to_message_port(peer_port, b"first".to_vec());
        registry.enqueue_message_to_message_port(peer_port, b"second".to_vec());

        assert_eq!(
            registry.take_pending_message_port_message(closed_port),
            Some(b"first".to_vec())
        );
        registry.begin_message_port_message_delivery(closed_port);
        registry.close_message_port(closed_port);
        registry.finish_message_port_message_delivery(closed_port);

        assert!(registry.contains_message_port(closed_port));
        assert_eq!(
            registry.take_pending_message_port_message(closed_port),
            Some(b"second".to_vec())
        );
        registry.begin_message_port_message_delivery(closed_port);
        registry.finish_message_port_message_delivery(closed_port);
        assert!(!registry.contains_message_port(closed_port));
    }
}
