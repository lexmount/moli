use crate::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleSnapshot, RendererLifecycleEventStamp, RendererPageCreationArtifacts,
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
    /// Commit the complete creation prefix before publishing the Document.
    /// Frontend visibility may lag; it never drives authoritative progress.
    pub fn from_creation_artifacts(artifacts: &RendererPageCreationArtifacts) -> Option<Self> {
        let mut lifecycle = Self::from_snapshot(Self::creation_prefix_snapshot(artifacts)?);
        for &event in &artifacts.initial_lifecycle_events {
            if !lifecycle.observe(event) {
                return None;
            }
        }
        let snapshot = artifacts.lifecycle_snapshot;
        if lifecycle.snapshot() != Some(snapshot) {
            return None;
        }
        // An inventory-only handoff still establishes the sequence floor.
        // A later live record cannot replay progress already in the snapshot.
        if lifecycle.last_sequence.is_none() {
            lifecycle.last_sequence = [
                Some(snapshot.started.sequence),
                snapshot.dom_content_loaded.map(|stamp| stamp.sequence),
                snapshot.load.map(|stamp| stamp.sequence),
                snapshot.terminated.map(|stamp| stamp.sequence),
            ]
            .into_iter()
            .flatten()
            .max();
        }
        Some(lifecycle)
    }

    /// The initial inventory for projecting an already committed prefix.
    /// This value grants no authority to reset a Browser Document.
    pub fn creation_prefix_snapshot(
        artifacts: &RendererPageCreationArtifacts,
    ) -> Option<RendererDocumentLifecycleSnapshot> {
        let snapshot = artifacts.lifecycle_snapshot;
        if snapshot.document != artifacts.active_document
            || snapshot.epoch != artifacts.active_epoch
        {
            return None;
        }
        let initial = artifacts
            .initial_lifecycle_events
            .iter()
            .find(|event| {
                event.frame == snapshot.frame
                    && event.document == artifacts.active_document
                    && matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Started { .. }
                    )
            })
            .map(|event| RendererDocumentLifecycleSnapshot {
                frame: event.frame,
                document: event.document,
                epoch: event.epoch,
                started: RendererLifecycleEventStamp {
                    sequence: event.sequence,
                    timestamp_micros: event.timestamp_micros,
                },
                dom_content_loaded: None,
                load: None,
                terminated: None,
            })
            .unwrap_or(snapshot);
        Some(initial)
    }

    /// Seeds a lifecycle before replaying its creation-event prefix.
    fn from_snapshot(snapshot: RendererDocumentLifecycleSnapshot) -> Self {
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
    fn creation_commits_native_progress_but_preserves_a_separate_projection_start() {
        let started = event(
            1,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let load = event(
            2,
            1,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let mut snapshot = started_lifecycle().snapshot().unwrap();
        snapshot.apply_event(load);
        let artifacts = RendererPageCreationArtifacts {
            active_document: snapshot.document,
            active_epoch: snapshot.epoch,
            lifecycle_snapshot: snapshot,
            initial_lifecycle_events: vec![started, load],
        };
        let mut lifecycle = DocumentLifecycle::from_creation_artifacts(&artifacts).unwrap();
        assert!(
            DocumentLifecycle::creation_prefix_snapshot(&artifacts)
                .unwrap()
                .load
                .is_none()
        );
        assert_eq!(lifecycle.snapshot(), Some(snapshot));
        assert!(!lifecycle.observe(started));
        assert!(!lifecycle.observe(load));
    }

    #[test]
    fn creation_artifacts_reject_inconsistent_inventory_and_allow_an_empty_prefix() {
        let snapshot = started_lifecycle().snapshot().unwrap();
        let mut artifacts = RendererPageCreationArtifacts {
            active_document: snapshot.document,
            active_epoch: snapshot.epoch,
            lifecycle_snapshot: snapshot,
            initial_lifecycle_events: Vec::new(),
        };
        assert_eq!(
            DocumentLifecycle::from_creation_artifacts(&artifacts)
                .unwrap()
                .snapshot(),
            Some(snapshot)
        );
        assert!(
            !DocumentLifecycle::from_creation_artifacts(&artifacts)
                .unwrap()
                .observe(event(
                    snapshot.started.sequence,
                    snapshot.epoch.0,
                    RendererDocumentLifecycleEventKind::Started {
                        reason: RendererLifecycleStartReason::InitialDocument,
                    },
                ))
        );
        artifacts.active_document = snapshot.document.successor_for_testing();
        assert!(DocumentLifecycle::from_creation_artifacts(&artifacts).is_none());
        artifacts.active_document = snapshot.document;
        artifacts.active_epoch = RendererLifecycleEpoch(snapshot.epoch.0 + 1);
        assert!(DocumentLifecycle::from_creation_artifacts(&artifacts).is_none());
    }

    #[test]
    fn creation_rejects_foreign_reordered_and_snapshot_inconsistent_prefixes() {
        let started = event(
            1,
            1,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        );
        let load = event(
            2,
            1,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load),
        );
        let mut snapshot = started_lifecycle().snapshot().unwrap();
        snapshot.apply_event(load);
        let mut artifacts = RendererPageCreationArtifacts {
            active_document: snapshot.document,
            active_epoch: snapshot.epoch,
            lifecycle_snapshot: snapshot,
            initial_lifecycle_events: vec![started, load],
        };
        assert!(DocumentLifecycle::from_creation_artifacts(&artifacts).is_some());
        for invalid in [
            vec![load, started],
            vec![started, load, load],
            vec![
                started,
                RendererDocumentLifecycleEvent {
                    document: load.document.successor_for_testing(),
                    ..load
                },
            ],
            vec![started],
        ] {
            artifacts.initial_lifecycle_events = invalid;
            assert!(DocumentLifecycle::from_creation_artifacts(&artifacts).is_none());
        }
    }

    #[test]
    fn creation_prefix_can_replay_a_document_open_before_the_active_epoch() {
        let prefix = vec![
            event(
                1,
                1,
                RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::InitialDocument,
                },
            ),
            event(
                2,
                1,
                RendererDocumentLifecycleEventKind::Terminated {
                    last_reached: None,
                    reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
                },
            ),
            event(
                3,
                2,
                RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
                },
            ),
        ];
        let mut snapshot = started_lifecycle().snapshot().unwrap();
        for event in &prefix {
            snapshot.apply_event(*event);
        }
        let artifacts = RendererPageCreationArtifacts {
            active_document: snapshot.document,
            active_epoch: snapshot.epoch,
            lifecycle_snapshot: snapshot,
            initial_lifecycle_events: prefix,
        };
        let lifecycle = DocumentLifecycle::from_creation_artifacts(&artifacts).unwrap();
        assert_eq!(
            DocumentLifecycle::creation_prefix_snapshot(&artifacts)
                .unwrap()
                .epoch,
            RendererLifecycleEpoch(1)
        );
        assert_eq!(lifecycle.snapshot(), Some(snapshot));
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
