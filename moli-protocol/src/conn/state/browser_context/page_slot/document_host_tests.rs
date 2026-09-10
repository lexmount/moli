use super::*;
use crate::conn::{CdpConnection, CommandOwnerScope};
use moli_core::browser::{
    BrowserNavigationWaiter, DocumentHandle, NavigationDecision, NavigationDecisionStage,
    WebContentsHandle,
};
use std::{
    future::Future,
    ops::{Deref, DerefMut},
    task::{Context, Poll, Waker},
};

const TARGET: &str = "TID-dialog-owner";

struct DocumentOwnerFixture {
    conn: CdpConnection,
    initial_artifacts: Option<RendererPageCreationArtifacts>,
}

impl Deref for DocumentOwnerFixture {
    type Target = BrowserContext;

    fn deref(&self) -> &Self::Target {
        self.conn.browser_context.as_ref().expect("fixture context")
    }
}

impl DerefMut for DocumentOwnerFixture {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn.browser_context.as_mut().expect("fixture context")
    }
}

async fn prepare_navigation(
    owner: &mut DocumentOwnerFixture,
    url: &str,
) -> BrowserNavigationWaiter {
    let command_owner = CommandOwnerScope::capture(&owner.conn, None);
    let waiter = owner
        .conn
        .start_native_navigation_fixture_for_test(
            &command_owner,
            moli_core::browser::web_contents::NavigationRequestInterception::new(
                url::Url::parse(url).unwrap(),
                "GET".into(),
                None,
                Vec::new(),
                crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            ),
            NavigationDecision::Continue,
        )
        .unwrap();
    let request = waiter.request();
    let (_, mut events) = owner.conn.subscribe_browser_events().unwrap();
    loop {
        if let Some(paused) = owner
            .browser_context
            .navigation_decision(request.web_contents)
            .unwrap()
        {
            assert_eq!(paused.permit.navigation(), request.navigation);
            if matches!(
                paused.stage,
                NavigationDecisionStage::PreparedDocument { .. }
            ) {
                return waiter;
            }
            assert!(
                owner
                    .browser_context
                    .resolve_navigation_decision(
                        request.web_contents,
                        paused.permit,
                        NavigationDecision::Continue,
                    )
                    .unwrap()
            );
        }
        let event = events.recv().await.unwrap();
        if let moli_core::browser::BrowserEvent::NavigationFailed {
            request: failed,
            reason,
        } = event.event
            && failed == request
        {
            panic!("native fixture failed before PreparedDocument: {reason:?}");
        }
    }
}

fn prepared_renderer(
    owner: &DocumentOwnerFixture,
    waiter: &BrowserNavigationWaiter,
) -> moli_core::browser::RendererPageResidenceIdentity {
    let request = waiter.request();
    let paused = owner
        .browser_context
        .navigation_decision(request.web_contents)
        .unwrap()
        .unwrap();
    assert_eq!(paused.permit.navigation(), request.navigation);
    let NavigationDecisionStage::PreparedDocument { renderer, .. } = paused.stage else {
        panic!("fixture must observe the exact prepared native Document");
    };
    renderer
}

async fn observe_navigation_commit(
    owner: &mut DocumentOwnerFixture,
    waiter: BrowserNavigationWaiter,
) -> Result<
    (
        std::sync::Arc<moli_core::browser::web_contents::DocumentCommitMetadata>,
        Option<crate::conn::state::browser_context::page_state::DocumentInspectionProjection>,
    ),
    String,
> {
    let committed = owner
        .conn
        .wait_for_native_navigation_commit_for_test(waiter, false)
        .await?;
    owner
        .conn
        .wait_for_native_document_load_for_test(committed.document)
        .await?;
    let metadata = committed.metadata.clone();
    let projection = owner.project_document_commit_snapshot(TARGET, *committed);
    Ok((metadata, projection))
}

fn empty_document_context_with_runtime_config(
    runtime_config: moli_core::runtime::NavigationRuntimeConfig,
) -> DocumentOwnerFixture {
    let mut conn = crate::test_support::connection();
    let mut owner = conn.new_browser_context_fixture_for_test("BID-dialog-owner");
    owner.bind_page_navigation_engines(runtime_config, None);
    owner.set_active_target_id(TARGET);
    conn.install_browser_context_fixture_for_test(owner);
    DocumentOwnerFixture {
        conn,
        initial_artifacts: None,
    }
}

fn empty_document_context() -> DocumentOwnerFixture {
    empty_document_context_with_runtime_config(Default::default())
}

async fn context_with_document_and_runtime_config(
    url: &str,
    runtime_config: moli_core::runtime::NavigationRuntimeConfig,
) -> DocumentOwnerFixture {
    let mut owner = empty_document_context_with_runtime_config(runtime_config);
    let waiter = prepare_navigation(&mut owner, url).await;
    let (metadata, projection) = observe_navigation_commit(&mut owner, waiter).await.unwrap();
    owner.initial_artifacts = Some(metadata.lifecycle.artifacts.clone());
    let projection_fence = projection
        .unwrap()
        .fence
        .expect("initial fixture must attach its exact renderer Document")
        .unwrap();
    owner
        .page_targets
        .get_mut(TARGET)
        .unwrap()
        .runtime_slot
        .publish_document_projection_fence(projection_fence)
        .expect("initial fixture inspection projection must finish");
    owner
}

async fn context_with_document(url: &str) -> DocumentOwnerFixture {
    context_with_document_and_runtime_config(url, Default::default()).await
}

#[tokio::test]
async fn loaded_navigation_commit_settles_document_history_and_navigation_together() {
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let old_document = owner.target_document_id(TARGET).unwrap();
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>second</title>").await;
    let expected_document = loaded.request().document;
    let expected_loader = owner
        .current_document_loader_id_for_target(TARGET)
        .unwrap()
        .to_owned();
    let (committed, projection) = observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let url = &committed.info.as_ref().unwrap().url;
    assert!(projection.unwrap().fence.is_ok());
    assert_eq!(
        owner.target_document_id(TARGET),
        Some(expected_document),
        "commit must install the Document allocated by this Browser navigation"
    );
    assert_ne!(expected_document, old_document);
    assert!(
        !owner.has_pending_document_navigation_for_target(TARGET),
        "returning a committed Page must not leave its Browser navigation pending"
    );
    assert_eq!(
        owner.committed_document_loader_id_for_target(TARGET),
        Some(expected_loader.as_str())
    );
    let (_, entries) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(entries.last().unwrap().url, url.as_str());
    assert_eq!(entries.last().unwrap().title, "second");
    assert_eq!(
        owner.page_targets.get(TARGET).unwrap().target_url(),
        url.as_str()
    );
    assert!(
        owner
            .document_lifecycle_snapshot_for_target(TARGET)
            .is_some()
    );
}

