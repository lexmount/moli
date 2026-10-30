use super::*;
async fn assert_transferred_stream_deserialization_feedback(kind: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-deserialization-feedback.test/",
        &loader,
    );
    let registry = vm._context_host.borrow().message_port_registry();
    vm.eval(&format!("globalThis.transferKind = {kind:?}"))
        .expect("transfer kind");
    vm.eval(
        r#"
globalThis.transferSourceStops = 0;
globalThis.transferSourceError = null;
globalThis.transferReceiverError = null;
globalThis.transferSinkWrites = 0;
const frame = document.createElement('iframe');
frame.setAttribute('sandbox', 'allow-scripts');
addEventListener('message', event => {
  if (event.source !== frame.contentWindow) return;
  if (event.data.kind === 'ready') {
    const stop = error => {
      transferSourceStops++;
      transferSourceError = {name: error.name, realm: error instanceof DOMException};
    };
    const stream = transferKind === 'readable'
      ? new ReadableStream({
          start(controller) {
            controller.enqueue(new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0])));
          },
          cancel: stop
        })
      : new WritableStream({write() {transferSinkWrites++;}, abort: stop});
    frame.contentWindow.postMessage({kind: transferKind, stream}, '*', [stream]);
  } else if (event.data.kind === 'result') {
    transferReceiverError = event.data.error;
  }
});
frame.srcdoc = `<!doctype html><script>
  onmessage = event => {
    const report = error => parent.postMessage({kind: 'result', error: {
      name: error.name, realm: error instanceof DOMException
    }}, '*');
    if (event.data.kind === 'readable') {
      event.data.stream.getReader().read().then(
        () => report({name: 'unexpected read fulfillment'}), report
      );
    } else {
      const writer = event.data.stream.getWriter();
      writer.closed.then(() => report({name: 'unexpected writer close'}), report);
      writer.write(new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0]))).catch(() => {});
    }
  };
  parent.postMessage({kind: 'ready'}, '*');
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
"#,
    )
    .expect("queue a sandboxed cross-origin stream recipient");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify([transferSourceStops, transferSourceError, transferReceiverError, transferSinkWrites])",
        r#"[1,{"name":"DataCloneError","realm":true},{"name":"DataCloneError","realm":true},0]"#,
        "stream deserialization failure must settle both native realms",
    )
    .await;
    for _ in 0..32 {
        if registry.endpoint_count() == 0
            && vm
                ._context_host
                .borrow()
                .message_port_execution_context_owners_for_test()
                .is_empty()
        {
            break;
        }
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("release failed stream transfer endpoints");
    }
    assert_eq!(registry.endpoint_count(), 0, "{kind}: native endpoint leak");
    assert!(
        vm._context_host
            .borrow()
            .message_port_execution_context_owners_for_test()
            .is_empty(),
        "{kind}: native wrapper leak"
    );
}

#[tokio::test]
async fn transferred_readable_stream_deserialization_failure_cancels_the_source() {
    assert_transferred_stream_deserialization_feedback("readable").await;
}

#[tokio::test]
async fn transferred_writable_stream_deserialization_failure_rejects_the_remote_writer() {
    assert_transferred_stream_deserialization_feedback("writable").await;
}
