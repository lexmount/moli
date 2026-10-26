use anyhow::Result;

use super::*;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver};
use crate::domains::activity::{ProtocolSchedulerWork, ProtocolSchedulerWorkKind};
use crate::domains::page::{CompletedTargetTermination, TargetTermination};

/// Owned protocol work submitted to the adapter's existing task supervisor.
/// Retaining a clone preserves the same native receipts for failure cleanup.
#[derive(Clone)]
pub struct PendingProtocolSchedulerWork {
    termination: TargetTermination,
    access: ProtocolStateAccess,
}

pub struct CompletedProtocolSchedulerWork {
    pub(crate) termination: Box<CompletedTargetTermination>,
}

impl PendingProtocolSchedulerWork {
    pub async fn wait(self) -> CompletedProtocolSchedulerWork {
        let termination = self
            .termination
            .run(&mut ProtocolWorkflowDriver::owned(self.access))
            .await;
        CompletedProtocolSchedulerWork {
            termination: Box::new(termination),
        }
    }
}

impl CdpConnection {
    /// Starts migrated owner work using only finite state access. Unsupported
    /// work is returned intact to its existing driver during migration.
    pub fn try_start_owned_protocol_scheduler_work(
        &mut self,
        work: ProtocolSchedulerWork,
    ) -> Result<Option<PendingProtocolSchedulerWork>, Box<ProtocolSchedulerWork>> {
        let Some(access) = self.protocol_state_access() else {
            return Err(Box::new(work));
        };
        if work.kind() != ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction {
            return Err(Box::new(work));
        }
        let ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action) =
            work.into_ready()
        else {
            unreachable!("the work kind identifies its owned payload");
        };
        Ok(TargetTermination::prepare(self, action).map(|termination| {
            PendingProtocolSchedulerWork {
                termination,
                access,
            }
        }))
    }

    pub fn complete_owned_protocol_scheduler_work(
        &mut self,
        completed: CompletedProtocolSchedulerWork,
    ) -> CdpTurnOutcome {
        completed.termination.finish(self)
    }
}