#[tokio::test]
async fn late_title_projection_does_not_rewind_native_history_or_cross_documents() {
    let mut owner = context_with_document("data:text/html,<title>native current</title>").await;
    let earlier = moli_core::RendererDocumentTitleChanged {
        source_document: owner
            .document_lifecycle_snapshot_for_target(TARGET)
            .unwrap()
            .into(),
        title: "earlier FIFO title".into(),
    };
    assert_eq!(
        owner.commit_target_document_title(TARGET, &earlier),
        Some(true)
    );
    assert_eq!(
        owner.commit_target_document_title(TARGET, &earlier),
        Some(false)
    );
    let (_, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(history.last().unwrap().title, "native current");

    let next = prepare_navigation(&mut owner, "data:text/html,<title>replacement</title>").await;
    observe_navigation_commit(&mut owner, next).await.unwrap();
    assert_eq!(owner.commit_target_document_title(TARGET, &earlier), None);
    let (_, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(history[history.len() - 2].title, "native current");
    assert_eq!(history.last().unwrap().title, "replacement");
}

#[tokio::test]
async fn disappearing_agent_host_cannot_cancel_an_admitted_browser_commit() {
    let mut owner = empty_document_context();
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>native</title>").await;
    let request = loaded.request();
    let navigation = request.navigation;
    let contents = request.web_contents;
    let expected_document = request.document;

    drop(owner.page_targets.remove(TARGET).unwrap());
    let (committed, projection) = observe_navigation_commit(&mut owner, loaded).await.unwrap();
    assert!(projection.is_none());
    let artifacts = &committed.lifecycle.artifacts;
    let url = &committed.info.as_ref().unwrap().url;
    let document = DocumentHandle::new(contents, expected_document);
    let native = owner
        .browser_context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .unwrap();
    assert_eq!(
        (native.frame, native.document, native.epoch),
        (
            artifacts.lifecycle_snapshot.frame,
            artifacts.active_document,
            artifacts.active_epoch
        )
    );
    assert!(native.sequence() >= artifacts.lifecycle_snapshot.sequence());
    assert!(
        owner
            .browser_context
            .document_lifecycle_snapshot(document)
            .unwrap()
            .unwrap()
            .load
            .is_some()
    );
    let renderer_page = owner
        .browser_context
        .document_renderer_residence(document)
        .unwrap();
    assert_eq!(
        owner
            .browser_context
            .document_handle(contents)
            .unwrap()
            .unwrap()
            .id(),
        expected_document
    );
    assert_eq!(
        owner.browser_context.pending_document_for_test(contents),
        Ok(None)
    );
    assert_eq!(
        owner
            .browser_context
            .current_document_navigation(contents)
            .unwrap(),
        Some(navigation)
    );
    let (_, history) = owner
        .browser_context
        .navigation_history_snapshot(contents)
        .unwrap();
    assert_eq!(history.last().unwrap().url, url.as_str());
    assert_eq!(history.last().unwrap().title, "native");
    assert_eq!(
        owner
            .browser_context
            .evaluate_document_expression_for_test(document, "40 + 2", false)
            .await
            .unwrap()["value"],
        42
    );
    let (_, mut events) = owner.conn.subscribe_browser_events().unwrap();
    let stopped = owner
        .browser_context
        .start_document_lifecycle_stop(document)
        .unwrap()
        .wait()
        .await;
    owner
        .browser_context
        .finish_document_lifecycle_stop(stopped)
        .unwrap();
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let moli_core::browser::BrowserEvent::DocumentLifecycleChanged(snapshot) =
                events.recv().await.unwrap().event
                && snapshot.document == document
                && snapshot.lifecycle.terminated.is_some()
            {
                break snapshot.lifecycle;
            }
        }
    })
    .await
    .expect("a native lifecycle must progress after its AgentHost disappears");
    let terminated = snapshot.terminated.unwrap();
    let event = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: terminated.sequence,
        timestamp_micros: terminated.timestamp_micros,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(RendererDocumentLifecycleMilestone::Load),
            reason: terminated.reason,
        },
    };
    assert!(
        owner
            .conn
            .browser
            .wait_for_renderer_document_lifecycle(
                renderer_page,
                RendererDocumentLifecycleEvent {
                    document: event.document.successor_for_testing(),
                    ..event
                }
            )
            .await
            .is_none()
    );
    let occurrence = owner
        .conn
        .browser
        .wait_for_renderer_document_lifecycle(renderer_page, event)
        .await
        .unwrap();
    assert_eq!(occurrence.document(), expected_document);
    assert_eq!(occurrence.event(), event);
    assert_eq!(
        owner
            .conn
            .browser
            .wait_for_renderer_document_lifecycle(renderer_page, event)
            .await,
        Some(occurrence)
    );
    assert_eq!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    );
    assert_eq!(
        owner
            .browser_context
            .document_lifecycle_snapshot(document)
            .unwrap()
            .unwrap()
            .terminated
            .unwrap()
            .sequence,
        event.sequence
    );
}

