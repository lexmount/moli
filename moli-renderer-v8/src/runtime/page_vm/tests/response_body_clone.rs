use super::blob_support::run_blob_fixture;

#[tokio::test(flavor = "current_thread")]
async fn response_body_clone_tees_all_sources_across_realms_and_workers() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/response-body-clone.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://response-body-clone.test/",
        2355,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn response_body_clone_uses_native_slots_and_receiver_realm() {
    run_blob_fixture(
        r#"
(async () => {
  const checks = [];
  const frame = document.createElement('iframe');document.body.appendChild(frame);
  const child = frame.contentWindow;
  const cloneMethod = Response.prototype.clone, savedResponse = Response;
  try {
    globalThis.Response = function() {throw Error('author Response constructor called');};
    for (const kind of ['none','text','blob','stream']) {
      const body = kind === 'none' ? undefined : kind === 'text' ? 'data'
        : kind === 'blob' ? new child.Blob(['data']) : new child.ReadableStream({start(c) {
          c.enqueue(new child.Uint8Array([100,97,116,97]));c.close();
        }});
      const input = new child.Response(body,{status:201,statusText:'Created',headers:{'x-original':'one'}});
      const original = input.body, headers = input.headers;
      if (original) original.tee = () => {throw Error('author tee called');};
      for (const key of ['body','bodyUsed','headers','status','statusText','type','url']) {
        Object.defineProperty(input,key,{get() {throw Error(`public ${key} read`);}});
      }
      const clone = cloneMethod.call(input);
      checks.push({name:`${kind}/clone realm`,passed:Object.getPrototypeOf(clone) === child.Response.prototype});
      checks.push({name:`${kind}/header realm`,passed:clone.headers instanceof child.Headers});
      checks.push({name:`${kind}/native metadata`,passed:clone.status === 201 && clone.statusText === 'Created'});
      checks.push({name:`${kind}/clone initially unused`,passed:!clone.bodyUsed});
      checks.push({name:`${kind}/null or tee body`,passed:kind === 'none' ? clone.body === null : original.locked && clone.body !== original});
      clone.headers.set('x-original','two');
      checks.push({name:`${kind}/independent headers`,passed:headers.get('x-original') === 'one'});
      checks.push({name:`${kind}/native body consumed`,passed:await clone.text() === (kind === 'none' ? '' : 'data')});
      checks.push({name:`${kind}/bodyUsed follows body presence`,passed:clone.bodyUsed === (kind !== 'none')});
    }
  } finally {globalThis.Response = savedResponse;frame.remove();}
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  globalThis.__uiEventResults = {checks:[{name:'clone completion',passed:false,detail:String(error.stack || error)}]};
});
"#,
        "'Response clone native slots'",
        "https://response-clone-native-slots.test/",
        32,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn response_body_clone_cancels_shared_source_after_both_branches() {
    run_blob_fixture(
        r#"
(async () => {
  const checks = [];
  const fence = async () => {for (let i = 0; i < 16; i++) await Promise.resolve();};
  {
    let controller, cancellations = 0;
    const response = new Response(new ReadableStream({
      start(value) {controller = value;value.enqueue(new Uint8Array([100,97,116,97]));},
      cancel() {cancellations++;},
    }));
    const original = response.body, clone = response.clone();
    let cancelled = false;
    const done = response.body.cancel({branch:1}).then(() => {cancelled = true;});
    await fence();
    checks.push({name:'one branch cancellation remains pending',passed:!cancelled && cancellations === 0});
    const reading = clone.text();controller.close();
    checks.push({name:'surviving branch retains data',passed:await reading === 'data'});
    await done;
    checks.push({name:'completion settles cancellation without cancelling closed source',passed:cancelled && cancellations === 0});
    checks.push({name:'original stream remains tee locked',passed:original.locked});
  }
  {
    let cancellations = 0, actual;
    const first = {branch:1}, second = {branch:2};
    const response = new Response(new ReadableStream({cancel(reason) {cancellations++;actual = reason;}}));
    const clone = response.clone();
    const one = response.body.cancel(first);await fence();
    checks.push({name:'first cancellation does not cancel source',passed:cancellations === 0});
    await Promise.all([one,clone.body.cancel(second)]);
    checks.push({name:'both branches cancel source exactly once',passed:cancellations === 1});
    checks.push({name:'cancellation preserves aggregate reason identity',passed:Array.isArray(actual) && actual.length === 2 && actual[0] === first && actual[1] === second});
    checks.push({name:'both cancelled bodies disturbed',passed:response.bodyUsed && clone.bodyUsed});
  }
  globalThis.__uiEventResults = {checks};
})().catch(error => {
  globalThis.__uiEventResults = {checks:[{name:'cancellation completion',passed:false,detail:String(error.stack || error)}]};
});
"#,
        "'Response clone shared source cancellation'",
        "https://response-clone-cancellation.test/",
        8,
    )
    .await;
}
