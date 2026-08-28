//! Cross-crate fixture construction for protocol scheduler tests.
//!
//! This module is available only with the `test-support` feature. It keeps
//! opaque protocol identities and work payloads private while allowing the
//! top-level adapter scheduler tests to construct exact inputs. Production
//! callers must obtain these values from their owning connection, navigation,
//! lifecycle observer, or publication boundary.

use moli_core::{RendererDocumentLifecycleIdentity, RendererOutputResidenceIdentity};

use crate::{
    BackgroundNavigationGateKey, DeferredMainDocumentLoadCompletionOutputInterest,
    DeferredMainDocumentLoadObservationId, ProtocolSchedulerWork,
    conn::{CdpConnection, DocumentNavigationToken, RendererPageResidenceIdentity},
};

/// Constructs the exact key carried by a synthetic Document-continuation
/// completion in scheduler-only tests.
pub fn background_navigation_gate_key(
    target_id: Option<String>,
    session_id: Option<String>,
    frame_id: String,
    loader_id: String,
    navigation_request_id: Option<u64>,
) -> BackgroundNavigationGateKey {
    BackgroundNavigationGateKey::from_test_parts(
        target_id,
        session_id,
        frame_id,
        loader_id,
        navigation_request_id,
    )
}

/// Opaque exact-token fixture for scheduler tests that need a real
/// target-owned background navigation request.
pub struct BackgroundNavigationRequestFixture {
    token: DocumentNavigationToken,
    cancellation: moli_fetch::FetchCancelHandle,
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
    let token = conn
        .browser_context
        .as_mut()
        .and_then(|context| {
            context.start_document_navigation_for_target(target_id, loader_id.to_owned())
        })
        .expect("the active target must accept a navigation request fixture");
    let cancellation = conn
        .document_navigation_cancellation_handle(&token)
        .expect("the target-owned request must expose its cancellation handle");
    assert!(conn.arm_background_navigation_completion(&token, None));
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
