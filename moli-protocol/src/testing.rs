//! Test helpers for exercising the CDP connection and its scheduler.
//!
//! `TestContext` owns a `CdpConnection`, accepts JSON commands, and exposes
//! focused assertions over emitted results, events, and errors.

use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;

use super::conn::{
    BackgroundProtocolEvent, CdpCommandTaskStep, CdpConnection, CdpInitialStoragePartition,
    CdpSchedulerEvent, CommandDispatchContext, CommandResponseFlushPermit, ParsedCdpCommand,
    PendingCdpCommandDispatch, RuntimeInspectorResponseReady,
};
use crate::devtools_runtime::{DevToolsCommand, DevToolsCommandResult, DevToolsError};
use crate::domains::activity::{
    ProtocolSchedulerWork, ProtocolSchedulerWorkKind, RendererCommandResponseCompletion,
    RendererCommandResponseOrder, RendererCommandResponsePermit,
};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererOutputFence, RendererOutputStreamControl,
    RendererOutputStreamIdentity, RendererOutputTransportMessage, runtime::NavigationRuntimeConfig,
};
use moli_fetch::FetchConfig;
#[cfg(test)]
use std::net::SocketAddr;
#[cfg(test)]
use tokio::io::AsyncWriteExt;
#[cfg(test)]
use tokio::net::TcpListener;
#[cfg(test)]
use tokio::task::JoinHandle;

#[cfg(test)]
// Full-workspace runs execute CPU-heavy crypto vectors beside protocol tests.
// Keep this as a bounded diagnostic guard, but leave enough headroom for a
// real renderer-owner wake to be scheduled under that contention.
const TEST_SCHEDULER_INPUT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TestContext {
    pub conn: CdpConnection,
    pub sent: Vec<Value>,
    pending_runtime_deferred_replies: VecDeque<PendingTestRuntimeDeferredReply>,
    pending_protocol_scheduler_work: VecDeque<ProtocolSchedulerWork>,
    renderer_command_response_order: RendererCommandResponseOrder,
    runtime_inspector_response_ready_rx:
        tokio::sync::mpsc::UnboundedReceiver<RuntimeInspectorResponseReady>,
    renderer_publication_rx: moli_core::RendererOutputTransportReceiver,
    background_event_tx: tokio::sync::mpsc::UnboundedSender<BackgroundProtocolEvent>,
    background_event_rx: tokio::sync::mpsc::UnboundedReceiver<BackgroundProtocolEvent>,
    background_navigation_completion_tx:
        tokio::sync::mpsc::UnboundedSender<crate::domains::page::BackgroundNavigationCompletion>,
    background_navigation_completion_rx:
        tokio::sync::mpsc::UnboundedReceiver<crate::domains::page::BackgroundNavigationCompletion>,
    background_navigation_scheduler_enabled: bool,
    browser_event_rx: Option<moli_core::browser::BrowserEventReceiver>,
}

struct PendingTestRuntimeDeferredReply {
    pending: PendingCdpCommandDispatch,
    command_context: CommandDispatchContext,
    renderer_response_permit: Option<RendererCommandResponsePermit>,
}

impl CdpConnection {
    /// Builds and commits a test document through the exact Browser owner
    /// selected by `session_id`. The returned fence is the last renderer
    /// publication that must cross protocol ingress before a later command.
    pub(crate) async fn install_navigation_fixture_for_session_owner_for_test(
        &mut self,
        raw_url: &str,
        session_id: Option<&str>,
    ) -> Option<RendererOutputFence> {
        self.commit_declared_session_fixtures_for_test();
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.install_navigation_fixture_for_owner_for_test(raw_url, &owner)
            .await
    }

    pub(crate) async fn install_navigation_fixture_for_owner_for_test(
        &mut self,
        raw_url: &str,
        owner: &crate::conn::CommandOwnerScope,
    ) -> Option<RendererOutputFence> {
        let token = self
            .start_document_navigation_for_owner(owner, crate::domains::page::LOADER_ID.to_owned())
            .expect("navigation fixture requires a live Browser owner");
        let navigation = self
            .load_navigation_via_runtime_for_owner_async(owner, raw_url)
            .await
            .expect("navigation fixture should load");
        self.install_loaded_navigation_fixture_for_owner_for_test(navigation, owner, token)
            .await
    }

    async fn install_loaded_navigation_fixture_for_owner_for_test(
        &mut self,
        navigation: crate::conn::LoadedNavigation<crate::conn::PreparedDocumentNavigation>,
        owner: &crate::conn::CommandOwnerScope,
        token: crate::conn::NavigationId,
    ) -> Option<RendererOutputFence> {
        let (_, target_id) = self
            .target_owner_identity_for_owner(owner)
            .expect("navigation fixture requires an installed browser context");
        let target_id = target_id.expect("navigation fixture requires an exact target");
        let renderer_output_predecessor = navigation.renderer_output_predecessor;
        let page_commit = self
            .commit_loaded_navigation(navigation.page)
            .expect("navigation fixture Page commit must succeed");
        let projection_fence = page_commit
            .inspection_projection
            .expect("navigation fixture must rebind its renderer inspection endpoint");
        assert!(
            page_commit
                .committed_document_post_response_continuation
                .is_none(),
            "lifecycle-target fixture must not retain a DocumentCommit response gate"
        );
        page_commit.previous_document_retirement.close().await;
        let (binding, _) = self.project_committed_document_lifecycle_for_owner(
            owner,
            page_commit.lifecycle,
            Some(token),
            target_id,
            crate::domains::page::LOADER_ID.to_owned(),
        );
        let binding =
            binding.expect("navigation fixture must install its exact renderer Document binding");
        let finished =
            self.publish_document_projection_fence_for_owner(owner, &binding, projection_fence);
        assert!(
            finished.released_output.is_empty(),
            "fixture output is ingested after the Document binding"
        );
        assert!(
            finished.renderer_call_replacements.is_none(),
            "fixture must not replace in-flight renderer calls"
        );
        assert_eq!(
            self.target_root_document_lifecycle_identity_for_owner(owner),
            Some(binding.renderer_document_identity()),
            "navigation fixture must retain its exact renderer Document binding"
        );
        renderer_output_predecessor
    }
}

impl PendingTestRuntimeDeferredReply {
    fn new(
        pending: PendingCdpCommandDispatch,
        command_context: CommandDispatchContext,
        renderer_response_permit: Option<RendererCommandResponsePermit>,
    ) -> Self {
        Self {
            pending,
            command_context,
            renderer_response_permit,
        }
    }

    fn command_id(&self) -> Option<u64> {
        self.pending.command_id()
    }
}

enum TestSchedulerWork {
    ProtocolEvents(Vec<BackgroundProtocolEvent>),
    SchedulerEvents(Vec<CdpSchedulerEvent>),
    BackgroundEvent(BackgroundProtocolEvent),
    BackgroundNavigationCompletion(crate::domains::page::BackgroundNavigationCompletion),
    RuntimeDeferredReplyReady(RuntimeInspectorResponseReady),
    RendererPublication(RendererOutputTransportMessage),
    ReleaseRendererResponsePermit(RendererCommandResponsePermit),
    CancelRendererResponsePermit(RendererCommandResponsePermit),
}

