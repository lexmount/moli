use std::time::{Duration, Instant};

use url::Url;

use crate::frame_owner_model::FrameDocumentTaskOwner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaRefreshNavigation {
    pub(crate) delay_ms: u32,
    pub(crate) url: Url,
}

#[derive(Debug)]
pub(crate) struct ScheduledMetaRefreshNavigation {
    pub(crate) owner: FrameDocumentTaskOwner,
    pub(crate) revision: u64,
    pub(crate) navigation: MetaRefreshNavigation,
    pub(crate) ready_at: Instant,
}

/// Document-owned state corresponding to Blink's `HttpRefreshScheduler`.
///
/// The scheduler owns one candidate and one exact-Document load boundary. The
/// caller owns posting and canceling delayed work, delivering the appropriate
/// load boundary, and navigating. Main and child Documents share these rules
/// while keeping independent scheduler instances.
#[derive(Debug, Default)]
pub(crate) struct MetaRefreshScheduler {
    candidate: Option<MetaRefreshNavigation>,
    revision: u64,
    load_finished_owner: Option<FrameDocumentTaskOwner>,
    armed_owner: Option<FrameDocumentTaskOwner>,
    ready_at: Option<Instant>,
}

impl MetaRefreshScheduler {
    pub(crate) fn has_candidate(&self) -> bool {
        self.candidate.is_some()
    }

    /// Register newly processed declarations, including equal declarations.
    /// Retrying an existing post must use the armed navigation instead.
    pub(crate) fn note_candidates(
        &mut self,
        candidates: impl IntoIterator<Item = MetaRefreshNavigation>,
    ) -> bool {
        let mut changed = false;
        for candidate in candidates {
            if self
                .candidate
                .as_ref()
                .is_some_and(|current| current.delay_ms < candidate.delay_ms)
            {
                continue;
            }
            self.candidate = Some(candidate);
            self.revision = self
                .revision
                .checked_add(1)
                .expect("meta refresh revision overflow");
            self.armed_owner = None;
            self.ready_at = None;
            changed = true;
        }
        changed
    }

    pub(crate) fn finish_load(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        self.load_finished_owner = Some(owner);
        self.schedule_for_loaded_owner()
    }

    pub(crate) fn schedule_for_loaded_owner(&mut self) -> Option<ScheduledMetaRefreshNavigation> {
        let owner = self.load_finished_owner?;
        let navigation = self.candidate.clone()?;
        if self.armed_owner.is_some() {
            return None;
        }
        let ready_at = self.ready_at.or_else(|| {
            Instant::now().checked_add(Duration::from_millis(u64::from(navigation.delay_ms)))
        })?;
        self.armed_owner = Some(owner);
        self.ready_at = Some(ready_at);
        Some(ScheduledMetaRefreshNavigation {
            owner,
            revision: self.revision,
            navigation,
            ready_at,
        })
    }

    pub(crate) fn take_if_current(
        &mut self,
        owner: FrameDocumentTaskOwner,
        revision: u64,
    ) -> Option<MetaRefreshNavigation> {
        if self.armed_owner != Some(owner) || self.revision != revision {
            return None;
        }
        let navigation = self.candidate.take()?;
        self.armed_owner = None;
        self.ready_at = None;
        Some(navigation)
    }

