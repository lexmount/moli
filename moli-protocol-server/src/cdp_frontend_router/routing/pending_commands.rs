use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Clone)]
pub(super) struct CdpCommandFrontend {
    pub(super) frontend_id: u64,
    pub(super) dispatch_session_id: Option<String>,
    pub(super) client_session_id: Option<String>,
}

pub(super) enum PendingCommandEffect {
    None,
    AttachToTarget { target_id: Option<String> },
    ReplayForegroundTabTargetCommand { method: String, params: Value },
}

pub(super) struct PendingCommandRoute {
    pub(super) frontend: CdpCommandFrontend,
    pub(super) client_command_id: u64,
    pub(super) effect: PendingCommandEffect,
}

#[derive(Default)]
pub(super) struct PendingCommandTable {
    next_internal_command_id: u64,
    routes: HashMap<u64, PendingCommandRoute>,
}

impl PendingCommandTable {
    pub(super) fn allocate_internal_command_id(&mut self) -> Result<u64> {
        let command_id = self
            .next_internal_command_id
            .checked_add(1)
            .context("CDP internal command id space exhausted")?;
        self.next_internal_command_id = command_id;
        Ok(command_id)
    }

    pub(super) fn insert(&mut self, command_id: u64, route: PendingCommandRoute) {
        let replaced = self.routes.insert(command_id, route);
        debug_assert!(replaced.is_none(), "internal command id must be unique");
    }

    pub(super) fn take(&mut self, command_id: u64) -> Option<PendingCommandRoute> {
        self.routes.remove(&command_id)
    }

    pub(super) fn remove_frontend(&mut self, frontend_id: u64) {
        self.routes
            .retain(|_, pending| pending.frontend.frontend_id != frontend_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(frontend_id: u64) -> PendingCommandRoute {
        PendingCommandRoute {
            frontend: CdpCommandFrontend {
                frontend_id,
                dispatch_session_id: None,
                client_session_id: None,
            },
            client_command_id: 7,
            effect: PendingCommandEffect::None,
        }
    }

    #[test]
    fn removing_one_frontend_preserves_other_pending_commands() {
        let mut pending = PendingCommandTable::default();
        let first = pending
            .allocate_internal_command_id()
            .expect("allocate first command id");
        pending.insert(first, route(5));
        let second = pending
            .allocate_internal_command_id()
            .expect("allocate second command id");
        pending.insert(second, route(6));

        pending.remove_frontend(5);

        assert!(pending.take(first).is_none());
        assert!(pending.take(second).is_some());
    }

    #[test]
    fn command_ids_are_never_reused_after_exhaustion() {
        let mut pending = PendingCommandTable {
            next_internal_command_id: u64::MAX,
            ..PendingCommandTable::default()
        };

        let error = pending
            .allocate_internal_command_id()
            .expect_err("exhausted command id space must fail");

        assert!(error.to_string().contains("command id space exhausted"));
        assert_eq!(pending.next_internal_command_id, u64::MAX);
    }
}
