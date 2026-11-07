use super::*;

#[tokio::test(flavor = "current_thread")]
async fn body_child_completion_does_not_rebind_opener_to_retired_frame() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://body-child-owner-checkpoint.test/",
        &loader,
    );
    vm.eval(r#"
        globalThis.childBodyOwnerResult = null;
        (async () => {
            const frame = document.createElement('iframe'); document.body.appendChild(frame);
            const child = frame.contentWindow;
            for (let i = 0; i < 3; i++) {
                if (await new child.Response('body').text() !== 'body') throw Error('body bytes');
            }
            frame.remove();
            const url = URL.createObjectURL(new Blob(['postMessage("ok");close();'], {type:'text/javascript'}));
            const worker = new Worker(url);
            try {
                return await new Promise((resolve, reject) => {
                    worker.onmessage = event => resolve(event.data);
                    worker.onerror = event => reject(Error(event.message));
                });
            } finally { worker.terminate(); URL.revokeObjectURL(url); }
        })().then(value => childBodyOwnerResult = value,
            error => childBodyOwnerResult = String(error.stack || error));
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(childBodyOwnerResult !== null)",
        "true",
        "opener Worker after child Body Networking checkpoints",
    )
    .await;
    assert_eq!(vm.eval("childBodyOwnerResult").unwrap(), "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn body_consumers_reject_detached_chunks_and_accept_attached_empty_chunks() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://detached-body-chunk.test/", &loader);
    vm.eval(r#"
        globalThis.detachedBodyChunkResult = null;
        (async () => {
            const failures = [];
            for (const kind of ['Request', 'Response']) {
                for (const method of ['text', 'arrayBuffer', 'bytes', 'blob']) {
                    for (const detached of [false, true]) {
                        const label = [kind, method, detached ? 'detached' : 'empty'].join('/');
                        const chunk = detached ? new Uint8Array([65, 66, 67]) : new Uint8Array(0);
                        if (detached) structuredClone(chunk.buffer, {transfer: [chunk.buffer]});
                        const stream = new ReadableStream({start(controller) {
                            controller.enqueue(chunk); controller.close();
                        }});
                        const body = kind === 'Request'
                            ? new Request(location.href, {method: 'POST', body: stream, duplex: 'half'})
                            : new Response(stream);
                        try {
                            const result = await body[method]();
                            const length = method === 'text' ? result.length
                                : method === 'blob' ? result.size : result.byteLength;
                            if (detached) failures.push(label + ': detached chunk accepted');
                            else if (length !== 0) failures.push(label + ': incorrect empty result');
                        } catch (error) {
                            if (!detached || !(error instanceof TypeError))
                                failures.push(label + ': ' + String(error));
                        }
                        if (!body.bodyUsed || !stream.locked)
                            failures.push(label + ': consumption lock/state changed');
                    }
                }
            }
            return failures;
        })().then(value => detachedBodyChunkResult = value,
            error => detachedBodyChunkResult = [String(error.stack || error)]);
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(detachedBodyChunkResult !== null)",
        "true",
        "native Body detached/empty chunk contract",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(detachedBodyChunkResult)").unwrap(),
        "[]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn body_blob_mime_uses_combined_lowercase_header_values() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://body-blob-mime.test/", &loader);
    vm.eval(r#"
      globalThis.blobMimeResult = null;
      (async () => {
        const cases = [
          [[], ''],
          [['', 'text/plain'], ', text/plain'],
          [['text/plain', ''], 'text/plain, '],
          [['text/plain', 'text/html'], 'text/plain, text/html'],
          [['TEXT/PLAIN;Charset=GBK', 'text/plain'], 'text/plain;charset=gbk, text/plain'],
          [['text/html', '*/*'], 'text/html, */*'],
          [['text/html; x="A,b"'], 'text/html; x="a,b"'],
          [['invalid'], 'invalid'],
          [['*/*'], '*/*'],
          [['Text/Plain; Charset="UTF-8"'], 'text/plain; charset="utf-8"'],
          [['text/plain;\tcharset=UTF-8'], ''],
          [['text/pl\u00e4in'], '']
        ];
        for (const kind of ['Request', 'Response']) {
          for (const [values, expected] of cases) {
            const headers = values.map(value => ['Content-Type', value]);
            const bytes = new Uint8Array([1, 2, 3]);
            const body = kind === 'Request'
              ? new Request('https://body-blob-mime.test/', {method:'POST', body:bytes, headers})
              : new Response(bytes, {headers});
            const before = body.headers.get('Content-Type');
            const blob = await body.blob();
            if (blob.type !== expected || blob.size !== 3 || body.headers.get('Content-Type') !== before)
              throw new Error(kind + ' ' + JSON.stringify(values) + ': ' + blob.type);
          }
        }
        return 'passed';
      })().then(value => blobMimeResult = value, error => blobMimeResult = String(error));
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(blobMimeResult !== null)",
        "true",
        "Body Blob MIME probe",
    )
    .await;
    assert_eq!(vm.eval("blobMimeResult").unwrap(), "passed");
}