#[tokio::test]
async fn creation_projection_cannot_rewind_native_progress_or_retarget_a_replacement() {
    use crate::conn::CommittedDocumentLifecycle;
    let mut owner = empty_document_context();
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>native</title>").await;
    let navigation = loaded.request().navigation;
    let renderer_page = prepared_renderer(&owner, &loaded);
    let (commit, _) = observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let artifacts = commit.lifecycle.artifacts.clone();
    let loader = owner
        .committed_document_loader_id_for_target(TARGET)
        .unwrap()
        .to_owned();
    let snapshot = artifacts.lifecycle_snapshot;
    let loaded_snapshot = owner
        .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
        .unwrap();
    assert!(loaded_snapshot.load.is_some());
    let document = commit.lifecycle.document;
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );

    let terminated = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: u64::MAX - 1,
        timestamp_micros: 100,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(RendererDocumentLifecycleMilestone::Load),
            reason: moli_core::page::RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    let occurrence = owner
        .apply_renderer_document_lifecycle_for_test(renderer_page, terminated)
        .unwrap();
    let native = owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET);
    // Projection is delayed until after the Browser has accepted more progress.
    // Replaying/rebinding the creation occurrence may only affect visibility.
    let browser_sequence = commit.lifecycle.browser_sequence;
    for _ in 0..2 {
        let projected = owner.project_committed_document_lifecycle_for_target(
            TARGET,
            CommittedDocumentLifecycle {
                document,
                browser_sequence,
                artifacts: artifacts.clone(),
            },
            Some(navigation),
            TARGET.into(),
            loader.clone(),
        );
        assert_eq!(projected, artifacts.initial_lifecycle_events);
        assert_eq!(
            owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
            native
        );
        assert_eq!(
            owner
                .page_slot_for_target(TARGET)
                .unwrap()
                .renderer_document_lifecycle_visible_snapshot(),
            Some(snapshot)
        );
    }
    // Replay the real native milestones after the older creation inventory.
    // These sequence/timestamp pairs came from the committed renderer, not a
    // manufactured fully-loaded creation artifact.
    let progress = [
        (
            loaded_snapshot.dom_content_loaded.unwrap(),
            RendererDocumentLifecycleMilestone::DomContentLoaded,
        ),
        (
            loaded_snapshot.load.unwrap(),
            RendererDocumentLifecycleMilestone::Load,
        ),
    ]
    .into_iter()
    .filter(|(stamp, _)| stamp.sequence > snapshot.sequence())
    .map(|(stamp, milestone)| RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: stamp.sequence,
        timestamp_micros: stamp.timestamp_micros,
        kind: RendererDocumentLifecycleEventKind::Milestone(milestone),
    })
    .collect::<Vec<_>>();
    assert_eq!(
        owner.project_renderer_document_lifecycle_events_for_target(TARGET, progress.clone()),
        progress
    );
    assert_eq!(
        owner.project_renderer_document_lifecycle_events_for_target(
            TARGET,
            vec![occurrence.event()]
        ),
        vec![terminated]
    );
    assert_eq!(
        owner
            .page_slot_for_target(TARGET)
            .unwrap()
            .renderer_document_lifecycle_visible_snapshot(),
        native
    );

    let loaded = prepare_navigation(&mut owner, "data:text/html,next").await;
    observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let next_snapshot = owner
        .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
        .unwrap();
    assert_ne!(
        owner.target_document_id(TARGET),
        Some(occurrence.document())
    );
    assert!(
        owner
            .apply_renderer_document_lifecycle_for_test(renderer_page, terminated)
            .is_none()
    );
    assert!(
        owner
            .project_committed_document_lifecycle_for_target(
                TARGET,
                commit.lifecycle.clone(),
                Some(navigation),
                TARGET.into(),
                loader,
            )
            .is_empty()
    );
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );
    assert_eq!(
        owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        Some(next_snapshot)
    );
}

#[tokio::test]
async fn failed_inspection_projection_cannot_veto_browser_document_commit() {
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let previous_document = owner.target_document_id(TARGET).unwrap();
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>committed</title>").await;
    // Fail only the DevTools projection. The Browser WebContents and its pending
    // navigation remain live and must not require that projection's approval.
    owner
        .page_targets
        .get_mut(TARGET)
        .unwrap()
        .runtime_slot
        .retire_for_target_close();
    let expected_document = loaded.request().document;
    let (committed, projection) = observe_navigation_commit(&mut owner, loaded)
        .await
        .expect("Browser commit is independent of its projection");
    let url = &committed.info.as_ref().unwrap().url;

    assert_eq!(
        owner.target_document_id(TARGET),
        Some(expected_document),
        "a closed DevTools channel must not veto the Browser commit"
    );
    assert_ne!(expected_document, previous_document);
    assert!(!owner.has_pending_document_navigation_for_target(TARGET));
    let (_, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(history.last().unwrap().url, url.as_str());
    assert_eq!(history.last().unwrap().title, "committed");
    assert_eq!(
        owner
            .evaluate_target_expression_for_test(TARGET, "40 + 2", false)
            .await
            .unwrap()["value"],
        42
    );
    assert!(projection.unwrap().fence.is_err());
}

#[tokio::test]
async fn committed_occurrence_retains_the_previous_document_output_projection() {
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let artifacts = owner.initial_artifacts.take().unwrap();
    let previous_renderer = owner
        .target_renderer_page_residence_identity(TARGET)
        .unwrap();
    let previous_document = owner.target_document_id(TARGET).unwrap();
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        TARGET.into(),
        "LOADER-first".into(),
    );
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_some()
    );
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>current</title>").await;
    let current_renderer = prepared_renderer(&owner, &loaded);
    observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let current_document = owner.target_document_id(TARGET).unwrap();
    assert_ne!(current_document, previous_document);
    let projection = &owner.page_targets.get(TARGET).unwrap().runtime_slot;
    assert!(
        projection.routes_retiring_renderer_page_owner(previous_renderer, previous_document),
        "post-commit retirement must match the occurrence's old Document, not the new current identity"
    );
    assert!(!projection.routes_retiring_renderer_page_owner(current_renderer, current_document));
}

