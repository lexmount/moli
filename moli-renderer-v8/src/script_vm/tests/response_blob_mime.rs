use super::*;

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
async fn body_blob_type_normalizes_pending_streams_without_author_constructors() {
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
          const stream = new ReadableStream({start(value) { controller = value; }});
          const options = {headers: {'Content-Type': type}};
          const body = kind === 'Request'
            ? new Request('https://pending-blob-mime.test/', {...options,method:'POST',body:stream,duplex:'half'})
            : new Response(stream, options);
          body.headers.get = () => { throw new Error('author Headers.get'); };
          body.headers.entries = () => { throw new Error('author Headers.entries'); };
          const pending = body.blob();
          globalThis.Blob = function() { throw new Error('author Blob constructor'); };
          try {
            controller.enqueue(new Uint8Array([0,65,255]));
            controller.close();
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
        "pending Body Blob MIME",
    )
    .await;
    assert_eq!(vm.eval("pendingBlobMimeResult").unwrap(), "passed");
}
