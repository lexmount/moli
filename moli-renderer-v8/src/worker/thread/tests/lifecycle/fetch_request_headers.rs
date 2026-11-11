use super::*;

#[tokio::test]
async fn service_worker_fetch_request_headers_preserve_native_list_and_guard() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener('fetch', event => {
            const original = event.request;
            const clone = original.clone();
            const copies = [original, clone, clone.clone()];
            const mutations = request => [
                () => request.headers.append('x-probe', 'added'),
                () => request.headers.set('x-native', 'changed'),
                () => request.headers.delete('x-native')
            ].map(run => { try { run(); return 'allowed'; } catch (e) { return e.name; } });
            const errors = copies.map(mutations);
            const copy = new Request(original);
            const preserved = [copy, new Request(original, {}),
                new Request(original, {unknown:true}), new Request(original, {headers:undefined})
            ].map(request => [...request.headers]);
            const filtered = [new Request(original, {cache:'reload'}),
                new Request(original, {body:null})].map(request => [...request.headers]);
            const copyWithInit = new Request(original, {headers: {accept: 'application/json'}});
            const headers = new Headers(original.headers);
            const supplied = new Request(original.url);
            for (const request of [copy, copyWithInit, supplied])
                request.headers.set('accept', 'text/html');
            headers.set('accept', 'text/css');
            event.respondWith(new Response(JSON.stringify({
                errors, preserved, filtered,
                native: copies.map(request => [...request.headers]),
                distinctHeaders: original.headers !== clone.headers && clone.headers !== copies[2].headers,
                sameHeaders: original.headers === original.headers,
                copies: [copy, copyWithInit, supplied].map(request => request.headers.get('accept')),
                headerCopy: headers.get('accept')
            })));
        });
        "#,
    );
    for (id, mode, destination) in [
        (
            930,
            moli_fetch::RequestMode::Cors,
            ServiceWorkerRequestDestination::Empty,
        ),
        (
            931,
            moli_fetch::RequestMode::NoCors,
            ServiceWorkerRequestDestination::Script,
        ),
        (
            932,
            moli_fetch::RequestMode::Navigate,
            ServiceWorkerRequestDestination::Iframe,
        ),
    ] {
        let mut request = service_worker_fetch_request_for_test();
        request.request_mode = mode;
        request.destination = destination;
        request.headers = vec![
            ("accept".into(), "text/plain".into()),
            ("x-native".into(), "first".into()),
            ("x-native".into(), "second".into()),
            ("cookie".into(), "native-cookie".into()),
            ("origin".into(), "https://sender.test".into()),
        ];
        let completion =
            dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, id, request)
                .await;
        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected header observations for {mode:?}");
        };
        let observed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            observed["errors"],
            serde_json::json!([
                ["TypeError", "TypeError", "TypeError"],
                ["TypeError", "TypeError", "TypeError"],
                ["TypeError", "TypeError", "TypeError"]
            ]),
            "{mode:?}: {observed}"
        );
        let expected = serde_json::json!([
            ["accept", "text/plain"],
            ["cookie", "native-cookie"],
            ["origin", "https://sender.test"],
            ["x-native", "first, second"]
        ]);
        assert_eq!(
            observed["native"],
            serde_json::json!([expected, expected, expected]),
            "{mode:?}: {observed}"
        );
        assert_eq!(
            observed["preserved"],
            serde_json::json!([expected, expected, expected, expected]),
            "{mode:?}: {observed}"
        );
        let sanitized = if mode == moli_fetch::RequestMode::NoCors {
            serde_json::json!([["accept", "text/plain"]])
        } else {
            serde_json::json!([["accept", "text/plain"], ["x-native", "first, second"]])
        };
        assert_eq!(
            observed["filtered"],
            serde_json::json!([sanitized, sanitized]),
            "{mode:?}: {observed}"
        );
        assert_eq!(observed["distinctHeaders"], true);
        assert_eq!(observed["sameHeaders"], true);
        assert_eq!(
            observed["copies"],
            serde_json::json!(["text/html", "text/html", "text/html"])
        );
        assert_eq!(observed["headerCopy"], "text/css");
    }
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_request_creation_avoids_author_constructors_and_getters() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        const RequestPrototype = Request.prototype;
        const signalGetter = Object.getOwnPropertyDescriptor(RequestPrototype, 'signal').get;
        const calls = [];
        for (const name of ['Request', 'Headers']) {
            Object.defineProperty(globalThis, name, {get() {
                calls.push(name); throw new Error('author constructor getter: ' + name);
            }});
        }
        Object.defineProperty(RequestPrototype, 'signal', {get() {
            calls.push('signal'); throw new Error('author signal getter');
        }});
        Object.defineProperty(Object.prototype, 'method', {set() {
            calls.push('method'); throw new Error('author RequestInit setter');
        }, configurable:true});
        self.addEventListener('fetch', event => {
            const request = event.request;
            event.respondWith(new Response(JSON.stringify({
                calls, nativePrototype:Object.getPrototypeOf(request) === RequestPrototype,
                method:request.method, url:request.url, accept:request.headers.get('accept'),
                signal:signalGetter.call(request) instanceof AbortSignal
            })));
        });
        "#,
    );
    let completion = dispatch_service_worker_fetch_event_for_test(&mut handle, 933).await;
    let ServiceWorkerFetchResult::Response(response) = completion.result else {
        panic!("native request construction must not invoke author constructors");
    };
    let observed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(
        observed,
        serde_json::json!({
            "calls": [], "nativePrototype": true, "method": "GET",
            "url": "https://example.test/app/data.txt", "accept": "text/plain", "signal": true
        })
    );
    handle.terminate_and_join();
}

#[tokio::test]
async fn service_worker_fetch_request_native_byte_bodies_and_header_clones_are_independent() {
    ensure_v8();
    let mut handle = spawn_service_worker_for_test(
        r#"
        self.addEventListener('fetch', event => event.respondWith((async () => {
            const clone = event.request.clone();
            const [originalBuffer, cloneBuffer] = await Promise.all([
                event.request.arrayBuffer(), clone.arrayBuffer()
            ]);
            const original = new Uint8Array(originalBuffer), copy = new Uint8Array(cloneBuffer);
            copy[0] = 99;
            let error = '';
            try { clone.headers.set('accept', 'changed'); } catch (e) { error = e.name; }
            return new Response(JSON.stringify({
                original:[...original], copy:[...copy],
                used:[event.request.bodyUsed, clone.bodyUsed], error,
                headers:[event.request.headers.get('accept'), clone.headers.get('accept')]
            }));
        })()));
        "#,
    );
    let bytes = bytes::Bytes::from_static(&[0, 255, 13, 37]);
    for id in [934, 935] {
        let mut request = service_worker_fetch_request_for_test();
        request.method = "POST".into();
        request.body = Some(bytes.clone());
        let completion =
            dispatch_service_worker_fetch_event_with_request_for_test(&mut handle, id, request)
                .await;
        let ServiceWorkerFetchResult::Response(response) = completion.result else {
            panic!("expected independently readable native request bodies");
        };
        let observed: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            observed,
            serde_json::json!({
                "original":[0,255,13,37], "copy":[99,255,13,37], "used":[true,true],
                "error":"TypeError", "headers":["text/plain","text/plain"]
            })
        );
        assert_eq!(bytes.as_ref(), &[0, 255, 13, 37]);
    }
    handle.terminate_and_join();
}
