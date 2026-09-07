use super::*;
use crate::conn::BrowserContext;
use moli_core::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleMilestone, RendererLifecycleEpoch, RendererLifecycleStartReason,
};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

const TARGET: &str = "initial-projection";

fn context() -> BrowserContext {
    let mut context = BrowserContext::new("initial-context".into());
    context.bind_page_navigation_engines(Default::default(), None);
    context.set_active_target_id(TARGET);
    context.begin_active_target_initial_empty_document("about:blank".into());
    context
}

fn build(context: &mut BrowserContext) -> AdmittedInitialDocumentBuild {
    let InitialDocumentAdmission::Build(build) = context
        .start_initial_document_for_target(
            TARGET,
            Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap()
    else {
        panic!("expected new native build");
    };
    *build
}

fn join(context: &mut BrowserContext) -> InitialDocumentPageBuildWaiter {
    let InitialDocumentAdmission::Join(waiter) = context
        .start_initial_document_for_target(
            TARGET,
            Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .unwrap()
    else {
        panic!("expected join");
    };
    waiter
}

#[tokio::test]
async fn dropping_unpolled_work_cancels_waiters_and_allows_fresh_admission() {
    let mut context = context();
    let first = build(&mut context);
    let first_key = first.key();
    let waiter = join(&mut context);
    drop(first);
    assert_eq!(
        waiter.wait().await,
        Err("InitialDocumentPageBuildCancelled".into())
    );
    let second = build(&mut context);
    assert_ne!(first_key.document(), second.key().document());
    let waiter = join(&mut context);
    let candidate = second.materialize().await.unwrap();
    context
        .commit_initial_document(candidate)
        .unwrap_or_else(|_| panic!("commit rejected"));
    assert_eq!(waiter.wait().await, Ok(()));
}

#[tokio::test]
async fn navigation_cancels_initial_work_before_materialization_and_notifies_joiners() {
    let mut context = context();
    let first = build(&mut context);
    let waiter = join(&mut context);
    context.begin_target_document_navigation(TARGET, "next".into());
    assert_eq!(
        waiter.wait().await,
        Err("InitialDocumentPageBuildCancelled".into())
    );
    assert_eq!(
        first.materialize().await.err().unwrap().to_string(),
        "InitialDocumentPageBuildCancelled"
    );
    assert!(!context.target_has_loaded_page(TARGET));
}

#[tokio::test]
async fn late_initial_candidate_cannot_commit_or_complete_a_retry() {
    let mut context = context();
    let first = build(&mut context);
    let first_waiter = join(&mut context);
    let candidate = first.materialize().await.unwrap();
    context.clear_document_navigation_state_for_active_target();
    assert_eq!(
        first_waiter.wait().await,
        Err("InitialDocumentPageBuildCancelled".into())
    );
    let second = build(&mut context);
    let second_key = second.key();
    let mut second_waiter = Box::pin(join(&mut context).wait());
    let rejected = context
        .commit_initial_document(candidate)
        .expect_err("stale candidate");
    rejected.retire().await;
    assert_eq!(
        second_waiter
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );
    let candidate = second.materialize().await.unwrap();
    context
        .commit_initial_document(candidate)
        .unwrap_or_else(|_| panic!("retry commit rejected"));
    assert_eq!(second_waiter.await, Ok(()));
    assert_eq!(
        context.target_document_id(TARGET),
        Some(second_key.document())
    );
}

#[tokio::test]
async fn native_initial_commit_survives_projection_loss_and_has_complete_lifecycle() {
    let mut context = context();
    let work = build(&mut context);
    let key = work.key();
    let waiter = join(&mut context);
    let candidate = work.materialize().await.unwrap();
    assert!(candidate.lifecycle.snapshot().unwrap().load.is_some());
    let projection = context.page_targets.remove(TARGET).unwrap();
    context
        .commit_initial_document(candidate)
        .unwrap_or_else(|_| panic!("projection vetoed Browser commit"));
    assert_eq!(waiter.wait().await, Ok(()));
    assert!(context.owns_web_contents(key.web_contents()));
    assert_eq!(
        RendererPageResidenceIdentity::from_page(context.loaded_page().unwrap()),
        key.renderer()
    );
    // Reintroduce only the projection to inspect the already committed native
    // identity/lifecycle; this performs no Browser operation or Page install.
    assert!(context.page_targets.insert(projection));
    assert_eq!(context.target_document_id(TARGET), Some(key.document()));
    assert!(
        context
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .unwrap()
            .load
            .is_some()
    );
}

#[tokio::test]
async fn initial_candidate_cannot_be_committed_into_another_web_contents() {
    let mut source = context();
    let mut peer = context();
    let work = build(&mut source);
    let candidate = work.materialize().await.unwrap();
    let waiter = join(&mut source);
    let candidate = peer
        .commit_initial_document(candidate)
        .expect_err("wrong WebContents");
    assert!(!peer.target_has_loaded_page(TARGET));
    source
        .commit_initial_document(*candidate)
        .unwrap_or_else(|_| panic!("exact owner must still accept candidate"));
    assert_eq!(waiter.wait().await, Ok(()));
}

#[tokio::test]
async fn initial_creation_lifecycle_exits_empty_document_before_projection_can_drive_it() {
    let mut owner = context();
    let mut candidate = build(&mut owner).materialize().await.unwrap();
    let mut artifacts = candidate.built.page_creation_artifacts.clone();
    // Feed a complete, consistent creation occurrence into the installation
    // boundary. A parser script's document.open() is not a way to construct
    // this fixture: the parser may ignore that call.
    let snapshot = artifacts.lifecycle_snapshot;
    let sequence = artifacts.initial_lifecycle_events.last().unwrap().sequence + 1;
    let terminated = RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence,
        timestamp_micros: snapshot.load.unwrap().timestamp_micros + 1,
        kind: RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(RendererDocumentLifecycleMilestone::Load),
            reason: moli_core::page::RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    let restarted = RendererDocumentLifecycleEvent {
        epoch: RendererLifecycleEpoch(snapshot.epoch.0 + 1),
        sequence: sequence + 1,
        timestamp_micros: terminated.timestamp_micros + 1,
        kind: RendererDocumentLifecycleEventKind::Started {
            reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
        },
        ..terminated
    };
    let load = RendererDocumentLifecycleEvent {
        sequence: sequence + 2,
        timestamp_micros: restarted.timestamp_micros + 1,
        kind: RendererDocumentLifecycleEventKind::Milestone(
            RendererDocumentLifecycleMilestone::Load,
        ),
        ..restarted
    };
    for event in [terminated, restarted, load] {
        artifacts.lifecycle_snapshot.apply_event(event);
        artifacts.initial_lifecycle_events.push(event);
    }
    artifacts.active_epoch = restarted.epoch;
    assert!(DocumentLifecycle::from_creation_artifacts(&artifacts).is_some());
    let snapshot = artifacts.lifecycle_snapshot;
    assert!(snapshot.epoch.0 > 1);
    assert!(snapshot.load.is_some());
    candidate.lifecycle = DocumentLifecycle::from_creation_artifacts(&artifacts).unwrap();
    candidate.built.page_creation_artifacts = artifacts;
    owner
        .commit_initial_document(candidate)
        .unwrap_or_else(|_| panic!("native commit rejected"));
    assert_eq!(
        owner.renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET),
        Some(snapshot)
    );
    assert_eq!(
        owner.target_is_on_initial_empty_document(TARGET),
        Some(false)
    );
}

#[tokio::test]
async fn initial_preload_observes_native_admission_policy_before_materialization() {
    let mut context = context();
    let mut defaults = Default::default();
    let permission = |setting: &str| moli_core::page::PermissionOverrideRegistration {
        permission: serde_json::json!({"name": "geolocation"}),
        setting: setting.into(),
        origin: None,
        embedded_origin: None,
    };
    context.set_permission_override(&mut defaults, permission("granted"));
    let InitialDocumentAdmission::Build(mut work) = context
        .start_initial_document_for_target(
            TARGET,
            Default::default(),
            &defaults,
            &Default::default(),
        )
        .unwrap()
    else {
        panic!("expected build");
    };
    context.set_permission_override(&mut defaults, permission("denied"));
    work.start_preparation().unwrap();
    let configured = work.inspection_endpoint().start_configure(
        moli_renderer_v8::RendererPreparedDocumentInspectionConfiguration {
            root_frame_projection_id: Some(TARGET.into()),
            document_start_scripts: vec![moli_core::page::DocumentStartScript {
                registry_key: None, devtools_session: None,
                source: "globalThis.__initialPermission = navigator.permissions.query({name:'geolocation'}).then(permission => permission.state);".into(),
                world_name: None, has_bidi_channel_argument: false, bidi_channel_handoffs: Vec::new(),
            }],
            ..Default::default()
        },
    );
    configured.await.unwrap();
    let mut candidate = work.materialize().await.unwrap();
    assert_eq!(
        candidate
            .built
            .page
            .evaluate_runtime_expression_with_await_async("globalThis.__initialPermission", true)
            .await
            .unwrap()["value"],
        "granted",
        "preload must observe the policy frozen by Browser admission, not the later Context override"
    );
}
