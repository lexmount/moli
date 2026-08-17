use crate::document_runtime::DomHandle;

use super::JsContextHost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingScrollObservableEffects {
    document: Option<DomHandle>,
    queue_document_events: bool,
}

impl PendingScrollObservableEffects {
    pub(crate) const fn new(document: Option<DomHandle>, queue_document_events: bool) -> Self {
        Self {
            document,
            queue_document_events,
        }
    }

    pub(crate) const fn document(self) -> Option<DomHandle> {
        self.document
    }

    pub(crate) const fn queue_document_events(self) -> bool {
        self.queue_document_events
    }
}

#[derive(Debug, Default)]
pub(super) struct ScrollObservableEffectBatchState {
    depth: usize,
    pending: Vec<PendingScrollObservableEffects>,
}

impl ScrollObservableEffectBatchState {
    fn begin(&mut self) {
        self.depth = self
            .depth
            .checked_add(1)
            .expect("scroll observable-effect batch depth exhausted");
    }

    fn defer(&mut self, document: Option<DomHandle>, queue_document_events: bool) -> bool {
        if self.depth == 0 {
            return false;
        }
        if let Some(existing) = self
            .pending
            .iter_mut()
            .find(|effects| effects.document == document)
        {
            existing.queue_document_events |= queue_document_events;
        } else {
            self.pending.push(PendingScrollObservableEffects {
                document,
                queue_document_events,
            });
        }
        true
    }

    fn finish(&mut self) -> Option<Vec<PendingScrollObservableEffects>> {
        assert!(
            self.depth != 0,
            "scroll observable-effect batch finished without a matching begin"
        );
        self.depth -= 1;
        (self.depth == 0).then(|| std::mem::take(&mut self.pending))
    }
}

impl JsContextHost {
    pub(crate) fn begin_scroll_observable_effect_batch(&mut self) {
        self.scroll_observable_effect_batch.begin();
    }

    /// Retains a scroll's derived work while an input batch is active.
    /// Returns `true` when the caller must not perform that work immediately.
    pub(crate) fn defer_scroll_observable_effects(
        &mut self,
        document: Option<DomHandle>,
        queue_document_events: bool,
    ) -> bool {
        self.scroll_observable_effect_batch
            .defer(document, queue_document_events)
    }

    /// Closes one batch level. The outermost close owns the accumulated
    /// effects; nested closes leave them with their caller.
    pub(crate) fn finish_scroll_observable_effect_batch(
        &mut self,
    ) -> Option<Vec<PendingScrollObservableEffects>> {
        self.scroll_observable_effect_batch.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(value: usize) -> DomHandle {
        DomHandle::new(value)
    }

    #[test]
    fn effects_are_not_deferred_while_idle() {
        let mut state = ScrollObservableEffectBatchState::default();

        assert!(!state.defer(Some(handle(1)), true));
        assert_eq!(state.depth, 0);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn repeated_document_effects_merge_and_preserve_first_seen_order() {
        let mut state = ScrollObservableEffectBatchState::default();
        let first = handle(1);
        let second = handle(2);
        state.begin();

        assert!(state.defer(Some(first), false));
        assert!(state.defer(Some(second), true));
        assert!(state.defer(Some(first), true));

        let pending = state.finish().expect("outer batch should own effects");
        assert_eq!(
            pending,
            vec![
                PendingScrollObservableEffects::new(Some(first), true),
                PendingScrollObservableEffects::new(Some(second), true),
            ]
        );
    }

    #[test]
    fn nested_batches_publish_only_at_the_outer_boundary() {
        let mut state = ScrollObservableEffectBatchState::default();
        state.begin();
        state.begin();
        assert!(state.defer(None, false));

        assert!(state.finish().is_none());
        assert_eq!(
            state.finish(),
            Some(vec![PendingScrollObservableEffects::new(None, false)])
        );
        assert_eq!(state.depth, 0);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_completed_batch_does_not_leak_effects_into_the_next_batch() {
        let mut state = ScrollObservableEffectBatchState::default();
        state.begin();
        assert!(state.defer(Some(handle(1)), false));
        assert_eq!(state.finish().expect("first batch").len(), 1);

        state.begin();
        assert_eq!(state.finish(), Some(Vec::new()));
    }

    #[test]
    #[should_panic(expected = "without a matching begin")]
    fn finishing_an_idle_batch_is_rejected() {
        let mut state = ScrollObservableEffectBatchState::default();
        let _ = state.finish();
    }
}
