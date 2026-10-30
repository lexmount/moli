use super::*;

#[derive(Debug)]
struct ImmediateSource {
    fail: bool,
}

impl BlobSource for ImmediateSource {
    fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
        complete(if self.fail {
            Err(io::Error::other("injected Blob source failure"))
        } else {
            Ok(b"payload"[range].to_vec())
        });
    }
}

fn install_body_blob(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    name: &'static str,
    fail: bool,
) {
    page.with_default_context_scope(|scope, _| {
        let data = BlobData::from_source(Arc::new(ImmediateSource { fail }), 7);
        let object =
            crate::blob::build_blob_object_from_data(scope, data, "text/x-test".into()).unwrap();
        let name = crate::util::v8str(scope, name);
        assert_eq!(
            scope
                .get_current_context()
                .global(scope)
                .set(scope, name.into(), object.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn constructed_lazy_blob_bodies_wait_for_consumption_and_deliver_io_results() {
    for fail in [false, true] {
        let manager = manager(None);
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut page = page(&manager, &loader);
        let source = ControlledSource::install(&mut page);
        assert_eq!(
            page.eval(
                r#"(() => {
                const blob = controlledBlob.slice(100,108,'text/x-control');
                globalThis.constructedBodies = [
                    new Request('/unused', {method:'POST', body:blob}), new Response(blob)
                ];
                return constructedBodies.map(body =>
                    body.headers.get('content-type') + ':' + body.bodyUsed).join('|');
            })()"#
            )
            .unwrap(),
            "text/x-control:false|text/x-control:false",
        );
        assert!(
            source.pending.lock().is_empty(),
            "constructing a Blob body must not start or block on source I/O"
        );
        page.eval(r#"
            globalThis.bodyReads = ['pending','pending'];
            constructedBodies.forEach((body,index) => body.text().then(
                text => bodyReads[index] = 'text:' + text,
                error => bodyReads[index] = error instanceof DOMException ? error.name : 'wrong error brand'
            ));
        "#).unwrap();
        let pending = std::mem::take(&mut *source.pending.lock());
        assert_eq!(
            pending.len(),
            2,
            "both bodies must read their native Blob source"
        );
        for (range, complete) in pending {
            assert_eq!(range, 100..108);
            complete(if fail {
                Err(io::Error::other("controlled body source failure"))
            } else {
                Ok(source.bytes[range].to_vec())
            });
        }
        assert_eq!(
            page.eval("bodyReads.join('|')").unwrap(),
            "pending|pending",
            "source completion cannot settle public Body promises inline"
        );
        advance_page_task_executor_until_eval_equals(
            &mut page,
            &loader,
            "bodyReads.join('|')",
            if fail {
                "NotReadableError|NotReadableError"
            } else {
                "text:defghijk|text:defghijk"
            },
            "controlled native Blob body consumption",
        )
        .await;
        drop(page);
        manager.lock().shutdown_sqlite().unwrap();
    }
}

#[tokio::test]
async fn unreadable_blob_and_multipart_bodies_fail_without_string_coercion() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    install_body_blob(&mut page, "unreadableBlob", true);
    let result = run(&mut page, &loader, r#"
        let coercions = 0;
        const coerce = () => { ++coercions; throw new RangeError('must not stringify unreadable Blob'); };
        Blob.prototype.toString = coerce;
        File.prototype.toString = coerce;
        FormData.prototype.toString = coerce;
        const file = new File([unreadableBlob], 'bad.bin');
        const form = new FormData();
        form.append('good', 'prefix');
        form.append('bad', file);
        const errors = [];
        const check = async operation => {
            try { await operation(); errors.push('success'); }
            catch (error) { errors.push(error.name); }
        };
        for (const body of [unreadableBlob, file, form]) {
            await check(() => new Request('/unused', {method:'POST', body}).text());
            await check(() => new Response(body).text());
            try { await fetch('/unused', {method:'POST', body}); errors.push('success'); }
            catch (error) { errors.push(error.name); }
            const xhr = new XMLHttpRequest();
            xhr.open('POST', '/unused');
            await check(() => xhr.send(body));
            xhr.abort();
        }
        return [errors, coercions];
    "#).await;
    assert_eq!(
        result,
        r#"[["NotReadableError","NotReadableError","TypeError","TypeError","NotReadableError","NotReadableError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"],0]"#
    );
}

#[tokio::test]
async fn readable_source_bodies_preserve_bytes_mime_and_multipart_file_fields() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    install_body_blob(&mut page, "readableBlob", false);
    let result = run(
        &mut page,
        &loader,
        r#"
        const request = new Request('/unused', {method:'POST', body:readableBlob});
        const response = new Response(readableBlob);
        const form = new FormData();
        form.append('greeting', 'hello');
        form.append('upload', readableBlob, 'kept.bin');
        const multipart = new Response(form);
        const text = await multipart.text();
        return [await request.text(), request.headers.get('content-type'),
            await response.text(), response.headers.get('content-type'),
            multipart.headers.get('content-type').startsWith('multipart/form-data; boundary='),
            text.includes('name="greeting"\r\n\r\nhello'),
            text.includes('name="upload"; filename="kept.bin"'),
            text.includes('Content-Type: text/x-test'), text.includes('\r\n\r\npayload\r\n')];
    "#,
    )
    .await;
    assert_eq!(
        result,
        r#"["payload","text/x-test","payload","text/x-test",true,true,true,true,true]"#
    );
    page.with_default_context_scope(|scope, _| {
        let name = crate::util::v8str(scope, "readableBlob");
        let blob = scope
            .get_current_context()
            .global(scope)
            .get(scope, name.into())
            .unwrap();
        let prepared = crate::network_host::prepare_xhr_send_body(scope, blob).unwrap();
        assert_eq!(prepared.body.as_deref(), Some(b"payload".as_slice()));
        assert_eq!(
            prepared.default_content_type.as_deref(),
            Some("text/x-test")
        );
        Ok(())
    })
    .unwrap();
}
