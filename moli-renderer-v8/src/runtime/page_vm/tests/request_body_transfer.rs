use super::blob_support::run_blob_fixture;
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn request_clone_uses_receiver_realm_and_native_construction_for_every_body_source() {
    run_blob_fixture(
        r#"
(async () => {
  const checks = [];
  const frame = document.createElement('iframe');document.body.appendChild(frame);
  const child = frame.contentWindow;
  const cloneMethod = Request.prototype.clone, savedRequest = Request;
  try {
    globalThis.Request = function() {throw Error('author Request constructor called');};
    for (const kind of ['none','text','blob','stream']) {
      const controller = new child.AbortController();
      const body = kind === 'none' ? undefined : kind === 'text' ? 'data'
        : kind === 'blob' ? new child.Blob(['data']) : new child.ReadableStream({start(c) {
          c.enqueue(new child.Uint8Array([100,97,116,97]));c.close();
        }});
      const input = new child.Request(location.href,{method:kind === 'none' ? 'GET' : 'POST',body,
        signal:controller.signal,...(kind === 'stream' ? {duplex:'half'} : {})});
      const original = input.body;
      const clone = cloneMethod.call(input);
      checks.push({name:`${kind}/clone realm`,passed:Object.getPrototypeOf(clone) === child.Request.prototype});
      checks.push({name:`${kind}/header realm`,passed:clone.headers instanceof child.Headers});
      checks.push({name:`${kind}/signal realm and identity`,passed:clone.signal instanceof child.AbortSignal && clone.signal !== input.signal});
      checks.push({name:`${kind}/both bodies unused`,passed:!input.bodyUsed && !clone.bodyUsed});
      checks.push({name:`${kind}/null or tee body`,passed:kind === 'none' ? clone.body === null : original.locked && input.body !== original && clone.body !== input.body});
      const values = await Promise.all([input.text(),clone.text()]);
      checks.push({name:`${kind}/both preserve data`,passed:values.every(value=>value === (kind === 'none' ? '' : 'data'))});
      checks.push({name:`${kind}/bodyUsed follows body presence`,passed:input.bodyUsed === (kind !== 'none') && clone.bodyUsed === (kind !== 'none')});
      const reason = {};controller.abort(reason);
      checks.push({name:`${kind}/dependent abort signal`,passed:input.signal.aborted && clone.signal.aborted && clone.signal.reason === reason});
    }
  } finally {globalThis.Request = savedRequest;frame.remove();}
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  globalThis.__uiEventResults = {checks:[{name:'clone completion',passed:false,detail:String(error.stack || error)}]};
});
"#,
        "'borrowed Request clone'",
        "https://request-clone-receiver-realm.test/",
        32,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn request_body_transfer_preserves_sources_realms_cancellation_and_validation() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/request-body-transfer.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://request-body-transfer.test/",
        3310,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn request_body_transfer_drains_the_proxy_before_native_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://request-transfer-native-completion.test/")?,
            );
        page.vm_mut().eval(
            r#"
globalThis.events = [];
const input = new Request(location.href, {method:'POST',body:'source'});
const original = input.body;
const transferred = new Request(input);
globalThis.transferState = [input.bodyUsed, original.locked, transferred.bodyUsed];
transferred.text().then(value => events.push(value));
"#,
        )?;
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(transferState)")?,
            "[true,true,false]",
        );
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
            "[]",
        );
        // Fully reading the proxy releases the pipe's input reader. Returning
        // cached bytes without reading the proxy would leave this lock held.
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("original.locked")?,
            "false",
        );
        let Some(crate::page_task_queue::RendererPageReadyDescriptor::Timer {
            deadline,
            selection,
        }) = page.due_page_timer_ready_descriptor(
            crate::page_task_queue::RendererPageTimerSelection::AnyReady,
        )
        else {
            anyhow::bail!("public Body completion must queue its native Networking callback");
        };
        page.apply_selected_page_scheduler_task_on_owner_lane_for_test(
            crate::page_task_queue::RendererPageSchedulerTask::Timer {
                deadline,
                selection,
            },
            loader.clone(),
        )
        .await?;
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
            r#"["source"]"#,
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Request transfer consumes its proxy and keeps public completion on a native task");
}

#[tokio::test(flavor = "current_thread")]
async fn request_body_transfer_chain_keeps_replay_metadata_and_releases_input_readers() {
    run_blob_fixture(
        r#"
(async () => {
  const checks = [];
  const first = new Request(location.href, {method:'POST',body:'data',mode:'no-cors',keepalive:true});
  const inputs = [];
  let current = first;
  for (let i = 0; i < 64; i++) {
    const input = current;
    current = new Request(input);
    inputs.push(input);
    checks.push({name:`transfer ${i} locks and disturbs input`,passed:input.bodyUsed && input.body.locked});
    checks.push({name:`transfer ${i} keeps non-null source permissions`,passed:current.mode === 'no-cors' && current.keepalive});
  }
  checks.push({name:'final data survives chain',passed:await current.text() === 'data'});
  checks.push({name:'all upstream readers released',passed:inputs.every(input => !input.body.locked && input.bodyUsed)});
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  globalThis.__uiEventResults = {checks:[{name:'chain completion',passed:false,detail:String(error.stack || error)}]};
});
"#,
        "'request transfer chain'",
        "https://request-transfer-chain.test/",
        130,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn request_body_transfer_uses_native_slots_under_public_property_poisoning() {
    run_blob_fixture(
        r#"
(async () => {
  const input = new Request(location.href, {method:'POST',body:'data'});
  const original = input.body;
  for (const key of ['body','bodyUsed','headers','method','signal','url']) {
    Object.defineProperty(input,key,{get() {throw Error(`public ${key} read`);}});
  }
  const transferred = new Request(input);
  globalThis.__uiEventResults = {checks:[
    {name:'native body stream transferred',passed:original.locked},
    {name:'native method copied',passed:transferred.method === 'POST'},
    {name:'native MIME copied',passed:transferred.headers.get('content-type') === 'text/plain;charset=UTF-8'},
    {name:'native body consumed',passed:await transferred.text() === 'data'},
    {name:'original native reader released',passed:!original.locked},
  ]};
})().catch(error => {
  globalThis.__uiEventResults = {checks:[{name:'poisoning completion',passed:false,detail:String(error.stack || error)}]};
});
"#,
        "'native Request slot poisoning'",
        "https://request-transfer-native-slots.test/",
        5,
    )
    .await;
}
