//! Native frontend commands publish from the renderer, just like Inspector.
//!
//! The pending adapter retains only cancellation and Page-cache bookkeeping.
//! Its wake-up order cannot decide when a successful reply reaches the client.
//! Internal queries deliberately keep their ordinary typed result channel.

mod node;
pub(crate) use node::{NodeReferenceParams, with_backend_node};

use anyhow::{Error, Result};
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};
use serde_json::json;

use moli_core::page::{CompletedPageCommand, PendingPageCommand};
use moli_core::{RendererNativeCommandResponseGuard, RendererNativeProtocolResponse};
use moli_core::{RendererPageCommand, RendererPageReply};

use super::command_output::CommandOutputPlan;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandDispatchContext, CommandOwnerScope,
    TargetPageResidenceIdentity,
};

pub(crate) fn unit_response(reply: Result<RendererPageReply>) -> RendererNativeProtocolResponse {
    match reply {
        Ok(RendererPageReply::Unit) => RendererNativeProtocolResponse::success(json!({})),
        Err(error) => RendererNativeProtocolResponse::error(-32000, error.to_string()),
        _ => unreachable!("native Unit command reply"),
    }
}

pub(crate) enum NativeCommandStep {
    Pending(PendingNativeCommand),
    Complete(CommandOutputPlan),
}

type UnpublishedFailureProjection =
    Box<dyn FnOnce(&CdpConnection, Error) -> CommandOutputPlan + Send>;

pub(crate) struct PendingNativeCommand {
    scope: NativeCommandFailureCleanup,
    pending: LocalBoxFuture<'static, NativeCommandCompletion>,
    unpublished_failure: Option<UnpublishedFailureProjection>,
    recovering: bool,
    _cancellation: NativeWaitCancellation,
}

pub(crate) struct CompletedNativeCommand {
    scope: NativeCommandFailureCleanup,
    result: NativeCommandCompletion,
    unpublished_failure: Option<UnpublishedFailureProjection>,
}

/// One native response authority survives cancellation and discarded ready
/// results. Its receipt owns no connection, Page or additional task.
#[derive(Clone)]
pub(crate) struct NativeCommandFailureCleanup {
    command_id: u64,
    session_id: Option<String>,
    publication: Shared<LocalBoxFuture<'static, Option<moli_core::RendererOutputFence>>>,
}

/// Cancels at wait-drop time, even if its context retains the original
/// receipt for recovery. An already claimed publication keeps that receipt.
struct NativeWaitCancellation(
    Shared<LocalBoxFuture<'static, Option<moli_core::RendererOutputFence>>>,
);

impl Drop for NativeWaitCancellation {
    fn drop(&mut self) {
        // Poll once to revoke the open authority synchronously. If the
        // producer already claimed it, recovery retains the same pending
        // receipt; this does not wait or spawn a task.
        let _ = self.0.clone().now_or_never();
    }
}

impl NativeCommandFailureCleanup {
    fn new(
        command_id: u64,
        session_id: Option<String>,
        guard: RendererNativeCommandResponseGuard,
    ) -> Self {
        Self {
            command_id,
            session_id,
            publication: guard.cancel_or_published().boxed_local().shared(),
        }
    }

    fn wait_cancellation(&self) -> NativeWaitCancellation {
        NativeWaitCancellation(self.publication.clone())
    }

    pub(crate) fn recover(self, response: CommandOutputPlan) -> PendingNativeCommand {
        let publication = self.publication.clone();
        let cancellation = self.wait_cancellation();
        PendingNativeCommand {
            scope: self,
            pending: async move {
                match publication.await {
                    Some(fence) => NativeCommandCompletion::Published(fence),
                    None => NativeCommandCompletion::AdapterFailure(response),
                }
            }
            .boxed_local(),
            unpublished_failure: None,
            recovering: true,
            _cancellation: cancellation,
        }
    }

    async fn settle(
        &self,
        result: Result<CompletedPageCommand>,
        residence: TargetPageResidenceIdentity,
    ) -> NativeCommandCompletion {
        match result {
            Ok(completed) => NativeCommandCompletion::Page {
                completed: Box::new(completed),
                residence,
            },
            Err(error) => match self.publication.clone().await {
                Some(fence) => NativeCommandCompletion::Published(fence),
                None => NativeCommandCompletion::Unpublished(error),
            },
        }
    }
}

