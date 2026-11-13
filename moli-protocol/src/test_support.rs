//! Cross-crate fixture construction for protocol scheduler tests.
//!
//! This module is available only with the `test-support` feature. It keeps
//! opaque protocol identities and work payloads private while allowing the
//! top-level adapter scheduler tests to construct exact inputs. Production
//! callers must obtain these values from their owning connection, navigation,
//! lifecycle observer, or publication boundary.

use moli_core::{
    RendererDocumentLifecycleIdentity, RendererOutputResidenceIdentity,
    RendererOutputStreamIdentity,
};

use crate::{
    DeferredMainDocumentLoadCompletionOutputInterest, DeferredMainDocumentLoadObservationId,
    ProtocolSchedulerWork,
    conn::{CdpConnection, DocumentNavigationToken, RendererPageResidenceIdentity},
};

pub fn command_response_flush_observer(
    context: &crate::CommandResponseFlushContext,
) -> tokio::sync::watch::Receiver<bool> {
    context
        .receiver()
        .expect("test command owns a flush permit")
}

/// A completed command carrying only scheduler wakes. Adapter tests can retain
/// this output while the connection still owns all published work.
pub fn command_output_with_scheduler_events(
    scheduler_events: Vec<crate::CdpSchedulerEvent>,
) -> crate::CdpCommandTaskStep {
    crate::CdpCommandTaskStep::Complete(
        crate::CdpTurnOutcome::new_with_protocol_events(Vec::new(), scheduler_events)
            .with_renderer_output_predecessor(None),
    )
}

/// Uses the production allocation-and-publication boundary for synthetic
/// concrete work. A caller cannot inject a separate ordering counter.
pub fn publish_protocol_work(
    conn: &mut CdpConnection,
    build: impl FnOnce(u64) -> ProtocolSchedulerWork,
) {
    conn.publish_protocol_work(|sequence| {
        let work = build(sequence.get());
        assert_eq!(work.publish_sequence(), sequence);
        work
    });
}

/// A real lifecycle dependency on an existing command permit. The target is
/// deliberately absent so adapter tests can exercise readiness independently
/// of Native teardown.
pub fn publish_retired_target_termination_after_command_output(
    conn: &mut CdpConnection,
    target_id: &str,
    output: crate::CommandResponseFlushContext,
) {
    let action = crate::domains::page::PageTargetTerminationOwnerAction::new(
        conn,
        crate::conn::CommandOwnerScope::capture(conn, None),
        target_id.to_owned(),
    );
    conn.publish_page_target_termination_after_command_output(action, output);
}

/// Publishes real, Page-owned channel cleanup without installing a V8 proxy.
/// Scheduler tests exercise its dependencies separately from Native disposal.
pub fn publish_bidi_channel_cleanup(conn: &mut CdpConnection, group: &str) {
    let owner = crate::conn::BidiChannelPageOwner::capture_for_owner(
        conn,
        crate::conn::CommandOwnerScope::capture(conn, None),
    )
    .expect("channel cleanup requires the fixture's current Page owner");
    let action = crate::conn::BidiChannelOwnerAction::release_object_group(owner, group.to_owned());
    conn.publish_protocol_work(move |sequence| {
        ProtocolSchedulerWork::bidi_channel_owner_action(sequence, action)
    });
}

/// Binds a synthetic Page stream to a retired protocol Page owner, allowing
/// scheduler tests to project historical output without a live Page.
pub fn bind_renderer_page_output_stream_to_retired_owner(
    conn: &mut CdpConnection,
    stream: RendererOutputStreamIdentity,
) {
    let renderer_page = RendererPageResidenceIdentity::from_residence(stream.residence())
        .expect("a synthetic Page output stream requires a Page residence");
    let page_owner = crate::conn::TargetPageResidenceIdentity::new(
        "BID-retired-scheduler-fixture".into(),
        Some("TID-retired-scheduler-fixture".into()),
        crate::conn::TargetPageAttachmentId::allocate(),
    );
    conn.bind_renderer_page_output_owner(renderer_page, page_owner);
}

/// Opaque exact-token fixture for scheduler tests that need a real
/// target-owned background navigation request.
pub struct BackgroundNavigationRequestFixture {
    token: DocumentNavigationToken,
    cancellation: moli_fetch::FetchCancelHandle,
}