#[must_use = "the held test command response must be released exactly once"]
pub(crate) struct TestCommandResponseFlushPermit {
    response_flush: CommandResponseFlushPermit,
    renderer_response_permit: Option<RendererCommandResponsePermit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestSchedulerTurnOutcome {
    Idle,
    Processed(TestSchedulerInputKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestSchedulerInputKind {
    BackgroundEvent,
    BackgroundNavigationCompletion,
    RuntimeDeferredReply,
    RendererPublication,
    NativeDownload,
}

// Fixture handlers still install other native projections explicitly. Downloads
// use the production Browser stream, never a separate transfer observer task.
async fn recv_native_download_input(
    receiver: &mut Option<moli_core::browser::BrowserEventReceiver>,
) -> Result<moli_core::browser::BrowserEvent, tokio::sync::broadcast::error::RecvError> {
    use moli_core::browser::BrowserEvent;
    let Some(receiver) = receiver.as_mut() else {
        return std::future::pending().await;
    };
    loop {
        if let event @ (BrowserEvent::DownloadCreated(_) | BrowserEvent::DownloadUpdated(_)) =
            receiver.recv().await?.event
        {
            return Ok(event);
        }
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

fn set_test_target_discovery(conn: &mut CdpConnection, enabled: bool) {
    conn.set_root_target_discovery_enabled(enabled);
}

fn real_layout_test_runtime_config(
    optional_resource_fetch_mask: OptionalResourceFetchMask,
) -> NavigationRuntimeConfig {
    NavigationRuntimeConfig::new(
        FetchConfig::default(),
        optional_resource_fetch_mask,
        true,
        LayoutPolicy::OnDemand,
    )
}

pub(crate) fn real_layout_test_connection() -> CdpConnection {
    crate::test_support::connection_with_config(
        CdpInitialStoragePartition::memory(),
        real_layout_test_runtime_config(OptionalResourceFetchMask::NONE),
    )
}

impl TestContext {
    /// Build the default CDP test harness used by older internal unit tests.
    ///
    /// Historically those tests treated `Target.targetCreated` as a baseline
    /// event after `Target.createTarget`. Real CDP clients only receive target
    /// discovery events after enabling discovery, so keep that convenience in
    /// `TestContext` instead of changing `crate::test_support::connection()`.
    pub fn new() -> Self {
        Self::new_with_target_discovery(true)
    }

    /// Build a CDP test harness with explicit Target discovery state.
    ///
    /// Use `true` for internal tests that assert Target-domain event payloads
    /// without spelling out the setup command. Use `false` for Chromium-parity
    /// tests that need to verify the default protocol behavior before
    /// `Target.setDiscoverTargets(true)` is called.
    pub fn new_with_target_discovery(target_discovery_enabled: bool) -> Self {
        let mut conn = real_layout_test_connection();
        set_test_target_discovery(&mut conn, target_discovery_enabled);
        Self::from_conn(conn)
    }

    pub fn new_with_layout_policy(layout_policy: LayoutPolicy) -> Self {
        let conn = crate::test_support::connection_with_config(
            CdpInitialStoragePartition::memory(),
            NavigationRuntimeConfig::new(
                FetchConfig::default(),
                OptionalResourceFetchMask::NONE,
                true,
                layout_policy,
            ),
        );
        Self::from_conn(conn)
    }

    pub fn new_with_target_discovery_and_image_fetch(
        target_discovery_enabled: bool,
        image_fetch_enabled: bool,
    ) -> Self {
        Self::new_with_target_discovery_and_optional_resource_fetch_mask(
            target_discovery_enabled,
            if image_fetch_enabled {
                OptionalResourceFetchMask::IMAGE
            } else {
                OptionalResourceFetchMask::NONE
            },
        )
    }

    pub fn new_with_target_discovery_and_optional_resource_fetch_mask(
        target_discovery_enabled: bool,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
    ) -> Self {
        let mut conn = crate::test_support::connection_with_config(
            CdpInitialStoragePartition::memory(),
            real_layout_test_runtime_config(optional_resource_fetch_mask),
        );
        set_test_target_discovery(&mut conn, target_discovery_enabled);
        Self::from_conn(conn)
    }

    pub fn from_conn(mut conn: CdpConnection) -> Self {
        let (renderer_publication_tx, renderer_publication_rx) =
            moli_core::renderer_output_transport_channel();
        let runtime_inspector_response_ready_rx = conn
            .bind_runtime_inspector_response_ready()
            .expect("test scheduler must own the connection's completion ingress");
        let (background_event_tx, background_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (background_navigation_completion_tx, background_navigation_completion_rx) =
            tokio::sync::mpsc::unbounded_channel();
        conn.set_renderer_publication_sender(renderer_publication_tx);
        Self {
            conn,
            sent: Vec::new(),
            pending_runtime_deferred_replies: VecDeque::new(),
            pending_protocol_scheduler_work: VecDeque::new(),
            renderer_command_response_order: RendererCommandResponseOrder::default(),
            runtime_inspector_response_ready_rx,
            renderer_publication_rx,
            background_event_tx,
            background_event_rx,
            background_navigation_completion_tx,
            background_navigation_completion_rx,
            background_navigation_scheduler_enabled: false,
            browser_event_rx: None,
        }
    }

    /// Enables the same asynchronous navigation channels used by the socket
    /// scheduler.
    ///
    /// Most protocol unit tests intentionally dispatch a domain command to
    /// completion without owning an actor. Chromium-ordering and lifecycle
    /// tests must opt into this production boundary: Page.navigate emits its
    /// start/early response first, while the later renderer Page commit arrives
    /// independently and is joined by its exact concrete-output cursor.
    pub(crate) fn enable_background_navigation_scheduler_for_test(&mut self) {
        if self.background_navigation_scheduler_enabled {
            return;
        }
        self.conn
            .set_background_event_sender(self.background_event_tx.clone());
        self.conn.set_background_navigation_completion_sender(
            self.background_navigation_completion_tx.clone(),
        );
        self.browser_event_rx = Some(self.conn.subscribe_browser_events().unwrap().1);
        self.background_navigation_scheduler_enabled = true;
    }

    /// Loads and installs one production-shaped navigation fixture for the
    /// exact Page target addressed by `session_id`.
    ///
    /// Tests that expect owner-produced lifecycle or child-frame output must
    /// not insert a bare `Page` into the runtime slot: production installs the
    /// renderer Page and its exact root-Document lifecycle binding together.
    /// Keeping that invariant in one helper prevents protocol tests from
    /// accidentally depending on a state that the real navigation path never
    /// exposes.
    pub(crate) async fn install_navigation_fixture_for_session_owner(
        &mut self,
        raw_url: &str,
        session_id: Option<&str>,
    ) {
        let predecessor = self
            .conn
            .install_navigation_fixture_for_session_owner_for_test(raw_url, session_id)
            .await;
        if let Some(predecessor) = predecessor {
            self.route_renderer_output_predecessor_before_command_response(predecessor)
                .await;
        }
    }

    /// Installs a fully routed Document as test setup without exposing that
    /// fixture's lifecycle as output of the next command under test.
    pub(crate) async fn install_quiet_navigation_fixture_for_session_owner(
        &mut self,
        raw_url: &str,
        session_id: Option<&str>,
    ) {
        let sent_start = self.sent.len();
        self.install_navigation_fixture_for_session_owner(raw_url, session_id)
            .await;
        self.fence_and_discard_navigation_fixture_output(sent_start, session_id)
            .await;
    }

    /// Installs an in-memory response through the same target/Page ownership
    /// transaction as a real navigation.
    ///
    /// Tests that need a non-fetchable origin (for example an IndexedDB
    /// fixture at `https://example.test`) use this entry point. Building the
    /// renderer Page first and assigning it directly to `TargetRuntimeSlot`
    /// skips the concrete-output owner binding and is not a state production
    /// can expose.
    pub(crate) async fn install_buffered_navigation_fixture_for_session_owner(
        &mut self,
        requested_url: url::Url,
        response_body: String,
        session_id: Option<&str>,
    ) {
        self.conn.commit_declared_session_fixtures_for_test();
        let owner = crate::conn::CommandOwnerScope::capture(&self.conn, session_id);
        let token = self
            .conn
            .start_document_navigation_for_owner(&owner, crate::domains::page::LOADER_ID.to_owned())
            .expect("navigation fixture requires a live Browser owner");
        let navigation = self
            .conn
            .build_loaded_navigation_from_buffered_response_for_session_owner_async(
                session_id,
                requested_url,
                "GET".into(),
                Vec::new(),
                200,
                Vec::new(),
                response_body,
            )
            .await
            .expect("buffered navigation fixture should build");
        let owner = crate::conn::CommandOwnerScope::capture(&self.conn, session_id);
        let predecessor = self
            .conn
            .install_loaded_navigation_fixture_for_owner_for_test(navigation, &owner, token)
            .await;
        if let Some(predecessor) = predecessor {
            // Production does not expose a completed navigation response until
            // the Page-creation cursor has crossed ordered protocol ingress.
            // Mirror that boundary here so a later enable command observes
            // the already-loaded target tail instead of racing publications
            // that merely happen to remain queued in the test transport.
            self.route_renderer_output_predecessor_before_command_response(predecessor)
                .await;
        }
    }

    async fn fence_and_discard_navigation_fixture_output(
        &mut self,
        sent_start: usize,
        session_id: Option<&str>,
    ) {
        // The build reply may precede the renderer's load continuation. First
        // observe protocol-visible load on this exact Document, then fence its
        // published tail; native progress alone does not drain protocol ingress.
        let owner = crate::conn::CommandOwnerScope::capture(&self.conn, session_id);
        let document = self
            .conn
            .resolve_browser_document_for_owner(&owner)
            .expect("navigation fixture must retain its committed Document");
        self.wait_until_scheduler_state("navigation fixture Document load", |conn| {
            assert_eq!(
                conn.resolve_browser_document_for_owner(&owner),
                Ok(document)
            );
            conn.renderer_document_lifecycle_visible_state_for_session_owner(session_id)
                .is_some_and(|(_, snapshot)| snapshot.load.is_some())
        })
        .await;
        let completed = self
            .conn
            .start_document_diagnostics_snapshot(document)
            .expect("navigation fixture Document must accept a diagnostics barrier")
            .wait()
            .await;
        let predecessor = completed.renderer_output_predecessor();
        self.conn
            .finish_document_diagnostics_snapshot(completed)
            .expect("navigation fixture diagnostics barrier must finish on the same Document");
        if let Some(predecessor) = predecessor {
            self.route_renderer_output_predecessor_before_command_response(predecessor)
                .await;
        }
        self.sent.truncate(sent_start);
    }

    /// Feed a JSON-serialisable message through the async CDP entrypoint and
    /// route the scheduler work directly requested by that command. Tests
    /// waiting for later external lifecycle input must use the event-wait
    /// helpers, which consume one renderer wake or runtime reply at a time.
    pub async fn process_async(&mut self, msg: impl serde::Serialize) {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        let command_id = command.request().id();
        let session_id = command.request().session_id().map(str::to_owned);
        let command_awaits_promise = match command.request().method() {
            "Runtime.awaitPromise" => true,
            "Runtime.evaluate" | "Runtime.callFunctionOn" | "Runtime.runScript" => command
                .request()
                .params()
                .and_then(|params| params.get("awaitPromise"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            _ => false,
        };
        let response_start = self.sent.len();
        Box::pin(self.process_parsed_command_like_scheduler(&command, true)).await;
        Box::pin(self.route_ready_test_command_response(command_id, response_start)).await;
        if self
            .conn
            .renderer_runtime_command_cause_for_frontend(session_id.as_deref(), command_id)
            .is_some()
            && !(command_awaits_promise
                && self
                    .conn
                    .has_pending_inspector_awaits_for_session_owner(session_id.as_deref()))
            && !self
                .pending_runtime_deferred_replies
                .iter()
                .any(|pending| pending.command_id() == Some(command_id))
        {
            Box::pin(self.wait_for_test_command_response(command_id, response_start)).await;
        }
    }

    /// Dispatch one command and keep running the real scheduler inputs until
    /// that command's response is routed. This mirrors Chromium's synchronous
    /// DevTools test client without draining unrelated page work to idle.
    pub async fn process_and_wait_for_response_async(&mut self, msg: impl serde::Serialize) {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        let command_id = command.request().id();
        let response_start = self.sent.len();
        Box::pin(self.process_parsed_command_like_scheduler(&command, true)).await;
        Box::pin(self.wait_for_test_command_response(command_id, response_start)).await;
    }

    #[cfg(test)]
    pub(crate) fn enable_page_events_for_test(&mut self, session_id: Option<&str>) {
        assert!(
            self.conn
                .set_page_domain_enabled_for_session_owner(session_id, true),
            "test Page subscription requires a loaded target session owner"
        );
    }

    #[cfg(test)]
    pub(crate) fn enable_dom_events_for_test(&mut self, session_id: Option<&str>) {
        assert!(
            self.conn
                .with_target_devtools_session_state_for_session_mut(session_id, |state| {
                    state.dom_session_state.enabled = true;
                })
                .is_some(),
            "test DOM subscription requires a loaded target session owner"
        );
    }

    /// Wait for and remove one protocol message produced by a real scheduler
    /// input. It does not synthesize a capture or ask a command path to advance
    /// renderer work: it only routes renderer publications and deferred
    /// inspector replies already published by the production owner scheduler.
    #[cfg(test)]
    pub(crate) async fn wait_for_scheduler_message(
        &mut self,
        description: &str,
        mut matches: impl FnMut(&Value) -> bool,
    ) -> Value {
        let message = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            loop {
                if let Some(position) = self.sent.iter().position(&mut matches) {
                    return self.sent.remove(position);
                }
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "test scheduler lost all external input while waiting for {description}"
                );
            }
        })
        .await;

        match message {
            Ok(message) => message,
            Err(_) => panic!(
                "timed out waiting for {description} from a real scheduler input; sent={:?}",
                self.sent
            ),
        }
    }

    /// Wait until routing real scheduler input makes connection state satisfy
    /// `predicate`. This observes owner-published wakes only; it does not
    /// synthesize a capture or ask protocol code to advance renderer lifecycle.
    #[cfg(test)]
    pub(crate) async fn wait_until_scheduler_state(
        &mut self,
        description: &str,
        predicate: impl Fn(&CdpConnection) -> bool,
    ) {
        let waited = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            loop {
                if predicate(&self.conn) {
                    return;
                }
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "test scheduler lost all external input while waiting for {description}"
                );
            }
        })
        .await;

        if waited.is_err() {
            panic!(
                "timed out waiting for {description} from a real scheduler input; sent={:?}; diagnostics={}",
                self.sent,
                self.conn.moli_memory_diagnostics()
            );
        }
    }

    /// Feed one command through the async CDP entrypoint without completing
    /// deferred protocol residences afterwards, returning the scheduler events
    /// that production would handle after the command response turn.
    ///
    /// Use this for protocol sequences that intentionally send another command
    /// before idle/runtime work gets a chance to run. The production socket
    /// actor has a client-turn boundary between command output and deferred
    /// CDP activity; eager test draining would otherwise make the test observe
    /// a stronger, more synchronous ordering than real clients get.
    #[cfg(test)]
    pub(crate) async fn process_command_only_async(
        &mut self,
        msg: impl serde::Serialize,
    ) -> Vec<CdpSchedulerEvent> {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        Box::pin(self.process_parsed_command_like_scheduler(&command, false)).await
    }

    /// Dispatch one command through the production response-flush boundary,
    /// but leave that boundary held so tests can inspect causally later page
    /// work before the wire response is considered flushed.
    #[cfg(test)]
    pub(crate) async fn process_command_holding_response_flush_for_test(
        &mut self,
        msg: impl serde::Serialize,
    ) -> TestCommandResponseFlushPermit {
        let command = ParsedCdpCommand::from_serializable(msg)
            .expect("test message must be a valid serialisable CDP command");
        let command_id = command.request().id();
        let response_start = self.sent.len();
        let (response_flush_permit, response_flush_context) =
            self.conn.begin_command_response_flush_permit();
        let mut command_context = CommandDispatchContext::new(response_flush_context);
        let step = self
            .conn
            .start_parsed_command_dispatch_with_context(&command, &mut command_context)
            .into();
        let mut renderer_response_permit = self.admit_renderer_response_permit(&command);
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut renderer_response_permit,
        ))
        .await;
        assert!(
            completed,
            "held-flush test helper requires an immediate command response boundary"
        );
        protocol_events.extend(command_context.take_protocol_events());
        protocol_events.extend(command_context.take_post_response_events());
        scheduler_events.extend(self.conn.take_scheduler_events());
        Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events)).await;
        Box::pin(self.route_ready_test_command_response(command_id, response_start)).await;
        TestCommandResponseFlushPermit {
            response_flush: response_flush_permit,
            renderer_response_permit,
        }
    }

    /// Releases a response boundary held by
    /// [`Self::process_command_holding_response_flush_for_test`].
    ///
    /// The response-flush permit becomes visible before command-owned
    /// after-response output, matching the production scheduler's
    /// `finish_command_dispatch_output_flush` ordering.
    pub(crate) async fn finish_held_command_response_flush_for_test(
        &mut self,
        permit: TestCommandResponseFlushPermit,
    ) {
        permit.response_flush.finish();
        if let Some(renderer_response_permit) = permit.renderer_response_permit {
            Box::pin(
                self.release_renderer_response_permit_like_scheduler(
                    renderer_response_permit,
                    true,
                ),
            )
            .await;
        }
    }

    #[cfg(test)]
    pub(crate) async fn complete_command_task_step_for_test(
        &mut self,
        step: CdpCommandTaskStep,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        let sent_start = self.sent.len();
        let command_id = match &step {
            CdpCommandTaskStep::Pending(pending) => pending.command_id(),
            CdpCommandTaskStep::Complete(_) => None,
        };
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let mut renderer_response_permit = None;
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut crate::conn::CommandDispatchContext::default(),
            &mut protocol_events,
            &mut scheduler_events,
            &mut renderer_response_permit,
        ))
        .await;
        if completed {
            let mut messages = protocol_events_into_messages(protocol_events);
            if let Some(command_id) = command_id
                && !messages
                    .iter()
                    .any(|message| message.get("id").and_then(Value::as_u64) == Some(command_id))
            {
                // Synchronous renderer-owned Runtime commands publish their
                // terminal response through the DevTools-session output
                // transport. The command completion only proves that the
                // publication was committed; admit that independently
                // transported response just as `process_async` does, while
                // leaving sibling renderer events in the scheduler queue.
                Box::pin(self.route_ready_test_command_response(command_id, sent_start)).await;
                if let Some(position) = self.sent.get(sent_start..).and_then(|sent| {
                    sent.iter()
                        .position(|message| {
                            message.get("id").and_then(Value::as_u64) == Some(command_id)
                        })
                        .map(|position| sent_start + position)
                }) {
                    messages.push(self.sent.remove(position));
                }
            }
            return (messages, scheduler_events);
        }

        Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events)).await;
        if !self.pending_runtime_deferred_replies.is_empty() {
            let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
        }
        let messages = self.sent.drain(sent_start..).collect();
        (messages, Vec::new())
    }

    /// Routes the output of one direct protocol-neutral command through the
    /// stateful test scheduler.
    ///
    /// Direct `DevToolsCommand` fixtures bypass parsed CDP dispatch but can
    /// still publish protocol work whose exact owner action is not ready yet.
    /// Keeping that work resident here mirrors the production scheduler;
    /// callers must use the scheduler wait helpers for later renderer input.
    #[cfg(test)]
    pub(crate) async fn route_direct_command_output_for_test(
        &mut self,
        protocol_events: Vec<BackgroundProtocolEvent>,
        scheduler_events: Vec<CdpSchedulerEvent>,
    ) {
        Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events)).await;
    }

    /// Waits until every concrete scheduler work item published by a direct
    /// command has completed.
    ///
    /// This is the protocol-neutral counterpart of waiting for a CDP event:
    /// WebDriver commands can require the same owner action without enabling a
    /// CDP domain, so their tests must synchronize with the work residence
    /// itself rather than manufacture a frontend subscription.
    #[cfg(test)]
    pub(crate) async fn wait_for_direct_command_work_completion_for_test(
        &mut self,
        description: &str,
    ) {
        let waited = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            while !self.pending_protocol_scheduler_work.is_empty() {
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "test scheduler lost all external input while waiting for {description}"
                );
            }
        })
        .await;
        if waited.is_err() {
            panic!(
                "timed out waiting for {description}; pending protocol work={:?}",
                self.pending_protocol_scheduler_work
            );
        }
    }

    async fn process_parsed_command_like_scheduler(
        &mut self,
        command: &ParsedCdpCommand,
        drain_after_command: bool,
    ) -> Vec<CdpSchedulerEvent> {
        // Some low-level fixtures mutate target-side session declarations
        // after installing their BrowserContext. Materialize those declarations
        // as committed attachments at the test scheduler boundary; production
        // route lookup remains control-plane-only.
        self.conn.commit_declared_session_fixtures_for_test();
        let output_session_id = command.command_output_session_id().map(str::to_owned);
        let output_owner =
            crate::conn::CommandOwnerScope::capture(&self.conn, output_session_id.as_deref());
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let mut command_context = crate::conn::CommandDispatchContext::default();
        let step = self
            .conn
            .start_parsed_command_dispatch_with_context(command, &mut command_context)
            .into();
        let mut renderer_response_permit = self.admit_renderer_response_permit(command);
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut renderer_response_permit,
        ))
        .await;

        if drain_after_command && completed {
            crate::domains::activity::project_protocol_local_command_outputs(
                &mut self.conn,
                &output_owner,
                &mut command_context,
            )
            .await;
            protocol_events.extend(command_context.take_protocol_events());
            protocol_events.extend(command_context.take_post_response_events());
            scheduler_events.extend(self.conn.take_scheduler_events());
        }

        if drain_after_command {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events))
                .await;
            if completed {
                if let Some(renderer_response_permit) = renderer_response_permit {
                    Box::pin(self.release_renderer_response_permit_like_scheduler(
                        renderer_response_permit,
                        true,
                    ))
                    .await;
                }
            } else {
                assert!(
                    renderer_response_permit.is_none(),
                    "a pending renderer command must transfer its response permit to the pending reply"
                );
            }
            if !completed && !self.pending_runtime_deferred_replies.is_empty() {
                let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
            }
            Vec::new()
        } else {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, Vec::new())).await;
            if completed {
                if let Some(renderer_response_permit) = renderer_response_permit {
                    let mut release_scheduler_events =
                        Box::pin(self.release_renderer_response_permit_like_scheduler(
                            renderer_response_permit,
                            false,
                        ))
                        .await;
                    scheduler_events.append(&mut release_scheduler_events);
                }
            } else {
                assert!(
                    renderer_response_permit.is_none(),
                    "a pending renderer command must transfer its response permit to the pending reply"
                );
            }
            if !completed && !self.pending_runtime_deferred_replies.is_empty() {
                let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
            }
            scheduler_events
        }
    }

    fn admit_renderer_response_permit(
        &mut self,
        command: &ParsedCdpCommand,
    ) -> Option<RendererCommandResponsePermit> {
        command
            .runtime_command_executes_page_javascript()
            .then(|| {
                self.renderer_command_response_order.admit(
                    &self.conn,
                    command.request().id(),
                    command.command_output_session_id(),
                )
            })
            .flatten()
    }

    async fn complete_command_step_like_scheduler(
        &mut self,
        mut step: CdpCommandTaskStep,
        command_context: &mut CommandDispatchContext,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
        scheduler_events: &mut Vec<CdpSchedulerEvent>,
        renderer_response_permit: &mut Option<RendererCommandResponsePermit>,
    ) -> bool {
        loop {
            match step {
                CdpCommandTaskStep::Complete(outcome) => {
                    let (
                        mut events,
                        mut post_renderer_output_events,
                        renderer_output_boundary,
                        mut post_response_events,
                        mut new_scheduler_events,
                        mut renderer_output_predecessor,
                    ) = outcome.into_renderer_owner_turn_parts();
                    if let Some(command_predecessor) =
                        command_context.take_renderer_output_predecessor()
                    {
                        command_predecessor
                            .merge_into_same_stream_tail(&mut renderer_output_predecessor);
                    }
                    if let Some(predecessor) = renderer_output_predecessor {
                        Box::pin(
                            self.route_renderer_output_predecessor_before_command_response(
                                predecessor,
                            ),
                        )
                        .await;
                    }
                    protocol_events.append(&mut events);
                    if let Some(renderer_output_boundary) = renderer_output_boundary {
                        // The production actor flushes the already-materialized
                        // prefix, admits the exact renderer publication, then
                        // continues with the suffix. Do the same here instead
                        // of letting the test harness's final batch flatten the
                        // independently transported commit in front of its
                        // Page.navigate/Fetch responses.
                        Box::pin(self.route_test_scheduler_causal_batch(
                            std::mem::take(protocol_events),
                            Vec::new(),
                        ))
                        .await;
                        Box::pin(
                            self.route_renderer_output_predecessor_before_command_response(
                                renderer_output_boundary,
                            ),
                        )
                        .await;
                    }
                    protocol_events.append(&mut post_renderer_output_events);
                    protocol_events.append(&mut post_response_events);
                    scheduler_events.append(&mut new_scheduler_events);
                    return true;
                }
                CdpCommandTaskStep::Pending(mut pending)
                    if pending.waits_for_scheduler_deferred_inspector_reply() =>
                {
                    let owner = pending
                        .runtime_command_owner_scope()
                        .expect("deferred Runtime commands have an exact owner")
                        .clone();
                    protocol_events
                        .extend(pending.take_scheduler_deferred_inspector_reply_events());
                    crate::domains::activity::project_protocol_local_command_outputs(
                        &mut self.conn,
                        &owner,
                        command_context,
                    )
                    .await;
                    protocol_events.extend(command_context.take_protocol_events());
                    protocol_events.extend(command_context.take_post_response_events());
                    scheduler_events.extend(self.conn.take_scheduler_events());
                    self.enqueue_pending_runtime_deferred_reply(
                        *pending,
                        std::mem::take(command_context),
                        renderer_response_permit.take(),
                    );
                    return false;
                }
                CdpCommandTaskStep::Pending(pending) => {
                    // Keep the test scheduler's pending-command boundary shaped
                    // like production. Some domain completions carry a complete
                    // renderer Page build, so composing both futures inline can
                    // exceed Rust's default test-thread stack before the owner
                    // turn gets a chance to yield.
                    let completed = Box::pin(pending.wait()).await;
                    step = Box::pin(self.conn.complete_pending_command_dispatch_with_context(
                        completed,
                        command_context,
                    ))
                    .await
                    .into();
                }
            }
        }
    }

    /// Project the exact concrete renderer output owned by a command before
    /// exposing that command's response.
    ///
    /// Production performs the same fence in
    /// `flush_renderer_publication_predecessor`: the command carries a cursor,
    /// while the publication itself arrives over the renderer transport. The
    /// test scheduler must consume that real transport input rather than
    /// rescanning renderer state or draining unrelated work. Merely removing
    /// the publication from the ordered transport is insufficient: async
    /// owner actions in that batch must have returned as well.
    async fn route_renderer_output_predecessor_before_command_response(
        &mut self,
        predecessor: RendererOutputFence,
    ) {
        let mut observed_transport = Vec::new();
        let cursor = predecessor.cursor();
        let projected = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            while !self.conn.renderer_output_cursor_is_projected(cursor) {
                let publication = self
                    .renderer_publication_rx
                    .recv()
                    .await
                    .expect("renderer output transport closed before command predecessor");
                observed_transport.push(format!("{publication:?}"));
                let mut work =
                    VecDeque::from([TestSchedulerWork::RendererPublication(publication)]);
                Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
            }
        })
        .await;
        assert!(
            projected.is_ok(),
            "timed out waiting for renderer output predecessor {predecessor:?}; \
             observed transport={observed_transport:#?}"
        );
    }

    /// Admits one direct command's exact renderer fence through the production-
    /// shaped transport and ordered ingress.
    ///
    /// Protocol-neutral command tests do not have a parsed CDP response for
    /// `complete_command_step_like_scheduler()` to hold. They still must route
    /// the cursor rather than inspecting renderer state or manufacturing the
    /// owner action that the concrete publication contains.
    #[cfg(test)]
    pub(crate) async fn route_direct_command_renderer_predecessor_for_test(
        &mut self,
        predecessor: RendererOutputFence,
    ) {
        Box::pin(self.route_renderer_output_predecessor_before_command_response(predecessor)).await;
    }

    /// Completes one protocol-neutral command across the same concrete
    /// renderer-output boundary as the production actor.
    pub(crate) async fn execute_devtools_command_through_renderer_fence_for_test(
        &mut self,
        command: DevToolsCommand,
    ) -> Result<DevToolsCommandResult, DevToolsError> {
        let (result, scheduler_events, protocol_events, renderer_output_predecessor) = self
            .conn
            .execute_devtools_command(command)
            .await
            .into_complete_parts();
        if let Some(predecessor) = renderer_output_predecessor {
            self.route_direct_command_renderer_predecessor_for_test(predecessor)
                .await;
        }
        self.route_direct_command_output_for_test(protocol_events, scheduler_events)
            .await;
        result
    }

    /// Routes an explicitly completed command turn through the same ordered
    /// renderer boundary used by the production actor.
    ///
    /// A few ownership tests drive `PendingCdpCommandDispatch` directly so
    /// they can inspect its scheduler sidecars. They must still admit concrete
    /// renderer output at its exact position rather than flattening the
    /// outcome into a message-only vector.
    pub(crate) async fn route_completed_command_outcome_for_test(
        &mut self,
        outcome: crate::CdpRendererOwnerTurnOutcome,
    ) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        let sent_start = self.sent.len();
        let (
            before_renderer_output,
            post_renderer_output,
            renderer_output_boundary,
            post_response_events,
            scheduler_events,
            renderer_output_predecessor,
        ) = outcome.into_renderer_owner_turn_parts();
        if let Some(predecessor) = renderer_output_predecessor {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(predecessor))
                .await;
        }
        Box::pin(self.route_test_scheduler_causal_batch(before_renderer_output, Vec::new())).await;
        if let Some(boundary) = renderer_output_boundary {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(boundary))
                .await;
        } else {
            assert!(
                post_renderer_output.is_empty(),
                "post-renderer output requires an exact boundary"
            );
        }
        let mut suffix = post_renderer_output;
        suffix.extend(post_response_events);
        Box::pin(self.route_test_scheduler_causal_batch(suffix, Vec::new())).await;
        (self.sent.drain(sent_start..).collect(), scheduler_events)
    }

    async fn release_renderer_response_permit_like_scheduler(
        &mut self,
        permit: RendererCommandResponsePermit,
        route_scheduler_events: bool,
    ) -> Vec<CdpSchedulerEvent> {
        let completion = self
            .conn
            .release_renderer_command_response_permit_turn_async(
                &mut self.renderer_command_response_order,
                permit,
            )
            .await;
        let (protocol_events, scheduler_events) =
            completion.into_outcome().into_protocol_event_parts();
        if route_scheduler_events {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, scheduler_events))
                .await;
            Vec::new()
        } else {
            Box::pin(self.route_test_scheduler_causal_batch(protocol_events, Vec::new())).await;
            scheduler_events
        }
    }

    fn enqueue_renderer_response_permit_completion(
        &mut self,
        completion: RendererCommandResponseCompletion,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let (protocol_events, scheduler_events) =
            completion.into_outcome().into_protocol_event_parts();
        if !scheduler_events.is_empty() {
            work.push_front(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
        if !protocol_events.is_empty() {
            work.push_front(TestSchedulerWork::ProtocolEvents(protocol_events));
        }
    }

    async fn route_test_scheduler_causal_batch(
        &mut self,
        initial_events: Vec<BackgroundProtocolEvent>,
        initial_scheduler_events: Vec<CdpSchedulerEvent>,
    ) {
        // Route only output caused by the current scheduler input. Deferred
        // external inputs remain separate turns, matching the actor.
        let mut work = VecDeque::new();
        if !initial_events.is_empty() {
            work.push_back(TestSchedulerWork::ProtocolEvents(initial_events));
        }
        if !initial_scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(initial_scheduler_events));
        }
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
    }

    /// Routes one explicit renderer publication through the same capture,
    /// barrier, and concrete-residence path as the production adapter.
    ///
    /// This helper is for boundary tests that manufacture a typed publication
    /// instead of receiving it from a live renderer. It does not provide a
    /// direct output drain or a broad source scan.
    pub(crate) async fn route_renderer_publication_for_test(
        &mut self,
        publication: RendererOutputTransportMessage,
    ) -> Vec<Value> {
        let sent_start = self.sent.len();
        let mut work = VecDeque::from([TestSchedulerWork::RendererPublication(publication)]);
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
        self.sent.drain(sent_start..).collect()
    }

    fn enqueue_pending_runtime_deferred_reply(
        &mut self,
        mut pending: PendingCdpCommandDispatch,
        command_context: CommandDispatchContext,
        renderer_response_permit: Option<RendererCommandResponsePermit>,
    ) {
        if let Some(command_id) = pending.command_id()
            && let Some(response_rx) = pending.take_scheduler_deferred_inspector_reply_receiver()
        {
            let session_id = pending.session_id().map(str::to_owned);
            let response_tx = self
                .conn
                .runtime_inspector_response_ready_sender()
                .expect("test scheduler should install its typed runtime-response channel");
            tokio::spawn(async move {
                let response = response_rx
                    .await
                    .map_err(|_| "RuntimeDeferredInspectorResponseCanceled".to_owned());
                let _ = response_tx.send(RuntimeInspectorResponseReady::new(
                    command_id,
                    session_id.as_deref(),
                    response,
                ));
            });
        }
        self.pending_runtime_deferred_replies
            .push_back(PendingTestRuntimeDeferredReply::new(
                pending,
                command_context,
                renderer_response_permit,
            ));
    }

    async fn route_test_scheduler_work_queue(&mut self, work: &mut VecDeque<TestSchedulerWork>) {
        self.route_ready_protocol_scheduler_work_for_test_context(work)
            .await;
        while let Some(item) = work.pop_front() {
            match item {
                TestSchedulerWork::ProtocolEvents(events) => {
                    Box::pin(self.route_protocol_events_like_scheduler(events, work)).await;
                }
                TestSchedulerWork::SchedulerEvents(scheduler_events) => {
                    Box::pin(self.route_scheduler_events_for_test_context(scheduler_events, work))
                        .await;
                }
                TestSchedulerWork::BackgroundEvent(event) => {
                    Box::pin(self.route_protocol_events_like_scheduler(vec![event], work)).await;
                }
                TestSchedulerWork::BackgroundNavigationCompletion(completion) => {
                    Box::pin(
                        self.route_background_navigation_completion_like_scheduler(
                            completion, work,
                        ),
                    )
                    .await;
                }
                TestSchedulerWork::RuntimeDeferredReplyReady(response) => {
                    Box::pin(self.complete_runtime_response_ready_like_scheduler(
                        response,
                        Vec::new(),
                        work,
                    ))
                    .await;
                }
                TestSchedulerWork::RendererPublication(publication) => {
                    Box::pin(self.ingest_renderer_publication_like_scheduler(publication, work))
                        .await;
                }
                TestSchedulerWork::ReleaseRendererResponsePermit(permit) => {
                    let completion = self
                        .conn
                        .release_renderer_command_response_permit_turn_async(
                            &mut self.renderer_command_response_order,
                            permit,
                        )
                        .await;
                    self.enqueue_renderer_response_permit_completion(completion, work);
                }
                TestSchedulerWork::CancelRendererResponsePermit(permit) => {
                    let completion = self
                        .conn
                        .cancel_renderer_command_response_permit_turn_async(
                            &mut self.renderer_command_response_order,
                            permit,
                        )
                        .await;
                    self.enqueue_renderer_response_permit_completion(completion, work);
                }
            }
            self.route_ready_protocol_scheduler_work_for_test_context(work)
                .await;
        }
    }

    async fn route_background_navigation_completion_like_scheduler(
        &mut self,
        completion: crate::domains::page::BackgroundNavigationCompletion,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        // Match the production actor's three-part boundary:
        //
        //   already-produced navigation output
        //   -> exact renderer Page cursor
        //   -> navigation commit output
        //
        // The event and renderer transports are independent, so flattening
        // them after the fact would allow the commit cursor to move the new
        // realm in front of frameStartedNavigating/Page.navigate's response.
        let mut prefix = Vec::new();
        while let Ok(event) = self.background_event_rx.try_recv() {
            prefix.push(event);
        }
        if !prefix.is_empty() {
            Box::pin(self.route_protocol_events_like_scheduler(prefix, work)).await;
        }

        let outcome = self
            .conn
            .drain_background_navigation_completion_turn_async(completion)
            .await;
        let (
            mut completion_prefix,
            mut completion_suffix,
            renderer_output_boundary,
            mut post_response_events,
            scheduler_events,
            renderer_output_predecessor,
        ) = outcome.into_renderer_owner_turn_parts();
        assert!(
            renderer_output_predecessor.is_none(),
            "background navigation completion must use an insertion boundary"
        );
        if !completion_prefix.is_empty() {
            Box::pin(self.route_protocol_events_like_scheduler(
                std::mem::take(&mut completion_prefix),
                work,
            ))
            .await;
        }
        if let Some(boundary) = renderer_output_boundary {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(boundary))
                .await;
        }
        completion_suffix.append(&mut post_response_events);
        while let Ok(event) = self.background_event_rx.try_recv() {
            completion_suffix.push(event);
        }
        if !completion_suffix.is_empty() {
            work.push_back(TestSchedulerWork::ProtocolEvents(completion_suffix));
        }
        if !scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
    }

    fn project_native_download_input(
        &mut self,
        event: Result<moli_core::browser::BrowserEvent, tokio::sync::broadcast::error::RecvError>,
    ) -> Vec<BackgroundProtocolEvent> {
        match event {
            Ok(moli_core::browser::BrowserEvent::DownloadCreated(event)) => {
                self.conn.project_created_browser_download(event)
            }
            Ok(moli_core::browser::BrowserEvent::DownloadUpdated(event)) => {
                self.conn.project_browser_download(event)
            }
            Ok(_) => unreachable!("test download ingress filters other native events"),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                let (snapshot, receiver) = self.conn.subscribe_browser_events().unwrap();
                self.browser_event_rx = Some(receiver);
                let mut events = snapshot
                    .downloads
                    .into_iter()
                    .flat_map(|event| self.conn.project_created_browser_download(event))
                    .collect::<Vec<_>>();
                events.extend(self.conn.project_retired_context_downloads());
                events
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                self.browser_event_rx = None;
                Vec::new()
            }
        }
    }

    fn try_native_download_input(
        &mut self,
    ) -> Option<Result<moli_core::browser::BrowserEvent, tokio::sync::broadcast::error::RecvError>>
    {
        use moli_core::browser::BrowserEvent;
        use tokio::sync::broadcast::error::{RecvError, TryRecvError};
        loop {
            match self.browser_event_rx.as_mut()?.try_recv() {
                Ok(record) => {
                    if let event @ (BrowserEvent::DownloadCreated(_)
                    | BrowserEvent::DownloadUpdated(_)) = record.event
                    {
                        return Some(Ok(event));
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Lagged(count)) => return Some(Err(RecvError::Lagged(count))),
                Err(TryRecvError::Closed) => return Some(Err(RecvError::Closed)),
            }
        }
    }

    async fn run_one_ready_test_scheduler_turn(&mut self) -> TestSchedulerTurnOutcome {
        let mut work = VecDeque::new();
        let input_kind = if self.background_navigation_scheduler_enabled
            && let Ok(completion) = self.background_navigation_completion_rx.try_recv()
        {
            work.push_back(TestSchedulerWork::BackgroundNavigationCompletion(
                completion,
            ));
            TestSchedulerInputKind::BackgroundNavigationCompletion
        } else if self.background_navigation_scheduler_enabled
            && let Ok(event) = self.background_event_rx.try_recv()
        {
            work.push_back(TestSchedulerWork::BackgroundEvent(event));
            TestSchedulerInputKind::BackgroundEvent
        } else if let Some(event) = self.try_native_download_input() {
            work.push_back(TestSchedulerWork::ProtocolEvents(
                self.project_native_download_input(event),
            ));
            TestSchedulerInputKind::NativeDownload
        } else if !self.pending_runtime_deferred_replies.is_empty() {
            match self.runtime_inspector_response_ready_rx.try_recv() {
                Ok(response) => {
                    work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
                    TestSchedulerInputKind::RuntimeDeferredReply
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if let Ok(publication) = self.renderer_publication_rx.try_recv() {
                        work.push_back(TestSchedulerWork::RendererPublication(publication));
                        TestSchedulerInputKind::RendererPublication
                    } else {
                        return TestSchedulerTurnOutcome::Idle;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return TestSchedulerTurnOutcome::Idle;
                }
            }
        } else if let Ok(publication) = self.renderer_publication_rx.try_recv() {
            work.push_back(TestSchedulerWork::RendererPublication(publication));
            TestSchedulerInputKind::RendererPublication
        } else if let Ok(response) = self.runtime_inspector_response_ready_rx.try_recv() {
            work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
            TestSchedulerInputKind::RuntimeDeferredReply
        } else {
            return TestSchedulerTurnOutcome::Idle;
        };
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
        TestSchedulerTurnOutcome::Processed(input_kind)
    }

    pub(crate) async fn wait_for_test_command_response(
        &mut self,
        command_id: u64,
        response_start: usize,
    ) {
        let response = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
            loop {
                if self
                    .sent
                    .get(response_start..)
                    .unwrap_or_default()
                    .iter()
                    .any(|message| message.get("id").and_then(Value::as_u64) == Some(command_id))
                {
                    return;
                }
                assert!(
                    matches!(
                        Box::pin(self.wait_for_one_test_scheduler_turn()).await,
                        TestSchedulerTurnOutcome::Processed(_)
                    ),
                    "CDP command `{command_id}` lost all scheduler input before its response"
                );
            }
        })
        .await;
        if response.is_err() {
            panic!(
                "timed out waiting for CDP command `{command_id}` response; sent={:?}; diagnostics={}",
                self.sent,
                self.conn.moli_memory_diagnostics()
            );
        }
    }

    async fn route_ready_test_command_response(&mut self, command_id: u64, response_start: usize) {
        while !self
            .sent
            .get(response_start..)
            .unwrap_or_default()
            .iter()
            .any(|message| message.get("id").and_then(Value::as_u64) == Some(command_id))
            && matches!(
                Box::pin(self.run_one_ready_test_scheduler_turn()).await,
                TestSchedulerTurnOutcome::Processed(_)
            )
        {}
    }

    async fn wait_for_one_test_scheduler_turn(&mut self) -> TestSchedulerTurnOutcome {
        let ready = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
        if matches!(ready, TestSchedulerTurnOutcome::Processed(_)) {
            return ready;
        }

        let mut work = VecDeque::new();
        let background_navigation_scheduler_enabled = self.background_navigation_scheduler_enabled;
        let input_kind = if !self.pending_runtime_deferred_replies.is_empty() {
            tokio::select! {
                biased;
                maybe_response = self.runtime_inspector_response_ready_rx.recv() => {
                    let Some(response) = maybe_response else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
                    TestSchedulerInputKind::RuntimeDeferredReply
                }
                maybe_completion = self.background_navigation_completion_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(completion) = maybe_completion else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundNavigationCompletion(completion));
                    TestSchedulerInputKind::BackgroundNavigationCompletion
                }
                maybe_event = self.background_event_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(event) = maybe_event else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundEvent(event));
                    TestSchedulerInputKind::BackgroundEvent
                }
                event = recv_native_download_input(&mut self.browser_event_rx) => {
                    work.push_back(TestSchedulerWork::ProtocolEvents(self.project_native_download_input(event)));
                    TestSchedulerInputKind::NativeDownload
                }
                maybe_publication = self.renderer_publication_rx.recv() => {
                    let Some(publication) = maybe_publication else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RendererPublication(publication));
                    TestSchedulerInputKind::RendererPublication
                }
            }
        } else {
            tokio::select! {
                biased;
                maybe_completion = self.background_navigation_completion_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(completion) = maybe_completion else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundNavigationCompletion(completion));
                    TestSchedulerInputKind::BackgroundNavigationCompletion
                }
                maybe_event = self.background_event_rx.recv(), if background_navigation_scheduler_enabled => {
                    let Some(event) = maybe_event else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::BackgroundEvent(event));
                    TestSchedulerInputKind::BackgroundEvent
                }
                event = recv_native_download_input(&mut self.browser_event_rx) => {
                    work.push_back(TestSchedulerWork::ProtocolEvents(self.project_native_download_input(event)));
                    TestSchedulerInputKind::NativeDownload
                }
                maybe_publication = self.renderer_publication_rx.recv() => {
                    let Some(publication) = maybe_publication else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RendererPublication(publication));
                    TestSchedulerInputKind::RendererPublication
                }
                maybe_response = self.runtime_inspector_response_ready_rx.recv() => {
                    let Some(response) = maybe_response else {
                        return TestSchedulerTurnOutcome::Idle;
                    };
                    work.push_back(TestSchedulerWork::RuntimeDeferredReplyReady(response));
                    TestSchedulerInputKind::RuntimeDeferredReply
                }
            }
        };
        Box::pin(self.route_test_scheduler_work_queue(&mut work)).await;
        TestSchedulerTurnOutcome::Processed(input_kind)
    }

    async fn route_protocol_events_like_scheduler(
        &mut self,
        mut events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        loop {
            if events.is_empty() {
                return;
            }
            if let Some(position) = events
                .iter()
                .position(|event| event.as_runtime_inspector_response_ready().is_some())
            {
                self.sent.extend(
                    events
                        .drain(..position)
                        .map(BackgroundProtocolEvent::into_protocol_message),
                );
                let response = events
                    .remove(0)
                    .take_runtime_inspector_response_ready()
                    .expect("runtime response event position should contain typed response");
                Box::pin(
                    self.complete_runtime_response_ready_like_scheduler(response, events, work),
                )
                .await;
                return;
            }
            let pending_ids = self.pending_runtime_deferred_reply_command_ids();
            if pending_ids.is_empty() {
                self.sent.extend(
                    events
                        .into_iter()
                        .map(BackgroundProtocolEvent::into_protocol_message),
                );
                return;
            }
            let Some(position) = events.iter().position(|event| {
                event
                    .protocol_message()
                    .and_then(|message| message.get("id"))
                    .and_then(Value::as_u64)
                    .is_some_and(|id| pending_ids.contains(&id))
            }) else {
                self.sent.extend(
                    events
                        .into_iter()
                        .map(BackgroundProtocolEvent::into_protocol_message),
                );
                return;
            };
            self.sent.extend(
                events
                    .drain(..position)
                    .map(BackgroundProtocolEvent::into_protocol_message),
            );
            let message = events.remove(0).into_protocol_message();
            let Some(command_id) = message.get("id").and_then(Value::as_u64) else {
                self.sent.push(message);
                continue;
            };
            let Some(index) = self
                .pending_runtime_deferred_replies
                .iter()
                .position(|pending| pending.command_id() == Some(command_id))
            else {
                self.sent.push(message);
                continue;
            };
            let mut pending = self
                .pending_runtime_deferred_replies
                .remove(index)
                .expect("pending runtime deferred reply index should exist");
            pending
                .pending
                .forget_scheduler_deferred_inspector_reply(&mut self.conn);
            self.sent.push(json!({
                "id": command_id,
                "error": {
                    "code": -32000,
                    "message": "RuntimeDeferredReplyLooseProtocolResponse",
                },
            }));
            if let Some(renderer_response_permit) = pending.renderer_response_permit.take() {
                work.push_front(TestSchedulerWork::CancelRendererResponsePermit(
                    renderer_response_permit,
                ));
            }
            if !events.is_empty() {
                work.push_front(TestSchedulerWork::ProtocolEvents(events));
            }
            return;
        }
    }

    async fn route_scheduler_events_for_test_context(
        &mut self,
        scheduler_events: Vec<CdpSchedulerEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let mut queue = VecDeque::new();
        enqueue_scheduler_events_like_scheduler(&mut queue, scheduler_events);
        while let Some(TestDeferredSchedulerWork(protocol_work)) = queue.pop_front() {
            self.pending_protocol_scheduler_work
                .push_back(protocol_work);
            let scheduler_events = self.conn.take_scheduler_events();
            if !scheduler_events.is_empty() {
                work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
            }
        }
    }

    async fn route_ready_protocol_scheduler_work_for_test_context(
        &mut self,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        loop {
            let selected_index = match self.pending_protocol_scheduler_work.front() {
                Some(front) if front.is_ready() => 0,
                Some(front)
                    if front.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction =>
                {
                    let Some(index) =
                        self.pending_protocol_scheduler_work
                            .iter()
                            .position(|candidate| {
                                candidate.is_ready()
                                    && candidate.is_top_level_location_navigation_owner_action()
                            })
                    else {
                        return;
                    };
                    // Production checks the pending load observer out of the
                    // FIFO while it waits for the renderer. An unconstrained
                    // location owner action may then run and replace that
                    // exact source Document, which completes the observer as
                    // Superseded. Keeping the pending observer at the front in
                    // this protocol-only harness would deadlock the action
                    // needed to make it terminal.
                    index
                }
                Some(_) | None => return,
            };
            if !self.background_navigation_scheduler_enabled
                && self
                    .pending_protocol_scheduler_work
                    .get(selected_index)
                    .is_some_and(ProtocolSchedulerWork::requires_background_navigation_scheduler)
            {
                // The default protocol fixture has no owner task lane. Keep
                // independent popup navigation resident rather than invoking
                // the production function's synchronous fallback while the
                // exact renderer cursor is still being projected. Tests that
                // assert navigation progress opt into the production-shaped
                // background scheduler and drive its typed completions.
                return;
            }
            let protocol_work = self
                .pending_protocol_scheduler_work
                .remove(selected_index)
                .expect("ready protocol work must remain resident");
            let (events, scheduler_events) = self
                .conn
                .complete_ready_protocol_scheduler_work_turn(protocol_work)
                .await
                .into_protocol_event_parts();
            if !events.is_empty() {
                work.push_back(TestSchedulerWork::ProtocolEvents(events));
            }
            if !scheduler_events.is_empty() {
                work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
            }
        }
    }

    async fn ingest_renderer_publication_like_scheduler(
        &mut self,
        publication: RendererOutputTransportMessage,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let scheduler_events = self.conn.take_scheduler_events();
        if !scheduler_events.is_empty() {
            work.push_front(TestSchedulerWork::RendererPublication(publication));
            work.push_front(TestSchedulerWork::SchedulerEvents(scheduler_events));
            return;
        }
        // Load-relative scheduling belongs to the adapter-level
        // `CdpScheduler`, which captures this publication first and delays
        // only its already-projected protocol batch. This protocol-only
        // harness must not park the raw publication: doing so prevents the
        // ordered ingress from admitting later records in the same renderer
        // stream and can deadlock sequence N behind output produced at N + 1.
        let outcome = self
            .conn
            .ingest_renderer_output_turn_async(
                publication,
                &mut self.renderer_command_response_order,
            )
            .await;
        let (protocol_events, scheduler_events) = outcome.into_protocol_event_parts();
        if !protocol_events.is_empty() {
            work.push_back(TestSchedulerWork::ProtocolEvents(protocol_events));
        }
        if !scheduler_events.is_empty() {
            work.push_back(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
    }

    fn pending_runtime_deferred_reply_command_ids(&self) -> Vec<u64> {
        self.pending_runtime_deferred_replies
            .iter()
            .filter_map(PendingTestRuntimeDeferredReply::command_id)
            .collect()
    }

    async fn complete_runtime_response_ready_like_scheduler(
        &mut self,
        response: RuntimeInspectorResponseReady,
        suffix_events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        // Production admits an exact renderer cursor before resolving the
        // response correlation. Keep the test scheduler on the same boundary:
        // a SharedWorker `Destroyed` publication must terminate the pending
        // call with `Target closed` before a later V8 context-destruction
        // response can become visible.
        if let Some(predecessor) = response.renderer_output_predecessor() {
            Box::pin(self.route_renderer_output_predecessor_before_command_response(predecessor))
                .await;
        }
        let command_id = response.command_id();
        let Some(index) = self
            .pending_runtime_deferred_replies
            .iter()
            .position(|pending| pending.command_id() == Some(command_id))
        else {
            let mut response_events = Vec::new();
            let mut background_events = Vec::new();
            self.conn.route_registered_runtime_inspector_response_into(
                response,
                &mut response_events,
                &mut background_events,
            );
            if !background_events.is_empty() {
                work.push_front(TestSchedulerWork::ProtocolEvents(background_events));
            }
            self.sent.extend(
                response_events
                    .into_iter()
                    .map(BackgroundProtocolEvent::into_protocol_message),
            );
            if !suffix_events.is_empty() {
                work.push_front(TestSchedulerWork::ProtocolEvents(suffix_events));
            }
            return;
        };
        let pending = self
            .pending_runtime_deferred_replies
            .remove(index)
            .expect("pending runtime deferred reply index should exist");
        Box::pin(
            self.complete_renderer_runtime_deferred_response_like_scheduler(
                pending,
                response,
                suffix_events,
                work,
            ),
        )
        .await;
    }

    async fn complete_renderer_runtime_deferred_response_like_scheduler(
        &mut self,
        mut pending: PendingTestRuntimeDeferredReply,
        response: RuntimeInspectorResponseReady,
        suffix_events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        if pending.pending.command_id().is_none() {
            self.pending_runtime_deferred_replies.push_back(pending);
            return;
        }
        pending
            .pending
            .route_scheduler_deferred_inspector_response(&mut self.conn, response)
            .await;
        Box::pin(self.complete_runtime_deferred_reply_like_scheduler(pending, suffix_events, work))
            .await;
    }

    async fn complete_runtime_deferred_reply_like_scheduler(
        &mut self,
        mut pending: PendingTestRuntimeDeferredReply,
        suffix_events: Vec<BackgroundProtocolEvent>,
        work: &mut VecDeque<TestSchedulerWork>,
    ) {
        let completed = pending
            .pending
            .complete_scheduler_deferred_inspector_reply(&mut self.conn);
        let step = self
            .conn
            .complete_pending_command_dispatch_with_context(completed, &mut pending.command_context)
            .await
            .into();
        let mut protocol_events = Vec::new();
        let mut scheduler_events = Vec::new();
        let completed = Box::pin(self.complete_command_step_like_scheduler(
            step,
            &mut pending.command_context,
            &mut protocol_events,
            &mut scheduler_events,
            &mut pending.renderer_response_permit,
        ))
        .await;
        if !suffix_events.is_empty() {
            work.push_front(TestSchedulerWork::ProtocolEvents(suffix_events));
        }
        if completed {
            if let Some(renderer_response_permit) = pending.renderer_response_permit {
                work.push_front(TestSchedulerWork::ReleaseRendererResponsePermit(
                    renderer_response_permit,
                ));
            }
        } else {
            assert!(
                pending.renderer_response_permit.is_none(),
                "a still-pending renderer command must transfer its response permit"
            );
        }
        if !scheduler_events.is_empty() {
            work.push_front(TestSchedulerWork::SchedulerEvents(scheduler_events));
        }
        if !protocol_events.is_empty() {
            work.push_front(TestSchedulerWork::ProtocolEvents(protocol_events));
        }
    }

    // ── Assertion helpers ─────────────────────────────────────────────────────

    /// Assert that a `{id, result, sessionId?}` message exists in the sent
    /// queue and remove it.  `session_id = None` means "no sessionId field".
    pub fn expect_result(&mut self, id: u64, result: Value, session_id: Option<&str>) {
        let expected = build_result(id, &result, session_id);
        self.find_and_remove(&expected, "result");
    }

    /// Assert that a `{id, error: {code, message}}` message exists.
    pub fn expect_error(&mut self, id: u64, code: i32, message: &str) {
        let expected = json!({ "id": id, "error": { "code": code, "message": message } });
        self.find_and_remove(&expected, "error");
    }

    /// Assert that an event `{method, params, sessionId?}` message exists.
    /// When `params` is `None`, only the method name is checked.
    pub fn expect_event(&mut self, method: &str, params: Option<&Value>) {
        let pos = self.sent.iter().position(|v| {
            if v["method"].as_str() != Some(method) {
                return false;
            }
            if let Some(expected_params) = params {
                values_subset(expected_params, &v["params"])
            } else {
                true // any params
            }
        });
        match pos {
            Some(i) => {
                self.sent.remove(i);
            }
            None => {
                let queue: String = self.sent.iter().map(|v| format!("  {}\n", v)).collect();
                panic!("expected event '{method}' not found in sent queue:\n{queue}");
            }
        }
    }

    /// Take and return the next message; panics if the queue is empty.
    pub fn take_one(&mut self) -> Value {
        if self.sent.is_empty() {
            panic!("expected a message in the sent queue but it is empty");
        }
        self.sent.remove(0)
    }

    /// Take and return the first sent message matching the predicate.
    pub fn take_first_matching(
        &mut self,
        description: &str,
        matches: impl FnMut(&Value) -> bool,
    ) -> Value {
        let pos = self
            .sent
            .iter()
            .position(matches)
            .unwrap_or_else(|| panic!("expected {description} in sent queue: {:?}", self.sent));
        self.sent.remove(pos)
    }

    /// Take and return the response with the requested id.
    pub fn take_response_by_id(&mut self, id: u64) -> Value {
        let pos = self
            .sent
            .iter()
            .position(|message| message["id"] == json!(id))
            .unwrap_or_else(|| {
                panic!(
                    "expected a response with id {id} in sent queue: {:?}",
                    self.sent
                )
            });
        self.sent.remove(pos)
    }

    /// Drain all pending sent messages (useful to discard setup noise).
    pub fn take_all(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.sent)
    }

    /// Completes at most one scheduler input that is already ready.
    ///
    /// Unlike the removed broad test capture, this cannot snapshot an
    /// arbitrary session. It only consumes a concrete renderer publication, Runtime
    /// response, scheduler event, or ready `ProtocolSchedulerWork` already
    /// resident in the production-shaped harness.
    #[cfg(test)]
    pub(crate) async fn complete_one_ready_scheduler_input_for_test(&mut self) {
        let scheduler_events = self.conn.take_scheduler_events();
        Box::pin(self.route_test_scheduler_causal_batch(Vec::new(), scheduler_events)).await;
        let _ = Box::pin(self.run_one_ready_test_scheduler_turn()).await;
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn find_and_remove(&mut self, expected: &Value, kind: &str) {
        let pos = self.sent.iter().position(|v| values_subset(expected, v));
        match pos {
            Some(i) => {
                self.sent.remove(i);
            }
            None => {
                let queue: String = self.sent.iter().map(|v| format!("  {}\n", v)).collect();
                panic!("expected {kind} not found.\nExpected:\n  {expected}\nSent queue:\n{queue}");
            }
        }
    }
}

fn protocol_events_into_messages(events: Vec<BackgroundProtocolEvent>) -> Vec<Value> {
    events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect()
}

#[cfg(test)]
pub(crate) fn protocol_events_into_internal_messages(
    events: Vec<BackgroundProtocolEvent>,
) -> Vec<Value> {
    events
        .into_iter()
        .map(|event| event.into_parts().0)
        .collect()
}

#[cfg(test)]
pub(crate) async fn drain_scheduler_events_like_scheduler(
    conn: &mut CdpConnection,
    out: &mut Vec<Value>,
    scheduler_events: Vec<CdpSchedulerEvent>,
) {
    drain_scheduler_events_like_scheduler_with_materializer(
        conn,
        out,
        scheduler_events,
        BackgroundProtocolEvent::into_protocol_message,
    )
    .await;
}

#[cfg(test)]
pub(crate) async fn drain_scheduler_events_like_scheduler_preserving_internal_fields(
    conn: &mut CdpConnection,
    out: &mut Vec<Value>,
    scheduler_events: Vec<CdpSchedulerEvent>,
) {
    drain_scheduler_events_like_scheduler_with_materializer(
        conn,
        out,
        scheduler_events,
        protocol_event_into_internal_message,
    )
    .await;
}

#[cfg(test)]
async fn drain_scheduler_events_like_scheduler_with_materializer(
    conn: &mut CdpConnection,
    out: &mut Vec<Value>,
    scheduler_events: Vec<CdpSchedulerEvent>,
    materialize_event: fn(BackgroundProtocolEvent) -> Value,
) {
    let mut queue = VecDeque::new();
    enqueue_scheduler_events_like_scheduler(&mut queue, scheduler_events);
    while let Some(TestDeferredSchedulerWork(protocol_work)) = queue.pop_front() {
        assert!(
            protocol_work.is_ready(),
            "the stateless compatibility materializer cannot own pending protocol work; use TestContext or the production CdpScheduler"
        );
        let (events, nested_scheduler_events) = conn
            .complete_ready_protocol_scheduler_work_turn(protocol_work)
            .await
            .into_protocol_event_parts();
        out.extend(events.into_iter().map(materialize_event));
        enqueue_scheduler_events_like_scheduler(&mut queue, nested_scheduler_events);
        enqueue_scheduler_events_like_scheduler(&mut queue, conn.take_scheduler_events());
    }
}

#[cfg(test)]
fn protocol_event_into_internal_message(event: BackgroundProtocolEvent) -> Value {
    event.into_parts().0
}

#[cfg(test)]
/// Compatibility materializer for protocol-domain fixtures.
///
/// It preserves scheduler-event FIFO and concrete work residence, but does not
/// model adapter client-turn predecessors. Tests that claim scheduling or
/// ordering behavior must use the production `CdpScheduler` instead.
struct TestDeferredSchedulerWork(ProtocolSchedulerWork);

#[cfg(test)]
fn enqueue_scheduler_events_like_scheduler(
    queue: &mut VecDeque<TestDeferredSchedulerWork>,
    events: Vec<CdpSchedulerEvent>,
) {
    for event in events {
        match event {
            CdpSchedulerEvent::ProtocolWorkPublished { work } => {
                queue.push_back(TestDeferredSchedulerWork(work));
            }
            CdpSchedulerEvent::PageScreencastStarted { .. } => {}
        }
    }
}

#[cfg(test)]
pub(crate) trait TestSessionId<'a> {}

#[cfg(test)]
impl<'a> TestSessionId<'a> for Option<&'a str> {}

#[cfg(test)]
impl<'a> TestSessionId<'a> for &'a str {}

#[cfg(test)]
pub async fn spawn_connection_drop_server() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let _ = stream.shutdown().await;
    });
    (addr, server)
}

