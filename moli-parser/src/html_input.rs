//! Tokenizer-owned HTML input and parser insertion frames.
//!
//! `html5ever` consumes one [`BufferQueue`] at a time, but `document.write()`
//! can insert a new source in front of input that the tokenizer has not
//! consumed yet. That unconsumed tail must not be discarded or appended to the
//! inserted source. It becomes the parent frame and is restored after the
//! inserted frame is empty:
//!
//! ```text
//! before insertion: [ current tail ]
//! during insertion: [ inserted input ] [ saved current tail ]
//! nested insertion: [ inner input ] [ outer input ] [ original tail ]
//! ```
//!
//! This module owns only input that has already been admitted to the tokenizer.
//! It does not decide whether an arriving chunk is parser-inserted or belongs
//! at the end of the document stream, and it does not decide whether an empty
//! parser should finish or wait for more input. Those decisions require runtime
//! provenance and parser lifetime, respectively, and remain with their owners.
//! The frames are explicit rather than represented by a stack-scoped guard
//! because parser execution can yield for an external script and resume on a
//! later runtime turn.

use html5ever::{tendril::StrTendril, tokenizer::BufferQueue};

/// The tokenizer's current input plus suspended parent insertion frames.
///
/// `parents` is stored from outermost to innermost. Consequently, pending input
/// is consumed from `current`, followed by `parents` in reverse order.
#[derive(Default)]
pub(super) struct InputStack {
    current: BufferQueue,
    parents: Vec<BufferQueue>,
}

impl InputStack {
    /// Append ordinary input to the tokenizer's current queue.
    pub(super) fn push_back(&mut self, input: StrTendril) {
        self.current.push_back(input);
    }

    /// Start a parser-inserted frame before the current unconsumed input.
    ///
    /// Even an empty current queue is saved. Its presence records that there is
    /// an active insertion point to which a later write may belong.
    pub(super) fn begin_inserted(&mut self, input: StrTendril) {
        if input.is_empty() {
            return;
        }
        let parent = std::mem::take(&mut self.current);
        self.parents.push(parent);
        self.current.push_back(input);
    }

    /// Append after the unconsumed tail of the active inserted frame.
    ///
    /// This is different from starting another inserted frame: a nested frame
    /// would run before the current tail, while this input must run after it.
    /// Empty input is accepted as a no-op; non-empty input returns `false` when
    /// no parser insertion frame is active.
    pub(super) fn append_to_current_inserted(&mut self, input: StrTendril) -> bool {
        if input.is_empty() {
            return true;
        }
        if self.parents.is_empty() {
            return false;
        }
        self.current.push_back(input);
        true
    }

    /// Return the queue that `html5ever` may consume in the current parser step.
    pub(super) fn current(&self) -> &BufferQueue {
        &self.current
    }

    /// Restore one parent after the current frame has been fully consumed.
    ///
    /// At most one frame is restored per tokenizer boundary. The caller feeds
    /// the restored frame on its next step, preserving the same incremental
    /// parser behavior for every insertion depth.
    pub(super) fn restore_parent_if_current_empty(&mut self) -> bool {
        if !self.current.is_empty() {
            return false;
        }
        let Some(parent) = self.parents.pop() else {
            return false;
        };
        self.current = parent;
        true
    }

    /// Return whether any frame still contains characters to tokenize.
    ///
    /// This deliberately says nothing about whether an insertion point exists:
    /// an active frame and all of its parents may legitimately be empty.
    pub(super) fn has_input(&self) -> bool {
        self.queues_in_consumption_order()
            .any(|input| !input.is_empty())
    }

    pub(super) fn len(&self) -> usize {
        self.queues_in_consumption_order()
            .fold(0usize, |total, input| {
                total.saturating_add(queue_len(input))
            })
    }

    /// Snapshot pending characters in the order the tokenizer will see them.
    pub(super) fn snapshot(&self) -> String {
        let mut pending = String::new();
        for input in self.queues_in_consumption_order() {
            append_queue_snapshot(input, &mut pending);
        }
        pending
    }

    /// Collapse every insertion frame into one queue for parser finalization.
    ///
    /// The innermost current frame stays first, followed by each parent from
    /// nearest to farthest. No input is reparsed or otherwise transformed.
    pub(super) fn into_buffer(mut self) -> BufferQueue {
        while let Some(parent) = self.parents.pop() {
            append_queue(parent, &self.current);
        }
        self.current
    }

    fn queues_in_consumption_order(&self) -> impl Iterator<Item = &BufferQueue> {
        std::iter::once(&self.current).chain(self.parents.iter().rev())
    }
}

fn queue_len(input: &BufferQueue) -> usize {
    let input = input.clone();
    let mut len = 0usize;
    while let Some(chunk) = input.pop_front() {
        len = len.saturating_add(chunk.len());
    }
    len
}

fn append_queue_snapshot(input: &BufferQueue, snapshot: &mut String) {
    let input = input.clone();
    while let Some(chunk) = input.pop_front() {
        snapshot.push_str(&chunk);
    }
}

fn append_queue(source: BufferQueue, destination: &BufferQueue) {
    while let Some(chunk) = source.pop_front() {
        destination.push_back(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tendril(input: &str) -> StrTendril {
        StrTendril::from(input)
    }

    #[test]
    fn nested_frames_are_snapshotted_and_flattened_in_consumption_order() {
        let mut input = InputStack::default();
        input.push_back(tendril("e")); // Original parser tail.
        input.begin_inserted(tendril("r")); // Tail behind a nested external script.
        assert!(input.append_to_current_inserted(tendril("k"))); // Same inserted frame.
        input.begin_inserted(tendril("wo")); // Writes made by that external script.

        assert_eq!(input.snapshot(), "worke");
        assert_eq!(input.len(), 5);

        let flattened = input.into_buffer();
        let mut snapshot = String::new();
        append_queue_snapshot(&flattened, &mut snapshot);
        assert_eq!(snapshot, "worke");
    }

    #[test]
    fn an_empty_boundary_restores_exactly_one_parent() {
        let mut input = InputStack::default();
        input.push_back(tendril("outer"));
        input.begin_inserted(tendril("middle"));
        input.begin_inserted(tendril("inner"));

        while input.current.pop_front().is_some() {}
        assert!(input.restore_parent_if_current_empty());
        assert_eq!(input.snapshot(), "middleouter");
        assert!(!input.restore_parent_if_current_empty());
    }

    #[test]
    fn appending_to_an_inserted_frame_requires_an_active_insertion_point() {
        let mut input = InputStack::default();
        assert!(!input.append_to_current_inserted(tendril("outside")));

        input.begin_inserted(tendril("inside"));
        assert!(input.append_to_current_inserted(tendril(" tail")));
        assert_eq!(input.snapshot(), "inside tail");
    }
}