/// Holds a real Native owner operation until dropped. The entry receipt lets
/// tests establish that later lifecycle work cannot run yet, without sleeps.
pub struct HeldRenderer {
    entered: tokio::sync::oneshot::Receiver<()>,
    release: Option<std::sync::mpsc::Sender<()>>,
    pending: Option<moli_core::page::PendingPageCommand>,
    response: Option<moli_core::RendererNativeCommandResponseGuard>,
}

impl HeldRenderer {
    pub async fn wait_until_entered(&mut self) {
        (&mut self.entered)
            .await
            .expect("Native owner gate entered");
    }
}

impl Drop for HeldRenderer {
    fn drop(&mut self) {
        self.release.take();
        self.response.take();
        self.pending.take();
    }
}

pub fn hold_renderer(
    conn: &mut CdpConnection,
    context: &crate::automation::AutomationContext,
) -> HeldRenderer {
    let owner = conn
        .command_owner_scope_for_devtools_context(context)
        .expect("held renderer requires a live target");
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let (pending, response) = conn
        .loaded_page_mut_for_owner(&owner)
        .expect("held renderer requires an installed Page")
        .start_cdp_call(
            9_000_199,
            None,
            moli_core::RendererNativeOperation::then(
                moli_core::RendererPageCommand::ChildFrameTreeSnapshot,
                move |_| {
                    let _ = entered.send(());
                    let _ = held.recv();
                    moli_core::RendererNativeOperationStep::Complete(
                        moli_core::RendererNativeProtocolResponse::success(serde_json::json!({})),
                    )
                },
            ),
        )
        .expect("Native owner gate accepted");
    HeldRenderer {
        entered: waiting,
        release: Some(release),
        pending: Some(pending),
        response: Some(response),
    }
}

impl BackgroundNavigationRequestFixture {
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn target_id(&self) -> &str {
        &self.token.target_id
    }
}

pub fn arm_background_navigation_request(
    conn: &mut CdpConnection,
    loader_id: &str,
) -> BackgroundNavigationRequestFixture {
    conn.install_default_browser_target();
    let target_id = conn
        .browser_context
        .as_ref()
        .and_then(|context| context.active_target_id())
        .expect("the default browser target must have an active target")
        .to_owned();
    arm_background_navigation_request_for_target(conn, &target_id, loader_id)
}

pub fn arm_background_navigation_request_for_target(
    conn: &mut CdpConnection,
    target_id: &str,
    loader_id: &str,
) -> BackgroundNavigationRequestFixture {
    let context = conn
        .browser_context
        .as_mut()
        .expect("a navigation request fixture requires a browser context");
    let token = context
        .start_document_navigation_for_target(target_id, loader_id.to_owned())
        .expect("the active target must accept a navigation request fixture");
    let cancellation = context
        .page_target_mut(target_id)
        .expect("the fixture's target must remain installed")
        .runtime_slot
        .claim_background_navigation_completion(&token, None)
        .expect("the fixture must claim its new request's background completion");
    BackgroundNavigationRequestFixture {
        token,
        cancellation,
    }
}

pub fn settle_background_navigation_request(
    conn: &mut CdpConnection,
    fixture: &BackgroundNavigationRequestFixture,
) -> bool {
    conn.settle_background_navigation_completion(&fixture.token)
}

/// Installs a dialog on an exact owned Page without running V8. The renderer
/// completion remains the caller's real one-shot.
pub fn push_page_javascript_dialog(
    conn: &mut CdpConnection,
    context: &crate::automation::AutomationContext,
    dialog: moli_core::page::RendererPendingJavaScriptDialog,
) {
    let owner = conn
        .command_owner_scope_for_devtools_context(context)
        .expect("dialog fixture requires a live target");
    let page = conn
        .target_page_residence_identity_for_owner(&owner)
        .or_else(|| conn.pending_target_page_residence_identity_for_owner(&owner))
        .expect("dialog fixture requires a current or reserved Page");
    let frame_id = page.target_id().expect("fixture target").to_owned();
    let dialog = crate::conn::TargetJavaScriptDialog::new(page, frame_id, dialog);
    conn.with_target_javascript_dialog_state_for_owner_mut(&owner, |state| state.push(dialog))
        .expect("dialog fixture requires target state");
}

/// Constructs one nonzero deferred-load observation identity.
pub fn deferred_main_document_load_observation_id(
    value: u64,
) -> DeferredMainDocumentLoadObservationId {
    DeferredMainDocumentLoadObservationId::from_test_value(value)
}

