use crate::conn::{AutoAttachOwnerPolicy, CdpConnection, CdpTargetFilter};

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
    ) {
        self.clear_service_worker_auto_attach_related_owner(session_id);
        let key = session_id.map(str::to_owned);
        if enabled {
            self.target_control.ensure_owner(session_id);
            self.auto_attach_owner_sessions.insert(
                key,
                AutoAttachOwnerPolicy {
                    wait_for_debugger_on_start,
                    target_filter,
                },
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
        let mut ctx = TestContext::new();
        let page_filter = CdpTargetFilter::default_auto_attach();
        ctx.conn
            .set_auto_attach_owner(None, true, false, page_filter.clone());
        ctx.conn
            .set_auto_attach_owner(Some("SID-browser"), true, true, page_filter.clone());

        assert_eq!(
            ctx.conn.auto_attach_owner_sessions_for_target_type("page"),
            vec![None, Some("SID-browser".to_owned())]
        );

        // Updating an existing owner's policy is not a new attachment and
        // must not move it behind newer owners.
        ctx.conn
            .set_auto_attach_owner(None, true, true, page_filter.clone());
        assert_eq!(
            ctx.conn.auto_attach_owner_sessions_for_target_type("page"),
            vec![None, Some("SID-browser".to_owned())]
        );

        // Removing and attaching again is a new protocol attachment.
        ctx.conn
            .set_auto_attach_owner(None, false, false, page_filter.clone());
        ctx.conn
            .set_auto_attach_owner(None, true, false, page_filter);
        assert_eq!(
            ctx.conn.auto_attach_owner_sessions_for_target_type("page"),
            vec![Some("SID-browser".to_owned()), None]
        );
    }
}
