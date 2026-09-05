use crate::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleSnapshot,
};

/// Authoritative lifecycle progress for one renderer Document.
///
/// A restart may advance the epoch of this Document, but events cannot replace
/// its identity. A replacement Document starts a new lifecycle owner instead.
#[derive(Debug, Default)]
pub struct DocumentLifecycle {
    snapshot: Option<RendererDocumentLifecycleSnapshot>,
    last_sequence: Option<u64>,
}

impl DocumentLifecycle {
    /// Seeds a lifecycle before replaying its creation-event prefix.
    pub fn from_snapshot(snapshot: RendererDocumentLifecycleSnapshot) -> Self {
        Self {
            snapshot: Some(snapshot),
            last_sequence: None,
        }
    }

    pub fn snapshot(&self) -> Option<RendererDocumentLifecycleSnapshot> {
        self.snapshot
    }

    /// Accepts an exact, ordered event without consulting any frontend binding.
    pub fn observe(&mut self, event: RendererDocumentLifecycleEvent) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else {
            return false;
        };
        if event.frame != snapshot.frame
            || event.document != snapshot.document
            || self
                .last_sequence
                .is_some_and(|sequence| event.sequence <= sequence)
        {
            return false;
        }
        let restarts = event.epoch.0 > snapshot.epoch.0
            && matches!(
                event.kind,
                RendererDocumentLifecycleEventKind::Started { .. }
            )
            && snapshot.terminated.is_some();
        if event.epoch != snapshot.epoch && !restarts {
            return false;
        }
        snapshot.apply_event(event);
        self.last_sequence = Some(event.sequence);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{
        RendererDocumentLifecycleMilestone, RendererDocumentTerminationReason,
        RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch,
        RendererLifecycleEventStamp, RendererLifecycleStartReason,
    };

    fn event(
        sequence: u64,
        epoch: u64,
        kind: RendererDocumentLifecycleEventKind,
    ) -> RendererDocumentLifecycleEvent {
        let page_id = crate::PageId::new_for_testing(7);
        RendererDocumentLifecycleEvent {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, 1),
            epoch: RendererLifecycleEpoch(epoch),
            sequence,
            timestamp_micros: sequence * 10,
            kind,
        }
    }

    fn started_lifecycle() -> DocumentLifecycle {
        let started = event(
            1,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let mut lifecycle = DocumentLifecycle::from_snapshot(RendererDocumentLifecycleSnapshot {
            frame: started.frame,
            document: started.document,
            epoch: started.epoch,
            started: RendererLifecycleEventStamp {
                sequence: started.sequence,
                timestamp_micros: started.timestamp_micros,
            },
            dom_content_loaded: None,
            load: None,
            terminated: None,
        });
        assert!(lifecycle.observe(started));
        lifecycle
    }

    #[test]
    fn rejects_foreign_and_reordered_events_without_advancing_state() {
        let mut lifecycle = started_lifecycle();
        let load = event(
            2,
            1,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        assert!(!DocumentLifecycle::default().observe(load));
        let before = lifecycle.snapshot();
        for invalid in [
            RendererDocumentLifecycleEvent {
                frame: RendererFrameToken {
                    page_id: crate::PageId::new_for_testing(8),
                },
                ..load
            },
            RendererDocumentLifecycleEvent {
                document: load.document.successor_for_testing(),
                ..load
            },
            RendererDocumentLifecycleEvent {
                epoch: RendererLifecycleEpoch(2),
                ..load
            },
            RendererDocumentLifecycleEvent {
                sequence: 1,
                ..load
            },
        ] {
            assert!(!lifecycle.observe(invalid));
            assert_eq!(lifecycle.snapshot(), before);
        }
        assert!(lifecycle.observe(load));
        assert!(!lifecycle.observe(load));
        assert_eq!(lifecycle.snapshot().unwrap().load.unwrap().sequence, 2);
    }

    #[test]
    fn restart_requires_termination_but_projection_may_omit_the_old_tail() {
        let mut lifecycle = started_lifecycle();
        let mut projection = lifecycle.snapshot().unwrap();
        let restarted = event(
            4,
            2,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
        );
        assert!(!lifecycle.observe(restarted));
        assert!(lifecycle.observe(event(
            3,
            1,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: None,
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
        )));
        assert!(lifecycle.observe(restarted));
        // A cancelled visibility barrier may have discarded the termination.
        // Projection applies an accepted occurrence, not the admission rules.
        projection.apply_event(restarted);
        assert_eq!(Some(projection), lifecycle.snapshot());
        assert!(projection.terminated.is_none());
        assert_eq!(projection.epoch, RendererLifecycleEpoch(2));

        let load = event(
            5,
            2,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        assert!(!lifecycle.observe(RendererDocumentLifecycleEvent {
            epoch: RendererLifecycleEpoch(1),
            sequence: 100,
            ..load
        }));
        assert!(lifecycle.observe(load));
    }
}
