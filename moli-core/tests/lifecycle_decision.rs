use moli_test_support as support;

use anyhow::{Context, Result, anyhow, bail, ensure};
use moli_core::{
    page::Page,
    runtime::{
        Browser, BrowserConfig as AppConfig, FetchedDocument, PageVmInitStage,
        RenderedDomWaitUntil, RendererLifecycleDecision,
    },
    testing::JsValueSnapshot,
};
use moli_fetch::Request;
use parking_lot::Mutex;
use std::sync::Arc;
use support::FixtureServer;
use tokio::{sync::oneshot, time::Duration};

fn executable_page(document: FetchedDocument) -> Result<Page> {
    match document {
        FetchedDocument::Page(page) => Ok(page),
        FetchedDocument::Raw(document) => Err(anyhow!(
            "expected an executable Page, got raw document status {}",
            document.status()
        )),
    }
}

fn diagnostic_global<'a>(page: &'a Page, name: &str) -> Option<&'a JsValueSnapshot> {
    page.script_execution().global(name)
}

async fn follow_http_error_navigation(
    browser: &Browser,
    url: &str,
    wait_until: RenderedDomWaitUntil,
    navigation_grace_ms: u64,
    timeout: Duration,
) -> Result<Page> {
    executable_page(
        browser
            .fetch_document_with_lifecycle_decider(
                Request::get(url)?,
                wait_until,
                timeout,
                move |target| {
                    ensure!(
                        (400..=599).contains(&target.status),
                        "expected an HTTP error lifecycle target, got status {}",
                        target.status
                    );
                    Ok(RendererLifecycleDecision::FollowNextDocument {
                        navigation_grace_ms,
                    })
                },
            )
            .await?,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_decider_finishes_without_extra_owner_command() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-domcontentloaded-runtime-script-slow");

    let observed_targets = Arc::new(Mutex::new(Vec::new()));
    let observed_targets_for_decider = observed_targets.clone();
    let mut page = executable_page(
        browser
            .fetch_document_with_lifecycle_decider(
                Request::get(&url)?,
                RenderedDomWaitUntil::DomContentLoaded,
                Duration::from_secs(5),
                move |target| {
                    observed_targets_for_decider.lock().push(target);
                    Ok(RendererLifecycleDecision::Finish)
                },
            )
            .await?,
    )?;
    assert_eq!(page.status(), 200);
    {
        let observed_targets = observed_targets.lock();
        assert_eq!(observed_targets.len(), 1);
        assert_eq!(observed_targets[0].stage, PageVmInitStage::DomContentLoaded);
        assert_eq!(observed_targets[0].status, 200);
        assert_eq!(observed_targets[0].final_url.as_str(), url);
    }
    assert!(
        !page
            .serialize_html_async()
            .await?
            .contains("id=\"late-dcl-script-slow\"")
    );

    browser
        .wait_for_page_delay(&mut page, Duration::from_millis(500))
        .await?;
    assert!(
        page.serialize_html_async()
            .await?
            .contains("id=\"late-dcl-script-slow\"")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_decider_supports_static_about_blank() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;
    let observed_target = Arc::new(Mutex::new(None));
    let observed_target_for_decider = observed_target.clone();
    let page = executable_page(
        browser
            .fetch_document_with_lifecycle_decider(
                Request::get("about:blank")?,
                RenderedDomWaitUntil::Done,
                Duration::from_secs(1),
                move |target| {
                    *observed_target_for_decider.lock() = Some(target);
                    Ok(RendererLifecycleDecision::Finish)
                },
            )
            .await?,
    )?;

    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().as_str(), "about:blank");
    let observed_target = observed_target.lock();
    let observed_target = observed_target.as_ref().unwrap();
    assert_eq!(observed_target.stage, PageVmInitStage::Load);
    assert_eq!(observed_target.status, 200);
    assert_eq!(observed_target.final_url.as_str(), "about:blank");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_decider_does_not_extend_initial_stage_timeout() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let decider_was_called = Arc::new(Mutex::new(false));
    let decider_was_called_in_hook = decider_was_called.clone();

    let error = browser
        .fetch_document_with_lifecycle_decider(
            Request::get(&server.url("/wait-until-domcontentloaded-runtime-script-very-slow"))?,
            RenderedDomWaitUntil::Load,
            Duration::from_millis(100),
            move |_| {
                *decider_was_called_in_hook.lock() = true;
                Ok(RendererLifecycleDecision::Finish)
            },
        )
        .await
        .expect_err("a lifecycle decider must not relax the initial Load deadline");

    assert!(
        format!("{error:#}").contains("timed out after 100 ms"),
        "error={error:#}"
    );
    assert!(!*decider_was_called.lock());
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn follow_navigation_grace_cannot_extend_fetch_timeout() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        browser.fetch_document_with_lifecycle_decider(
            Request::get("about:blank")?,
            RenderedDomWaitUntil::Done,
            Duration::from_millis(100),
            |_| {
                Ok(RendererLifecycleDecision::FollowNextDocument {
                    navigation_grace_ms: 10_000,
                })
            },
        ),
    )
    .await
    .context("successor grace escaped the fetch deadline")?;
    let error = result.expect_err("the original fetch timeout must interrupt successor grace");

    assert!(
        format!("{error:#}").contains("timed out after 100 ms"),
        "error={error:#}"
    );

    let page = tokio::time::timeout(
        Duration::from_secs(1),
        browser.fetch_request_document_allow_http_error(Request::get("about:blank")?),
    )
    .await
    .context("timed-out lifecycle follow left the renderer owner blocked")??;
    assert_eq!(executable_page(page)?.status(), 200);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_decider_error_and_panic_retire_only_pending_page() -> Result<()> {
    let browser = Browser::new(AppConfig::default())?;

    let error = browser
        .fetch_document_with_lifecycle_decider(
            Request::get("about:blank")?,
            RenderedDomWaitUntil::Done,
            Duration::from_secs(1),
            |_| Err(anyhow!("policy rejected target")),
        )
        .await
        .expect_err("a decision error must fail page creation");
    assert!(
        format!("{error:#}").contains("policy rejected target"),
        "error={error:#}"
    );

    let error = browser
        .fetch_document_with_lifecycle_decider(
            Request::get("about:blank")?,
            RenderedDomWaitUntil::Done,
            Duration::from_secs(1),
            |_| -> Result<RendererLifecycleDecision> { panic!("policy panic sentinel") },
        )
        .await
        .expect_err("a decision panic must fail page creation without unwinding the owner");
    assert!(
        format!("{error:#}").contains("lifecycle decider panicked: policy panic sentinel"),
        "error={error:#}"
    );

    // The panic is contained to the failed pending Page; the same renderer
    // owner must remain usable for the next creation.
    let page = executable_page(
        browser
            .fetch_request_document_allow_http_error(Request::get("about:blank")?)
            .await?,
    )?;
    assert_eq!(page.status(), 200);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_navigation_wait_follows_same_url_reload_to_domcontentloaded() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-http-error-navigation");

    let observed_target = Arc::new(Mutex::new(None));
    let observed_target_for_decider = observed_target.clone();
    let page = executable_page(
        browser
            .fetch_document_with_lifecycle_decider(
                Request::get(&url)?,
                RenderedDomWaitUntil::DomContentLoaded,
                Duration::from_secs(5),
                move |target| {
                    *observed_target_for_decider.lock() = Some(target);
                    Ok(RendererLifecycleDecision::FollowNextDocument {
                        navigation_grace_ms: 1_000,
                    })
                },
            )
            .await?,
    )?;
    {
        let observed_target = observed_target.lock();
        let observed_target = observed_target.as_ref().unwrap();
        assert_eq!(observed_target.stage, PageVmInitStage::DomContentLoaded);
        assert_eq!(observed_target.status, 403);
        assert_eq!(observed_target.final_url.as_str(), url);
    }
    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().as_str(), url);
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationLoad"),
        Some(&JsValueSnapshot::Bool(false)),
        "the successor snapshot must stop at DCL rather than drifting to Load"
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationSlowScript"),
        None,
        "the DCL-inserted slow script must not complete before the DCL reply"
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_navigation_wait_follows_five_same_url_reloads_to_domcontentloaded() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-http-error-five-navigations");

    let page = follow_http_error_navigation(
        &browser,
        &url,
        RenderedDomWaitUntil::DomContentLoaded,
        1_000,
        Duration::from_secs(10),
    )
    .await?;

    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().as_str(), url);
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationChainStep"),
        Some(&JsValueSnapshot::Number(5.0))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationLoad"),
        Some(&JsValueSnapshot::Bool(false))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationSlowScript"),
        None
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_navigation_wait_follows_five_same_url_reloads_to_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-http-error-five-navigations");

    let page = follow_http_error_navigation(
        &browser,
        &url,
        RenderedDomWaitUntil::Load,
        1_000,
        Duration::from_secs(10),
    )
    .await?;

    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().as_str(), url);
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationChainStep"),
        Some(&JsValueSnapshot::Number(5.0))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationLoad"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationSlowScript"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_navigation_wait_follows_same_url_reload_to_load() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-http-error-navigation");

    let observed_target = Arc::new(Mutex::new(None));
    let observed_target_for_decider = observed_target.clone();
    let page = executable_page(
        browser
            .fetch_document_with_lifecycle_decider(
                Request::get(&url)?,
                RenderedDomWaitUntil::Load,
                Duration::from_secs(5),
                move |target| {
                    *observed_target_for_decider.lock() = Some(target);
                    Ok(RendererLifecycleDecision::FollowNextDocument {
                        navigation_grace_ms: 1_000,
                    })
                },
            )
            .await?,
    )?;
    {
        let observed_target = observed_target.lock();
        let observed_target = observed_target.as_ref().unwrap();
        assert_eq!(observed_target.stage, PageVmInitStage::Load);
        assert_eq!(observed_target.status, 403);
        assert_eq!(observed_target.final_url.as_str(), url);
    }
    assert_eq!(page.status(), 200);
    assert_eq!(page.final_url().as_str(), url);
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationDcl"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationLoad"),
        Some(&JsValueSnapshot::Bool(true))
    );
    assert_eq!(
        diagnostic_global(&page, "httpErrorNavigationSlowScript"),
        Some(&JsValueSnapshot::Bool(true))
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_navigation_wait_reports_no_navigation_without_refetching() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/net/upstream/xhr/404-then-200");

    let error = browser
        .fetch_document_with_lifecycle_decider(
            Request::get(&url)?,
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
            |target| {
                assert_eq!(target.status, 404);
                Ok(RendererLifecycleDecision::FollowNextDocument {
                    navigation_grace_ms: 100,
                })
            },
        )
        .await
        .expect_err("a static 404 document must not be refetched or accepted");
    let error = format!("{error:#}");
    assert!(error.contains("404 Not Found"), "error={error}");
    assert!(error.contains("100 ms grace period"), "error={error}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn same_document_navigation_does_not_satisfy_http_error_replacement_wait() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-http-error-same-document-navigation");

    let error = browser
        .fetch_document_with_lifecycle_decider(
            Request::get(&url)?,
            RenderedDomWaitUntil::Load,
            Duration::from_secs(5),
            |target| {
                ensure!(target.status == 403, "expected 403, got {}", target.status);
                Ok(RendererLifecycleDecision::FollowNextDocument {
                    navigation_grace_ms: 150,
                })
            },
        )
        .await
        .expect_err("a fragment-only navigation must not replace the HTTP error Document");
    let error = format!("{error:#}");
    assert!(error.contains("403 Forbidden"), "error={error}");
    assert!(error.contains("150 ms grace period"), "error={error}");

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_http_error_navigation_wait_keeps_renderer_owner_usable() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/net/upstream/xhr/404");
    let (decision_tx, decision_rx) = oneshot::channel();

    let mut waiting_fetch = Box::pin(browser.fetch_document_with_lifecycle_decider(
        Request::get(&url)?,
        RenderedDomWaitUntil::Load,
        Duration::from_secs(11),
        move |target| {
            ensure!(target.status == 404, "expected 404, got {}", target.status);
            decision_tx
                .send(())
                .map_err(|_| anyhow!("decision observer was dropped"))?;
            Ok(RendererLifecycleDecision::FollowNextDocument {
                navigation_grace_ms: 10_000,
            })
        },
    ));
    tokio::select! {
        decision = decision_rx => decision.context("lifecycle decider did not signal")?,
        result = &mut waiting_fetch => {
            result?;
            bail!("HTTP error wait completed before it could be cancelled");
        }
    }
    drop(waiting_fetch);

    let page = tokio::time::timeout(
        Duration::from_secs(2),
        browser.fetch_request_document_allow_http_error(Request::get("about:blank")?),
    )
    .await
    .context("renderer owner stayed blocked after cancelling the lifecycle wait")??;
    assert_eq!(executable_page(page)?.status(), 200);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parked_http_error_navigation_wait_does_not_block_another_page() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/net/upstream/xhr/404");
    let (decision_tx, decision_rx) = oneshot::channel();

    let mut waiting_fetch = Box::pin(browser.fetch_document_with_lifecycle_decider(
        Request::get(&url)?,
        RenderedDomWaitUntil::Load,
        Duration::from_secs(11),
        move |target| {
            ensure!(target.status == 404, "expected 404, got {}", target.status);
            decision_tx
                .send(())
                .map_err(|_| anyhow!("decision observer was dropped"))?;
            Ok(RendererLifecycleDecision::FollowNextDocument {
                navigation_grace_ms: 10_000,
            })
        },
    ));
    tokio::select! {
        decision = decision_rx => decision.context("lifecycle decider did not signal")?,
        result = &mut waiting_fetch => {
            result?;
            bail!("HTTP error wait completed before the concurrency check");
        }
    }

    let quick_page = tokio::time::timeout(
        Duration::from_secs(2),
        browser.fetch_request_document_allow_http_error(Request::get("about:blank")?),
    )
    .await
    .context("a parked lifecycle wait blocked an unrelated Page creation")??;
    assert_eq!(executable_page(quick_page)?.status(), 200);
    drop(waiting_fetch);

    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_replacement_wait_keeps_chained_navigation_limit() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let url = server.url("/wait-until-http-error-navigation-loop");

    let error = browser
        .fetch_document_with_lifecycle_decider(
            Request::get(&url)?,
            RenderedDomWaitUntil::Load,
            Duration::from_secs(20),
            |target| {
                ensure!(target.status == 403, "expected 403, got {}", target.status);
                Ok(RendererLifecycleDecision::FollowNextDocument {
                    navigation_grace_ms: 1_000,
                })
            },
        )
        .await
        .expect_err("the replacement navigation loop must hit the owner chain limit");
    let error = format!("{error:#}");
    assert!(
        error.contains("too many chained location navigations"),
        "error={error}"
    );

    server.shutdown().await;
    Ok(())
}
