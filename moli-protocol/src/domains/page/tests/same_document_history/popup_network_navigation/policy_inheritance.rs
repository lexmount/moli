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
