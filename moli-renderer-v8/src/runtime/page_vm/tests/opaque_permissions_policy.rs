use super::*;

const PROBE: &str = include_str!("../../../../tests/fixtures/opaque-permissions-policy.js");
const CHILD: &str = "<!doctype html><script>onmessage=event=>event.source.postMessage({enabled:document.fullscreenEnabled,origin:origin,url:location.href},'*')</script>";

#[tokio::test]
async fn iframe_src_delegation_preserves_parent_headers_for_opaque_and_tuple_origins() {
    run_page_vm_async_test(async move {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server_origin = origin.clone();
        let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let listener = TcpListener::from_std(listener).unwrap();
                    let mut connections = tokio::task::JoinSet::new();
                    loop {
                        let accepted = tokio::select! {
                            result = listener.accept() => result.ok(),
                            _ = &mut stopped => None,
                            _ = tokio::time::sleep(Duration::from_secs(30)) => None,
                        };
                        let Some((mut stream, _)) = accepted else { break };
                        let server_origin = server_origin.clone();
                        connections.spawn(async move {
                            let request = read_http_request_with_body(&mut stream).await.unwrap();
                            let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                            let url = Url::parse(&format!("{server_origin}{path}")).unwrap();
                            let header = url.query_pairs().find(|(key, _)| key == "policy").map(|(_, value)| value.into_owned()).unwrap_or_default();
                            assert!(["", "fullscreen=()", "fullscreen=(self)", "fullscreen=*"].contains(&header.as_str()));
                            let body = if url.path() == "/child" {
                                CHILD.to_owned()
                            } else {
                                assert_eq!(url.path(), "/page");
                                let probe = PROBE.replace("</script>", "<\\/script>");
                                format!("<!doctype html><body><script>{probe}\n(async()=>{{while(!policyDone)await new Promise(resolve=>setTimeout(resolve,0));parent.postMessage({{rows:policyRows,error:policyError}},'*');}})();</script>")
                            };
                            let extra = if header.is_empty() { String::new() } else { format!("Permissions-Policy: {header}\r\n") };
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n{extra}\r\n{body}", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                    connections.abort_all();
                    while connections.join_next().await.is_some() {}
                });
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        hooks.resource_task_runner = Some(loader.browser_resource_runtime().resource_task_runner());
        let mut page = test_page_vm_with_loader_document_url_and_hooks(
            &loader,
            Vec::new(),
            Url::parse(&format!("{origin}/root")).unwrap(),
            hooks,
        );
        page.retain_standalone_request_client_owner_for_test(loader);
        let local = page.local_executor.clone();
        local.run(async move {
            page.vm_mut().eval(r#"
globalThis.policyResult=null;
(async()=>{
 const html=document.documentElement||document.appendChild(document.createElement('html'));
 const body=document.body||html.appendChild(document.createElement('body'));
 const rows=[];
 for(const header of ['', 'fullscreen=()', 'fullscreen=(self)', 'fullscreen=*']) {
  const frame=document.createElement('iframe');
  const response=new Promise(resolve=>{
   const listener=event=>{
    if(event.source!==frame.contentWindow)return;
    removeEventListener('message',listener);resolve(event.data);
   };addEventListener('message',listener);
  });
  frame.src='/page?policy='+encodeURIComponent(header);body.appendChild(frame);
  const value=await response;
  if(value.error!==null)throw Error(value.error);
  for(const row of value.rows)rows.push({header,...row});
  frame.remove();
 }
 policyResult={rows};
})().catch(error=>policyResult={error:String(error)});
void 0;
            "#)?;
            drive_page_work_until_done_with_explicit_producer_admission(
                &mut page,
                "String(globalThis.policyResult !== null)",
                "Real iframe permission reports must finish",
                |_| {},
            ).await?;
            let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(policyResult)")?)?;
            assert_eq!(result["error"], serde_json::Value::Null, "{result}");
            let rows = result["rows"].as_array().unwrap();
            assert_eq!(rows.len(), 64);
            for row in rows {
                let allow = row["allow"].as_str();
                let parent_allows = matches!(row["header"].as_str().unwrap(), "" | "fullscreen=*");
                let container_allows = row["fullscreenAttribute"] == true
                    || matches!(allow, Some("fullscreen" | "fullscreen 'src'" | "fullscreen *" | "payment;fullscreen"));
                assert_eq!(row["observed"]["enabled"], parent_allows && container_allows, "{row}");
                assert_eq!(row["sourceMatches"], true);
                assert_eq!(row["observed"]["origin"], if row["source"] == "data" { "null".to_owned() } else { origin.replace("127.0.0.1", "localhost") });
            }
            anyhow::Ok(())
        }).await.unwrap();
        let _ = stop.send(());
        server.join().unwrap();
    }).await;
}