impl PendingNativeCommand {
    fn new(
        command_id: u64,
        session_id: Option<String>,
        residence: TargetPageResidenceIdentity,
        pending: PendingPageCommand,
        guard: RendererNativeCommandResponseGuard,
    ) -> Self {
        let scope = NativeCommandFailureCleanup::new(command_id, session_id, guard);
        let wait = scope.clone();
        let cancellation = scope.wait_cancellation();
        Self {
            scope,
            pending: async move { wait.settle(pending.wait().await, residence).await }
                .boxed_local(),
            unpublished_failure: None,
            recovering: false,
            _cancellation: cancellation,
        }
    }

    /// Browser policy may outlive a rejected old attachment. This projection
    /// only runs after publication lost; it cannot replace a committed reply.
    pub(crate) fn on_unpublished_failure(
        mut self,
        project: impl FnOnce(&CdpConnection, Error) -> CommandOutputPlan + Send + 'static,
    ) -> Self {
        self.unpublished_failure = Some(Box::new(project));
        self
    }

    pub(crate) fn failure_cleanup(&self) -> NativeCommandFailureCleanup {
        self.scope.clone()
    }
    pub(crate) fn continues_after_session_detachment(&self) -> bool {
        self.recovering
    }
    pub(crate) fn command_id(&self) -> u64 {
        self.scope.command_id
    }
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.scope.session_id.as_deref()
    }

    pub(crate) async fn wait(self) -> CompletedNativeCommand {
        CompletedNativeCommand {
            scope: self.scope,
            result: self.pending.await,
            unpublished_failure: self.unpublished_failure,
        }
    }
}

/// All renderer waits, including publication settlement after an adapter
/// failure, finish before any of these results enter protocol state.
enum NativeCommandCompletion {
    Page {
        completed: Box<CompletedPageCommand>,
        residence: TargetPageResidenceIdentity,
    },
    Published(moli_core::RendererOutputFence),
    Unpublished(Error),
    AdapterFailure(CommandOutputPlan),
}

impl CompletedNativeCommand {
    pub(crate) fn command_id(&self) -> u64 {
        self.scope.command_id
    }
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.scope.session_id.as_deref()
    }

    pub(crate) fn complete(
        self,
        conn: &mut CdpConnection,
        context: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        match self.result {
            NativeCommandCompletion::Page {
                completed,
                residence,
            } => {
                let output = conn.settle_page_command_turn_for_owner(&residence, *completed);
                let completion = context.consume_renderer_command_turn_output(output);
                assert!(completion.native_response_was_published());
                CommandOutputPlan::default()
            }
            NativeCommandCompletion::Published(fence) => {
                context.set_renderer_output_predecessor(fence);
                CommandOutputPlan::default()
            }
            NativeCommandCompletion::Unpublished(error) => match self.unpublished_failure {
                Some(project) => project(conn, error),
                None => CommandOutputPlan::error(-32000, error),
            },
            NativeCommandCompletion::AdapterFailure(response) => response,
        }
    }
}

pub(crate) fn start(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    operation: RendererPageCommand,
    project: impl FnOnce(Result<RendererPageReply>) -> RendererNativeProtocolResponse + Send + 'static,
) -> NativeCommandStep {
    start_operation(
        conn,
        cmd,
        moli_core::RendererNativeOperation::new(operation, project),
    )
}

