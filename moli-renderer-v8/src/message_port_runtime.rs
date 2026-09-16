use std::sync::Arc;

use moli_message_port::{MessagePortRegistry as CoreMessagePortRegistry, MessagePortWake};
use tokio::sync::mpsc;

use crate::{
    page_task_queue::RendererPageMessagePortDeliveryProducer,
    structured_clone::V8StructuredClonePayload, types::MessagePortId, worker::WorkerMessage,
};

#[derive(Clone, Debug)]
pub(crate) enum MessagePortOwner {
    Page(RendererPageMessagePortDeliveryProducer),
    Worker(mpsc::UnboundedSender<WorkerMessage>),
}

type CoreRegistry = CoreMessagePortRegistry<V8StructuredClonePayload, MessagePortOwner>;

#[derive(Debug, Default)]
pub(crate) struct RendererMessagePortRegistry {
    inner: CoreRegistry,
}

pub(crate) type SharedMessagePortRegistry = Arc<RendererMessagePortRegistry>;

pub(crate) fn new_message_port_registry() -> SharedMessagePortRegistry {
    Arc::new(RendererMessagePortRegistry::default())
}

fn wake_owner(owner: MessagePortOwner, port_id: MessagePortId) -> bool {
    match owner {
        MessagePortOwner::Page(producer) => producer.send(port_id).is_ok(),
        MessagePortOwner::Worker(sender) => {
            sender.send(WorkerMessage::MessagePortWake(port_id)).is_ok()
        }
    }
}

impl RendererMessagePortRegistry {
    fn wake_port_if_owned(&self, wake: Option<MessagePortWake<MessagePortOwner>>) {
        let Some(wake) = wake else {
            return;
        };
        if wake_owner(wake.owner, wake.port_id) {
            return;
        }
        self.inner.clear_message_port_owner(wake.port_id);
    }

    pub(crate) fn create_entangled_message_port_pair(
        &self,
        owner: MessagePortOwner,
    ) -> (MessagePortId, MessagePortId) {
        self.inner.create_entangled_message_port_pair(owner)
    }

    pub(crate) fn attach_message_port_owner(
        &self,
        port_id: MessagePortId,
        owner: MessagePortOwner,
    ) {
        // Materializing a transferred MessagePort only binds its new owner.
        // Like Blink's Entangle(), it does not enable the port's message
        // queue. `start()` or installing `onmessage` publishes the first task
        // for payloads retained during transfer.
        let _ = self.inner.attach_message_port_owner(port_id, owner);
    }

    pub(crate) fn detach_message_port_owner_for_transfer(&self, port_id: MessagePortId) {
        self.inner.detach_message_port_owner_for_transfer(port_id);
    }

    pub(crate) fn wake_message_port_if_pending(&self, port_id: MessagePortId) {
        self.wake_port_if_owned(self.inner.wake_message_port_if_pending(port_id));
    }

    pub(crate) fn enqueue_message_to_message_port(
        &self,
        port_id: MessagePortId,
        payload: V8StructuredClonePayload,
    ) -> Option<MessagePortId> {
        self.inner.enqueue_message_to_message_port(port_id, payload)
    }

    pub(crate) fn message_port_peer_id(&self, port_id: MessagePortId) -> Option<MessagePortId> {
        self.inner.message_port_peer_id(port_id)
    }

    pub(crate) fn enqueue_message_to_endpoint(
        &self,
        port_id: MessagePortId,
        payload: V8StructuredClonePayload,
    ) -> bool {
        self.inner.enqueue_message_to_endpoint(port_id, payload)
    }

    pub(crate) fn take_pending_message_port_message(
        &self,
        port_id: MessagePortId,
    ) -> Option<V8StructuredClonePayload> {
        self.inner.take_pending_message_port_message(port_id)
    }

    pub(crate) fn contains_message_port(&self, port_id: MessagePortId) -> bool {
        self.inner.contains_message_port(port_id)
    }

    pub(crate) fn discard_message_port_channel(
        &self,
        port_id: MessagePortId,
    ) -> Vec<MessagePortId> {
        self.inner.discard_message_port_channel(port_id)
    }

