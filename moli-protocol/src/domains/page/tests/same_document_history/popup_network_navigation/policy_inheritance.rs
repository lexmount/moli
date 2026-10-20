use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn popup_location_navigation_captures_delivered_meta_policies() {
    run_history_test(async {
        for late_policy in [false, true] {
            let mut page = SameDocumentPage::new().await;
            let result = page
                .evaluate(&format!(
                    r#"(async () => {{
                        const popup = open('history.html?initial-popup');
                        await new Promise(resolve => popup.addEventListener('load', resolve, {{once:true}}));
                        const oldDocument = popup.document;
                        const referrer = document.createElement('meta');
                        referrer.name = 'referrer';
                        referrer.content = 'no-referrer';
                        document.head.appendChild(referrer);
                        const csp = document.createElement('meta');
                        csp.httpEquiv = 'Content-Security-Policy';
                        csp.content = "require-trusted-types-for 'script'";
                        if (!{late_policy}) document.head.appendChild(csp);
                        popup.location.href = 'about:blank';
                        if ({late_policy}) document.head.appendChild(csp);
                        referrer.content = 'unsafe-url';
                        for (let i = 0; i < 500; ++i) {{
                            if (popup.document !== oldDocument &&
                                popup.document.URL === 'about:blank' &&
                                popup.document.readyState === 'complete') {{
                                let outcome = 'allowed';
                                try {{
                                    popup.document.createElement('div').innerHTML = '<p>untrusted</p>';
                                }} catch (error) {{
                                    outcome = error.name + ':' + (error instanceof popup.TypeError);
                                }}
                                const result = {{
                                    outcome,
                                    referrer: popup.document.referrer,
                                    baseURI: popup.document.baseURI,
                                    originInherited: popup.origin === location.origin,
                                }};
                                popup.close();
                                return result;
                            }}
                            await new Promise(resolve => setTimeout(resolve, 10));
                        }}
                        throw new Error('about:blank popup navigation did not commit');
                    }})()"#,
                ))
                .await;
            assert_eq!(
                result,
                json!({
                    "outcome": if late_policy { "allowed" } else { "TypeError:true" },
                    "referrer": "",
                    "baseURI": page.base_url,
                    "originInherited": true,
                }),
                "late_policy={late_policy}"
            );
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn local_navigation_inherits_the_initiator_policy_without_mutating_network_responses() {
    const TARGET: &str = r#"<!doctype html><body><script>
        addEventListener('load', () => {
            let outcome = 'allowed';
            try { document.createElement('div').innerHTML = '<p>untrusted</p>'; }
            catch (error) { outcome = error.name + ':' + (error instanceof TypeError); }
            (opener || parent).postMessage({policyOutcome: outcome}, '*');
        });
    </script>"#;
    const INITIATOR: &str = r#"<!doctype html><body><script>
        addEventListener('message', event => {
            const policy = document.createElement('meta');
            policy.httpEquiv = 'Content-Security-Policy';
            policy.content = "require-trusted-types-for 'script'";
            if (!event.data.late) document.head.appendChild(policy);
            parent.__policyTarget.location.href = event.data.url;
            if (event.data.late) document.head.appendChild(policy);
        });
    </script>"#;
    run_history_test(async {
        for mode in ["popup-blob", "child-blob", "child-data", "popup-network"] {
            for late in [false, true] {
                let routes = axum::Router::new()
                    .route(
                        "/policy-initiator.html",
                        axum::routing::get(|| async { axum::response::Html(INITIATOR) }),
                    )
                    .route(
                        "/policy-target.html",
                        axum::routing::get(|| async { axum::response::Html(TARGET) }),
                    );
                let mut page = SameDocumentPage::with_routes(routes).await;
                let script = r#"(async () => {
                    const mode = MODE, late = LATE, markup = TARGET_DOCUMENT;
                    const child = mode.startsWith('child-');
                    const createFrame = async path => {
                        const frame = document.createElement('iframe');
                        frame.src = path;
                        const loaded = new Promise(resolve => frame.onload = resolve);
                        document.body.appendChild(frame);
                        await loaded;
                        return frame;
                    };
                    let targetFrame, initiatorFrame, target;
                    if (child) {
                        targetFrame = await createFrame('history.html?target');
                        target = globalThis.__policyTarget = targetFrame.contentWindow;
                        initiatorFrame = await createFrame('policy-initiator.html');
                    } else {
                        target = open('history.html?initial-popup');
                        await new Promise(resolve => target.addEventListener('load', resolve, {once:true}));
                    }
                    const completed = new Promise(resolve => {
                        addEventListener('message', function listener(event) {
                            if (event.source === target && event.data?.policyOutcome) {
                                removeEventListener('message', listener);
                                resolve(event.data.policyOutcome);
                            }
                        });
                    });
                    const url = mode === 'child-data'
                        ? 'data:text/html;charset=utf-8,' + encodeURIComponent(markup)
                        : mode === 'popup-network' ? new URL('policy-target.html', location.href).href
                        : URL.createObjectURL(new Blob([markup], {type:'text/html'}));
                    if (child) {
                        initiatorFrame.contentWindow.postMessage({url, late}, '*');
                    } else {
                        const policy = document.createElement('meta');
                        policy.httpEquiv = 'Content-Security-Policy';
                        policy.content = "require-trusted-types-for 'script'";
                        if (!late) document.head.appendChild(policy);
                        target.location.href = url;
                        if (late) document.head.appendChild(policy);
                    }
                    const result = await completed;
                    if (child) { targetFrame.remove(); initiatorFrame.remove(); }
                    else target.close();
                    if (url.startsWith('blob:')) URL.revokeObjectURL(url);
                    return result;
                })()"#
                    .replace("MODE", &serde_json::to_string(mode).unwrap())
                    .replace("LATE", if late { "true" } else { "false" })
                    .replace("TARGET_DOCUMENT", &serde_json::to_string(TARGET).unwrap());
                let result = page.evaluate(&script).await;
                assert_eq!(
                    result,
                    if late || mode == "popup-network" { "allowed" } else { "TypeError:true" },
                    "mode={mode}, late={late}"
                );
            }
        }
    })
    .await;
}
