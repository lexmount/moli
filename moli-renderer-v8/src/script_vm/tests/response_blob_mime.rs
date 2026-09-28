use super::*;

#[tokio::test(flavor = "current_thread")]
async fn body_blob_mime_uses_the_shared_header_list_parser() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://body-blob-mime.test/", &loader);
    vm.eval(r#"
      globalThis.blobMimeResult = null;
      (async () => {
        const cases = [
          [[], ''],
          [['', 'text/plain'], 'text/plain'],
          [['text/plain', ''], 'text/plain'],
          [['text/plain', 'text/html'], 'text/html'],
          [['TEXT/PLAIN;Charset=GBK', 'text/plain'], 'text/plain;charset=GBK'],
          [['text/html', '*/*'], 'text/html'],
          [['text/html; x="A,b"'], 'text/html;x="A,b"'],
          [['invalid'], ''],
          [['*/*'], '']
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
