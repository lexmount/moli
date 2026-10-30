use super::*;
async fn assert_transferred_wasm_stream_chunks(kind: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-wasm-chunks.test/",
        &loader,
    );
    let registry = vm._context_host.borrow().message_port_registry();
    vm.eval(&format!("globalThis.transferKind = {kind:?};"))
        .expect("stream kind");
    vm.eval(
        r#"
globalThis.wasmTransferResult = null;
globalThis.wasmSinkClosed = false;
globalThis.wasmSinkValues = [];
globalThis.wasmSinkFirstModule = null;
globalThis.wasmSinkDistinctModules = false;
const wasmBytes = [
  0,97,115,109,1,0,0,0,
  1,5,1,96,0,1,127,
  3,2,1,0,
  7,10,1,6,97,110,115,119,101,114,0,0,
  10,6,1,4,0,65,42,11
];
const frame = document.createElement('iframe');
addEventListener('message', event => {
  if (event.source !== frame.contentWindow) return;
  if (event.data.kind === 'ready') {
    let stream;
    if (transferKind === 'readable') {
      const module = new WebAssembly.Module(new Uint8Array(wasmBytes));
      stream = new ReadableStream({start(controller) {
        controller.enqueue(module);
        controller.enqueue(37);
        controller.enqueue(module);
        controller.close();
      }});
    } else {
      stream = new WritableStream({
        write(value) {
          if (typeof value === 'number') {
            wasmSinkValues.push(value);
          } else {
            if (wasmSinkFirstModule === null) wasmSinkFirstModule = value;
            else wasmSinkDistinctModules = value !== wasmSinkFirstModule;
            wasmSinkValues.push([
              value instanceof WebAssembly.Module,
              value instanceof frame.contentWindow.WebAssembly.Module,
              new WebAssembly.Instance(value).exports.answer()
            ]);
          }
        },
        close() {wasmSinkClosed = true;}
      });
    }
    frame.contentWindow.postMessage({kind: transferKind, stream}, '*', [stream]);
  } else if (event.data.kind === 'result') {
    wasmTransferResult = event.data.result;
  }
});
frame.srcdoc = `<!doctype html><script>
  onmessage = async event => {
    const report = result => parent.postMessage({kind: 'result', result}, '*');
    try {
      if (event.data.kind === 'readable') {
        const reader = event.data.stream.getReader();
        const first = await reader.read();
        const middle = await reader.read();
        const last = await reader.read();
        const end = await reader.read();
        report({
          status: 'closed',
          values: [
            [first.value instanceof WebAssembly.Module,
             new WebAssembly.Instance(first.value).exports.answer()],
            middle.value,
            [last.value instanceof WebAssembly.Module,
             new WebAssembly.Instance(last.value).exports.answer()]
          ],
          distinct: first.value !== last.value,
          done: end.done
        });
      } else {
        const module = new WebAssembly.Module(new Uint8Array(${JSON.stringify(wasmBytes)}));
        const writer = event.data.stream.getWriter();
        await writer.write(module);
        await writer.write(37);
        await writer.write(module);
        await writer.close();
        report({status: 'closed'});
      }
    } catch (error) {
      report({status: error.name});
    }
  };
  parent.postMessage({kind: 'ready'}, '*');
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
"#,
    )
    .expect("transfer a native stream to another realm");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(wasmTransferResult !== null && (wasmTransferResult.status !== 'closed' || transferKind === 'readable' || wasmSinkClosed))",
        "true",
        "Wasm stream chunks and close must settle",
    )
    .await;
    let (expression, expected) = if kind == "readable" {
        (
            "JSON.stringify(wasmTransferResult)",
            r#"{"status":"closed","values":[[true,42],37,[true,42]],"distinct":true,"done":true}"#,
        )
    } else {
        (
            "JSON.stringify([wasmTransferResult, wasmSinkValues, wasmSinkDistinctModules, wasmSinkClosed])",
            r#"[{"status":"closed"},[[false,true,42],37,[false,true,42]],true,true]"#,
        )
    };
    assert_eq!(vm.eval(expression).unwrap(), expected, "{kind}");
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
            .expect("release completed stream transfer endpoints");
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
async fn transferred_readable_stream_preserves_wasm_chunks_and_close() {
    assert_transferred_wasm_stream_chunks("readable").await;
}

#[tokio::test]
async fn transferred_writable_stream_preserves_wasm_chunks_and_close() {
    assert_transferred_wasm_stream_chunks("writable").await;
}
