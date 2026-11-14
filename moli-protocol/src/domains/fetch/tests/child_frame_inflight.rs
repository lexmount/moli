use super::*;
use axum::{Router, http::header::CONTENT_TYPE, routing::get};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_child_navigation_stays_inflight_until_response_or_detach() {
    for (detach, status, attachment) in [
        (false, 200, false),
        (true, 200, false),
        (false, 204, false),
        (false, 205, false),
        (false, 200, true),
    ] {
        let release = Arc::new(tokio::sync::Notify::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let response_release = release.clone();
        let server = tokio::spawn(async move {
            let app = Router::new()
                .route(
                    "/page",
                    get(|| async {
                        (
                            [(CONTENT_TYPE.as_str(), "text/html")],
                            "<!doctype html><body>parent",
                        )
                    }),
                )
                .route(
                    "/never",
                    get(move || {
                        let release = response_release.clone();
                        async move {
                            release.notified().await;
                            let mut headers = axum::http::HeaderMap::new();
                            headers.insert(CONTENT_TYPE, "text/html".parse().unwrap());
                            if attachment {
                                headers.insert(
                                    "content-disposition",
                                    "attachment; filename=child.html".parse().unwrap(),
                                );
                            }
                            (
                                axum::http::StatusCode::from_u16(status).unwrap(),
                                headers,
                                if matches!(status, 204 | 205) {
                                    ""
                                } else {
                                    "<!doctype html><p>child"
                                },
                            )
                        }
                    }),
                );
            axum::serve(listener, app).await.unwrap();
        });
        let mut ctx = TestContext::new();
        with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
        enable_runtime_async(&mut ctx, "SID-1", 70800).await;
        ctx.process_async(json!({"id":70801,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"globalThis.pending=document.createElement('iframe');pending.src='/never';document.body.appendChild(pending)"}})).await;
        let _ = take_response_by_id(&mut ctx, 70801);
        wait_until_message(
            &mut ctx,
            "SID-1",
            "ordinary child request before headers",
            |m| {
                m["method"] == "Network.requestWillBeSent"
                    && m["params"]["request"]["url"] == format!("http://{addr}/never")
            },
        )
        .await;
        let announced = ctx
            .sent
            .iter()
            .find(|m| {
                m["method"] == "Network.requestWillBeSent"
                    && m["params"]["request"]["url"] == format!("http://{addr}/never")
            })
            .unwrap()
            .clone();
        let id = announced["params"]["requestId"].clone();
        let frame = announced["params"]["frameId"].clone();
        assert_eq!(
            id, announced["params"]["loaderId"],
            "Document loader identity must exist before commit"
        );
        assert!(!ctx.sent.iter().any(|m| matches!(
            m["method"].as_str(),
            Some("Network.loadingFinished" | "Network.loadingFailed")
        ) && m["params"]["requestId"] == id));
        ctx.with_fixture(|conn| {
            assert!(
                !conn
                    .runtime_session_owner_slot_mut(Some("SID-1"))
                    .unwrap()
                    .renderer_subresources_are_idle(),
                "an outstanding child Document is not idle"
            )
        });
        let start = ctx
            .sent
            .iter()
            .position(|m| {
                m["method"] == "Page.frameRequestedNavigation" && m["params"]["frameId"] == frame
            })
            .expect("navigation start");
        let request = ctx
            .sent
            .iter()
            .position(|m| {
                m["method"] == "Network.requestWillBeSent" && m["params"]["requestId"] == id
            })
            .unwrap();
        assert!(start < request);
        if detach {
            ctx.process_async(json!({"id":70802,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"pending.remove()"}})).await;
            let _ = take_response_by_id(&mut ctx, 70802);
        } else {
            release.notify_one();
        }
        let terminal = if detach || attachment {
            "Network.loadingFailed"
        } else {
            "Network.loadingFinished"
        };
        wait_until_message(&mut ctx, "SID-1", "child request terminal", |m| {
            m["method"] == terminal && m["params"]["requestId"] == id
        })
        .await;
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|m| m["method"] == "Network.requestWillBeSent"
                    && m["params"]["requestId"] == id)
                .count(),
            1,
            "each child Document request must be announced once"
        );
        if !detach {
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|m| m["method"] == "Network.responseReceived"
                        && m["params"]["requestId"] == id
                        && m["params"]["response"]["status"] == status)
                    .count(),
                1,
                "response headers must survive no-content and download handoff"
            );
        }
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|m| matches!(
                    m["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ) && m["params"]["requestId"] == id)
                .count(),
            1
        );
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|m| m["method"] == "Page.frameRequestedNavigation"
                    && m["params"]["frameId"] == frame)
                .count(),
            1
        );
        ctx.with_fixture(|conn| {
            assert!(
                conn.runtime_session_owner_slot_mut(Some("SID-1"))
                    .unwrap()
                    .renderer_subresources_are_idle()
            )
        });
        release.notify_one();
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_classic_script_is_inflight_and_interceptable_until_release_or_detach() {
    for intercepted in [false, true] {
        for detach in [false, true] {
            let release = Arc::new(tokio::sync::Notify::new());
            let requested = Arc::new(tokio::sync::Notify::new());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let response_release = release.clone();
            let observed = requested.clone();
            let server = tokio::spawn(async move {
                let app = Router::new()
                    .route(
                        "/page",
                        get(|| async {
                            (
                                [(CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><body>parent",
                            )
                        }),
                    )
                    .route(
                        "/child",
                        get(|| async {
                            (
                                [(CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><script src='/script.js'></script><p>child",
                            )
                        }),
                    )
                    .route(
                        "/script.js",
                        get(move || {
                            let release = response_release.clone();
                            let observed = observed.clone();
                            async move {
                                observed.notify_one();
                                release.notified().await;
                                (
                                    [(CONTENT_TYPE.as_str(), "application/javascript")],
                                    "globalThis.fixtureLoaded=1",
                                )
                            }
                        }),
                    );
                axum::serve(listener, app).await.unwrap();
            });
            let mut ctx = TestContext::new();
            with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1")
                .await;
            enable_runtime_async(&mut ctx, "SID-1", 70900).await;
            if intercepted {
                ctx.process_async(json!({"id":70901,"sessionId":"SID-1","method":"Fetch.enable","params":{"patterns":[{"resourceType":"Script","requestStage":"Request"}]}})).await;
                ctx.expect_result(70901, json!({}), Some("SID-1"));
            }
            ctx.process_async(json!({"id":70902,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"globalThis.pending=document.createElement('iframe');pending.src='/child';document.body.appendChild(pending)"}})).await;
            let _ = take_response_by_id(&mut ctx, 70902);
            wait_until_message(
                &mut ctx,
                "SID-1",
                "child script start before headers",
                |m| {
                    m["method"] == "Network.requestWillBeSent"
                        && m["params"]["request"]["url"] == format!("http://{addr}/script.js")
                },
            )
            .await;
            let started = ctx
                .sent
                .iter()
                .find(|m| {
                    m["method"] == "Network.requestWillBeSent"
                        && m["params"]["request"]["url"] == format!("http://{addr}/script.js")
                })
                .unwrap()
                .clone();
            let id = started["params"]["requestId"].clone();
            if intercepted {
                wait_until_message(&mut ctx, "SID-1", "child script route", |m| {
                    m["method"] == "Fetch.requestPaused"
                        && m["params"]["request"]["url"] == format!("http://{addr}/script.js")
                })
                .await;
                let paused = ctx.take_first_matching("child script pause", |m| {
                    m["method"] == "Fetch.requestPaused"
                        && m["params"]["request"]["url"] == format!("http://{addr}/script.js")
                });
                assert_eq!(paused["params"]["networkId"], id);
                ctx.process_async(json!({"id":70903,"sessionId":"SID-1","method":"Fetch.continueRequest","params":{"requestId":paused["params"]["requestId"],"url":format!("http://{addr}/script.js?routed")}})).await;
                ctx.expect_result(70903, json!({}), Some("SID-1"));
            }
            tokio::time::timeout(Duration::from_secs(5), requested.notified())
                .await
                .expect("script reaches gated HTTP server");
            ctx.with_fixture(|conn| {
                assert!(
                    !conn
                        .runtime_session_owner_slot_mut(Some("SID-1"))
                        .unwrap()
                        .renderer_subresources_are_idle()
                )
            });
            assert!(!ctx.sent.iter().any(|m| matches!(
                m["method"].as_str(),
                Some("Network.loadingFinished" | "Network.loadingFailed")
            ) && m["params"]["requestId"] == id));
            if detach {
                ctx.process_async(json!({"id":70904,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"pending.remove()"}})).await;
                let _ = take_response_by_id(&mut ctx, 70904);
            } else {
                release.notify_one();
            }
            let terminal = if detach {
                "Network.loadingFailed"
            } else {
                "Network.loadingFinished"
            };
            wait_until_message(&mut ctx, "SID-1", "child script terminal", |m| {
                m["method"] == terminal && m["params"]["requestId"] == id
            })
            .await;
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|m| matches!(
                        m["method"].as_str(),
                        Some("Network.loadingFinished" | "Network.loadingFailed")
                    ) && m["params"]["requestId"] == id)
                    .count(),
                1
            );
            ctx.with_fixture(|conn| {
                assert!(
                    conn.runtime_session_owner_slot_mut(Some("SID-1"))
                        .unwrap()
                        .renderer_subresources_are_idle()
                )
            });
            release.notify_one();
            server.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_page_navigate_returns_the_committed_loader_identity() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = Router::new().fallback(get(|| async {
            (
                [(CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><p>fixture",
            )
        }));
        axum::serve(listener, app).await.unwrap();
    });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 71000).await;
    ctx.process_async(json!({"id":71001,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":"globalThis.pending=document.createElement('iframe');pending.src='/initial';document.body.appendChild(pending)"}})).await;
    let _ = take_response_by_id(&mut ctx, 71001);
    wait_until_message(&mut ctx, "SID-1", "initial child commit", |m| {
        m["method"] == "Page.frameNavigated"
            && m["params"]["frame"]["url"] == format!("http://{addr}/initial")
    })
    .await;
    let initial = ctx
        .sent
        .iter()
        .find(|m| {
            m["method"] == "Page.frameNavigated"
                && m["params"]["frame"]["url"] == format!("http://{addr}/initial")
        })
        .unwrap()
        .clone();
    let frame = initial["params"]["frame"]["id"].clone();
    let initial_loader = initial["params"]["frame"]["loaderId"].clone();
    let url = format!("http://{addr}/next");
    ctx.process_async(json!({"id":71002,"sessionId":"SID-1","method":"Page.navigate","params":{"frameId":frame,"url":url}})).await;
    let reply = take_response_by_id(&mut ctx, 71002);
    let loader = reply["result"]["loaderId"].clone();
    assert!(
        loader.is_string(),
        "cross-document Page.navigate must return a loader: {reply}"
    );
    assert_ne!(loader, initial_loader);
    assert!(ctx.sent.iter().any(|m| {
        m["method"] == "Network.requestWillBeSent"
            && m["params"]["frameId"] == frame
            && m["params"]["request"]["url"] == url
            && m["params"]["loaderId"] == loader
    }));
    assert!(ctx.sent.iter().any(|m| {
        m["method"] == "Page.frameNavigated"
            && m["params"]["frame"]["id"] == frame
            && m["params"]["frame"]["url"] == url
            && m["params"]["frame"]["loaderId"] == loader
    }));
    server.abort();
}
