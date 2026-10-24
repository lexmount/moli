use super::*;

#[tokio::test(flavor = "current_thread")]
async fn fetch_keepalive_quota_uses_encoded_body_bytes_after_request_construction() {
    let server = StaticHttpServer::spawn(2).await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
    vm.eval(
        r#"
globalThis.__quotaResult = null;
(async () => {
  const form = new FormData();
  form.set('file', new Blob(['a'.repeat(65536)]), 'file');
  const bodies = ['a'.repeat(65537), 'é'.repeat(32769), new Uint8Array(65537),
    new Blob(['a'.repeat(65537)]), new URLSearchParams({q:' '.repeat(65535)}), form];
  const errors = [];
  const used = [];
  for (const body of bodies) {
    const request = new Request('/too-large', {method:'POST',body,keepalive:true});
    used.push(request.bodyUsed);
    errors.push(await fetch(request).then(() => false, error => error instanceof TypeError));
    used.push(request.bodyUsed);
  }
  const exact = new Request('/exact', {method:'POST',body:'é'.repeat(32768),keepalive:true});
  const response = await fetch(exact);
  await response.text();
  const overridden = new Request('/override', {method:'POST',body:'old',keepalive:true});
  await overridden.text();
  const overrideResponse = await fetch(overridden, {body:'é'.repeat(32768)});
  await overrideResponse.text();
  __quotaResult = {errors,used,ok:response.ok && overrideResponse.ok,exactUsed:exact.bodyUsed,overrideUsed:overridden.bodyUsed};
})().catch(error => { __quotaResult = {error:String(error)}; });
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__quotaResult !== null)",
        "true",
        "keepalive encoded body quota",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__quotaResult)").unwrap()).unwrap();
    assert_eq!(
        result,
        serde_json::json!({"errors":vec![true;6],"used":[false,true,false,true,false,true,false,true,false,true,false,true],"ok":true,"exactUsed":true,"overrideUsed":true})
    );
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].target, "/exact");
    assert_eq!(requests[1].target, "/override");
    for request in requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.body.len(), 65536);
        assert_eq!(request.body, "é".repeat(32768).as_bytes());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_keepalive_quota_is_shared_with_beacons_and_released_after_completion() {
    let server = StaticHttpServer::spawn(2).await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
    vm.eval(
        r#"
globalThis.__quotaResult = null;
const first = fetch('/fetch-full', {method:'POST',body:'a'.repeat(65536),keepalive:true});
const fetchOver = fetch('/fetch-over', {method:'POST',body:'b',keepalive:true})
  .then(() => false,error => error instanceof TypeError);
const beaconOver = navigator.sendBeacon('/beacon-over','b');
Promise.all([first.then(response => response.text()),fetchOver]).then(([body,rejected]) => {
  __quotaResult = {body:body.length>0,rejected,beaconOver};
});
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__quotaResult !== null)",
        "true",
        "fetch and beacon shared quota",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__quotaResult)").unwrap(),
        r#"{"body":true,"rejected":true,"beaconOver":false}"#
    );
    vm.eval(
        r#"
globalThis.__beaconQuotaResult = null;
const accepted = navigator.sendBeacon('/beacon-full','a'.repeat(65536));
fetch('/over-beacon', {method:'POST',body:'b',keepalive:true})
  .then(() => {__beaconQuotaResult={accepted,rejected:false}},
    error => {__beaconQuotaResult={accepted,rejected:error instanceof TypeError}});
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__beaconQuotaResult !== null)",
        "true",
        "beacon blocks over-quota fetch",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__beaconQuotaResult)").unwrap(),
        r#"{"accepted":true,"rejected":true}"#
    );
    let requests = tokio::time::timeout(std::time::Duration::from_secs(8), server.finish())
        .await
        .expect("both admitted requests must reach the real transport");
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        ["/fetch-full", "/beacon-full"]
    );
    assert!(
        requests
            .iter()
            .all(|r| r.method == "POST" && r.body.len() == 65536)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn worker_fetch_keepalive_quota_rejects_before_transport() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/javascript\r\nCache-Control: no-store\r\n",
        |_, request| {
            if request.target == "/worker.js" {
                r#"
const first=fetch(new Request('/worker-full',{method:'POST',body:'a'.repeat(65536),keepalive:true}));
const over=fetch(new Request('/worker-over',{method:'POST',body:'b',keepalive:true}))
  .then(()=>false,error=>error instanceof TypeError);
Promise.all([first.then(response=>response.text()),over]).then(([body,rejected])=>{
  postMessage({body:body==='ok',rejected});close();
});
"#
                .to_owned()
            } else {
                "ok".to_owned()
            }
        },
    )
    .await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
    vm.eval("globalThis.__workerQuotaResult=null;const worker=new Worker('/worker.js');worker.onmessage=event=>{__workerQuotaResult=event.data};").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__workerQuotaResult !== null)",
        "true",
        "real worker keepalive quota",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__workerQuotaResult)").unwrap(),
        r#"{"body":true,"rejected":true}"#
    );
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        ["/worker.js", "/worker-full"]
    );
    assert_eq!(requests[1].body.len(), 65536);
}