#[tokio::test]
async fn old_network_ingress_survives_native_commit_before_projection() {
    use moli_core::page::{
        ScriptNetworkOutputItem, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
        SubresourceRequestInitiatorType, SubresourceRequestStarted, SubresourceResourceType,
    };
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let artifacts = owner.initial_artifacts.take().unwrap();
    let renderer = owner
        .target_renderer_page_residence_identity(TARGET)
        .unwrap();
    let document = owner.target_document_id(TARGET).unwrap();
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        TARGET.into(),
        "LOADER-first".into(),
    );
    let binding = owner
        .renderer_document_lifecycle_binding_for_target(TARGET)
        .unwrap()
        .clone();
    let handle = SubresourceNetworkRequestHandle::new(7);
    let url = url::Url::parse("https://old.example/held-xhr").unwrap();
    let started = ScriptNetworkOutputItem::SubresourceRequestStarted(std::sync::Arc::new(
        SubresourceRequestStarted::new(
            handle,
            None,
            url.clone(),
            url.clone(),
            "GET".into(),
            Vec::new(),
            None,
            SubresourceResourceType::Xhr,
            SubresourceRequestInitiatorType::Script,
            None,
        ),
    ));
    let mut allocator = crate::conn::ConnectionNetworkRequestIdAllocator::default();
    assert!(
        owner
            .ingest_renderer_network_output_item_and_prepare_live_delivery_for_target(
                TARGET,
                Some(renderer),
                binding.renderer_document_identity(),
                &started,
                None,
                None,
                None,
                &mut allocator,
            )
            .is_some()
    );

    let waiter = prepare_navigation(&mut owner, "data:text/html,<title>successor</title>").await;
    // Deliberately finish the Browser transaction without projecting its commit.
    // The old renderer has closed; its final FIFO facts still belong to the
    // observer's old request correlations, not to the successor Document.
    let committed = owner
        .conn
        .wait_for_native_navigation_commit_for_test(waiter, false)
        .await
        .unwrap();
    assert_ne!(owner.target_document_id(TARGET), Some(document));
    assert!(
        owner
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );
    let terminal = ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
        SubresourceNetworkRecord::failure(
            None,
            url.clone(),
            url,
            "GET".into(),
            Vec::new(),
            None,
            SubresourceResourceType::Xhr,
            "net::ERR_ABORTED".into(),
        )
        .with_request_handle(handle),
    ));
    let foreign_renderer = RendererPageResidenceIdentity::from_parts(
        moli_core::RendererOwnerLocalHostId::new_for_testing(
            renderer.owner_local_host_id().as_u64() + 1,
        ),
        renderer.page_id(),
    );
    assert!(
        owner
            .ingest_renderer_network_output_item_and_prepare_live_delivery_for_target(
                TARGET,
                Some(foreign_renderer),
                binding.renderer_document_identity(),
                &terminal,
                None,
                None,
                None,
                &mut allocator,
            )
            .is_none(),
        "a colliding renderer-local Page id must not claim the old request"
    );
    assert!(
        owner
            .ingest_renderer_network_output_item_and_prepare_live_delivery_for_target(
                TARGET,
                Some(renderer),
                binding.renderer_document_identity(),
                &terminal,
                None,
                None,
                None,
                &mut allocator,
            )
            .is_some(),
        "native retirement must not discard an unprojected predecessor's terminal"
    );
    assert!(
        owner
            .page_targets
            .get(TARGET)
            .unwrap()
            .runtime_slot
            .routes_retiring_renderer_page_owner(renderer, document)
    );
    assert!(
        owner
            .project_document_commit_snapshot(TARGET, *committed)
            .is_some()
    );
    assert!(
        owner
            .page_targets
            .get(TARGET)
            .unwrap()
            .runtime_slot
            .routes_retiring_renderer_page_owner(renderer, document)
    );
}

#[tokio::test]
async fn inspection_configuration_failure_cannot_roll_back_a_committed_browser_document() {
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>committed</title><script>Object.defineProperty(globalThis,'protectedBinding',{value:1,configurable:false})</script>",
    )
    .await;
    let (committed, projection) = observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let url = &committed.info.as_ref().unwrap().url;
    assert!(projection.unwrap().fence.is_ok());
    let document = owner.target_document_id(TARGET);
    let registration = moli_core::page::RuntimeBindingRegistration {
        devtools_session: None,
        name: "protectedBinding".into(),
        execution_context_name: None,
    };
    let pending = owner
        .page_targets
        .get(TARGET)
        .unwrap()
        .runtime_slot
        .current_renderer_inspection_binding()
        .unwrap()
        .runtime_inspection(None)
        .start_apply_runtime_protocol_state(
            &[],
            &[],
            std::slice::from_ref(&registration),
            std::slice::from_ref(&registration),
        )
        .unwrap();
    let error = moli_core::page::PendingPageCommand::from_inspector_main_route(pending)
        .wait()
        .await
        .and_then(|completion| completion.into_unit_page_command_turn())
        .map(|_| ())
        .expect_err("a non-configurable global must reject binding installation");
    assert!(error.to_string().contains("runtime binding"), "{error:#}");
    assert_eq!(owner.target_document_id(TARGET), document);
    assert!(!owner.has_pending_document_navigation_for_target(TARGET));
    let (_, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(history.last().unwrap().url, url.as_str());
    assert_eq!(history.last().unwrap().title, "committed");
    assert_eq!(
        owner
            .evaluate_target_expression_for_test(TARGET, "protectedBinding + 41", false)
            .await
            .unwrap()["value"],
        42
    );
}

#[tokio::test]
async fn rejected_browser_candidate_cannot_rotate_inspection_or_document_projection() {
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let document = owner.target_document_id(TARGET);
    let attachment = owner
        .page_targets
        .get(TARGET)
        .unwrap()
        .runtime_slot
        .current_renderer_attachment();
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>stale</title>").await;
    let request = loaded.request();
    let stale_permit = owner
        .browser_context
        .navigation_decision(request.web_contents)
        .unwrap()
        .unwrap()
        .permit;
    let replacement = prepare_navigation(&mut owner, "data:text/html,current").await;
    let current = replacement.request().navigation;
    let history = owner.target_navigation_history_snapshot(TARGET).unwrap();
    let target_url = owner
        .page_targets
        .get(TARGET)
        .unwrap()
        .target_url()
        .to_owned();
    assert!(
        !owner
            .browser_context
            .resolve_navigation_decision(
                request.web_contents,
                stale_permit,
                NavigationDecision::Continue
            )
            .unwrap()
    );
    assert!(observe_navigation_commit(&mut owner, loaded).await.is_err());
    assert_eq!(owner.target_document_id(TARGET), document);
    assert_eq!(
        owner
            .browser_context
            .pending_document_for_test(owner.web_contents_handle_for_target(TARGET).unwrap())
            .unwrap()
            .unwrap()
            .0,
        current
    );
    let projection = owner.page_targets.get(TARGET).unwrap();
    assert_eq!(
        projection.runtime_slot.current_renderer_attachment(),
        attachment
    );
    assert!(projection.runtime_slot.has_renderer_navigation(&current));
    assert_eq!(projection.target_url(), target_url);
    assert_eq!(
        owner.target_navigation_history_snapshot(TARGET).unwrap(),
        history
    );
}

#[tokio::test]
async fn document_replacement_updates_inspection_binding_with_physical_page() {
    let mut owner = context_with_document("data:text/html,<title>first</title>").await;
    let old_attachment = owner
        .active_page_target()
        .runtime_slot
        .current_renderer_attachment()
        .unwrap();
    let loaded = prepare_navigation(&mut owner, "data:text/html,<title>second</title>").await;
    observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let attachment = owner
        .active_page_target()
        .runtime_slot
        .current_renderer_attachment()
        .unwrap();
    assert_ne!(attachment.id(), old_attachment.id());
    assert_eq!(
        owner
            .target_document_renderer_agent_for_test(TARGET)
            .unwrap(),
        attachment.agent_token()
    );
    assert!(
        owner
            .active_page_target()
            .runtime_slot
            .current_renderer_inspection_binding()
            .is_some()
    );
}

