use super::*;
use crate::util::v8str;

#[test]
fn xml_http_request_response_document_uses_response_url_and_requester_origin() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    for child_realm in [false, true] {
        for (mime, response_type) in [
            ("application/xml", ""),
            ("application/xml", "document"),
            ("text/html", "document"),
        ] {
            let mut vm = new_parsed_test_vm(
                "https://requester.example/page/index.html",
                "<!doctype html><html><body></body></html>",
            );
            environment.set_timezone(Some("UTC")).unwrap();
            vm.eval(&format!(
                r#"(() => {{
                    const frame = document.createElement('iframe');
                    document.body.appendChild(frame);
                    globalThis.__originRealm = {child_realm} ? frame.contentWindow : self;
                    globalThis.__documentXhr = new __originRealm.XMLHttpRequest();
                    __documentXhr.open('GET', '/initial');
                    __documentXhr.responseType = '{response_type}';
                }})()"#
            ))
            .expect("create XHR in its owning realm");
            let context_ptr = &vm.current_main_world().unwrap().context as *const _;
            vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, move |scope, _| {
                let context = scope.get_current_context();
                let xhr = context.global(scope).get(scope, v8str(scope, "__documentXhr").into())
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()).unwrap();
                crate::network_host::apply_xhr_response_body_source(
                    scope,
                    xhr,
                    moli_fetch::ResponseHead {
                        status_text: None,
                        final_url: Url::parse("https://response.example/resource/doc#fragment").unwrap(),
                        status: 200,
                        headers: vec![
                            ("Content-Type".to_owned(), mime.as_bytes().to_vec()),
                            ("lAsT-mOdIfIeD".to_owned(), b"Thu, 01 Jan 1970 01:23:45 GMT".to_vec()),
                        ],
                        request_cookie_report: None,

                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        cache_state: Default::default(),

                        negotiated_http_version: None,
                    },
                    moli_fetch::ResponseBody::materialized_bytes(br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><base href="../assets/" /></head><body><a id="link" href="child">link</a></body></html>"#.to_vec()),
                );
                Ok(())
            }).expect("deliver response from a different origin in the parent realm");
            let result = vm
                .eval(
                    r#"(() => {
                const doc = __documentXhr.responseXML;
                const check = (value, message) => { if (!value) throw new Error(message); };
                const url = 'https://response.example/resource/doc#fragment';
                check(doc instanceof __originRealm.Document, 'XHR creation realm');
                check(doc.URL === url && doc.documentURI === url, 'response URL metadata');
                check(__documentXhr.responseURL === url.split('#')[0], 'responseURL serialization');
                check(doc.domain === 'requester.example', 'origin belongs to requester');
                check(doc.defaultView === null && doc.hidden, 'windowless response document');
                check(doc.lastModified === '01/01/1970 01:23:45', 'response source modification time');
                const getter = Object.getOwnPropertyDescriptor(Document.prototype, 'lastModified').get;
                check(getter.call(doc) === doc.lastModified, 'cross-realm metadata getter');
                check(doc.baseURI === 'https://response.example/assets/', 'relative parsed base');
                const link = doc.getElementById('link');
                check(link.href === 'https://response.example/assets/child', 'relative link');
                doc.querySelector('base').remove();
                check(doc.baseURI === url && link.baseURI === url, 'base removal');
                const base = doc.createElementNS('http://www.w3.org/1999/xhtml', 'base');
                base.href = '../changed/';
                doc.documentElement.appendChild(base);
                check(doc.baseURI === 'https://response.example/changed/', 'base insertion');
                document.head.appendChild(base);
                check(doc.baseURI === url, 'base adoption');
                __documentXhr.open('GET', '/next');
                check(__documentXhr.responseXML === null && doc.URL === url, 'XHR reuse');
                check(doc.lastModified === '01/01/1970 01:23:45', 'source time survives XHR reuse');
                return 'ok';
            })()"#,
                )
                .expect("response document metadata should remain coherent");
            assert_eq!(
                result, "ok",
                "child_realm={child_realm}, mime={mime}, response_type={response_type}"
            );
        }
    }
}

