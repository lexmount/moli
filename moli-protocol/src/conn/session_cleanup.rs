use anyhow::Result;

use moli_core::page::{CompletedPageCommand, Page, PendingPageCommand, RendererAgentAttachmentId};

use super::{CdpConnection, CommandOwnerScope, RendererPageResidenceIdentity};

pub(crate) struct SessionScriptCleanup {
    owner: CommandOwnerScope,
    session: moli_page_types::DevToolsSessionKey,
    removals: Vec<(String, SessionPageUpdates)>,
}

pub(crate) struct CompletedSessionScriptCleanup {
    owner: CommandOwnerScope,
    session: moli_page_types::DevToolsSessionKey,
    removals: Vec<(String, CompletedSessionPageUpdates)>,
}

impl CdpConnection {
    pub(crate) fn prepare_session_script_cleanup(
        &mut self,
        session_id: &str,
    ) -> SessionScriptCleanup {
        let owner = CommandOwnerScope::capture(self, Some(session_id));
        let inspector =
            self.target_renderer_runtime_inspector_session_id_for_session(Some(session_id));
        let session =
            moli_page_types::DevToolsSessionKey::from_wire_session_id(inspector.as_deref());
        let keys = self
            .target_owner_state_for_session(Some(session_id))
            .map(|state| state.document_start_script_registry_keys_for_session(&session))
            .unwrap_or_default();
        let mut removals = Vec::with_capacity(keys.len());
        if let Some(page) = self.loaded_page_mut_for_owner(&owner) {
            for key in keys {
                let mut updates = SessionPageUpdates::default();
                updates.push(
                    page,
                    "detached-session document-start script cleanup",
                    page.start_remove_document_start_script_by_registry_key(&key),
                );
                removals.push((key, updates));
            }
        } else {
            let _ = self.with_target_owner_state_for_session_mut(Some(session_id), |state| {
                state.remove_document_start_scripts_for_session(&session)
            });
        }
        SessionScriptCleanup {
            owner,
            session,
            removals,
        }
    }
}

impl SessionScriptCleanup {
    pub(crate) async fn wait(self) -> CompletedSessionScriptCleanup {
        let mut removals = Vec::with_capacity(self.removals.len());
        for (key, updates) in self.removals {
            removals.push((key, updates.wait().await));
        }
        CompletedSessionScriptCleanup {
            owner: self.owner,
            session: self.session,
            removals,
        }
    }
}

impl CompletedSessionScriptCleanup {
    pub(crate) fn finish(self, mut conn: Option<&mut CdpConnection>) -> SessionPageUpdateResult {
        let mut result = SessionPageUpdateResult::default();
        for (key, updates) in self.removals {
            let completed = updates.finish(
                conn.as_deref_mut()
                    .and_then(|conn| conn.loaded_page_mut_for_owner(&self.owner)),
            );
            if completed.first_error.is_none()
                && let Some(conn) = conn.as_deref_mut()
            {
                let _ = conn.with_target_owner_state_for_session_mut(
                    self.owner.session_id(),
                    |state| {
                        state.remove_document_start_script_registry_key_for_session(
                            &self.session,
                            &key,
                        )
                    },
                );
            }
            result.extend(completed);
        }
        // Failed registry keys retain cleanup authority until retry or exact
        // Page retirement. Unkeyed scripts require no renderer round trip.
        if result.first_error.is_none()
            && let Some(conn) = conn
        {
            let _ = conn
                .with_target_owner_state_for_session_mut(self.owner.session_id(), |state| {
                    state.remove_document_start_scripts_for_session(&self.session)
                });
        }
        result
    }
}

/// An ordered set of native updates already admitted by a domain's disposal.
/// Waiting neither owns nor borrows protocol state. A failed update does not
/// prevent the remaining admitted updates from being observed.
#[derive(Default)]
pub(crate) struct SessionPageUpdates {
    pending: Vec<SessionPageUpdate<Result<PendingPageCommand>>>,
    first_error: Option<anyhow::Error>,
}

pub(crate) struct CompletedSessionPageUpdates {
    completed: Vec<SessionPageUpdate<Result<CompletedPageCommand>>>,
    first_error: Option<anyhow::Error>,
}

struct SessionPageUpdate<T> {
    page: RendererPageResidenceIdentity,
    attachment: Option<RendererAgentAttachmentId>,
    operation: &'static str,
    result: T,
}

#[derive(Default)]
pub(crate) struct SessionPageUpdateResult {
    pub(crate) first_error: Option<anyhow::Error>,
    pub(crate) predecessor: Option<moli_core::RendererOutputFence>,
}

impl SessionPageUpdateResult {
    pub(crate) fn extend(&mut self, other: Self) {
        if self.first_error.is_none() {
            self.first_error = other.first_error;
        }
        if let Some(predecessor) = other.predecessor {
            predecessor.merge_into_same_stream_tail(&mut self.predecessor);
        }
    }
    #[cfg(test)]
    pub(crate) fn into_result(self) -> Result<()> {
        self.first_error.map_or(Ok(()), Err)
    }
}

