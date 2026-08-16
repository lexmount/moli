//! Renderer-owner housekeeping that is deliberately outside HTML task
//! scheduling.
//!
//! A maintenance task is Page-scoped because it targets the Page's currently
//! attached V8 isolate, but it is not a Page/HTML task:
//!
//! - it does not participate in [`PageTurnScheduler`](super::page_turn_scheduler::PageTurnScheduler)
//!   arbitration;
//! - it does not create a microtask checkpoint or protocol-output boundary;
//! - it survives same-Page Document replacement and targets the replacement
//!   isolate when it eventually runs;
//! - retiring the stable Page slot retires both the residence and any admitted
//!   task.
//!
//! The residence separates a scheduled deadline from an admitted task. That
//! prevents an expired periodic deadline from being re-admitted while the
//! concrete owner turn is waiting behind another bounded turn.

use std::time::{Duration, Instant};

use anyhow::{Result, ensure};

use super::owner_local_store::{
    LivePageEntry, RendererPageToken, run_entry_on_bound_owner_local_store_local_task,
};
use crate::local_executor::JsLocalExecutor;

const ACTIVE_PAGE_MODERATE_MEMORY_PRESSURE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RendererOwnerMaintenanceAction {
    /// Ask V8 to perform its moderate-memory-pressure maintenance for the
    /// isolate currently attached to this Page.
    ModerateMemoryPressure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RendererOwnerMaintenanceTask {
    token: RendererPageToken,
    action: RendererOwnerMaintenanceAction,
    scheduled_for: Instant,
}

impl RendererOwnerMaintenanceTask {
    pub(super) const fn token(self) -> RendererPageToken {
        self.token
    }

    const fn action(self) -> RendererOwnerMaintenanceAction {
        self.action
    }
}

#[derive(Debug)]
enum RendererPageOwnerMaintenanceState {
    /// The Page slot owns a future maintenance deadline. Only this state is
    /// present in the owner-wide maintenance deadline index.
    Scheduled { deadline: Instant },
    /// A concrete owner turn owns this deadline. The residence remains in the
    /// Page slot so duplicate deadline admission is impossible.
    Admitted { scheduled_for: Instant },
    /// The stable Page slot is retiring. Neither a retained deadline nor an
    /// already-admitted task may re-arm maintenance after this transition.
    Retired,
}

#[derive(Debug)]
pub(super) struct RendererPageOwnerMaintenanceResidence {
    state: RendererPageOwnerMaintenanceState,
}

impl RendererPageOwnerMaintenanceResidence {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            state: RendererPageOwnerMaintenanceState::Scheduled {
                deadline: next_moderate_memory_pressure_deadline_after(now),
            },
        }
    }

    pub(super) const fn indexed_deadline(&self) -> Option<Instant> {
        match self.state {
            RendererPageOwnerMaintenanceState::Scheduled { deadline } => Some(deadline),
            RendererPageOwnerMaintenanceState::Admitted { .. }
            | RendererPageOwnerMaintenanceState::Retired => None,
        }
    }

    pub(super) fn claim_if_due(
        &mut self,
        token: RendererPageToken,
        now: Instant,
    ) -> Option<RendererOwnerMaintenanceTask> {
        let RendererPageOwnerMaintenanceState::Scheduled { deadline } = self.state else {
            return None;
        };
        if deadline > now {
            return None;
        }
        self.state = RendererPageOwnerMaintenanceState::Admitted {
            scheduled_for: deadline,
        };
        Some(RendererOwnerMaintenanceTask {
            token,
            action: RendererOwnerMaintenanceAction::ModerateMemoryPressure,
            scheduled_for: deadline,
        })
    }

    pub(super) fn settle(
        &mut self,
        task: RendererOwnerMaintenanceTask,
        now: Instant,
    ) -> Result<()> {
        let RendererPageOwnerMaintenanceState::Admitted { scheduled_for } = self.state else {
            anyhow::bail!("owner maintenance task settled without an admitted residence");
        };
        ensure!(
            scheduled_for == task.scheduled_for,
            "owner maintenance task settled a different admitted deadline"
        );
        self.state = RendererPageOwnerMaintenanceState::Scheduled {
            deadline: next_moderate_memory_pressure_deadline_after(now),
        };
        Ok(())
    }

    pub(super) fn retire(&mut self) {
        self.state = RendererPageOwnerMaintenanceState::Retired;
    }
}

fn next_moderate_memory_pressure_deadline_after(now: Instant) -> Instant {
    now.checked_add(ACTIVE_PAGE_MODERATE_MEMORY_PRESSURE_PERIOD)
        .unwrap_or(now)
}

pub(super) async fn execute_owner_maintenance_task_on_local_lane(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    task: RendererOwnerMaintenanceTask,
) -> (LivePageEntry, Result<()>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            match task.action() {
                RendererOwnerMaintenanceAction::ModerateMemoryPressure => entry
                    .page_vm_mut()
                    .vm_mut()
                    .renderer_document_isolate_ops()
                    .notify_renderer_document_isolate_moderate_memory_pressure(),
            }
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PageId;

    fn token() -> RendererPageToken {
        RendererPageToken::new_for_testing(PageId::new_for_testing(7))
    }

    #[test]
    fn maintenance_residence_claims_once_and_rearms_after_settlement() {
        let now = Instant::now();
        let mut residence = RendererPageOwnerMaintenanceResidence::new(now);
        let deadline = residence
            .indexed_deadline()
            .expect("new residence should publish its deadline");

        assert!(residence.claim_if_due(token(), deadline).is_some());
        assert_eq!(residence.indexed_deadline(), None);
        assert!(
            residence.claim_if_due(token(), deadline).is_none(),
            "an admitted deadline must not create a second owner turn"
        );

        let mut second_residence = RendererPageOwnerMaintenanceResidence::new(now);
        let task = second_residence
            .claim_if_due(token(), deadline)
            .expect("deadline should be due");
        second_residence
            .settle(task, deadline)
            .expect("matching admitted task should settle");
        assert!(
            second_residence
                .indexed_deadline()
                .expect("settlement should rearm the residence")
                > deadline
        );
    }

    #[test]
    fn maintenance_residence_rejects_foreign_settlement() {
        let now = Instant::now();
        let mut residence = RendererPageOwnerMaintenanceResidence::new(now);
        let deadline = residence.indexed_deadline().expect("deadline should exist");
        let task = residence
            .claim_if_due(token(), deadline)
            .expect("deadline should be due");
        let foreign = RendererOwnerMaintenanceTask {
            scheduled_for: deadline
                .checked_add(Duration::from_millis(1))
                .expect("test deadline should fit"),
            ..task
        };

        assert!(residence.settle(foreign, deadline).is_err());
    }

    #[test]
    fn retired_maintenance_residence_cannot_be_claimed_or_rearmed() {
        let now = Instant::now();
        let mut residence = RendererPageOwnerMaintenanceResidence::new(now);
        let deadline = residence.indexed_deadline().expect("deadline should exist");
        let task = residence
            .claim_if_due(token(), deadline)
            .expect("deadline should be due");

        residence.retire();

        assert_eq!(residence.indexed_deadline(), None);
        assert!(residence.claim_if_due(token(), deadline).is_none());
        assert!(residence.settle(task, deadline).is_err());
    }
}