async fn page_with_installed_dialog_for_test() -> (
    DocumentOwnerFixture,
    moli_core::page::RendererJavaScriptDialogCompletion,
) {
    use moli_core::page::{
        RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
    };

    let mut owner = context_with_document("data:text/html,<p>dialog owner</p>").await;
    let artifacts = owner.initial_artifacts.take().unwrap();
    let source = artifacts.lifecycle_snapshot;
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        "FRAME-dialog-owner".into(),
        "loader".into(),
    );
    owner.attach_active_session("SID-dialog-owner");
    let completion = RendererJavaScriptDialogCompletion::pending();
    let document = owner.document_handle_for_target(TARGET).unwrap();
    let key = owner
        .install_document_javascript_dialog_for_test(
            document,
            RendererPendingJavaScriptDialog::new(
                RendererJavaScriptDialogId::new(1),
                RendererDocumentLifecycleIdentity {
                    frame: source.frame,
                    document: source.document,
                    epoch: source.epoch,
                },
                RendererJavaScriptDialogSource::RootFrame,
                "about:blank".into(),
                "prompt".into(),
                "owned dialog".into(),
                "default".into(),
                Some(completion.clone()),
            ),
        )
        .unwrap();
    let key = key.unwrap();
    let wrong_document = moli_core::browser::DocumentHandle::new(
        moli_core::browser::WebContentsHandle::new(
            document.web_contents().context(),
            moli_core::browser::WebContentsId::allocate(),
        ),
        document.id(),
    );
    assert!(!owner.project_javascript_dialog_for_session(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        "FRAME-dialog-owner".into(),
        wrong_document,
        key,
    ));
    assert!(owner.project_javascript_dialog_for_session(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        "FRAME-dialog-owner".into(),
        document,
        key,
    ));
    (owner, completion)
}

#[tokio::test]
async fn document_replacement_dismisses_dialog_without_protocol_session_cleanup() {
    let (mut owner, completion) = page_with_installed_dialog_for_test().await;
    let loaded = prepare_navigation(&mut owner, "data:text/html,<p>replacement</p>").await;
    observe_navigation_commit(&mut owner, loaded).await.unwrap();

    assert!(
        !completion.finish(true, "late reply".into()),
        "Browser Document replacement must dismiss its dialog before Protocol cleanup"
    );
    assert!(!completion.wait().accepted);
}

#[tokio::test]
async fn browser_drop_dismisses_dialog_even_when_session_snapshot_survives() {
    let (mut owner, completion) = page_with_installed_dialog_for_test().await;
    let dialog_projection_snapshot = owner.active_page_target().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .javascript_dialog_state
        .clone();
    let id = owner.selected_web_contents_id().unwrap();
    let handle = WebContentsHandle::new(owner.browser_context_id(), id);
    drop(owner.page_targets.remove(TARGET).unwrap());
    assert!(owner.browser_context.has_loaded_document(handle));
    assert!(
        owner
            .browser_context
            .web_contents_has_pending_javascript_dialog(handle)
            .unwrap()
    );
    drop(owner);

    assert!(
        !completion.finish(true, "late reply".into()),
        "Browser drop must dismiss the dialog even if a dialog projection snapshot survives"
    );
    assert!(!completion.wait().accepted);
    drop(dialog_projection_snapshot);
}

#[tokio::test]
async fn browser_dialog_can_be_handled_after_protocol_projection_is_dropped() {
    let (mut owner, completion) = page_with_installed_dialog_for_test().await;
    let key = owner.active_page_target().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .javascript_dialog_state
        .pending_dialogs()[0]
        .key;
    let document = owner.document_handle_for_target(TARGET).unwrap();
    drop(owner.page_targets.remove(TARGET).unwrap());

    assert_eq!(
        owner
            .browser_context
            .document_javascript_dialog_snapshot(document, key)
            .unwrap()
            .message,
        "owned dialog"
    );
    owner
        .browser_context
        .set_document_javascript_dialog_prompt_text(document, key, "Browser input".into())
        .unwrap();
    let closed = owner
        .browser_context
        .finish_document_javascript_dialog(document, key, true, None)
        .unwrap();
    assert_eq!(closed.dialog_type, "prompt");
    assert_eq!(closed.user_input, "Browser input");
    assert!(
        owner
            .browser_context
            .document_javascript_dialog_snapshot(document, key)
            .is_none()
    );
    assert!(
        owner
            .browser_context
            .finish_document_javascript_dialog(document, key, false, None)
            .is_none()
    );
    assert!(!completion.finish(false, "late reply".into()));
    let result = completion.wait();
    assert!(result.accepted);
    assert_eq!(result.user_input, "Browser input");
}

#[tokio::test]
async fn browser_dialog_retirement_follows_admitted_document_lifecycle_without_projection() {
    use moli_core::page::RendererDocumentTerminationReason;
    let (mut owner, completion) = page_with_installed_dialog_for_test().await;
    let contents_id = owner.selected_web_contents_id().unwrap();
    let handle = WebContentsHandle::new(owner.browser_context_id(), contents_id);
    let document = owner.document_handle_for_target(TARGET).unwrap();
    drop(owner.page_targets.remove(TARGET).unwrap());
    let id = document.id();
    let snapshot = owner
        .browser_context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .unwrap();
    owner
        .browser_context
        .begin_initial_empty_document(handle, "about:blank".into(), None, None)
        .unwrap();
    owner
        .browser_context
        .mark_initial_empty_document_materialized_for_test(handle)
        .unwrap();
    assert!(
        owner
            .browser_context
            .web_contents_has_pending_javascript_dialog(handle)
            .unwrap()
    );
    let terminated = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: u64::MAX - 2,
        timestamp_micros: 10,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: None,
            reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    assert!(
        !owner
            .browser_context
            .apply_document_lifecycle_for_test(
                document,
                RendererDocumentLifecycleEvent {
                    document: snapshot.document.successor_for_testing(),
                    ..terminated
                }
            )
            .unwrap()
    );
    assert!(
        owner
            .browser_context
            .web_contents_has_pending_javascript_dialog(handle)
            .unwrap(),
        "foreign lifecycle must not dismiss current dialog"
    );
    assert!(
        owner
            .browser_context
            .apply_document_lifecycle_for_test(document, terminated)
            .unwrap()
    );
    assert!(
        !owner
            .browser_context
            .web_contents_has_pending_javascript_dialog(handle)
            .unwrap()
    );
    assert!(!completion.finish(true, "late reply".into()));
    assert!(!completion.wait().accepted);
    assert!(
        owner
            .browser_context
            .apply_document_lifecycle_for_test(
                document,
                RendererDocumentLifecycleEvent {
                    epoch: RendererLifecycleEpoch(snapshot.epoch.0 + 1),
                    sequence: u64::MAX - 1,
                    kind: RendererDocumentLifecycleEventKind::Started {
                        reason: RendererLifecycleStartReason::ExplicitDocumentOpen
                    },
                    ..terminated
                }
            )
            .unwrap()
    );
    assert_eq!(
        owner
            .browser_context
            .document_handle(handle)
            .unwrap()
            .unwrap()
            .id(),
        id,
    );
    assert_eq!(
        owner.browser_context.is_on_initial_document(handle),
        Ok(Some(false))
    );
}

