use super::*;

async fn wait_for_policy_popup_document(page: &mut TestCdpSocket, expression: &str) {
    timeout(Duration::from_secs(10), async {
        loop {
            if evaluate_window_name_probe(page, 4, expression).await == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("popup policy navigation should commit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn popup_location_navigation_captures_delivered_meta_policies() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html(
                "<!doctype html><html><head></head><body>policy owner</body></html>",
            )
        })),
        "popup-location-meta-policy",
    );
    let parent_url = format!("http://{fixture_addr}/parent");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    for (index, late_policy) in [false, true].into_iter().enumerate() {
        let opener_id = create_dynamic_target(&mut browser, index as u64 + 1).await;
        let mut opener = connect_dynamic_page(addr, &opener_id).await;
        send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
        navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &parent_url).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                3,
                "globalThis.policyPopup = open('history.html?initial-popup'); true",
            )
            .await,
            true
        );
        wait_for_policy_popup_document(
            &mut opener,
            "policyPopup.document.URL === new URL('history.html?initial-popup', location.href).href && policyPopup.document.readyState === 'complete'",
        )
        .await;
        let source = format!(
            r#"
            globalThis.policyOldDocument = policyPopup.document;
            const referrer = document.createElement('meta');
            referrer.name = 'referrer'; referrer.content = 'no-referrer';
            document.head.appendChild(referrer);
            const csp = document.createElement('meta');
            csp.httpEquiv = 'Content-Security-Policy';
            csp.content = "require-trusted-types-for 'script'";
            if (!{late_policy}) document.head.appendChild(csp);
            policyPopup.location.href = 'about:blank';
            if ({late_policy}) document.head.appendChild(csp);
            referrer.content = 'unsafe-url';
            true
        "#
        );
        assert_eq!(
            evaluate_window_name_probe(&mut opener, 5, &source).await,
            true
        );
        wait_for_policy_popup_document(
            &mut opener,
            "policyPopup.document !== policyOldDocument && policyPopup.document.URL === 'about:blank' && policyPopup.document.readyState === 'complete'",
        )
        .await;
        let result = evaluate_window_name_probe(
            &mut opener,
            6,
            r#"(() => {
                let outcome = 'allowed';
                try { policyPopup.document.createElement('div').innerHTML = '<p>untrusted</p>'; }
                catch (error) { outcome = error.name + ':' + (error instanceof policyPopup.TypeError); }
                return {outcome, referrer: policyPopup.document.referrer,
                    baseURI: policyPopup.document.baseURI,
                    originInherited: policyPopup.origin === location.origin};
            })()"#,
        )
        .await;
        assert_eq!(
            result,
            json!({
                "outcome": if late_policy { "allowed" } else { "TypeError:true" },
                "referrer": "", "baseURI": parent_url, "originInherited": true,
            }),
            "late_policy={late_policy}"
        );
        evaluate_window_name_probe(&mut opener, 7, "policyPopup.close(); true").await;
    }
    abort_test_cdp_server(server).await;
}