impl SessionPageUpdates {
    pub(crate) fn record_error(&mut self, error: anyhow::Error) {
        self.first_error.get_or_insert(error);
    }

    pub(crate) fn push(
        &mut self,
        page: &Page,
        operation: &'static str,
        pending: Result<PendingPageCommand>,
    ) {
        self.pending.push(SessionPageUpdate {
            page: RendererPageResidenceIdentity::from_page(page),
            attachment: page.renderer_agent_attachment_id(),
            operation,
            result: pending,
        });
    }

    pub(crate) async fn wait(self) -> CompletedSessionPageUpdates {
        let mut completed = Vec::with_capacity(self.pending.len());
        for update in self.pending {
            let result = match update.result {
                Ok(pending) => pending.wait().await,
                Err(error) => Err(error),
            };
            completed.push(SessionPageUpdate {
                page: update.page,
                attachment: update.attachment,
                operation: update.operation,
                result,
            });
        }
        CompletedSessionPageUpdates {
            completed,
            first_error: self.first_error,
        }
    }
}

impl CompletedSessionPageUpdates {
    pub(crate) fn finish(self, mut page: Option<&mut Page>) -> SessionPageUpdateResult {
        let mut result = SessionPageUpdateResult {
            first_error: self.first_error,
            predecessor: None,
        };
        for update in self.completed {
            let completion = match update.result {
                Ok(completion) => completion,
                Err(error) => {
                    result
                        .first_error
                        .get_or_insert_with(|| error.context(update.operation));
                    continue;
                }
            };
            if let Some(predecessor) = completion.renderer_output_predecessor() {
                predecessor.merge_into_same_stream_tail(&mut result.predecessor);
            }
            let applied = if let Some(page) = page.as_deref_mut()
                && page.renderer_owner_local_host_id() == update.page.owner_local_host_id()
                && page.renderer_page_id() == update.page.page_id()
                && page.renderer_agent_attachment_id() == update.attachment
            {
                // PageStateCache also rejects an older snapshot revision on
                // the same Page, including a later document.open() capture.
                page.finish_unit_runtime_page_command(completion, update.operation)
            } else {
                completion.into_unit_page_command_turn().map(drop)
            };
            if let Err(error) = applied {
                result
                    .first_error
                    .get_or_insert_with(|| error.context(update.operation));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;
    use crate::testing::TestContext;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-cleanup".into());
        browser.set_active_target_id("TID-cleanup");
        browser.attach_active_session("SID-cleanup");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank#original",
                Some("SID-cleanup"),
            )
            .await;
        context
    }

    #[tokio::test]
    async fn page_cleanup_preserves_native_error_type_and_observes_later_updates() {
        let mut context = fixture().await;
        let owner = CommandOwnerScope::capture(&context.conn, Some("SID-cleanup"));
        let page = context.conn.loaded_page_mut_for_owner(&owner).unwrap();
        let mut updates = SessionPageUpdates::default();
        updates.push(
            page,
            "controlled native policy rejection",
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "policy denied").into()),
        );
        updates.push(
            page,
            "later native cleanup",
            page.start_set_network_offline(false),
        );
        let completed = updates.wait().await;
        let result = completed.finish(context.conn.loaded_page_mut_for_owner(&owner));
        assert_eq!(
            result
                .first_error
                .as_ref()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(
            result.predecessor.is_some(),
            "a failure must not skip later admitted native acknowledgements"
        );
    }

    #[tokio::test]
    async fn page_cleanup_cannot_install_a_retired_pages_snapshot_on_its_successor() {
        let mut context = fixture().await;
        let owner = CommandOwnerScope::capture(&context.conn, Some("SID-cleanup"));
        let page = context.conn.loaded_page_mut_for_owner(&owner).unwrap();
        let original = RendererPageResidenceIdentity::from_page(page);
        let mut updates = SessionPageUpdates::default();
        updates.push(
            page,
            "old native cleanup",
            page.start_set_network_offline(false),
        );
        let completed = updates.wait().await;
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank#successor",
                Some("SID-cleanup"),
            )
            .await;
        let successor = context.conn.loaded_page_mut_for_owner(&owner).unwrap();
        assert_ne!(
            RendererPageResidenceIdentity::from_page(successor),
            original
        );
        let successor_url = successor.final_url().clone();
        let result = completed.finish(Some(successor));
        assert!(result.first_error.is_none());
        assert!(
            result.predecessor.is_some(),
            "the old turn retains its own publication fence"
        );
        assert_eq!(
            context
                .conn
                .loaded_page_mut_for_owner(&owner)
                .unwrap()
                .final_url(),
            &successor_url
        );
    }
}
