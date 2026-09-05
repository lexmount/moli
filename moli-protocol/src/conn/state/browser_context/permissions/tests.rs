use super::*;
use moli_core::runtime::{Browser, BrowserConfig};
use serde_json::json;

fn rule(setting: &str) -> PermissionOverrideRegistration {
    PermissionOverrideRegistration {
        permission: json!({"name": "geolocation"}),
        setting: setting.into(),
        origin: None,
        embedded_origin: None,
    }
}

#[tokio::test]
async fn permission_refresh_visits_all_physical_documents_after_projection_drop() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut defaults = PermissionDefaults::default();
    defaults.set(rule("denied"));
    let mut physical = {
        let mut projection = BrowserContext::new("BID-permission-owner".into());
        for target in ["first", "second"] {
            projection.set_active_target_id(target);
            projection.attach_active_session(format!("SID-{target}"));
            let page = browser
                .fetch(&format!("data:text/html,<title>{target}</title>"))
                .await
                .unwrap();
            assert!(projection.replace_loaded_page(Some(page)).is_none());
        }
        projection.set_permission_override(&mut defaults, rule("granted"));
        // All Target/session projections and their inspection bindings drop.
        // The Browser operation must still reach both owned Documents.
        projection.physical
    };
    assert_eq!(physical.permission_overrides.override_count(), 1);
    let ids = physical.web_contents.keys().copied().collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    for expected in ["granted", "denied"] {
        let pending = physical
            .start_permission_update(&defaults)
            .unwrap()
            .unwrap();
        assert_eq!(pending.pages.len(), 2);
        assert!(physical.select_web_contents(ids[0]));
        let completed = pending.wait().await;
        assert!(physical.select_web_contents(ids[1]));
        physical.finish_permission_update(completed).unwrap();
        for (contents, title) in physical.web_contents.values_mut().zip(["first", "second"]) {
            let page = &mut contents.main_frame.current_document.as_mut().unwrap().page;
            assert_eq!(page.document_title(), title);
            let result = page.evaluate_runtime_expression_with_await_async(
                "navigator.permissions.query({name:'geolocation'}).then(status => status.state)",
                true,
            ).await.unwrap();
            assert_eq!(result, json!({"type": "string", "value": expected}));
        }
        physical.permission_overrides.clear();
    }
    assert_eq!(defaults.snapshot(), vec![rule("denied")]);
}

#[tokio::test]
async fn permission_completion_cannot_retarget_a_reused_devtools_target_id() {
    let browser = Browser::new(BrowserConfig::default()).unwrap();
    let mut context = BrowserContext::new("BID-permission-close".into());
    context.set_active_target_id("same-target");
    context.replace_loaded_page(Some(
        browser
            .fetch("data:text/html,<title>first</title>")
            .await
            .unwrap(),
    ));
    let old_id = context.selected_web_contents_id().unwrap();
    let mut defaults = PermissionDefaults::default();
    context.set_permission_override(&mut defaults, rule("granted"));
    let completed = context
        .start_permission_update(&defaults)
        .unwrap()
        .unwrap()
        .wait()
        .await;

    let (projection, closing) = context.take_page_target_for_close("same-target").unwrap();
    drop(projection);
    closing.close_async().await;
    context.set_active_target_id("same-target");
    context.replace_loaded_page(Some(
        browser
            .fetch("data:text/html,<title>replacement</title>")
            .await
            .unwrap(),
    ));
    assert_ne!(context.selected_web_contents_id(), Some(old_id));
    assert_eq!(
        context.finish_permission_update(completed),
        Err("NoDocumentLoaded".into())
    );
    assert_eq!(
        context
            .loaded_page_for_target("same-target")
            .unwrap()
            .document_title(),
        "replacement"
    );
}
