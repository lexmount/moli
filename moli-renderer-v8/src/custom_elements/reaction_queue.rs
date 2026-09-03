use crate::document_runtime::DomHandle;

use super::reaction_queue_storage::{ElementQueue, ElementReactionQueue};
use super::reactions::CustomElementReaction;

/// Custom element callback reactions are ordered in two dimensions.
///
/// The current element queue preserves the order in which DOM operations
/// enqueue elements. It intentionally allows duplicate handles, matching
/// Chromium's `ElementQueue`: duplicates are cheap to append, and they matter
/// when a callback re-enqueues an element after its earlier reactions have
/// already been drained.
///
/// The per-element reaction queue preserves FIFO callback order for one target
/// element. This is what lets a nested DOM operation discover that a child
/// already has a pending `connectedCallback` before it enqueues and invokes the
/// child's `disconnectedCallback`.
///
/// The map's iteration order is never observable. Observable order comes from
/// `ElementQueue`, while the map is only the lookup table from element handle to
/// that element's pending FIFO.
#[derive(Default)]
pub(crate) struct CustomElementReactionCoordinator {
    element_reactions: std::collections::HashMap<DomHandle, ElementReactionQueue>,
    stack: Vec<ElementQueue>,
    backup_queue: ElementQueue,
    backup_queue_flush_scheduled: bool,
}

impl CustomElementReactionCoordinator {
    pub(super) fn push_element_queue(&mut self) {
        self.stack.push(ElementQueue::default());
    }

    pub(super) fn pop_element_queue(&mut self) {
        self.stack.pop();
    }

    pub(super) fn enqueue_reaction(
        &mut self,
        handle: DomHandle,
        reaction: CustomElementReaction,
    ) -> bool {
        let needs_backup_microtask = self.stack.is_empty() && !self.backup_queue_flush_scheduled;
        let queue = if let Some(queue) = self.stack.last_mut() {
            queue
        } else {
            &mut self.backup_queue
        };
        queue.push(handle);
        self.element_reactions
            .entry(handle)
            .or_default()
            .push(reaction);
        needs_backup_microtask
    }

    pub(super) fn pending_reactions_end_with(
        &self,
        handle: DomHandle,
        reaction: &CustomElementReaction,
    ) -> bool {
        self.element_reactions
            .get(&handle)
            .is_some_and(|queue| queue.pending_reactions_end_with(reaction))
    }

    pub(super) fn pending_reactions_contain(
        &self,
        handle: DomHandle,
        reaction: &CustomElementReaction,
    ) -> bool {
        self.element_reactions
            .get(&handle)
            .is_some_and(|queue| queue.pending_reactions_contain(reaction))
    }

    pub(super) fn next_current_element(&mut self) -> Option<DomHandle> {
        self.stack.last_mut()?.next()
    }

    pub(super) fn current_element_queue_is_empty(&self) -> bool {
        self.stack.last().is_some_and(ElementQueue::is_empty)
    }

    pub(super) fn next_backup_element(&mut self) -> Option<DomHandle> {
        self.backup_queue.next()
    }

    pub(super) fn mark_backup_queue_flush_scheduled(&mut self) {
        self.backup_queue_flush_scheduled = true;
    }

    pub(super) fn finish_backup_queue_flush(&mut self) {
        self.backup_queue.clear();
        self.backup_queue_flush_scheduled = false;
    }

    pub(super) fn next_reaction(&mut self, handle: DomHandle) -> Option<CustomElementReaction> {
        self.element_reactions.get_mut(&handle)?.next()
    }

    pub(super) fn remove_reaction_queue_if_drained(&mut self, handle: DomHandle) {
        if self
            .element_reactions
            .get(&handle)
            .is_some_and(ElementReactionQueue::is_drained)
        {
            self.element_reactions.remove(&handle);
        }
    }

    pub(super) fn clear_reactions(&mut self, handle: DomHandle) {
        self.element_reactions.remove(&handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(index: usize) -> DomHandle {
        DomHandle::new(index)
    }

    #[test]
    fn backup_queue_requests_one_microtask_until_flush_finishes() {
        let mut reactions = CustomElementReactionCoordinator::default();

        assert!(reactions.enqueue_reaction(handle(1), CustomElementReaction::Connected));
        reactions.mark_backup_queue_flush_scheduled();
        assert!(!reactions.enqueue_reaction(handle(2), CustomElementReaction::Disconnected));

        assert_eq!(reactions.next_backup_element(), Some(handle(1)));
        assert_eq!(
            reactions.next_reaction(handle(1)),
            Some(CustomElementReaction::Connected)
        );
        assert_eq!(reactions.next_backup_element(), Some(handle(2)));
        assert_eq!(
            reactions.next_reaction(handle(2)),
            Some(CustomElementReaction::Disconnected)
        );
        assert_eq!(reactions.next_backup_element(), None);

        reactions.finish_backup_queue_flush();
        assert!(reactions.enqueue_reaction(handle(3), CustomElementReaction::Connected));
    }

    #[test]
    fn backup_queue_flush_sees_reactions_appended_while_draining() {
        let mut reactions = CustomElementReactionCoordinator::default();

        assert!(reactions.enqueue_reaction(handle(1), CustomElementReaction::Connected));
        reactions.mark_backup_queue_flush_scheduled();
        assert_eq!(reactions.next_backup_element(), Some(handle(1)));

        assert!(!reactions.enqueue_reaction(handle(2), CustomElementReaction::Disconnected));
        assert_eq!(reactions.next_backup_element(), Some(handle(2)));
        assert_eq!(
            reactions.next_reaction(handle(2)),
            Some(CustomElementReaction::Disconnected)
        );
    }

    #[test]
    fn current_reaction_queue_does_not_request_backup_microtask() {
        let mut reactions = CustomElementReactionCoordinator::default();

        reactions.push_element_queue();
        assert!(!reactions.enqueue_reaction(handle(1), CustomElementReaction::Connected));
        assert_eq!(reactions.next_current_element(), Some(handle(1)));
        reactions.pop_element_queue();
    }
}