#[cfg(test)]
pub(crate) async fn wait_until_message(
    ctx: &mut TestContext,
    session_id: impl TestSessionId<'_> + Copy,
    description: &str,
    predicate: impl Fn(&Value) -> bool,
) {
    wait_until_messages(ctx, session_id, description, |messages| {
        messages.iter().any(&predicate)
    })
    .await;
}

#[cfg(test)]
pub(crate) async fn wait_until_messages(
    ctx: &mut TestContext,
    _session_id: impl TestSessionId<'_> + Copy,
    description: &str,
    predicate: impl Fn(&[Value]) -> bool,
) {
    for _ in 0..256 {
        if predicate(&ctx.sent) {
            return;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        if predicate(&ctx.sent) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ctx.complete_one_ready_scheduler_input_for_test().await;
    if predicate(&ctx.sent) {
        return;
    }
    panic!("timed out waiting for {description}; sent={:?}", ctx.sent);
}

/// Wait for a protocol message produced by a real scheduler input without
/// synthesizing broad capture turns or moving messages already collected in
/// `ctx.sent`.
///
/// Resource and runtime completions publish typed wakes into `TestContext`.
/// Tests asserting those completion boundaries should block on that channel
/// instead of tying correctness to a polling iteration budget.
#[cfg(test)]
pub(crate) async fn wait_until_scheduler_message(
    ctx: &mut TestContext,
    description: &str,
    predicate: impl Fn(&Value) -> bool,
) {
    let waited = tokio::time::timeout(TEST_SCHEDULER_INPUT_TIMEOUT, async {
        loop {
            if ctx.sent.iter().any(&predicate) {
                return;
            }
            assert!(
                matches!(
                    Box::pin(ctx.wait_for_one_test_scheduler_turn()).await,
                    TestSchedulerTurnOutcome::Processed(_)
                ),
                "test scheduler lost all external input while waiting for {description}"
            );
        }
    })
    .await;

    if waited.is_err() {
        panic!(
            "timed out waiting for {description} from a real scheduler input; sent={:?}",
            ctx.sent
        );
    }
}

/// Wait for the terminal loading event of one concrete frame while preserving
/// the complete event sequence for assertions that follow.
///
/// `Page.navigate` only acknowledges that navigation was accepted; Chromium
/// likewise allows document replacement to continue after that response. DOM
/// tests must synchronize with the frame lifecycle before retaining frontend
/// node ids from the new document.
#[cfg(test)]
pub(crate) async fn wait_until_frame_stopped_loading(ctx: &mut TestContext, frame_id: &str) {
    let description = format!("Page.frameStoppedLoading for {frame_id}");
    wait_until_scheduler_message(ctx, &description, |message| {
        message["method"] == json!("Page.frameStoppedLoading")
            && message["params"]["frameId"] == json!(frame_id)
    })
    .await;
}

/// Wait for protocol-visible renderer load of one exact document generation.
///
/// The binding's loader id rejects earlier documents in the same frame. Native
/// load can precede protocol ingress, so tests retaining frontend node or remote
/// object ids must also wait for the new document's projection to catch up.
#[cfg(test)]
pub(crate) async fn wait_until_renderer_document_load(
    ctx: &mut TestContext,
    session_id: Option<&str>,
    frame_id: &str,
    loader_id: &str,
) {
    let description = format!("renderer load for {frame_id}/{loader_id}");
    ctx.wait_until_scheduler_state(&description, |conn| {
        conn.renderer_document_lifecycle_visible_state_for_session_owner(session_id)
            .is_some_and(|(binding, snapshot)| {
                binding.frame_id == frame_id
                    && binding.loader_id == loader_id
                    && snapshot.load.is_some()
            })
    })
    .await;
}

/// Build a result message, omitting sessionId when it is None.
fn build_result(id: u64, result: &Value, session_id: Option<&str>) -> Value {
    let mut v = json!({ "id": id, "result": result });
    if let Some(sid) = session_id {
        v["sessionId"] = json!(sid);
    }
    v
}

/// Return true when every field of `expected` appears in `actual` with the
/// same value.  Arrays and nested objects are compared recursively.
fn values_subset(expected: &Value, actual: &Value) -> bool {
    match expected {
        Value::Object(exp_map) => {
            let Value::Object(act_map) = actual else {
                return false;
            };
            exp_map.iter().all(|(k, ev)| {
                act_map
                    .get(k)
                    .map(|av| values_subset(ev, av))
                    .unwrap_or(false)
            })
        }
        Value::Array(exp_arr) => {
            let Value::Array(act_arr) = actual else {
                return false;
            };
            if exp_arr.len() != act_arr.len() {
                return false;
            }
            exp_arr
                .iter()
                .zip(act_arr.iter())
                .all(|(e, a)| values_subset(e, a))
        }
        _ => expected == actual,
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{PageId, RendererRuntimeInspectorAsyncCompletion};

    use super::*;

    #[tokio::test]
    async fn test_context_drops_unmatched_typed_runtime_response_like_scheduler() {
        let mut ctx = TestContext::new();
        let response = RuntimeInspectorResponseReady::new(
            42,
            None,
            Ok(
                RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                    42,
                    json!({
                        "id": 42,
                        "result": {}
                    }),
                ),
            ),
        );

        ctx.complete_runtime_response_ready_like_scheduler(
            response,
            Vec::new(),
            &mut VecDeque::new(),
        )
        .await;

        assert!(
            ctx.sent.is_empty(),
            "unmatched typed runtime completion must stay internal in the test harness too"
        );
    }

    #[tokio::test]
    async fn test_context_consumes_selected_task_output_publications_one_per_turn() {
        let mut conn = crate::test_support::connection();
        let (publication_tx, publication_rx) = moli_core::renderer_output_transport_channel();
        let runtime_response_rx = conn
            .bind_runtime_inspector_response_ready()
            .expect("test scheduler must own the connection's completion ingress");
        let (background_event_tx, background_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (background_navigation_completion_tx, background_navigation_completion_rx) =
            tokio::sync::mpsc::unbounded_channel();
        conn.set_renderer_publication_sender(publication_tx.clone());
        let mut ctx = TestContext {
            conn,
            sent: Vec::new(),
            pending_runtime_deferred_replies: VecDeque::new(),
            pending_protocol_scheduler_work: VecDeque::new(),
            renderer_command_response_order: RendererCommandResponseOrder::default(),
            runtime_inspector_response_ready_rx: runtime_response_rx,
            renderer_publication_rx: publication_rx,
            background_event_tx,
            background_event_rx,
            background_navigation_completion_tx,
            background_navigation_completion_rx,
            background_navigation_scheduler_enabled: false,
            browser_event_rx: None,
        };
        let opened = |page_id| {
            RendererOutputTransportMessage::from(RendererOutputStreamControl::Opened {
                stream: RendererOutputStreamIdentity::new_page_for_protocol_test(
                    PageId::new_for_testing(page_id),
                ),
            })
        };
        let first = opened(1);
        let second = opened(2);
        let second_stream = match &second {
            RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream },
            ) => *stream,
            RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Closed { .. },
            )
            | RendererOutputTransportMessage::PageReservationReleased { .. }
            | RendererOutputTransportMessage::CursorLeaseDeclared { .. }
            | RendererOutputTransportMessage::CursorLeaseReleased { .. }
            | RendererOutputTransportMessage::Publication(_) => {
                unreachable!("test input is an opened stream control")
            }
        };
        publication_tx.send(first).expect("first scheduler input");
        publication_tx.send(second).expect("second scheduler input");

        assert_eq!(
            ctx.run_one_ready_test_scheduler_turn().await,
            TestSchedulerTurnOutcome::Processed(TestSchedulerInputKind::RendererPublication)
        );
        let queued = ctx
            .renderer_publication_rx
            .try_recv()
            .expect("second input must remain queued for the next turn");
        assert!(matches!(
            queued,
            RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream }
            ) if stream == second_stream
        ));
    }
}
