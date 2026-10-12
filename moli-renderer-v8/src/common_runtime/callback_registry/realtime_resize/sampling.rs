use super::*;
use crate::dom::native::{DomHost, DomMutationEffects};

/// Owns only the native source/retirement index for a single broadcast batch.
/// No Host borrow or V8 value survives through this guard into author code.
pub(crate) struct RealtimeResizeObserverSampling {
    samples: Rc<RefCell<Option<HashMap<DomHandle, bool>>>>,
}

impl Drop for RealtimeResizeObserverSampling {
    fn drop(&mut self) {
        self.samples.borrow_mut().take();
    }
}

impl CallbackRegistry {
    fn realtime_resize_sampling(&self) -> Option<Rc<RefCell<Option<HashMap<DomHandle, bool>>>>> {
        self.state
            .borrow()
            .realtime_resize
            .as_ref()
            .map(|resize| Rc::clone(&resize.sampling))
    }

    pub(crate) fn begin_realtime_resize_observer_sampling(
        &self,
        sources: Vec<DomHandle>,
    ) -> Result<RealtimeResizeObserverSampling, moli_layout::LayoutError> {
        let samples = self
            .realtime_resize_sampling()
            .expect("active Realtime RO batch has a registry");
        {
            let mut current = samples.borrow_mut();
            if current.is_some() {
                return Err(moli_layout::LayoutError::source_contract(
                    "ResizeObserver geometry",
                    "a broadcast batch cannot reenter another sampling batch",
                ));
            }
            *current = Some(sources.into_iter().map(|source| (source, false)).collect());
        }
        Ok(RealtimeResizeObserverSampling { samples })
    }

    pub(crate) fn realtime_resize_observer_box_was_removed(&self, source: DomHandle) -> bool {
        let Some(samples) = self.realtime_resize_sampling() else {
            return false;
        };
        samples
            .borrow()
            .as_ref()
            .and_then(|sources| sources.get(&source))
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn discard_realtime_resize_observer_tree_boxes(
        &self,
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) {
        let Some(samples) = self.realtime_resize_sampling() else {
            return;
        };
        let mut current = samples.borrow_mut();
        let Some(sources) = current.as_mut() else {
            return;
        };
        // Child-list effects include ordinary remove-and-reinsert operations
        // whose intermediate disconnection notification was coalesced away.
        // The mutation owner excludes state-preserving moveBefore operations.
        for removed in effects
            .style()
            .child_list_mutations()
            .iter()
            .flat_map(|mutation| mutation.removed_nodes())
        {
            for (&source, discarded) in sources.iter_mut() {
                *discarded |= dom
                    .dom()
                    .is_host_including_inclusive_ancestor(*removed, source);
            }
        }
    }

    pub(crate) fn discard_realtime_resize_observer_document_boxes(
        &self,
        dom: &DomHost,
        document: DomHandle,
    ) {
        let Some(samples) = self.realtime_resize_sampling() else {
            return;
        };
        let mut current = samples.borrow_mut();
        let Some(sources) = current.as_mut() else {
            return;
        };
        for (&source, discarded) in sources.iter_mut() {
            *discarded |= dom.owner_document_handle(source) == Some(document);
        }
    }

    pub(crate) fn did_publish_realtime_resize_observer_layout(&self) {
        let Some(samples) = self.realtime_resize_sampling() else {
            return;
        };
        if let Some(sources) = samples.borrow_mut().as_mut() {
            // A successful canonical pass reestablishes the box sample for
            // every participating source. Temporary capture never calls this.
            sources
                .values_mut()
                .for_each(|discarded| *discarded = false);
        }
    }
}
