use super::*;

#[tokio::test]
async fn lazy_blob_popup_continuation_preserves_referrer_policy() {
    check_popup_blob_continuation_policy(false).await;
}

#[tokio::test]
async fn memory_blob_popup_continuation_preserves_referrer_policy() {
    check_popup_blob_continuation_policy(true).await;
}

async fn check_popup_blob_continuation_policy(in_memory: bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let root_url = format!("http://{address}/opener.html");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                head.push(byte[0]);
                assert!(head.len() < 8192);
            }
            let head = String::from_utf8(head).unwrap();
            let referer = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("referer")
                    .then(|| value.trim().to_owned())
            });
            requests.push((head.lines().next().unwrap().to_owned(), referer));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        }
        requests
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse(&root_url).unwrap(),
        &loader,
    );
    let source = ControlledSource::install(&mut page);
    let read_expression = if in_memory {
        "new Blob(['memory'])"
    } else {
        "controlledBlob.slice(0, 7)"
    };
    page.eval(&format!(
        r#"
        const topPolicy = document.createElement('meta');
        topPolicy.name = 'referrer'; topPolicy.content = 'unsafe-url';
        document.head.appendChild(topPolicy);
        globalThis.popupBlobReadStarted = false;
        globalThis.popupBlobPolicyResult = 'pending';
        globalThis.blobPolicyPopup = window.open('about:blank');
        const popupPolicy = blobPolicyPopup.document.createElement('meta');
        popupPolicy.name = 'referrer'; popupPolicy.content = 'no-referrer';
        blobPolicyPopup.document.head.appendChild(popupPolicy);
        blobPolicyPopup.onmessage = async () => {{
            popupBlobReadStarted = true;
            await ({read_expression}).text();
            const xhr = new XMLHttpRequest();
            xhr.open('GET', 'http://{address}/popup-probe');
            xhr.onload = () => {{ popupBlobPolicyResult = xhr.responseText; }};
            xhr.onerror = () => {{ popupBlobPolicyResult = 'error'; }};
            xhr.send();
        }};
        blobPolicyPopup.postMessage('read', '*');
        'queued';
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(popupBlobReadStarted)",
        "true",
        "popup starts its Blob read",
    )
    .await;
    if !in_memory {
        source.finish(0..7);
    }
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "popupBlobPolicyResult",
        "ok",
        "popup XHR after Blob completion",
    )
    .await;
    page.eval(&format!(
        r#"
        globalThis.openerBlobPolicyResult = 'pending';
        const openerXhr = new XMLHttpRequest();
        openerXhr.open('GET', 'http://{address}/opener-probe');
        openerXhr.onload = () => {{ openerBlobPolicyResult = openerXhr.responseText; }};
        openerXhr.send();
        'queued';
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "openerBlobPolicyResult",
        "ok",
        "opener keeps its own policy after the popup checkpoint",
    )
    .await;
    let requests = server.await.unwrap();
    assert!(requests[0].0.contains("/popup-probe"));
    assert_eq!(
        requests[0].1, None,
        "popup continuation must keep no-referrer"
    );
    assert!(requests[1].0.contains("/opener-probe"));
    assert_eq!(requests[1].1.as_deref(), Some(root_url.as_str()));
}

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
        const check = operation => {
            try { operation(); errors.push('success'); }
            catch (error) { errors.push(error.name); }
        };
        for (const body of [unreadableBlob, file, form]) {
            check(() => new Request('/unused', {method:'POST', body}));
            check(() => new Response(body));
            try { await fetch('/unused', {method:'POST', body}); errors.push('success'); }
            catch (error) { errors.push(error.name); }
            const xhr = new XMLHttpRequest();
            xhr.open('POST', '/unused');
            check(() => xhr.send(body));
            xhr.abort();
        }
        return [errors, coercions];
    "#).await;
    assert_eq!(
        result,
        r#"[["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"],0]"#
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
