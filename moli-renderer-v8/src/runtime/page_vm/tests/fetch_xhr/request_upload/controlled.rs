use super::*;

async fn controlled_probe(
    server: &UploadServer,
    script: &str,
    dedicated: bool,
) -> serde_json::Value {
    // Blob Worker URLs cannot resolve relative request URLs.
    let script = script.replace("'/", &format!("'{}", server.url));
    let mut config = FetchConfig::default();
    config.set_tls_verify_host(false);
    config.set_http_no_proxy(Some("*".to_owned()));
    let owner = ResourceRequestClient::new(&config).unwrap();
    let mut page = test_page_vm_with_loader_and_document_url(
        &owner,
        Vec::new(),
        server.url.join("page").unwrap(),
    );
    page.retain_standalone_request_client_owner_for_test(owner);
    let executor = page.local_executor.clone();
    let source = format!(
        r#"
        (async () => {{
            globalThis.__wsEvents = ['register'];
            const registration = await navigator.serviceWorker.register('/sw.js', {{scope:'/'}});
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller) await new Promise(resolve =>
                navigator.serviceWorker.addEventListener('controllerchange', resolve, {{once:true}}));
            __wsEvents.push('controlled');
            async function run() {{ {script} }}
            let result;
            if ({dedicated}) {{
                const url = URL.createObjectURL(new Blob(['onmessage=async()=>{{try{{postMessage(await ('+run.toString()+')());}}catch(e){{postMessage({{failure:String(e.stack||e)}});}}}};']));
                const worker = new Worker(url);
                try {{
                    result = await new Promise((resolve,reject) => {{
                        worker.onmessage = event => resolve(event.data);
                        worker.onerror = event => reject(new Error(event.message));
                        worker.postMessage(1);
                    }});
                }} finally {{worker.terminate();URL.revokeObjectURL(url);}}
            }} else result = await run();
            await registration.unregister();
            return result;
        }})().then(value => globalThis.__controlledUploadResult = value,
                  error => globalThis.__controlledUploadResult = {{failure:String(error.stack||error)}})
            .finally(() => globalThis.__controlledUploadDone = true);
    "#
    );
    let result = executor
        .run(async move {
            page.vm_mut().eval(&source)?;
            drive_page_work_until_done_with_explicit_producer_admission(
                &mut page,
                "String(globalThis.__controlledUploadDone===true)",
                "controlled author upload should complete",
                |page| {
                    page.runtime_hooks
                        .browser_context_runtime
                        .drain_service_worker_service_lane();
                },
            )
            .await?;
            page.vm_mut()
                .eval("JSON.stringify(globalThis.__controlledUploadResult)")
        })
        .await
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[tokio::test]
async fn controlled_upload_snapshots_views_and_exposes_byob_across_window_and_worker() {
    run_page_vm_async_test(async {
        for dedicated in [false, true] {
            let server = UploadServer::new(false, 200).await.unwrap();
            let result = controlled_probe(&server, r#"
                const backing = new Uint8Array([88,0,255,65,66,99]);
                const body = new ReadableStream({
                    start(c){c.enqueue(backing.subarray(1,5));c.enqueue(new Uint8Array());},
                    pull(c){backing.fill(123);c.enqueue(new Uint8Array([3,4]));c.close();}
                },{highWaterMark:0});
                ReadableStream.prototype.getReader = () => {throw Error('public input getReader');};
                ReadableStreamDefaultReader.prototype.read = () => {throw Error('public input read');};
                const request = new Request('/upload?mode=byob',{method:'POST',body,duplex:'half'});
                const response = await fetch(request);
                return {bytes:await response.json(),locked:body.locked,used:request.bodyUsed};
            "#, dedicated).await;
            assert_eq!(result, serde_json::json!({"bytes":[0,255,65,66,3,4],"locked":true,"used":true}), "dedicated={dedicated}");
            assert!(server.wire.lock().bytes.is_empty());
            server.shutdown().await;
        }
    }).await;
}

#[tokio::test]
async fn controlled_upload_fallback_clone_and_forward_send_bytes_before_eof() {
    run_page_vm_async_test(async {
        for dedicated in [false, true] {
            for mode in ["fallback", "clone", "forward"] {
                let server = UploadServer::new(false, 200).await.unwrap();
                let script = r#"
                    let controller,settled=false;
                    const body = new ReadableStream({start(c){controller=c;c.enqueue(new Uint8Array([0,255,65,66]));}});
                    const promise=fetch('/upload?mode=MODE',{method:'POST',body,duplex:'half'})
                        .then(async response=>{settled=true;return response.text();});
                    await (await fetch('/first')).text();
                    if(settled)throw Error('fetch settled before request EOF');
                    controller.enqueue(new Uint8Array());
                    controller.enqueue(new Uint8Array(196613).fill(7));controller.close();
                    const text=await promise;await (await fetch('/finished')).text();
                    return {text,locked:body.locked};
                "#.replace("MODE", mode);
                let result=controlled_probe(&server,&script,dedicated).await;
                assert_eq!(result,serde_json::json!({"text":"uploaded","locked":true}),"mode={mode}, dedicated={dedicated}");
                {
                    let wire=server.wire.lock();assert!(wire.ended);
                    assert_eq!(&wire.bytes[..4],&[0,255,65,66]);assert_eq!(wire.bytes.len(),196617);
                    assert!(wire.bytes[4..].iter().all(|byte|*byte==7));
                    assert!(wire.headers.get("content-length").is_none());
                }
                server.shutdown().await;
            }
        }
    }).await;
}

#[tokio::test]
async fn controlled_upload_retains_backpressure_through_fallback_and_forward() {
    run_page_vm_async_test(async {
        for mode in ["fallback","forward"] {
            let server=UploadServer::new(true,200).await.unwrap();
            // Keep 4 MiB on the wire while limiting external owner turns in
            // this standalone PageVm's explicitly admitted Service Worker lane.
            let script=r#"
                let pulls=0,settled=false;
                const body=new ReadableStream({pull(c){c.enqueue(new Uint8Array(262144).fill(1));if(++pulls===16)c.close();}},{highWaterMark:0});
                const promise=fetch('/upload?mode=MODE',{method:'POST',body,duplex:'half'})
                    .then(async response=>{settled=true;return response.text();});
                __wsEvents.push('await headers');
                await (await fetch('/headers')).text();const bounded=pulls<16&&!settled;
                __wsEvents.push('headers, pulls='+pulls);
                await (await fetch('/release')).text();
                __wsEvents.push('released, pulls='+pulls);
                const text=await promise;
                await (await fetch('/finished')).text();return {bounded,pulls,text};
            "#.replace("MODE",mode);
            let result=controlled_probe(&server,&script,false).await;
            assert_eq!(result,serde_json::json!({"bounded":true,"pulls":16,"text":"uploaded"}),"mode={mode}");
            assert_eq!(server.wire.lock().bytes.len(),4*1024*1024);
            server.shutdown().await;
        }
    }).await;
}

#[tokio::test]
async fn controlled_upload_echo_exposes_headers_and_bytes_before_source_eof() {
    run_page_vm_async_test(async {
        for dedicated in [false, true] {
            let server = UploadServer::new(false, 200).await.unwrap();
            let result = controlled_probe(&server, r#"
                let controller;
                const body = new ReadableStream({start(c){controller=c;c.enqueue(new Uint8Array([0,255]));}});
                const response = await fetch('/upload?mode=echo',{method:'POST',body,duplex:'half'});
                const reader = response.body.getReader(),first = await reader.read();
                controller.enqueue(new Uint8Array([65,66]));controller.close();
                const bytes = Array.from(first.value);
                while (true) {const {value,done}=await reader.read();if(done)break;bytes.push(...value);}
                return {status:response.status,bytes};
            "#, dedicated).await;
            assert_eq!(result, serde_json::json!({"status":200,"bytes":[0,255,65,66]}), "dedicated={dedicated}");
            assert!(server.wire.lock().bytes.is_empty());
            server.shutdown().await;
        }
    }).await;
}

#[tokio::test]
async fn controlled_upload_synthetic_responses_keep_body_errors_and_abort_lifetimes() {
    run_page_vm_async_test(async {
        for dedicated in [false,true] {
            let server=UploadServer::new(false,200).await.unwrap();
            let result=controlled_probe(&server,r#"
                const rows=[];
                for(const kind of ['open','error','invalid']){
                    const controller=new AbortController(),reason={marker:'after response'};
                    let cancelled=0,same=false;
                    const body=new ReadableStream({start(c){if(kind==='error')c.error(Error('source'));else {c.enqueue(kind==='invalid'?'wrong':new Uint8Array([65,66]));if(kind==='invalid')c.close();}},cancel(value){cancelled++;same=value===reason;}});
                    const response=await fetch('/upload?mode=constant',{method:'POST',body,duplex:'half',signal:controller.signal});
                    const text=await response.text(),before=cancelled;controller.abort(reason);
                    rows.push({kind,text,before,cancelled,same});
                }
                for(const kind of ['error','invalid']){
                    const body=new ReadableStream({start(c){if(kind==='error')c.error(Error('source'));else {c.enqueue('wrong');c.close();}}});
                    const response=await fetch('/upload?mode=echo',{method:'POST',body,duplex:'half'});
                    let name;try{await response.text();}catch(error){name=error.name;}
                    rows.push({kind,status:response.status,name});
                }return rows;
            "#,dedicated).await;
            assert_eq!(result,serde_json::json!([
                {"kind":"open","text":"constant","before":0,"cancelled":1,"same":true},
                {"kind":"error","text":"constant","before":0,"cancelled":0,"same":false},
                {"kind":"invalid","text":"constant","before":0,"cancelled":0,"same":false},
                {"kind":"error","status":200,"name":"TypeError"},
                {"kind":"invalid","status":200,"name":"TypeError"}
            ]),"dedicated={dedicated}");
            server.shutdown().await;
        }
    }).await;
}

#[tokio::test]
async fn controlled_upload_rejects_used_fallback_but_keeps_source_abort_algorithm() {
    run_page_vm_async_test(async {
        let server=UploadServer::new(false,200).await.unwrap();
        let result=controlled_probe(&server,r#"
            const controller=new AbortController(),reason={marker:'used fallback'};
            let cancelled=0,same=false;
            const body=new ReadableStream({start(c){c.enqueue(new Uint8Array([65]));},cancel(value){cancelled++;same=value===reason;}});
            let name;try{await fetch('/upload?mode=used',{method:'POST',body,duplex:'half',signal:controller.signal});}catch(error){name=error.name;}
            const before=cancelled;controller.abort(reason);return {name,before,cancelled,same};
        "#,false).await;
        assert_eq!(result,serde_json::json!({"name":"TypeError","before":0,"cancelled":1,"same":true}));
        assert!(server.wire.lock().bytes.is_empty());
        server.shutdown().await;
    }).await;
}

#[tokio::test]
async fn controlled_upload_checks_source_before_synthetic_redirect_method_rewriting() {
    run_page_vm_async_test(async {
        let server=UploadServer::new(false,200).await.unwrap();
        let result=controlled_probe(&server,r#"
            const rows=[];
            for(const status of [301,302,303,307,308]){
                const body=new ReadableStream({start(c){c.enqueue(new Uint8Array([65]));c.close();}});
                try{const response=await fetch('/upload?mode=redirect&status='+status,{method:'POST',body,duplex:'half'});rows.push({status,text:await response.text()});}
                catch(error){rows.push({status,name:error.name});}
            }return rows;
        "#,false).await;
        assert_eq!(result,serde_json::json!([
            {"status":301,"name":"TypeError"},{"status":302,"name":"TypeError"},
            {"status":303,"text":"uploaded"},{"status":307,"name":"TypeError"},
            {"status":308,"name":"TypeError"}
        ]));
        server.shutdown().await;
    }).await;
}
