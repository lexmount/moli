use super::*;

#[tokio::test]
async fn native_request_url_copy_preserves_credentials_and_upload_body() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        let attempt = 0;
        addEventListener('fetch', event => event.respondWith((async () => {
            const original = event.request;
            const url = original.url;
            const variant = attempt++;
            Object.defineProperty(original, 'url', {get() { throw new Error('public url getter'); }});
            Object.defineProperty(original, 'toString', {value() { throw new Error('public toString'); }});
            try {
                const copy = variant === 0 ? new Request(original) :
                    new Request(original, variant === 1 ? {} : {cache:'no-store'});
                const body = await copy.text();
                copy.headers.set('accept', 'changed');
                const rejected = input => {
                    try { new Request(input); return 'allowed'; } catch (e) { return e.name; }
                };
                return new Response(JSON.stringify({
                    url:copy.url, body, used:copy.bodyUsed,
                    mode:copy.mode, credentials:copy.credentials, cache:copy.cache,
                    referrer:copy.referrer, signal:copy.signal instanceof AbortSignal,
                    headers:[original.headers.get('accept'), copy.headers.get('accept')],
                    rejected:[rejected(url), rejected({url})]
                }));
            } catch (e) {
                return new Response(JSON.stringify({error:e.name, message:e.message}));
            }
        })()));
        "#,
    );
    let url = "https://alice:secret@example.test/app/data.txt";
    for (id, changed) in [(940, false), (941, false), (942, true)] {
        let mut request = service_worker_fetch_request_for_test();
        request.url = url::Url::parse(url).unwrap();
        request.method = "POST".into();
        request.body = Some(bytes::Bytes::from_static(b"native request body"));
        request.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
        request.metadata.referrer = "https://example.test/from".into();
        let completion =
            dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, id, request)
                .await;
        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected native Request copy observations");
        };
        let observed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            observed,
            serde_json::json!({
                "url":url, "body":"native request body", "used":true,
                "mode":"cors", "credentials":"include",
                "cache":if changed {"no-store"} else {"default"},
                "referrer":if changed {"about:client"} else {"https://example.test/from"},
                "signal":true, "headers":["text/plain","changed"],
                "rejected":["TypeError","TypeError"]
            })
        );
    }
    handle.terminate_and_join();
}
