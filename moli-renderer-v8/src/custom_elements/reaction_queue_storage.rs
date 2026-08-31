use crate::document_runtime::DomHandle;

use super::reactions::CustomElementReaction;

#[derive(Default)]
pub(super) struct ElementQueue {
    handles: Vec<DomHandle>,
    /// Current delivery position.
    ///
    /// The queue is not drained by removing from the front. Delivery advances
    /// by index so callbacks can append more handles to the same current queue
    /// and the flush loop can still observe them through the growing vector
    /// length, matching Chromium's `for (i < queue.size())` behavior.
    index: usize,
}

impl ElementQueue {
    pub(super) fn push(&mut self, handle: DomHandle) {
        self.handles.push(handle);
    }

    pub(super) fn next(&mut self) -> Option<DomHandle> {
        let handle = self.handles.get(self.index).copied()?;
        self.index += 1;
        Some(handle)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.index >= self.handles.len()
    }

    pub(super) fn clear(&mut self) {
        self.handles.clear();
        self.index = 0;
    }
}

#[derive(Default)]
pub(super) struct ElementReactionQueue {
    /// FIFO reactions for a single element.
    ///
    /// Chromium stores reactions in a vector and advances an index during
    /// invocation. `Option` lets Moli move each reaction out before
    /// calling into V8 while keeping the vector stable for recursive appends.
    reactions: Vec<Option<CustomElementReaction>>,
    index: usize,
}

impl ElementReactionQueue {
    pub(super) fn push(&mut self, reaction: CustomElementReaction) {
        self.reactions.push(Some(reaction));
    }

    pub(super) fn next(&mut self) -> Option<CustomElementReaction> {
        while self.index < self.reactions.len() {
            let reaction = self.reactions[self.index].take();
            self.index += 1;
            if reaction.is_some() {
                return reaction;
            }
        }
        None
    }

    pub(super) fn is_drained(&self) -> bool {
        self.index >= self.reactions.len()
    }

    pub(super) fn pending_reactions_end_with(&self, reaction: &CustomElementReaction) -> bool {
        self.reactions
            .iter()
            .skip(self.index)
            .rev()
            .find_map(Option::as_ref)
            == Some(reaction)
    }

    pub(super) fn pending_reactions_contain(&self, reaction: &CustomElementReaction) -> bool {
        self.reactions
            .iter()
            .skip(self.index)
            .flatten()
            .any(|pending| pending == reaction)
    }
}