/// Freezes the Page and optional Document observed by a scheduler-only load
/// wait fixture.
pub fn deferred_main_document_load_output_interest(
    renderer_residence: RendererOutputResidenceIdentity,
    renderer_document: Option<RendererDocumentLifecycleIdentity>,
) -> DeferredMainDocumentLoadCompletionOutputInterest {
    let renderer_page = RendererPageResidenceIdentity::from_residence(renderer_residence)
        .expect("deferred main-document load fixtures require a Page residence");
    DeferredMainDocumentLoadCompletionOutputInterest::from_test_residence(
        renderer_page,
        renderer_document,
    )
}

/// Constructs concrete stopped-loading observation work for scheduler ordering
/// tests without exposing its private payload or attachment representation.
pub fn root_frame_stopped_loading_work(
    publish_sequence: u64,
    session_ids: Vec<Option<String>>,
    frame_id: String,
    loader_id: String,
) -> ProtocolSchedulerWork {
    ProtocolSchedulerWork::root_frame_stopped_loading_for_test_support(
        publish_sequence,
        session_ids,
        frame_id,
        loader_id,
    )
}

pub fn renderer_fenced_output_work(
    sequence: u64,
    events: Vec<crate::BackgroundProtocolEvent>,
    predecessors: Vec<moli_core::RendererOutputFence>,
) -> ProtocolSchedulerWork {
    ProtocolSchedulerWork::renderer_fenced_output(
        crate::ProtocolWorkPublishSequence::new(sequence),
        events,
        predecessors,
        None,
    )
}

pub fn protocol_work_after_renderer_delivery(
    mut work: ProtocolSchedulerWork,
    predecessor: moli_core::RendererOutputCursor,
) -> ProtocolSchedulerWork {
    work.bind_renderer_delivery_predecessor(predecessor);
    work
}

pub fn protocol_work_from_renderer_output(
    mut work: ProtocolSchedulerWork,
    source: moli_core::RendererOutputCursor,
) -> ProtocolSchedulerWork {
    work.bind_renderer_output_source(source);
    work
}

pub fn command_completion_with_scheduler_events(
    events: Vec<crate::BackgroundProtocolEvent>,
    scheduler_events: Vec<crate::CdpSchedulerEvent>,
) -> crate::CdpCommandTaskStep {
    crate::CdpCommandTaskStep::Complete(
        crate::CdpTurnOutcome::new_with_protocol_events(events, scheduler_events)
            .with_renderer_output_predecessor(None),
    )
}

pub fn root_frame_stopped_loading_work_for_target(
    publish_sequence: u64,
    session_ids: Vec<Option<String>>,
    browser_context_id: String,
    target_id: String,
    frame_id: String,
    loader_id: String,
) -> ProtocolSchedulerWork {
    ProtocolSchedulerWork::root_frame_stopped_loading_for_target_test_support(
        publish_sequence,
        session_ids,
        browser_context_id,
        target_id,
        frame_id,
        loader_id,
    )
}

/// A replacement action with a deliberately retired Page owner. Scheduler
/// tests can check selection independently of the later owner authorization.
pub fn retired_location_navigation_work_for_target(
    publish_sequence: u64,
    browser_context_id: String,
    target_id: String,
    source_document: RendererDocumentLifecycleIdentity,
) -> ProtocolSchedulerWork {
    let owner =
        crate::conn::CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id: browser_context_id.clone(),
            target_id: target_id.clone(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        });
    let page_owner = crate::conn::TargetPageResidenceIdentity::new(
        browser_context_id,
        Some(target_id),
        crate::conn::TargetPageAttachmentId::allocate(),
    );
    ProtocolSchedulerWork::top_level_navigation_owner_action(
        crate::ProtocolWorkPublishSequence::new(publish_sequence),
        crate::conn::TopLevelNavigationOwnerAction::from_prepared(
            owner,
            page_owner,
            moli_core::page::RendererDocumentSourcedTopLevelLocationNavigation::new(
                source_document,
                "data:text/html,replacement".to_owned(),
            ),
        ),
    )
}

/// Creates one real domain lifetime for cross-crate task/receipt tests.
pub fn revocable_command_wait_lifetime() -> (crate::CommandWaitLifetime, impl FnOnce()) {
    let owner = crate::conn::CommandWaitLifetimeOwner::default();
    (owner.lifetime(), move || owner.revoke())
}