pub(crate) fn start_operation(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    operation: moli_core::RendererNativeOperation,
) -> NativeCommandStep {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(residence) = conn.target_page_residence_identity_for_owner(&owner) else {
        return NativeCommandStep::Complete(CommandOutputPlan::error(-32000, "NoDocumentLoaded"));
    };
    let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner) {
        Ok(page) => page,
        Err(error) => return NativeCommandStep::Complete(CommandOutputPlan::error(-32000, error)),
    };
    let command_id = cmd.id.expect("a native frontend command has a call id");
    match page.start_cdp_call(command_id, cmd.session_id, operation) {
        Ok((pending, guard)) => NativeCommandStep::Pending(PendingNativeCommand::new(
            command_id,
            cmd.session_id.map(str::to_owned),
            residence,
            pending,
            guard,
        )),
        Err(error) => {
            NativeCommandStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

/// Admit only frontend calls with an accessible document and renderer.
/// Return the checked attachment so preparation can retain its replay identity.
/// Enqueue revalidates access after preparation, which may update Browser policy.
pub(crate) fn frontend_attachment(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<moli_page_types::RendererAgentAttachmentId> {
    if cmd.id.is_none()
        || cmd.terminal_response_delivery()
            != moli_page_types::RendererInspectorResponseDelivery::SessionSink
    {
        return None;
    }
    conn.loaded_page_mut_for_protocol_access_for_owner(&CommandOwnerScope::capture(
        conn,
        cmd.session_id,
    ))
    .ok()?
    .renderer_agent_attachment_id()
}

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let start: fn(&mut CdpConnection, &Cmd<'_>) -> Option<NativeCommandStep> =
        match cmd.method.split_once('.')?.0 {
            "DOMDebugger" => super::dom_debugger::native::try_start,
            "Autofill" => super::autofill::try_start_native_command,
            "WebMCP" => super::web_mcp::try_start_native_command,
            "Page" => super::page::native::try_start,
            "DOM" => super::dom::try_start_native_command,
            "DOMSnapshot" => super::dom_snapshot::try_start_native_command,
            "CSS" => super::css::native::try_start,
            "Accessibility" => super::accessibility::native::try_start,
            "Runtime" => super::runtime::try_start_native_command,
            _ => return None,
        };
    frontend_attachment(conn, cmd)?;
    start(conn, cmd)
}

/// Apply Browser mirrors at the same ordered boundary as the response. The
/// Snapshot results survive retirement; document caches and node events are
/// valid only for the exact attachment that produced them. Runtime.enable also
/// commits session state and projects its frozen replay here, or fails the
/// terminal if its original owner has retired.
pub(crate) fn project_terminal_for_owner(
    conn: &mut CdpConnection,
    publication_owner: Option<&CommandOwnerScope>,
    mut terminal: moli_core::RendererNativeCommandTerminal,
) -> Vec<BackgroundProtocolEvent> {
    let owner = publication_owner.map(|owner| match terminal.session.wire_session_id() {
        Some(session) => CommandOwnerScope::for_session(session),
        None => owner.clone(),
    });
    let owner = owner.filter(|owner| {
        conn.runtime_session_owner_slot_for_owner(owner)
            .is_some_and(|slot| {
                slot.has_loaded_page()
                    && slot
                        .current_renderer_attachment()
                        .is_some_and(|attachment| attachment.id() == terminal.attachment_id)
            })
    });
    let response = &mut terminal.reply;
    let mut events = Vec::new();
    if owner.is_none() {
        response.notifications.clear();
    }
    for update in std::mem::take(&mut response.state_updates) {
        match (owner.as_ref(), update) {
            (
                owner,
                moli_core::RendererNativeProtocolStateUpdate::RuntimeBinding {
                    binding,
                    remove,
                    output,
                },
            ) => {
                match super::runtime::project_binding_terminal(
                    conn, owner, binding, remove, *output,
                ) {
                    Ok(replay) => events.extend(replay),
                    Err(message) => {
                        response.notifications.clear();
                        response.result = Err(moli_core::RendererNativeProtocolError {
                            code: -32000,
                            message: message.to_string(),
                        });
                    }
                }
            }
            (
                owner,
                moli_core::RendererNativeProtocolStateUpdate::RuntimeSubscription {
                    enabled,
                    output,
                },
            ) => {
                match super::runtime::project_subscription_terminal(conn, owner, enabled, *output) {
                    Ok(replay) => events.extend(replay),
                    Err(message) => {
                        response.notifications.clear();
                        response.result = Err(moli_core::RendererNativeProtocolError {
                            code: -32000,
                            message: message.to_string(),
                        });
                    }
                }
            }
            (Some(owner), moli_core::RendererNativeProtocolStateUpdate::WebMcpEnabled(enabled)) => {
                conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
                    state.web_mcp_enabled = enabled
                });
            }
            (
                Some(owner),
                moli_core::RendererNativeProtocolStateUpdate::RemoteObjects { object_group },
            ) => {
                let Ok(result) = &response.result else {
                    continue;
                };
                if let Some(group) = object_group {
                    conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner, result, &group,
                    );
                } else {
                    conn.register_runtime_remote_object_ids_from_value_for_owner(owner, result);
                }
            }
            (
                Some(owner),
                moli_core::RendererNativeProtocolStateUpdate::DomRemoteObjectNode {
                    object_id,
                    node,
                },
            ) => {
                super::dom::cache_dom_remote_object_node_for_owner(conn, owner, object_id, node);
            }
            (None, _) => {}
        }
    }
    events.extend(project_terminal(terminal));
    events
}

