use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const WORKER: &str = r#"
addEventListener('fetch', event => {
    if (new URL(event.request.url).pathname !== '/native-request/echo') return;
    event.respondWith((async () => {
        const original = event.request, url = original.url;
        Object.defineProperty(original, 'url', {get() { throw new Error('public url getter'); }});
        Object.defineProperty(original, 'toString', {value() { throw new Error('public toString'); }});
        const constructors = [];
        let copy;
        for (const init of [undefined, {}, {cache:'no-store'}]) {
            try {
                const value = new Request(original, init);
                constructors.push({url:value.url});
                copy ||= value;
            } catch (e) { constructors.push({error:e.name}); }
        }
        const forwarded = [];
        for (const input of [original, copy]) {
            if (!input) { forwarded.push({error:'no copy'}); continue; }
            try {
                const response = await fetch(input);
                forwarded.push({status:response.status, data:await response.json()});
            } catch (e) { forwarded.push({error:e.name}); }
        }
        const refused = [];
        for (const input of [url, {url}]) {
            try { await fetch(input); refused.push('allowed'); } catch (e) { refused.push(e.name); }
        }
        const cache = await caches.open('native-request-url');
        await cache.put(original, new Response('seed'));
        return new Response(JSON.stringify({url, constructors, forwarded, refused}));
    })());
});
"#;

#[tokio::test(flavor = "multi_thread")]
async fn native_request_urls_are_forwarded_by_window_and_worker_fetch() {
    run_history_test(async {
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let routes = axum::Router::new()
            .route("/request-worker.js", axum::routing::get(|| async {
                ([(axum::http::header::CONTENT_TYPE, "application/javascript")], WORKER)
            }))
            .route("/native-request/frame.html", axum::routing::get(|| async {
                ([(axum::http::header::CONTENT_TYPE, "text/html")], "<!doctype html><body>controlled</body>")
            }))
            .route("/native-request/echo", axum::routing::get(move || {
                let requests = observed.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    ([(axum::http::header::CACHE_CONTROL, "no-store")], axum::Json(json!({"network":true})))
                }
            }));
        let mut page = SameDocumentPage::with_routes(routes).await;
        let report = page.evaluate(r#"(async () => {
            const registration = await navigator.serviceWorker.register('/request-worker.js', {scope:'/native-request/'});
            const tick = () => new Promise(resolve => setTimeout(resolve,10));
            for (let n=0; n<500 && registration.active?.state !== 'activated'; ++n) await tick();
            if (registration.active?.state !== 'activated') throw new Error('worker not activated');
            const frame = document.createElement('iframe');
            const loaded = new Promise(resolve => frame.onload = resolve);
            frame.src = '/native-request/frame.html';
            document.body.appendChild(frame);
            await loaded;
            if (!frame.contentWindow.navigator.serviceWorker.controller) throw new Error('missing controller');
            const worker = await new Promise((resolve,reject) => {
                const xhr = new frame.contentWindow.XMLHttpRequest();
                xhr.open('GET','/native-request/echo',true,'alice','secret');
                xhr.onload = () => { try { resolve(JSON.parse(xhr.responseText)); } catch(e) { reject(e); } };
                xhr.onerror = () => reject(new Error('controlled XHR failed'));
                xhr.send();
            });
            const cache = await caches.open('native-request-url');
            const keys = await cache.keys();
            if (keys.length !== 1) throw new Error('missing native cache key');
            const key = keys[0], url = key.url;
            Object.defineProperty(key, 'url', {get() { throw new Error('public key url getter'); }});
            Object.defineProperty(key, 'toString', {value() { throw new Error('public key toString'); }});
            const constructors = [];
            let copy;
            for (const init of [undefined, {}, {cache:'no-store'}]) {
                try { const value = new Request(key,init); constructors.push({url:value.url}); copy ||= value; }
                catch(e) { constructors.push({error:e.name}); }
            }
            const forwarded = [];
            for (const input of [key,copy]) {
                if (!input) { forwarded.push({error:'no copy'}); continue; }
                try {
                    const response = await fetch(input);
                    forwarded.push({status:response.status,data:await response.json()});
                } catch(e) { forwarded.push({error:e.name}); }
            }
            const refused = [];
            for (const input of [url,{url}]) {
                try { new Request(input); refused.push('allowed'); } catch(e) { refused.push(e.name); }
                try { await fetch(input); refused.push('allowed'); } catch(e) { refused.push(e.name); }
            }
            frame.remove();
            await registration.unregister();
            await caches.delete('native-request-url');
            return {worker,window:{url,constructors,forwarded,refused}};
        })()"#).await;
        let expected_url = page.base_url.replace("http://", "http://alice:secret@").replace("/history.html", "/native-request/echo");
        for realm in ["worker", "window"] {
            assert_eq!(report[realm]["url"], expected_url, "{realm}: {report}");
            assert_eq!(report[realm]["constructors"], json!([
                {"url":expected_url},{"url":expected_url},{"url":expected_url}
            ]), "{realm}: {report}");
            assert_eq!(report[realm]["forwarded"], json!([
                {"status":200,"data":{"network":true}}, {"status":200,"data":{"network":true}}
            ]), "{realm}: {report}");
        }
        assert_eq!(report["worker"]["refused"], json!(["TypeError","TypeError"]));
        assert_eq!(report["window"]["refused"], json!(["TypeError","TypeError","TypeError","TypeError"]));
        assert_eq!(requests.load(Ordering::SeqCst),4, "rejected external inputs must not reach HTTP: {report}");
    }).await;
}
