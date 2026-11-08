use super::*;
use crate::conn::{BrowserContext, SessionDisposalPlan};
use crate::testing::TestContext;

#[derive(Clone, Copy)]
enum Interleaving {
    ForegroundChange,
    SessionClosing,
    TargetClosing,
    RecreatedHost,
}

#[tokio::test]
async fn delayed_stream_registration_preserves_its_page_and_rejects_retired_owners() {
    for interleaving in [
        Interleaving::ForegroundChange,
        Interleaving::SessionClosing,
        Interleaving::TargetClosing,
        Interleaving::RecreatedHost,
    ] {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-output".into());
        browser.set_active_target_id("TID-output");
        browser.attach_active_session("SID-output");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-output"))
            .await;
        let command_owner = CommandOwnerScope::for_session("SID-output");
        let scope = context
            .with_fixture(|conn| RuntimePageScope::capture(conn, &command_owner))
            .unwrap();
        match interleaving {
            Interleaving::ForegroundChange => {
                context.with_fixture(|conn| {
                    let browser = conn.browser_context.as_mut().unwrap();
                    browser.stage_background_target(
                        "TID-peer".into(),
                        Some("SID-peer".into()),
                        "about:blank".into(),
                        None,
                        None,
                    );
                    browser.set_active_target_id("TID-peer");
                });
            }
            Interleaving::SessionClosing => {
                let route =
                    context.with_fixture(|conn| conn.session_route(Some("SID-output")).unwrap());
                let plan = SessionDisposalPlan::for_session_route("SID-output", &route).unwrap();
                context.with_fixture(|conn| conn.mark_session_closing(&plan));
                context.with_fixture(|conn| assert!(scope.is_current(conn)));
            }
            Interleaving::TargetClosing => {
                context.with_fixture(|conn| conn.mark_page_target_closing(&command_owner));
                context.with_fixture(|conn| assert!(scope.is_current(conn)));
            }
            Interleaving::RecreatedHost => {
                let mut replacement = BrowserContext::new("BID-output".into());
                replacement.set_active_target_id("TID-output");
                replacement.attach_active_session("SID-output");
                context.with_fixture(|conn| {
                    conn.install_browser_context_fixture_for_test(replacement)
                });
                context.with_fixture(|conn| assert!(!scope.is_current(conn)));
            }
        }
        let owner = context.state;
        let handle = owner.handle();
        // The immutable output is completed after the original admission and
        // the controlled foreground/Closing/host change.
        let body = crate::conn::run_body_storage_work(|| {
            Ok(CapturedBody::from_bytes_spooled(vec![b'a'; 1_100_000]))
        })
        .await
        .unwrap();
        let result = handle
            .with_state(move |state| state.open_io_stream_for_page(scope, body))
            .unwrap();
        if matches!(interleaving, Interleaving::ForegroundChange) {
            let stream = result.unwrap();
            let read = handle
                .with_state(|state| {
                    let original = state
                        .connection
                        .io_stream_for_stream_owner(Some("SID-output"), &stream)
                        .expect("the stream belongs to its original Page");
                    assert!(
                        state
                            .connection
                            .browser_context
                            .as_ref()
                            .unwrap()
                            .page_target("TID-peer")
                            .unwrap()
                            .runtime_slot
                            .io_stream(&stream)
                            .is_none()
                    );
                    original.prepare_read(None, None)
                })
                .unwrap();
            let value = read.run().await.unwrap();
            assert!(value["data"].as_str().unwrap().as_bytes() == vec![b'a'; 1_100_000]);
            assert_eq!(value["eof"], true);
        } else {
            assert!(result.unwrap_err().is::<NoDocumentLoaded>());
        }
    }
}
