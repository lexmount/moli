use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn javascript_form_navigation_runs_before_unexpired_idle_callbacks() {
    let runtime = JsRuntime::initialize();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    for dispatch in [
        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    ] {
        let (base_url, completed, release_response, server) =
            spawn_owner_wake_gated_server_with_content_type(
                "/navigation-idle-completed",
                "ok",
                "text/plain",
            )
            .await;
        let mut page = create_test_html_page_with_navigation_dispatch(
            &runtime,
            &loader,
            url::Url::parse(&format!("{base_url}/page")).expect("page URL"),
            "<!doctype html><meta http-equiv=Content-Security-Policy \
             content=\"require-trusted-types-for 'script'\"><body><form id=probe></form>",
            dispatch,
        )
        .await;
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
                globalThis.__navigationIdleProbe = {order: [], didTimeout: null};
                trustedTypes.createPolicy('default', {createScript: source => {
                    __navigationIdleProbe.order.push('policy');
                    return source;
                }});
                setTimeout(() => {
                    const form = document.getElementById('probe');
                    form.action = "javascript:__navigationIdleProbe.order.push('javascript'); void 0";
                    form.submit();
                    requestIdleCallback(deadline => {
                        __navigationIdleProbe.didTimeout = deadline.didTimeout;
                        __navigationIdleProbe.order.push('idle');
                        fetch('/navigation-idle-completed');
                    });
                    // Both the form task and idle transport must be ready at
                    // the next owner boundary, independent of machine speed.
                    const until = performance.now() + 5;
                    while (performance.now() < until) {}
                }, 50);
                'scheduled'
            "#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("form and idle probe should schedule");
        tokio::time::timeout(Duration::from_secs(3), completed)
            .await
            .expect("owner loop should complete without an observation command")
            .expect("completion witness should remain open");
        release_response.send(()).expect("release witness response");
        server.await.expect("completion witness should finish");
        let (reply, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: "JSON.stringify(__navigationIdleProbe)".to_owned(),
                await_promise: false,
            })
            .await
            .expect("completed probe should evaluate");
        let value = renderer_json_value(reply).expect("probe result");
        let row: serde_json::Value = serde_json::from_str(value.as_str().expect("probe JSON"))
            .expect("probe observations should parse");
        page.close_async().await.expect("probe page should close");
        assert_eq!(row["didTimeout"], false, "{dispatch:?}: {row}");
        assert_eq!(
            row["order"],
            serde_json::json!(["policy", "javascript", "idle"]),
            "{dispatch:?}: {row}"
        );
    }
}