#[test]
fn xhr_streamed_response_documents_keep_distinct_source_modification_times() {
    let environment = moli_v8_platform::ProcessEnvironmentOwner::default();
    for (mime, response_type) in [
        ("application/xml", ""),
        ("application/xml", "document"),
        ("text/html", "document"),
    ] {
        let mut vm = new_storage_test_vm("https://xhr-document-modified.test/");
        vm.document_runtime
            .set_document_source_last_modified(Some(5_025_000.0));
        environment.set_timezone(Some("UTC")).unwrap();
        vm.set_fetch_subresource_interception(
            crate::types::SubresourceInterceptionClasses::for_resource_type(
                crate::types::SubresourceResourceType::Xhr,
            ),
        );
        vm.eval(
            r#"
            globalThis.__modifiedXhr = new XMLHttpRequest();
            globalThis.__responseDocuments = [];
            __modifiedXhr.onload = () => __responseDocuments.push(__modifiedXhr.responseXML);
        "#,
        )
        .unwrap();
        for (header, expected) in [
            (
                Some("Sun, 06 Nov 1994 08:49:37 GMT"),
                Some("11/06/1994 08:49:37"),
            ),
            (
                Some("Thu, 01 Jan 1970 00:00:00 GMT"),
                Some("01/01/1970 00:00:00"),
            ),
            (Some("not a date"), None),
            (None, None),
        ] {
            vm.eval(&format!(
                r#"
                __modifiedXhr.open('GET', '/response');
                __modifiedXhr.responseType = '{response_type}';
                __modifiedXhr.send();
            "#
            ))
            .unwrap();
            let pending = vm.take_pending_subresource_fetch_infos();
            assert_eq!(pending.len(), 1);
            let request = &pending[0];
            let mut headers = vec![("Content-Type".to_owned(), mime.as_bytes().to_vec())];
            if let Some(header) = header {
                headers.push(("lAsT-mOdIfIeD".to_owned(), header.as_bytes().to_vec()));
            }
            let body_source_id = crate::network_host::new_network_body_source_id();
            vm.start_streaming_async_subresource_fetch(
                crate::types::AsyncSubresourceStreamingStarted {
                    internal_id: request.internal_id,
                    request_url: request.url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Default::default(),
                    request_body: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    body_source_id,
                    head: moli_fetch::ResponseHead {
                        final_url: request.url.clone(),
                        status: 200,
                        status_text: None,
                        headers,
                        request_cookie_report: None,

                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        cache_state: Default::default(),

                        negotiated_http_version: None,
                    },
                    network_request_headers: None,
                },
            )
            .unwrap();
            for chunk in [
                b"<html><body>".as_slice(),
                b"response</body></html>".as_slice(),
            ] {
                vm.append_streaming_async_subresource_fetch_chunk(body_source_id, chunk.to_vec());
                assert_eq!(
                    vm.eval("__modifiedXhr.responseXML === null").unwrap(),
                    "true"
                );
            }
            vm.finish_streaming_async_subresource_fetch(
                request.internal_id,
                body_source_id,
                Ok(()),
            )
            .unwrap();
            if let Some(expected) = expected {
                assert_eq!(
                    vm.eval("__modifiedXhr.responseXML.lastModified").unwrap(),
                    expected
                );
            } else {
                assert_eq!(vm.eval(r#"(() => {
                    const doc = __modifiedXhr.responseXML;
                    const match = /^(\d\d)\/(\d\d)\/(\d{4}) (\d\d):(\d\d):(\d\d)$/.exec(doc.lastModified);
                    const time = Date.UTC(+match[3], +match[1] - 1, +match[2], +match[4], +match[5], +match[6]);
                    return Math.abs(Date.now() - time) < 5000;
                })()"#).unwrap(), "true", "mime={mime}, response_type={response_type}, header={header:?}");
            }
        }
        assert_eq!(
            vm.eval(
                r#"(() => {
            const [first, second, third, fourth] = __responseDocuments;
            __modifiedXhr.abort();
            first.documentElement.remove();
            return JSON.stringify([
                first.lastModified, second.lastModified,
                document.lastModified, first !== second && third !== fourth
            ]);
        })()"#
            )
            .unwrap(),
            r#"["11/06/1994 08:49:37","01/01/1970 00:00:00","01/01/1970 01:23:45",true]"#
        );
        environment.set_timezone(Some("Asia/Shanghai")).unwrap();
        assert_eq!(
            vm.eval("__responseDocuments[0].lastModified").unwrap(),
            "11/06/1994 16:49:37"
        );
    }
}