#[tokio::test(flavor = "current_thread")]
async fn body_blob_type_normalizes_pending_response_streams() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://pending-blob-mime.test/", &loader);
    vm.eval(r#"
      globalThis.pendingBlobMimeResult = null;
      (async () => {
        const getHeader = Headers.prototype.get;
        const originalBlob = globalThis.Blob;
        const type = 'Text/Plain ; Charset="UTF-8"; X-Label="Aa B"';
        for (const kind of ['Request', 'Response']) {
          let controller;
          const options = {headers: {'Content-Type': type}};
          const body = kind === 'Request'
            ? new Request('https://pending-blob-mime.test/', {
                ...options, method: 'POST', body: new Uint8Array([0,65,255])
              })
            : new Response(new ReadableStream({start(value) { controller = value; }}), options);
          body.headers.get = () => { throw new Error('author Headers.get'); };
          body.headers.entries = () => { throw new Error('author Headers.entries'); };
          globalThis.Blob = function() { throw new Error('author Blob constructor'); };
          try {
            const pending = body.blob();
            if (controller) {
              controller.enqueue(new Uint8Array([0,65,255]));
              controller.close();
            }
            const blob = await pending;
            if (!(blob instanceof originalBlob) || blob.type !== type.toLowerCase())
              throw new Error(kind + ': Blob type/prototype');
            const bytes = Array.from(new Uint8Array(await blob.arrayBuffer())).join(',');
            if (bytes !== '0,65,255' || !body.bodyUsed)
              throw new Error(kind + ': body bytes/state');
            if (getHeader.call(body.headers, 'Content-Type') !== type)
              throw new Error(kind + ': changed header');
          } finally {
            globalThis.Blob = originalBlob;
          }
        }
        return 'passed';
      })().then(value => pendingBlobMimeResult = value, error => pendingBlobMimeResult = String(error));
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(pendingBlobMimeResult !== null)",
        "true",
        "pending Response Blob MIME",
    )
    .await;
    assert_eq!(vm.eval("pendingBlobMimeResult").unwrap(), "passed");
}

#[tokio::test(flavor = "current_thread")]
async fn pending_body_blob_completion_uses_its_intrinsic_constructor() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://pending-blob-intrinsic.test/", &loader);
    vm.eval(r#"
      globalThis.lateBlobResult = null;
      (async () => {
        const OriginalBlob = Blob;
        const type = 'Text/Plain ; Charset="UTF-8"; X-Label="Aa B"';
        let controller;
        const response = new Response(new ReadableStream({ start(value) { controller = value; } }), {
          headers: { 'Content-Type': type }
        });
        const pending = response.blob();
        globalThis.Blob = function() { throw new Error('late author constructor'); };
        try {
          controller.enqueue(new Uint8Array([0, 65, 255]));
          controller.close();
          const blob = await pending;
          if (!(blob instanceof OriginalBlob) || blob.type !== type.toLowerCase())
            throw new Error('Blob identity or MIME normalization');
          if (Array.from(new Uint8Array(await blob.arrayBuffer())).join(',') !== '0,65,255')
            throw new Error('body bytes changed');
          if (!response.bodyUsed) throw new Error('body was not consumed');
          return 'passed';
        } finally { globalThis.Blob = OriginalBlob; }
      })().then(value => lateBlobResult = value, error => lateBlobResult = String(error));
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(lateBlobResult !== null)",
        "true",
        "pending Blob intrinsic constructor",
    )
    .await;
    assert_eq!(vm.eval("lateBlobResult").unwrap(), "passed");
}
