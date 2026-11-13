use super::*;

#[tokio::test]
async fn service_worker_synthetic_redirect_without_location_preserves_client_mode() {
    let (base, server) = spawn_service_worker_response_server(vec![(
        "/app/worker.js", "text/javascript", r#"
        onactivate = event => event.waitUntil(clients.claim());
        onfetch = event => {
            const url = new URL(event.request.url);
            if (url.pathname === '/app/synthetic') {
                const body = url.searchParams.get('stream') === 'true'
                    ? new ReadableStream({start(controller) {controller.enqueue(new TextEncoder().encode('body'));controller.close();}})
                    : 'body';
                event.respondWith(new Response(body, {status:Number(url.searchParams.get('status')),
                    headers:{'Content-Type':'text/plain','X-Probe':'visible'}}));
            }
        };
        "#,
    )]).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, runtime) = new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
        &format!("{base}/app/page.html"),
        &loader,
    );
    vm.eval(r#"
        globalThis.redirectRows = [];
        globalThis.redirectDone = 'pending';
        (async () => {
            await navigator.serviceWorker.register('worker.js', {scope:'./'});
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller) {
                await new Promise(resolve => navigator.serviceWorker.addEventListener(
                    'controllerchange', resolve, {once:true}
                ));
            }
            for (const stream of [false,true]) {
                for (const status of [200,301,302,303,307,308]) {
                    for (const redirect of ['error','manual','follow']) {
                        try {
                            const response = await fetch('synthetic?status='+status+'&stream='+stream, {redirect});
                            const clone = response.clone();
                            redirectRows.push({type:response.type,status:response.status,bodyNull:response.body===null,
                                header:response.headers.get('x-probe'),body:await response.text(),
                                cloneType:clone.type,cloneBody:await clone.text()});
                        } catch(error) {redirectRows.push({error:error.name});}
                    }
                }
            }
            redirectDone = 'done';
        })().catch(error => redirectDone = String(error));
    "#).unwrap();
    drain_service_worker_test_until_eval_equals(
        &mut vm,
        &runtime,
        &loader,
        "String(redirectDone)",
        "done",
    )
    .await;
    let actual = vm.eval("JSON.stringify(redirectRows)").unwrap();
    let rows: serde_json::Value = serde_json::from_str(&actual).unwrap();
    let mut index = 0;
    for stream in [false, true] {
        for status in [200, 301, 302, 303, 307, 308] {
            for redirect in ["error", "manual", "follow"] {
                let expected = if status != 200 && redirect == "manual" {
                    serde_json::json!({"type":"opaqueredirect","status":0,"bodyNull":true,
                        "header":null,"body":"","cloneType":"opaqueredirect","cloneBody":""})
                } else {
                    serde_json::json!({"type":"basic","status":status,"bodyNull":false,
                        "header":"visible","body":"body","cloneType":"basic","cloneBody":"body"})
                };
                assert_eq!(
                    rows[index], expected,
                    "stream={stream}, status={status}, redirect={redirect}"
                );
                index += 1;
            }
        }
    }
    server.await.unwrap();
}