#[tokio::test]
async fn dialog_disable_and_exact_detach_dismiss_only_their_browser_dialogs() {
    use moli_core::page::{
        RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
    };
    use moli_page_types::DevToolsSessionKey;
    let (mut owner, primary_completion) = page_with_installed_dialog_for_test().await;
    let peer = DevToolsSessionKey::Attached("SID-dialog-peer".into());
    let peer_completion = RendererJavaScriptDialogCompletion::pending();
    let document = owner.target_document_id(TARGET).unwrap();
    let snapshot = owner
        .document_lifecycle_snapshot_for_target(TARGET)
        .unwrap();
    let document_handle = owner.document_handle_for_target(TARGET).unwrap();
    let key = owner
        .install_document_javascript_dialog_for_test(
            document_handle,
            RendererPendingJavaScriptDialog::new(
                RendererJavaScriptDialogId::new(2),
                RendererDocumentLifecycleIdentity {
                    frame: snapshot.frame,
                    document: snapshot.document,
                    epoch: snapshot.epoch,
                },
                RendererJavaScriptDialogSource::RootFrame,
                "about:blank".into(),
                "alert".into(),
                "peer".into(),
                String::new(),
                Some(peer_completion.clone()),
            ),
        )
        .unwrap();
    assert!(owner.project_javascript_dialog_for_session(
        TARGET,
        &peer,
        "FRAME-dialog-owner".into(),
        document_handle,
        key.unwrap(),
    ));
    owner.disable_devtools_page_domain_for_target(TARGET, &DevToolsSessionKey::Primary);
    assert!(!primary_completion.finish(true, "late primary".into()));
    assert!(!primary_completion.wait().accepted);
    assert!(
        owner
            .browser_context
            .web_contents_has_pending_javascript_dialog(document_handle.web_contents())
            .unwrap()
    );
    let (projected_document, key) = owner
        .projected_javascript_dialog_for_session(TARGET, &peer)
        .unwrap();
    assert_eq!(
        owner
            .document_javascript_dialog_snapshot(projected_document, key)
            .unwrap()
            .message,
        "peer"
    );
    assert!(!owner.dispose_devtools_session_for_target(TARGET, "SID-wrong", &peer));
    let (projected_document, key) = owner
        .projected_javascript_dialog_for_session(TARGET, &peer)
        .unwrap();
    assert!(
        owner
            .document_javascript_dialog_snapshot(projected_document, key)
            .is_some()
    );
    assert!(owner.dispose_devtools_session_for_target(TARGET, "SID-dialog-peer", &peer));
    assert!(!peer_completion.finish(true, "late peer".into()));
    assert!(!peer_completion.wait().accepted);
    assert!(
        !owner
            .browser_context
            .web_contents_has_pending_javascript_dialog(document_handle.web_contents())
            .unwrap()
    );
    assert_eq!(owner.target_document_id(TARGET), Some(document));
}

#[tokio::test]
async fn document_policy_completion_rejects_replacement_document() {
    let mut owner = context_with_document("data:text/html,<p>first policy owner</p>").await;
    let document = owner.document_handle_for_target(TARGET).unwrap();
    let completed = owner
        .start_document_policy_update(
            document,
            crate::conn::DocumentPolicyUpdate::CpuThrottlingRate(3.0),
        )
        .unwrap()
        .wait()
        .await;
    let navigator_completed = owner
        .start_document_policy_update(
            document,
            crate::conn::DocumentPolicyUpdate::NavigatorOverrides(
                moli_page_types::NavigatorOverrides {
                    online: Some(false),
                    max_touch_points: 2,
                    ..Default::default()
                },
            ),
        )
        .unwrap()
        .wait()
        .await;
    let surface_completed = owner
        .browser_context
        .start_document_page_surface_update(document, false, None, None)
        .unwrap()
        .wait()
        .await;

    let loaded =
        prepare_navigation(&mut owner, "data:text/html,<p>replacement policy owner</p>").await;
    observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let replacement = owner.document_handle_for_target(TARGET).unwrap();
    assert_ne!(replacement, document);
    for completed in [completed, navigator_completed] {
        assert_eq!(
            owner
                .browser_context
                .finish_document_policy_update(completed),
            Err("Document changed".to_owned())
        );
    }
    assert_eq!(
        owner
            .browser_context
            .finish_document_policy_batch(surface_completed),
        Err("Document changed".to_owned())
    );
    assert_eq!(owner.document_handle_for_target(TARGET), Some(replacement));

    let observed = owner
        .browser_context
        .evaluate_document_expression_for_test(
            replacement,
            "JSON.stringify([navigator.onLine, navigator.maxTouchPoints])",
            false,
        )
        .await
        .unwrap();
    assert_eq!(observed["value"], serde_json::json!("[true,0]"));
    owner
        .clear_target_page_for_test(TARGET)
        .unwrap()
        .close()
        .await;
}

#[tokio::test]
async fn document_native_command_completions_reject_replacement_document() {
    let mut owner = context_with_document("data:text/html,<input id=field>").await;
    let document = owner.document_handle_for_target(TARGET).unwrap();
    let autofill = owner
        .start_document_autofill_trigger(
            document,
            moli_core::page::RendererAutofillTriggerRequest {
                frame_id: None,
                field_id: 1,
                card: None,
                address: None,
            },
        )
        .unwrap()
        .wait()
        .await;
    let stopped = owner
        .start_document_lifecycle_stop(document)
        .unwrap()
        .wait()
        .await;

    let loaded = prepare_navigation(
        &mut owner,
        "data:text/html,<p>replacement native command owner</p>",
    )
    .await;
    observe_navigation_commit(&mut owner, loaded).await.unwrap();
    let replacement = owner.document_handle_for_target(TARGET).unwrap();
    assert_ne!(replacement, document);
    assert_eq!(
        owner.finish_document_autofill_trigger(autofill),
        Err("Document changed".to_owned())
    );
    assert!(matches!(
        owner.finish_document_lifecycle_stop(stopped),
        Err(error) if error == "Document changed"
    ));
    assert_eq!(owner.document_handle_for_target(TARGET), Some(replacement));

    owner
        .clear_target_page_for_test(TARGET)
        .unwrap()
        .close()
        .await;
}

