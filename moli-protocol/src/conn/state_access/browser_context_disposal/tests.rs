use crate::conn::{
    BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
    PendingCdpCommandDispatch, ProtocolStateHandle,
};
use crate::testing::TestContext;
use futures_util::FutureExt as _;
use serde_json::{Value, json};

struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

fn start(
    handle: &ProtocolStateHandle,
    command: Value,
    context: &mut CommandDispatchContext,
) -> CdpCommandTaskStep {
    let command = ParsedCdpCommand::parse_str(command.to_string()).unwrap();
    handle
        .with_state(|state| state.start_parsed_command_dispatch_with_context(&command, context))
        .unwrap()
}

fn recover(
    handle: &ProtocolStateHandle,
    context: &mut CommandDispatchContext,
) -> PendingCdpCommandDispatch {
    let CdpCommandTaskStep::Pending(pending) = handle
        .with_state(|state| {
            state.complete_failed_command_wait_with_context(
                Some(2001),
                None,
                crate::OwnedProtocolWaitFailure::Cancelled.message(),
                context,
            )
        })
        .unwrap()
    else {
        panic!("the original cleanup retains its continuation and output");
    };
    *pending
}

#[tokio::test]
async fn context_disposal_owns_native_close_through_cancellation_and_ready_loss() {
    for phase in 0..3 {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-dispose-owned".into());
        browser.set_active_target_id("TID-dispose-owned");
        browser.attach_active_session("SID-dispose-owned");
        fixture.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-dispose-owned"))
            .await;
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (native, guard) = fixture.with_fixture(|conn| {
            conn.loaded_page_mut_for_protocol_access(Some("SID-dispose-owned"))
                .unwrap()
                .start_cdp_call(
                    2099,
                    None,
                    moli_core::RendererNativeOperation::then(
                        moli_core::RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            moli_core::RendererNativeOperationStep::Complete(
                                moli_core::RendererNativeProtocolResponse::success(json!({})),
                            )
                        },
                    ),
                )
                .unwrap()
        });
        let release = ReleaseOnDrop(Some(release));
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        let owner = fixture.state;
        let handle = owner.handle();
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = start(
            &handle,
            json!({
                "id":2001,"method":"Target.disposeBrowserContext",
                "params":{"browserContextId":"BID-dispose-owned"},
            }),
            &mut context,
        ) else {
            panic!("context disposal starts its supervised workflow at admission");
        };
        assert_eq!(pending.kind_name(), "Workflow");
        let mut wait = Box::pin(pending.wait());
        assert!(wait.as_mut().now_or_never().is_none());
        handle
            .with_state(|state| {
                let context = state
                    .connection
                    .browser_context_by_id("BID-dispose-owned")
                    .unwrap();
                assert!(context.closing);
                assert!(state.connection.state_access.is_some());
            })
            .unwrap();
        assert!(matches!(
            start(
                &handle,
                json!({
                    "id":2002,"method":"Browser.getVersion",
                }),
                &mut CommandDispatchContext::default()
            ),
            CdpCommandTaskStep::Complete(_)
        ));
        let CdpCommandTaskStep::Complete(rejected) = start(
            &handle,
            json!({
                "id":2003,"method":"Target.createTarget",
                "params":{"browserContextId":"BID-dispose-owned","url":"about:blank"},
            }),
            &mut CommandDispatchContext::default(),
        ) else {
            panic!("a context being disposed cannot admit another target");
        };
        assert!(
            rejected
                .into_parts()
                .0
                .iter()
                .any(|reply| reply["id"] == 2003 && reply["error"].is_object())
        );
        if phase == 1 {
            drop(wait);
            wait = Box::pin(recover(&handle, &mut context).wait());
            assert!(wait.as_mut().now_or_never().is_none());
        }
        drop(release);
        native.wait().await.unwrap();
        drop(guard);
        let ready = tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .unwrap();
        let ready = if phase == 2 {
            drop(ready);
            recover(&handle, &mut context).wait().await
        } else {
            ready
        };
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| state.complete_command_dispatch_with_context(ready, &mut context))
            .unwrap()
        else {
            panic!("one finite owner turn publishes context disposal");
        };
        let (events, _, predecessor) = outcome.into_protocol_event_parts();
        assert!(
            predecessor.is_none(),
            "a Browser ACK cannot join unrelated Page streams"
        );
        let messages = events
            .into_iter()
            .map(|event| {
                event
                    .into_protocol_message()
                    .into_json()
                    .expect("scalar protocol fixture")
            })
            .collect::<Vec<_>>();
        let replies = messages
            .iter()
            .filter(|reply| reply["id"] == 2001)
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 1, "phase={phase} {messages:?}");
        assert_eq!(
            replies[0]["result"],
            json!({}),
            "phase={phase} {messages:?}"
        );
        let mut previous = messages
            .iter()
            .position(|reply| reply["id"] == 2001)
            .unwrap();
        for method in ["Inspector.detached", "Target.detachedFromTarget"] {
            let indices = messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message["method"] == method)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            assert_eq!(indices.len(), 1, "phase={phase} {messages:?}");
            assert!(indices[0] > previous, "phase={phase} {messages:?}");
            previous = indices[0];
        }
        handle
            .with_state(|state| {
                assert!(!state.connection.has_browser_context_id("BID-dispose-owned"));
                assert!(
                    !state
                        .connection
                        .has_attached_devtools_session("SID-dispose-owned")
                );
            })
            .unwrap();
    }
}

#[tokio::test]
async fn retired_context_disposal_preserves_a_reused_context_id() {
    let fixture = TestContext::new();
    let mut original = BrowserContext::new("BID-reused-disposal".into());
    original.set_active_target_id("TID-retired-disposal");
    fixture.with_fixture(|conn| conn.install_browser_context_fixture_for_test(original));
    let disposal = fixture
        .with_fixture(|conn| {
            crate::domains::target::PreparedBrowserContextDisposal::prepare(
                conn,
                "BID-reused-disposal".into(),
            )
        })
        .unwrap();
    let mut removed = fixture.with_fixture(|conn| {
        conn.remove_browser_context_by_id_restoring_active("BID-reused-disposal", None)
            .unwrap()
    });
    let mut successor = BrowserContext::new("BID-reused-disposal".into());
    successor.set_active_target_id("TID-successor-disposal");
    fixture.with_fixture(|conn| conn.insert_browser_context(successor));
    let owner = fixture.state;
    let handle = owner.handle();
    let mut original_context = CommandDispatchContext::default();
    let (mut context, _scope) =
        crate::conn::CommandWorkflowContext::take_from(&mut original_context, Some(2010), None);
    disposal
        .run(
            &mut crate::conn::ProtocolWorkflowDriver::new(handle.clone()),
            &mut context,
        )
        .await
        .unwrap();
    handle
        .with_state(|state| {
            state.finish_browser_context_disposal(disposal);
        })
        .unwrap();
    handle
        .with_state(|state| {
            let successor = state
                .connection
                .browser_context_by_id("BID-reused-disposal")
                .unwrap();
            assert!(!successor.closing);
            assert_eq!(successor.active_target_id(), Some("TID-successor-disposal"));
        })
        .unwrap();
    removed.close_all_pages_async().await;
}