/// Bounded projection of a terminal already committed by its renderer owner.
/// It intentionally needs no live Page: replacing a document cannot change
/// an earlier snapshot or steal the original frontend call's response.
pub(crate) fn project_terminal(
    terminal: moli_core::RendererNativeCommandTerminal,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    for notification in terminal.reply.notifications {
        let session = terminal.session.wire_session_id();
        events.push(match notification {
            moli_core::RendererNativeProtocolNotification::Json(mut notification) => {
                if let Some(session) = session {
                    notification["sessionId"] = json!(session);
                }
                BackgroundProtocolEvent::immediate(notification)
            }
            moli_core::RendererNativeProtocolNotification::DomSetChildNodes {
                parent_node_id,
                nodes,
            } => BackgroundProtocolEvent::dom_set_child_nodes(session, parent_node_id, nodes),
        });
    }
    let mut message = json!({ "id": terminal.command_id.get() });
    match terminal.reply.result {
        Ok(result) => message["result"] = result,
        Err(error) => message["error"] = json!({ "code": error.code, "message": error.message }),
    }
    if let Some(session) = terminal.session.wire_session_id() {
        message["sessionId"] = json!(session);
    }
    events.push(BackgroundProtocolEvent::immediate(message));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_wait_drop_cancels_before_a_later_renderer_claim() {
        use crate::conn::BrowserContext;
        use crate::testing::TestContext;
        struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                if let Some(release) = self.0.take() {
                    let _ = release.send(());
                }
            }
        }
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-native-before-claim".into());
        browser.set_active_target_id("TID-native-before-claim");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner("about:blank", None)
            .await;
        let (entered, before_claim) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (pending, guard) = context.with_fixture(|conn| {
            conn.loaded_page_mut_for_protocol_access(None)
                .unwrap()
                .start_cdp_call(
                    84,
                    None,
                    moli_core::RendererNativeOperation::then(
                        RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            moli_core::RendererNativeOperationStep::Complete(
                                RendererNativeProtocolResponse::success(json!({})),
                            )
                        },
                    ),
                )
                .unwrap()
        });
        let release = ReleaseOnDrop(Some(release));
        let scope = NativeCommandFailureCleanup::new(84, None, guard);
        let cancellation = scope.wait_cancellation();
        tokio::time::timeout(std::time::Duration::from_secs(5), before_claim)
            .await
            .unwrap()
            .unwrap();
        // Keep both the recovery scope and backend receipt alive. Only dropping
        // the supervised wait must revoke publication, before recovery runs.
        drop(cancellation);
        drop(release);
        assert!(
            pending.wait().await.is_err(),
            "a producer completing after cancellation cannot publish a terminal"
        );
        let completed = scope
            .recover(CommandOutputPlan::error(-32000, "cancelled wait"))
            .wait()
            .await;
        let mut dispatch = CommandDispatchContext::default();
        let plan = context.with_fixture(|conn| completed.complete(conn, &mut dispatch));
        assert!(dispatch.take_renderer_output_predecessor().is_none());
        let mut messages = Vec::new();
        plan.emit_into(&mut messages, Some(84), None);
        assert_eq!(
            messages,
            vec![json!({"id":84,"error":{"code":-32000,"message":"cancelled wait"}})]
        );
    }

    #[tokio::test]
    async fn unpublished_native_failure_preserves_its_typed_cause() {
        use crate::conn::{BrowserContext, NoDocumentLoaded};
        use crate::testing::TestContext;
        use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};

        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-native-failure".into());
        browser.set_active_target_id("TID-native-failure");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner("about:blank", None)
            .await;
        let residence = context.with_fixture(|conn| {
            conn.target_page_residence_identity_for_owner(&CommandOwnerScope::capture(conn, None))
                .unwrap()
        });
        let (_unsubmitted, guard) = moli_core::RendererCdpCall::new(
            FrontendCommandId::new(81),
            DevToolsSessionKey::Primary,
            RendererAgentAttachmentId::allocate(),
            moli_core::RendererNativeOperation::new(
                RendererPageCommand::ChildFrameTreeSnapshot,
                |_| RendererNativeProtocolResponse::success(json!({})),
            ),
        );
        let scope = NativeCommandFailureCleanup::new(81, None, guard);
        let result = scope
            .settle(
                Err(Error::new(NoDocumentLoaded).context("native query failed")),
                residence,
            )
            .await;
        let NativeCommandCompletion::Unpublished(error) = &result else {
            panic!("an unsubmitted call cannot publish");
        };
        assert!(error.is::<NoDocumentLoaded>());
        let completed = CompletedNativeCommand {
            scope,
            result,
            unpublished_failure: Some(Box::new(|_, error| {
                assert!(
                    error.is::<NoDocumentLoaded>(),
                    "the callback retains the original cause"
                );
                CommandOutputPlan::success()
            })),
        };
        let plan = context
            .with_fixture(|conn| completed.complete(conn, &mut CommandDispatchContext::default()));
        let mut messages = Vec::new();
        plan.emit_into(&mut messages, Some(81), None);
        assert_eq!(messages, vec![json!({"id":81,"result":{}})]);
    }

    #[tokio::test]
    async fn claimed_native_publication_survives_a_cancelled_error_wait() {
        use crate::conn::{BrowserContext, CdpCommandTaskStep, NoDocumentLoaded, ParsedCdpCommand};
        use crate::testing::TestContext;
        use futures_util::poll;

        struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                if let Some(release) = self.0.take() {
                    let _ = release.send(());
                }
            }
        }
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-native-receipt".into());
        browser.set_active_target_id("TID-native-receipt");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner("about:blank", None)
            .await;
        let residence = context.with_fixture(|conn| {
            conn.target_page_residence_identity_for_owner(&CommandOwnerScope::capture(conn, None))
                .unwrap()
        });
        let (entered, claimed) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (pending, guard) = context.with_fixture(|conn| {
            conn.loaded_page_mut_for_protocol_access(None)
                .unwrap()
                .start_cdp_call(
                    82,
                    None,
                    moli_core::RendererNativeOperation::new(
                        RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            RendererNativeProtocolResponse::success(json!({"winner":"renderer"}))
                        },
                    ),
                )
                .unwrap()
        });
        let scope = NativeCommandFailureCleanup::new(82, None, guard);
        let owner = context.state;
        let release = ReleaseOnDrop(Some(release));
        let handle = owner.handle();
        tokio::time::timeout(std::time::Duration::from_secs(5), claimed)
            .await
            .unwrap()
            .unwrap();
        let mut waiting = Box::pin(scope.settle(Err(Error::new(NoDocumentLoaded)), residence));
        assert!(
            poll!(waiting.as_mut()).is_pending(),
            "publication owns the receipt but has not released its fence"
        );
        drop(waiting);
        let recovery = scope.recover(CommandOutputPlan::error(-32000, "cancelled adapter wait"));
        let mut recovery = Box::pin(recovery.wait());
        assert!(
            poll!(recovery.as_mut()).is_pending(),
            "recovery must wait for the same claimed publication"
        );
        let version =
            ParsedCdpCommand::parse_str(r#"{"id":83,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| state.start_parsed_command_dispatch_with_context(
                    &version,
                    &mut CommandDispatchContext::default()
                ))
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        drop(release);
        let completed = recovery.await;
        let native = pending.wait().await.unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let plan = completed.complete(&mut CdpConnection::new(), &mut dispatch);
        assert_eq!(
            dispatch
                .take_renderer_output_predecessor()
                .unwrap()
                .cursor(),
            native.renderer_output_predecessor().unwrap().cursor(),
            "recovery retains the renderer terminal's exact publication cursor",
        );
        let mut messages = Vec::new();
        plan.emit_into(&mut messages, Some(82), None);
        assert!(
            messages.is_empty(),
            "the claimed renderer response wins over adapter cancellation"
        );
    }

    #[test]
    fn retired_native_terminal_keeps_response_and_discards_document_side_effects() {
        use moli_core::{
            RendererNativeCommandTerminal, RendererNativeProtocolNotification,
            RendererNativeProtocolStateUpdate,
        };
        use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};
        let ctx = crate::testing::TestContext::new();
        let mut response = RendererNativeProtocolResponse::success(
            json!({"object": {"objectId": "retired-object"}}),
        );
        response
            .notifications
            .push(RendererNativeProtocolNotification::DomSetChildNodes {
                parent_node_id: 3,
                nodes: vec![json!({"nodeId": 7})],
            });
        response
            .state_updates
            .push(RendererNativeProtocolStateUpdate::DomRemoteObjectNode {
                object_id: "retired-object".to_owned(),
                node: json!({"nodeId": 7}),
            });
        let messages = ctx.with_fixture(|conn| {
            project_terminal_for_owner(
                conn,
                None,
                RendererNativeCommandTerminal {
                    command_id: FrontendCommandId::new(42),
                    session: DevToolsSessionKey::Attached("retired-session".to_owned()),
                    attachment_id: RendererAgentAttachmentId::allocate(),
                    reply: response,
                },
            )
        });
        assert_eq!(
            messages.len(),
            1,
            "a retired document cannot emit node notifications"
        );
        assert_eq!(
            messages.into_iter().next().unwrap().into_parts().0,
            json!({
                "id": 42, "sessionId": "retired-session", "result": {"object": {"objectId": "retired-object"}},
            })
        );
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context.is_none(),
                "projection cannot recreate a retired owner"
            )
        });
    }

    #[test]
    fn native_dom_notification_keeps_automation_sidecar_before_terminal() {
        use moli_core::{RendererNativeCommandTerminal, RendererNativeProtocolNotification};
        use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};
        let nodes = vec![json!({"nodeId": 7, "nodeName": "BUTTON"})];
        let mut response = RendererNativeProtocolResponse::success(json!({"nodeId": 7}));
        response
            .notifications
            .push(RendererNativeProtocolNotification::DomSetChildNodes {
                parent_node_id: 3,
                nodes: nodes.clone(),
            });
        let mut events = project_terminal(RendererNativeCommandTerminal {
            command_id: FrontendCommandId::new(42),
            session: DevToolsSessionKey::Attached("SID-native".to_owned()),
            attachment_id: RendererAgentAttachmentId::allocate(),
            reply: response,
        })
        .into_iter();
        let (event, sidecar) = events.next().unwrap().into_parts();
        assert_eq!(
            event.as_json().expect("scalar protocol fixture")["method"],
            "DOM.setChildNodes"
        );
        assert_eq!(
            event.as_json().expect("scalar protocol fixture")["sessionId"],
            "SID-native"
        );
        let Some(crate::automation::AutomationEvent::DomSetChildNodes(sidecar)) = sidecar else {
            panic!("native DOM notifications retain their typed automation event");
        };
        assert_eq!(sidecar.parent_node_id, 3);
        assert_eq!(sidecar.nodes, nodes);
        let (terminal, _) = events.next().unwrap().into_parts();
        assert_eq!(
            terminal.as_json().expect("scalar protocol fixture")["id"],
            42
        );
        assert_eq!(
            terminal.as_json().expect("scalar protocol fixture")["sessionId"],
            "SID-native"
        );
        assert_eq!(
            terminal.as_json().expect("scalar protocol fixture")["result"]["nodeId"],
            7
        );
        assert!(events.next().is_none());
    }
}