#[tokio::test]
async fn document_replacement_preserves_stable_page_engine_history_and_storage() {
    let mut config = moli_fetch::FetchConfig::default();
    config.set_user_agent("stable-engine");
    let mut owner = context_with_document_and_runtime_config(
        "data:text/html,<title>first</title>",
        moli_core::runtime::NavigationRuntimeConfig::new(
            config,
            moli_core::OptionalResourceFetchMask::NONE,
            true,
            Default::default(),
        ),
    )
    .await;
    let stable_ids = (
        owner.selected_web_contents_id().unwrap(),
        owner.active_page_target().main_frame_slot_id(),
    );
    let stable_renderer_owner = owner.page_navigation_renderer_owner_id(TARGET).unwrap();
    let web_contents = owner.web_contents_handle_for_target(TARGET).unwrap();
    owner
        .update_web_contents_window_surface(
            web_contents,
            Some(crate::conn::WindowSurfaceState::Fullscreen),
            Some(800),
            Some(600),
            Some(10),
            Some(20),
        )
        .unwrap();
    let window = owner.web_contents_window_surface(web_contents).unwrap();
    owner.apply_target_emulation_policy_change(
        TARGET,
        crate::conn::EmulationPolicyChange::CpuThrottlingRate(4.0),
    );
    owner.mutate_devtools_network_session_state_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        |raw| {
            raw.network_enabled = true;
            raw.cache_disabled = true;
            raw.bypass_service_worker = true;
            raw.blocked_url_patterns = vec!["blocked/*".into()];
            raw.extra_headers = vec![("X-Stable".into(), "contents".into())];
        },
    );
    owner.set_devtools_browser_identity_override_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        crate::conn::DevToolsBrowserIdentityOverride::from_command(
            &moli_browser_profile::BrowserIdentityProfile::default(),
            "Moli/Stable-Identity".into(),
            Some("fr-FR".into()),
            None,
            None,
        ),
    );
    owner
        .set_devtools_locale_override_for_target(
            TARGET,
            &moli_page_types::DevToolsSessionKey::Primary,
            Some("de-DE".into()),
        )
        .unwrap();
    owner
        .set_devtools_timezone_override_for_target(
            TARGET,
            &moli_page_types::DevToolsSessionKey::Primary,
            Some("Europe/Berlin".into()),
        )
        .unwrap();
    let policy = owner.effective_policy_for_target(TARGET);
    owner.set_network_offline_for_target(TARGET, true);
    owner.set_tls_verify_host_override_for_target(TARGET, Some(false));
    owner.set_devtools_bypass_csp_enabled_for_target(
        TARGET,
        &moli_page_types::DevToolsSessionKey::Primary,
        true,
    );
    let first_document = owner.target_document_id(TARGET).unwrap();
    let storage = owner
        .page_storage_handles_for_target(TARGET)
        .unwrap()
        .session_storage_store
        .clone();
    assert!(
        storage
            .lock()
            .set_item("https://example.test", "key", "value")
    );
    let observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();

    let loaded = prepare_navigation(
        &mut owner,
        "data:text/html,<title>second</title><p>second</p>",
    )
    .await;
    let reserved = loaded.request().document;
    observe_navigation_commit(&mut owner, loaded).await.unwrap();

    assert_eq!(
        (
            owner.selected_web_contents_id().unwrap(),
            owner.active_page_target().main_frame_slot_id()
        ),
        stable_ids
    );
    assert_eq!(owner.target_document_id(TARGET), Some(reserved));
    assert_eq!(
        owner.web_contents_window_surface(web_contents).unwrap(),
        window
    );
    assert_eq!(
        owner
            .target_emulation_policy(TARGET)
            .unwrap()
            .cpu_throttling_rate,
        4.0
    );
    assert_eq!(owner.effective_policy_for_target(TARGET), policy);
    assert!(owner.network_offline_for_target(TARGET));
    assert_eq!(
        owner.tls_verify_host_override_for_target(TARGET),
        Some(false)
    );
    assert!(owner.bypass_content_security_policy_for_target(TARGET));
    assert_ne!(first_document, reserved);
    assert_eq!(
        owner.page_navigation_renderer_owner_id(TARGET),
        Some(stable_renderer_owner),
        "Document replacement must reuse the WebContents navigation engine"
    );
    let current_storage = owner
        .page_storage_handles_for_target(TARGET)
        .unwrap()
        .session_storage_store;
    assert!(std::sync::Arc::ptr_eq(&storage, &current_storage));
    assert_eq!(
        storage.lock().get_item("https://example.test", "key"),
        Some("value".into())
    );
    let (index, history) = owner.target_navigation_history_snapshot(TARGET).unwrap();
    assert_eq!(index, 1);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].title, "first");
    assert_eq!(history[1].title, "second");
    assert_eq!(
        observer.wait().await,
        moli_core::browser::DocumentRetirement::Superseded
    );
}

