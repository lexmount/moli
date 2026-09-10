use super::*;
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult, DevToolsGetRealmsCommand,
    DevToolsProtocol, DevToolsTargetId, RuntimeExecutionContextEvent,
};

const SESSION: &str = "SID-worker-realm";

#[derive(Clone, Copy)]
enum WorkerKind {
    Dedicated,
    Shared,
    Service,
}

impl WorkerKind {
    fn install(self, ctx: &mut TestContext) -> &'static str {
        match self {
            Self::Dedicated => {
                load_dedicated_worker_target(ctx, SESSION);
                "TID-dedicated-worker"
            }
            Self::Shared => {
                load_shared_worker_target(ctx, SESSION);
                "TID-shared-worker"
            }
            Self::Service => {
                let mut context = ctx
                    .conn
                    .new_browser_context_fixture_for_test("BID-service".to_owned());
                let mut target = crate::conn::ServiceWorkerTargetState::new(
                    3,
                    7,
                    "TID-service-worker".to_owned(),
                    "https://example.test/service-worker.js".to_owned(),
                    "https://example.test/".to_owned(),
                    moli_core::page::RendererServiceWorkerVersionStatus::Activated,
                    None,
                );
                target.attach_session(SESSION.to_owned());
                context.insert_service_worker_target(target);
                ctx.conn.install_browser_context_fixture_for_test(context);
                "TID-service-worker"
            }
        }
    }

    fn realm_type(self) -> &'static str {
        match self {
            Self::Dedicated => "worker",
            Self::Shared => "shared-worker",
            Self::Service => "service-worker",
        }
    }
}

fn route_context(ctx: &mut TestContext, message: serde_json::Value) -> serde_json::Value {
    let mut responses = Vec::new();
    let mut events = Vec::new();
    ctx.conn.route_inspector_messages_into(
        vec![message],
        None,
        Some(SESSION),
        &mut responses,
        &mut events,
    );
    assert!(responses.is_empty());
    assert_eq!(events.len(), 1);
    events.pop().unwrap().into_protocol_message()
}

fn create_context(ctx: &mut TestContext, realm: Option<&str>) -> serde_json::Value {
    route_context(
        ctx,
        json!({
            "method": "Runtime.executionContextCreated",
            "params": { "context": {
                "id": 71,
                "uniqueId": realm,
                "origin": "https://runtime.example",
                "name": "actual worker global",
                "auxData": { "isDefault": true, "type": "worker" }
            }}
        }),
    )
}

async fn get_realms(ctx: &mut TestContext, target: &str) -> Vec<RuntimeExecutionContextEvent> {
    let (result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: Some(DevToolsTargetId::from(target)),
                browser_context_id: None,
            },
            realm_type: None,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(result) = result.expect("worker realm inventory") else {
        panic!("expected realms");
    };
    result.realms
}

async fn assert_realm_and_replay(
    ctx: &mut TestContext,
    kind: WorkerKind,
    target: &str,
    created: &serde_json::Value,
    realm_id: Option<&str>,
) {
    let realms = get_realms(ctx, target).await;
    assert_eq!(realms.len(), 1);
    let realm = &realms[0];
    assert_eq!(realm.context_id, Some(71));
    assert_eq!(realm.realm_id.as_ref().map(|id| id.as_str()), realm_id);
    assert_eq!(realm.target_id.as_ref().map(|id| id.as_str()), Some(target));
    assert_eq!(realm.origin.as_deref(), Some("https://runtime.example"));
    assert_eq!(realm.name.as_deref(), Some("actual worker global"));
    assert_eq!(realm.context_type.as_deref(), Some(kind.realm_type()));
    assert_eq!(realm.frame_id, None);

    ctx.process_async(json!({
        "id": 1, "method": "Runtime.enable", "sessionId": SESSION
    }))
    .await;
    ctx.expect_result(1, json!({}), Some(SESSION));
    let replay = ctx.take_first_matching("worker context replay", |message| {
        message["method"] == "Runtime.executionContextCreated"
    });
    assert_eq!(
        replay, *created,
        "replay must retain the exact renderer context"
    );
}

async fn worker_realm_lifecycle(kind: WorkerKind) {
    let mut ctx = TestContext::new();
    let target = kind.install(&mut ctx);
    assert!(get_realms(&mut ctx, target).await.is_empty());

    let first = create_context(&mut ctx, Some("first-run"));
    let first_id = format!("{target}:first-run");
    assert_eq!(first["params"]["context"]["uniqueId"], first_id);
    assert_realm_and_replay(&mut ctx, kind, target, &first, Some(&first_id)).await;

    // The numeric V8 id may be reused; the old uniqueId must not retire its replacement.
    let second = create_context(&mut ctx, Some("second-run"));
    let second_id = format!("{target}:second-run");
    route_context(
        &mut ctx,
        json!({
            "method": "Runtime.executionContextDestroyed",
            "params": { "executionContextId": 71, "executionContextUniqueId": "first-run" }
        }),
    );
    assert_realm_and_replay(&mut ctx, kind, target, &second, Some(&second_id)).await;
    route_context(
        &mut ctx,
        json!({
            "method": "Runtime.executionContextDestroyed",
            "params": { "executionContextId": 71, "executionContextUniqueId": "second-run" }
        }),
    );
    assert!(get_realms(&mut ctx, target).await.is_empty());

    create_context(&mut ctx, Some("third-run"));
    route_context(
        &mut ctx,
        json!({
            "method": "Runtime.executionContextsCleared", "params": {}
        }),
    );
    assert!(get_realms(&mut ctx, target).await.is_empty());

    // An absent native uniqueId is not permission to invent a target-derived realm.
    let without_id = create_context(&mut ctx, None);
    assert_realm_and_replay(&mut ctx, kind, target, &without_id, None).await;
}

#[tokio::test]
async fn dedicated_worker_realm_inventory_and_replay_keep_exact_context() {
    worker_realm_lifecycle(WorkerKind::Dedicated).await;
}

#[tokio::test]
async fn shared_worker_realm_inventory_and_replay_keep_exact_context() {
    worker_realm_lifecycle(WorkerKind::Shared).await;
}

#[tokio::test]
async fn service_worker_realm_inventory_and_replay_keep_exact_context() {
    worker_realm_lifecycle(WorkerKind::Service).await;
}