    pub(crate) fn prepare_for_document_open(&mut self) {
        self.load_finished_owner = None;
        self.armed_owner = None;
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| candidate.delay_ms == 0)
        {
            *self = Self::default();
        }
    }

    pub(crate) fn rebind_armed_navigation(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ScheduledMetaRefreshNavigation> {
        let navigation = self.candidate.clone()?;
        let ready_at = self.ready_at?;
        self.armed_owner = Some(owner);
        Some(ScheduledMetaRefreshNavigation {
            owner,
            revision: self.revision,
            navigation,
            ready_at,
        })
    }

    pub(crate) fn armed_navigation(&self) -> Option<ScheduledMetaRefreshNavigation> {
        Some(ScheduledMetaRefreshNavigation {
            owner: self.armed_owner?,
            revision: self.revision,
            navigation: self.candidate.clone()?,
            ready_at: self.ready_at?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId};

    fn document_owner(id: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(id), LocalWindowId(id), DocumentId(id))
    }

    fn replacement_document_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            document_owner(1).scheduler_lane_id,
            document_owner(1).local_window_id,
            DocumentId(2),
        )
    }

    #[test]
    fn schedulers_keep_document_owners_and_load_boundaries_independent() {
        let first_owner = document_owner(1);
        let second_owner = document_owner(2);
        let first_navigation = MetaRefreshNavigation {
            delay_ms: 1_000,
            url: Url::parse("https://example.test/first").unwrap(),
        };
        let second_navigation = MetaRefreshNavigation {
            delay_ms: 30_000,
            url: Url::parse("https://example.test/second").unwrap(),
        };
        let mut first = MetaRefreshScheduler::default();
        let mut second = MetaRefreshScheduler::default();
        assert!(first.note_candidates([first_navigation.clone()]));
        assert!(second.note_candidates([second_navigation.clone()]));
        assert!(first.schedule_for_loaded_owner().is_none());
        assert!(second.schedule_for_loaded_owner().is_none());

        let scheduled = first.finish_load(first_owner).unwrap();
        assert!(second.armed_navigation().is_none());
        assert_eq!(
            first.take_if_current(second_owner, scheduled.revision),
            None
        );
        assert_eq!(
            first.armed_navigation().unwrap().ready_at,
            scheduled.ready_at
        );
        assert!(first.finish_load(first_owner).is_none());
        assert_eq!(
            first.armed_navigation().unwrap().ready_at,
            scheduled.ready_at
        );

        let second_scheduled = second.finish_load(second_owner).unwrap();
        assert_eq!(
            first.take_if_current(first_owner, scheduled.revision),
            Some(first_navigation)
        );
        assert_eq!(
            second.take_if_current(second_owner, second_scheduled.revision),
            Some(second_navigation)
        );
    }

    #[test]
    fn scheduler_keeps_the_earliest_delay_and_replaces_equal_candidates() {
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([
            MetaRefreshNavigation {
                delay_ms: 500,
                url: Url::parse("https://example.test/first").unwrap(),
            },
            MetaRefreshNavigation {
                delay_ms: 1_000,
                url: Url::parse("https://example.test/ignored").unwrap(),
            },
            MetaRefreshNavigation {
                delay_ms: 200,
                url: Url::parse("https://example.test/earlier").unwrap(),
            },
            MetaRefreshNavigation {
                delay_ms: 200,
                url: Url::parse("https://example.test/equal-and-later").unwrap(),
            },
        ]));
        let scheduled = scheduler
            .finish_load(document_owner(1))
            .expect("accepted candidate should schedule after load");
        assert_eq!(scheduled.navigation.delay_ms, 200);
        assert_eq!(
            scheduled.navigation.url.as_str(),
            "https://example.test/equal-and-later"
        );
    }

    #[test]
    fn scheduler_new_identical_declaration_replaces_the_posted_navigation() {
        let owner = document_owner(1);
        let navigation = MetaRefreshNavigation {
            delay_ms: 60_000,
            url: Url::parse("https://example.test/final").unwrap(),
        };
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([navigation.clone()]));
        let original = scheduler
            .finish_load(owner)
            .expect("load should arm the original refresh");

        assert!(!scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: navigation.delay_ms + 1,
            url: navigation.url.clone(),
        }]));
        assert_eq!(
            scheduler.armed_navigation().unwrap().revision,
            original.revision
        );
        assert!(scheduler.note_candidates([navigation.clone()]));
        let replacement = scheduler.schedule_for_loaded_owner().unwrap();
        assert_ne!(replacement.revision, original.revision);
        assert!(replacement.ready_at >= original.ready_at);
        assert_eq!(scheduler.take_if_current(owner, original.revision), None);
        assert_eq!(
            scheduler.take_if_current(owner, replacement.revision),
            Some(navigation)
        );
        assert_eq!(scheduler.take_if_current(owner, replacement.revision), None);
    }

    #[test]
    fn scheduler_repeated_load_delivery_keeps_the_original_post() {
        let owner = document_owner(1);
        let mut scheduler = MetaRefreshScheduler::default();
        scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 1_000,
            url: Url::parse("https://example.test/unchanged").unwrap(),
        }]);
        let original = scheduler.finish_load(owner).unwrap();
        assert!(scheduler.finish_load(owner).is_none());
        let retry = scheduler.armed_navigation().unwrap();
        assert_eq!(retry.ready_at, original.ready_at);
        assert_eq!(retry.revision, original.revision);
    }

    #[test]
    fn scheduler_rebinds_an_armed_nonzero_refresh_without_resetting_its_deadline() {
        let original_owner = document_owner(1);
        let replacement_owner = replacement_document_owner();
        let target = Url::parse("https://example.test/final").unwrap();
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 60_000,
            url: target.clone(),
        }]));
        let original = scheduler
            .finish_load(original_owner)
            .expect("load should arm the refresh");

        scheduler.prepare_for_document_open();
        let rebound = scheduler
            .rebind_armed_navigation(replacement_owner)
            .expect("document.open should rebind an active nonzero refresh");

        assert_eq!(rebound.owner, replacement_owner);
        assert_eq!(rebound.ready_at, original.ready_at);
        assert_eq!(rebound.revision, original.revision);
        assert_eq!(
            scheduler.take_if_current(original_owner, original.revision),
            None
        );
        assert_eq!(
            scheduler.take_if_current(replacement_owner, rebound.revision),
            Some(MetaRefreshNavigation {
                delay_ms: 60_000,
                url: target
            })
        );
    }

    #[test]
    fn scheduler_cancels_a_zero_delay_refresh_during_document_open() {
        let owner = document_owner(1);
        let target = Url::parse("https://example.test/final").unwrap();
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 0,
            url: target.clone(),
        }]));
        assert!(scheduler.finish_load(owner).is_some());

        scheduler.prepare_for_document_open();

        assert!(
            scheduler
                .rebind_armed_navigation(replacement_document_owner())
                .is_none()
        );
        assert_eq!(scheduler.take_if_current(owner, 1), None);
    }

    #[test]
    fn scheduler_rejects_a_rebound_payload_after_a_new_candidate_supersedes_it() {
        let original_owner = document_owner(1);
        let replacement_owner = replacement_document_owner();
        let old_target = Url::parse("https://example.test/old").unwrap();
        let new_target = Url::parse("https://example.test/new").unwrap();
        let mut scheduler = MetaRefreshScheduler::default();
        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 60_000,
            url: old_target.clone(),
        }]));
        assert!(scheduler.finish_load(original_owner).is_some());
        scheduler.prepare_for_document_open();
        assert!(
            scheduler
                .rebind_armed_navigation(replacement_owner)
                .is_some()
        );

        assert!(scheduler.note_candidates([MetaRefreshNavigation {
            delay_ms: 1_000,
            url: new_target.clone(),
        }]));
        assert!(
            scheduler.take_if_current(replacement_owner, 1).is_none(),
            "a queued payload must prove it still matches scheduler state before navigating"
        );
        let replacement = scheduler
            .finish_load(replacement_owner)
            .expect("the new candidate should arm after the replacement load");
        assert_eq!(replacement.navigation.url, new_target);
        assert_eq!(replacement.navigation.delay_ms, 1_000);
    }
}
