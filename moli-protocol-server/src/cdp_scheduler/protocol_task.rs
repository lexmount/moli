use std::collections::HashMap;

use moli_protocol::{CompletedProtocolSchedulerWork, PendingProtocolSchedulerWork};
use tokio::sync::mpsc;

use super::command_task::{
    CommandWaitCompletion, CommandWaitFailure, CommandWaitIdentity, CommandWaitTask,
    spawn_command_wait,
};

pub(crate) type ProtocolWorkCompletion = CommandWaitCompletion<CompletedProtocolSchedulerWork>;

/// The existing owner-work supervision, shared by every adapter and by
/// renderer-cursor waits. Dropping a socket waiter does not drop admitted work.
pub(super) struct ProtocolTaskSupervisor {
    sender: mpsc::UnboundedSender<ProtocolWorkCompletion>,
    tasks: HashMap<u64, ProtocolTask>,
    next_token: u64,
}

struct ProtocolTask {
    pending: PendingProtocolSchedulerWork,
    task: CommandWaitTask,
    turn: u64,
}

impl ProtocolTaskSupervisor {
    pub(super) fn new(sender: mpsc::UnboundedSender<ProtocolWorkCompletion>) -> Self {
        Self {
            sender,
            tasks: HashMap::new(),
            next_token: 0,
        }
    }

    pub(super) fn start(&mut self, pending: PendingProtocolSchedulerWork) {
        let token = self.next_token;
        self.next_token = token.checked_add(1).expect("protocol task token overflow");
        self.start_with_identity(CommandWaitIdentity { token, turn: 0 }, pending);
    }

    fn start_with_identity(
        &mut self,
        identity: CommandWaitIdentity,
        pending: PendingProtocolSchedulerWork,
    ) {
        let wait = pending.clone();
        let task = spawn_command_wait(identity, wait.wait(), &self.sender);
        let previous = self.tasks.insert(
            identity.token,
            ProtocolTask {
                pending,
                task,
                turn: identity.turn,
            },
        );
        assert!(previous.is_none(), "one task owns each protocol wait");
    }

    pub(super) fn complete(
        &mut self,
        completion: ProtocolWorkCompletion,
    ) -> Result<Option<CompletedProtocolSchedulerWork>, CommandWaitFailure> {
        let identity = completion.identity;
        if self
            .tasks
            .get(&identity.token)
            .is_none_or(|task| task.turn != identity.turn)
        {
            return Ok(None);
        }
        let task = self
            .tasks
            .remove(&identity.token)
            .expect("matching protocol task");
        match completion.result {
            Ok(completed) => Ok(Some(completed)),
            Err(error) if identity.turn == 0 => {
                tracing::warn!(token = identity.token, %error, "protocol workflow failed; rejoining admitted owner work");
                self.start_with_identity(
                    CommandWaitIdentity {
                        turn: 1,
                        ..identity
                    },
                    task.pending,
                );
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn take_tasks(&mut self) -> Vec<CommandWaitTask> {
        self.tasks.drain().map(|(_, task)| task.task).collect()
    }
}
