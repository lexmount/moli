use crate::conn::{
    BrowserContext, CdpSchedulerEvent, CommandOwnerScope, CompletedInitialDocumentPageBuild,
    PendingProtocolSchedulerWork, ProtocolWorkflowDriver,
};
use crate::testing::TestContext;
use moli_core::page::RendererDocumentSourcedTopLevelLocationNavigation;

const TARGET_URL: &str = "data:text/html,%3Cmain%3Einitial-location%3C/main%3E";
const SESSION: &str = "SID-initial-location";

struct InitialNavigationFixture {
    context: TestContext,
    built: CompletedInitialDocumentPageBuild,
    owner: CommandOwnerScope,
    pending: PendingProtocolSchedulerWork,
}

async fn initial_navigation_fixture() -> InitialNavigationFixture {
    let mut browser = BrowserContext::new("BID-initial-location".into());
    browser.set_active_target_id("TID-initial-location");
    browser.set_target_url("about:blank".into());
    browser.attach_active_session(SESSION);
    let context = TestContext::new();
    context.with_fixture(|connection| connection.install_browser_context_fixture_for_test(browser));
    let owner = CommandOwnerScope::for_session(SESSION);
    let build = context
        .with_fixture(|connection| connection.start_initial_document_page_ensure_for_owner(&owner))
        .unwrap()
        .expect("reserve and bind the initial Page through the production build path");
    let built = build.wait().await.unwrap_or_else(|error| panic!("{error}"));
    let CompletedInitialDocumentPageBuild::Built { built: native, .. } = &built else {
        panic!("this fixture owns the initial Page build");
    };
    let artifacts = &native.page_creation_artifacts;
    let source_document = moli_core::RendererDocumentLifecycleIdentity {
        frame: artifacts.lifecycle_snapshot.frame,
        document: artifacts.active_document,
        epoch: artifacts.active_epoch,
    };
    let work = context.with_fixture(|connection| {
        let page_owner = connection
            .pending_target_page_residence_identity_for_owner(&owner)
            .expect("the initial build retains its exact Page reservation");
        assert!(!connection.target_page_output_residence_is_current(&page_owner));
        connection.publish_prepared_top_level_navigation_owner_action(
            &owner,
            page_owner,
            RendererDocumentSourcedTopLevelLocationNavigation::new(
                source_document,
                TARGET_URL.into(),
            ),
        );
        connection
            .take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.is_top_level_navigation_owner_action() =>
                {
                    Some(work)
                }
                _ => None,
            })
            .expect("the accepted location request reaches the owner scheduler")
    });
    let pending = context.with_fixture(|connection| {
        connection
            .try_start_owned_protocol_scheduler_work(work)
            .unwrap_or_else(|_| panic!("location navigation uses an owned workflow"))
            .expect("retain the accepted navigation while its initial Page is being installed")
    });
    InitialNavigationFixture {
        context,
        built,
        owner,
        pending,
    }
}

#[tokio::test]
async fn location_from_initial_page_waits_for_installation_and_navigates_once() {
    let InitialNavigationFixture {
        mut context,
        built,
        owner,
        pending,
    } = initial_navigation_fixture().await;
    let mut wait = Box::pin(pending.wait());
    assert!(futures_util::poll!(&mut wait).is_pending());
    context.with_fixture(|connection| {
        assert!(!connection.has_pending_document_navigation_for_owner(&owner));
    });
    let mut driver = ProtocolWorkflowDriver::new(context.handle());
    context
        .drive_owned_wait(built.install_using(&mut driver))
        .await
        .unwrap();
    let completed = context.drive_owned_wait(wait).await;
    context
        .state
        .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
        .expect("complete the accepted navigation");
    context.with_fixture(|connection| {
        assert_eq!(
            connection
                .loaded_page_mut_for_owner(&owner)
                .expect("the navigation installs its destination")
                .final_url()
                .as_str(),
            TARGET_URL
        );
    });
}

#[tokio::test]
async fn failed_initial_page_does_not_start_its_accepted_location_navigation() {
    let InitialNavigationFixture {
        mut context,
        built,
        owner,
        pending,
    } = initial_navigation_fixture().await;
    let mut wait = Box::pin(pending.wait());
    assert!(futures_util::poll!(&mut wait).is_pending());
    context.with_fixture(|connection| {
        connection
            .runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap()
            .fail_initial_document_page_build("initial Page failed".into());
    });
    let completed = context.drive_owned_wait(wait).await;
    context
        .state
        .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
        .unwrap();
    context.with_fixture(|connection| {
        assert!(!connection.has_pending_document_navigation_for_owner(&owner));
        assert!(connection.loaded_page_mut_for_owner(&owner).is_none());
    });
    if let CompletedInitialDocumentPageBuild::Built { built, .. } = built {
        built.page.close_async().await.unwrap();
    }
}

#[tokio::test]
async fn initial_location_navigation_cannot_rebind_after_its_page_is_replaced() {
    let InitialNavigationFixture {
        mut context,
        built,
        owner,
        pending,
    } = initial_navigation_fixture().await;
    let mut wait = Box::pin(pending.wait());
    assert!(futures_util::poll!(&mut wait).is_pending());
    let mut driver = ProtocolWorkflowDriver::new(context.handle());
    context
        .drive_owned_wait(built.install_using(&mut driver))
        .await
        .unwrap();
    context.with_fixture(|connection| {
        let slot = connection
            .runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap();
        slot.replace_page_attachment_id_for_test();
    });
    let completed = context.drive_owned_wait(wait).await;
    context
        .state
        .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
        .unwrap();
    context.with_fixture(|connection| {
        assert!(!connection.has_pending_document_navigation_for_owner(&owner));
        assert_eq!(
            connection
                .loaded_page_mut_for_owner(&owner)
                .unwrap()
                .final_url()
                .as_str(),
            "about:blank"
        );
    });
}
