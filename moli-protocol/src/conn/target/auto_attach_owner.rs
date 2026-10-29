use crate::conn::{AutoAttachOwnerPolicy, CdpConnection, CdpTargetFilter};
use std::rc::{Rc, Weak};

/// Identity of one installed policy. Replacing even an equal policy retires
/// the previous sweep without keeping its owner or configuration alive.
#[derive(Clone, Debug)]
pub(crate) struct AutoAttachPolicyIdentity {
    owner: Option<String>,
    policy: Weak<AutoAttachOwnerPolicy>,
}

impl AutoAttachPolicyIdentity {
    pub(crate) fn capture(conn: &CdpConnection, owner: Option<&str>) -> Option<Self> {
        let owner = owner.map(str::to_owned);
        let policy = Rc::downgrade(conn.auto_attach_owner_sessions.get(&owner)?);
        Some(Self { owner, policy })
    }

    pub(crate) fn is_current(&self, conn: &CdpConnection) -> bool {
        self.owner
            .as_deref()
            .is_none_or(|owner| !conn.session_is_closing(owner))
            && conn
                .auto_attach_owner_sessions
                .get(&self.owner)
                .is_some_and(|policy| Weak::ptr_eq(&Rc::downgrade(policy), &self.policy))
    }
}

impl CdpConnection {
    pub(crate) fn auto_attach_enabled(&self) -> bool {
        !self.auto_attach_owner_sessions.is_empty()
    }

    pub(crate) fn auto_attach_owner_sessions_for_target_type(
        &self,
        target_type: &str,
    ) -> Vec<Option<String>> {
        self.auto_attach_owner_sessions
            .iter()
            .filter(|(_, policy)| policy.target_filter.matches(target_type))
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>()
    }

    pub(crate) fn auto_attach_owner_allows_target_type(
        &self,
        session_id: Option<&str>,
        target_type: &str,
    ) -> bool {
        self.auto_attach_owner_sessions
            .get(&session_id.map(str::to_owned))
            .is_some_and(|policy| policy.target_filter.matches(target_type))
    }

    pub(crate) fn auto_attach_owner_waits_for_debugger_on_start(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.auto_attach_owner_sessions
            .get(&session_id.map(str::to_owned))
            .is_some_and(|policy| policy.wait_for_debugger_on_start)
    }

    pub(crate) fn set_auto_attach_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
        wait_for_debugger_on_start: bool,
        target_filter: CdpTargetFilter,
        flattened_protocol: bool,
    ) {
        self.clear_service_worker_auto_attach_related_owner(session_id);
        let key = session_id.map(str::to_owned);
        if enabled {
            self.target_control.ensure_owner(session_id);
            self.auto_attach_owner_sessions.insert(
                key,
                Rc::new(AutoAttachOwnerPolicy {
                    wait_for_debugger_on_start,
                    target_filter,
                    flattened_protocol,
                }),
            );
        } else {
            self.auto_attach_owner_sessions.shift_remove(&key);
        }
    }

    pub(crate) fn clear_auto_attach_owner(&mut self, session_id: Option<&str>) {
        self.clear_service_worker_auto_attach_related_owner(session_id);
        let key = session_id.map(str::to_owned);
        self.auto_attach_owner_sessions.shift_remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestContext;

    #[test]
    fn auto_attach_owners_keep_protocol_attachment_order() {
        let ctx = TestContext::new();
        let page_filter = CdpTargetFilter::default_auto_attach();
        ctx.with_fixture(|conn| {
            conn.set_auto_attach_owner(None, true, false, page_filter.clone(), true)
        });
        ctx.with_fixture(|conn| {
            conn.set_auto_attach_owner(Some("SID-browser"), true, true, page_filter.clone(), true)
        });

        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.auto_attach_owner_sessions_for_target_type("page"),
                vec![None, Some("SID-browser".to_owned())]
            )
        });

        // Updating an existing owner's policy is not a new attachment and
        // must not move it behind newer owners.
        ctx.with_fixture(|conn| {
            conn.set_auto_attach_owner(None, true, true, page_filter.clone(), true)
        });
        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.auto_attach_owner_sessions_for_target_type("page"),
                vec![None, Some("SID-browser".to_owned())]
            )
        });

        // Removing and attaching again is a new protocol attachment.
        ctx.with_fixture(|conn| {
            conn.set_auto_attach_owner(None, false, false, page_filter.clone(), true)
        });
        ctx.with_fixture(|conn| conn.set_auto_attach_owner(None, true, false, page_filter, true));
        ctx.with_fixture(|conn| {
            assert_eq!(
                conn.auto_attach_owner_sessions_for_target_type("page"),
                vec![Some("SID-browser".to_owned()), None]
            )
        });
    }
}