#[tokio::test]
async fn web_contents_owns_live_document_and_navigation_after_protocol_residence_is_dropped() {
    let mut owner = context_with_document("data:text/html,<p>owned</p>").await;
    let renderer = owner
        .target_renderer_page_residence_identity(TARGET)
        .unwrap();
    let artifacts = owner.initial_artifacts.take().unwrap();
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        artifacts,
        None,
        "frame".into(),
        "loader".into(),
    );
    let document = owner.target_document_id(TARGET).unwrap();
    let observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();
    let handle = owner.web_contents_handle_for_target(TARGET).unwrap();
    let (stable_id, frame_id) = owner.browser_context.web_contents_identity(handle).unwrap();
    let pending = prepare_navigation(&mut owner, "data:text/html,pending").await;
    let navigation = pending.request().navigation;
    let cancellation = owner
        .document_navigation_cancellation_handle_for_target(TARGET, &navigation)
        .unwrap();
    let snapshot = owner
        .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
        .unwrap();

    // Drop only DevTools; the registered Browser subtree stays in its Context.
    drop(owner.page_targets.remove(TARGET).unwrap());
    assert_eq!(owner.selected_web_contents_id(), Some(stable_id));
    assert_eq!(
        owner.browser_context.web_contents_identity(handle),
        Ok((stable_id, frame_id))
    );
    let current = DocumentHandle::new(handle, document);
    assert_eq!(
        owner.browser_context.document_handle(handle),
        Ok(Some(current))
    );
    assert_eq!(
        owner
            .browser_context
            .document_renderer_residence(current)
            .unwrap(),
        renderer
    );
    assert_eq!(
        owner.browser_context.document_lifecycle_snapshot(current),
        Ok(Some(snapshot))
    );
    assert!(
        owner
            .browser_context
            .accepts_pending_navigation(handle, &navigation)
            .unwrap()
    );
    assert!(!cancellation.is_cancelled());
    let mut wait = Box::pin(observer.wait());
    assert_eq!(
        wait.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );

    drop(owner);
    assert!(pending.wait().await.is_err());
    assert!(cancellation.is_cancelled());
    assert_eq!(
        wait.await,
        moli_core::browser::DocumentRetirement::Unavailable
    );
}

#[tokio::test]
async fn replacement_retires_document_identity_lifecycle_and_lifetime_together() {
    let mut owner = context_with_document("data:text/html,<p>first</p>").await;
    let first_renderer = owner
        .target_renderer_page_residence_identity(TARGET)
        .unwrap();
    let first_artifacts = owner.initial_artifacts.take().unwrap();
    let first_id = owner.target_document_id(TARGET).unwrap();
    assert_eq!(owner.target_document_id(TARGET), Some(first_id));
    owner.bind_renderer_document_lifecycle_for_target(
        TARGET,
        first_artifacts.clone(),
        None,
        "frame".into(),
        "first-loader".into(),
    );
    assert!(
        owner
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .is_some()
    );
    let first_observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();
    let another_first_observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();

    // Moving the whole Context or failing a pending navigation must not retire
    // the current Document. Its Page/lifecycle/identity move as one object.
    let mut moved = owner;
    let failed = prepare_navigation(&mut moved, "data:text/html,canceled").await;
    let failed_navigation = failed.request().navigation;
    let before = moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET);
    assert!(
        moved.clear_pending_document_navigation_if_matches_for_target(TARGET, &failed_navigation)
    );
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        before
    );
    assert!(failed.wait().await.is_err());
    let mut first_wait = Box::pin(first_observer.wait());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(first_wait.as_mut().poll(&mut context), Poll::Pending);

    let loaded = prepare_navigation(&mut moved, "data:text/html,<p>second</p>").await;
    let navigation = loaded.request().navigation;
    let reserved_id = loaded.request().document;
    let second_renderer = prepared_renderer(&moved, &loaded);
    assert!(
        moved.bind_pending_document_navigation_renderer_page_for_target(
            TARGET,
            &navigation,
            second_renderer
        )
    );
    let (replacement, _) = observe_navigation_commit(&mut moved, loaded).await.unwrap();
    let second_artifacts = replacement.lifecycle.artifacts.clone();
    let loaded_snapshot = moved
        .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
        .unwrap();
    assert_eq!(moved.target_document_id(TARGET), Some(reserved_id));
    assert_ne!(first_id, reserved_id);
    assert!(!moved.routes_renderer_page_for_target(TARGET, first_renderer));
    assert!(moved.routes_renderer_page_for_target(TARGET, second_renderer));
    assert_eq!(
        first_wait.await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    assert_eq!(
        another_first_observer.wait().await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        Some(loaded_snapshot),
        "the replacement must expose its own lifecycle, not the previous Document's"
    );
    assert!(
        moved
            .page_slot_for_target(TARGET)
            .unwrap()
            .renderer_document_lifecycle_visible_snapshot()
            .is_none()
    );
    assert!(
        moved
            .renderer_document_lifecycle_binding_for_target(TARGET)
            .is_none()
    );

    moved.bind_renderer_document_lifecycle_for_target(
        TARGET,
        second_artifacts,
        Some(navigation),
        "frame".into(),
        "second-loader".into(),
    );
    let second_snapshot =
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET);
    assert!(second_snapshot.is_some());
    assert!(
        moved
            .ingest_renderer_document_lifecycle_events_for_target(
                TARGET,
                first_artifacts.initial_lifecycle_events
            )
            .is_empty()
    );
    assert_eq!(
        moved.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        second_snapshot
    );
    let second_observer = moved.document_lifetime_observer_for_target(TARGET).unwrap();
    let second_page = moved
        .retire_loaded_document_with_reason_for_target(
            TARGET,
            TargetPageAbsenceReason::TargetClosed,
        )
        .unwrap();
    assert!(!moved.target_has_loaded_page(TARGET));
    assert_eq!(moved.target_document_id(TARGET), None);
    assert!(
        moved
            .document_lifetime_observer_for_target(TARGET)
            .is_none()
    );
    assert!(
        moved
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .is_none()
    );
    assert_eq!(
        second_observer.wait().await,
        moli_core::browser::DocumentRetirement::Superseded
    );
    second_page.close().await;
}

#[tokio::test]
async fn discarded_prepared_candidate_preserves_current_document_until_owner_loss() {
    let mut owner = context_with_document("data:text/html,<p>first</p>").await;
    let first_renderer = owner
        .target_renderer_page_residence_identity(TARGET)
        .unwrap();
    let first_id = owner.target_document_id(TARGET);
    let observer = owner.document_lifetime_observer_for_target(TARGET).unwrap();
    let candidate = prepare_navigation(&mut owner, "data:text/html,<p>candidate</p>").await;
    let navigation = candidate.request().navigation;
    assert_ne!(prepared_renderer(&owner, &candidate), first_renderer);
    drop(candidate);
    assert_eq!(owner.target_document_id(TARGET), first_id);
    assert_eq!(
        owner
            .target_renderer_page_residence_identity(TARGET)
            .unwrap(),
        first_renderer
    );
    assert!(owner.accepts_pending_document_navigation_event_for_target(TARGET, &navigation));
    let mut wait = Box::pin(observer.wait());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(wait.as_mut().poll(&mut context), Poll::Pending);
    drop(owner);
    assert_eq!(
        wait.await,
        moli_core::browser::DocumentRetirement::Unavailable
    );
}