    #[cfg(test)]
    pub(crate) fn endpoint_count(&self) -> usize {
        self.inner.endpoint_count()
    }

    pub(crate) fn begin_message_port_message_delivery(&self, port_id: MessagePortId) {
        self.inner.begin_message_port_message_delivery(port_id);
    }

    pub(crate) fn finish_message_port_message_delivery(&self, port_id: MessagePortId) {
        self.inner.finish_message_port_message_delivery(port_id);
    }

    pub(crate) fn take_pending_message_port_close(&self, port_id: MessagePortId) -> bool {
        self.inner.take_pending_message_port_close(port_id)
    }

    pub(crate) fn close_message_port(&self, port_id: MessagePortId) {
        self.wake_port_if_owned(self.inner.close_message_port(port_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_owner() -> (MessagePortOwner, mpsc::UnboundedReceiver<WorkerMessage>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (MessagePortOwner::Worker(sender), receiver)
    }

    fn take_message_port_wake(
        receiver: &mut mpsc::UnboundedReceiver<WorkerMessage>,
    ) -> MessagePortId {
        match receiver.try_recv() {
            Ok(WorkerMessage::MessagePortWake(port_id)) => port_id,
            Ok(other) => panic!("expected MessagePort wake, got {other:?}"),
            Err(error) => panic!("expected MessagePort wake, got {error:?}"),
        }
    }

    #[test]
    fn close_message_port_drops_closed_side_and_wakes_peer() {
        let (owner, mut wake_rx) = worker_owner();
        let registry = new_message_port_registry();
        let (closed_port, peer_port) = registry.create_entangled_message_port_pair(owner);

        assert_eq!(
            registry
                .enqueue_message_to_message_port(peer_port, V8StructuredClonePayload::default()),
            Some(closed_port)
        );

        registry.close_message_port(closed_port);

        assert!(
            registry
                .take_pending_message_port_message(closed_port)
                .is_none()
        );
        assert!(registry.take_pending_message_port_close(peer_port));
        assert!(!registry.take_pending_message_port_close(peer_port));

        assert_eq!(take_message_port_wake(&mut wake_rx), peer_port);

        registry.close_message_port(peer_port);
        assert!(
            registry
                .take_pending_message_port_message(peer_port)
                .is_none()
        );
    }

    #[test]
    fn separate_registries_do_not_share_port_state() {
        let (left_owner, mut left_wake_rx) = worker_owner();
        let (right_owner, mut right_wake_rx) = worker_owner();
        let left = new_message_port_registry();
        let right = new_message_port_registry();
        let (left_source, left_target) = left.create_entangled_message_port_pair(left_owner);
        let (right_source, right_target) = right.create_entangled_message_port_pair(right_owner);

        assert_eq!(left_source, right_source);
        assert_eq!(left_target, right_target);

        left.enqueue_message_to_message_port(left_source, V8StructuredClonePayload::default());
        left.wake_message_port_if_pending(left_target);

        assert!(
            left.take_pending_message_port_message(left_target)
                .is_some()
        );
        assert!(
            right
                .take_pending_message_port_message(right_target)
                .is_none()
        );
        assert_eq!(take_message_port_wake(&mut left_wake_rx), left_target);
        assert!(matches!(
            right_wake_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn transferred_message_waits_for_activation_after_owner_reattach() {
        let (stale_owner, stale_wake_rx) = worker_owner();
        let registry = new_message_port_registry();
        let (source, target) = registry.create_entangled_message_port_pair(stale_owner);
        drop(stale_wake_rx);

        assert_eq!(
            registry.enqueue_message_to_message_port(source, V8StructuredClonePayload::default()),
            Some(target)
        );
        registry.wake_message_port_if_pending(target);

        let (fresh_owner, mut fresh_wake_rx) = worker_owner();
        registry.attach_message_port_owner(target, fresh_owner);
        assert!(matches!(
            fresh_wake_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        registry.wake_message_port_if_pending(target);
        assert_eq!(take_message_port_wake(&mut fresh_wake_rx), target);
        assert!(
            registry.take_pending_message_port_message(target).is_some(),
            "pending message should survive stale owner wake failure"
        );
    }
}
