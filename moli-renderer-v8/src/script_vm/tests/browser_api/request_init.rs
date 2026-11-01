use super::*;

#[test]
fn request_constructor_resets_inherited_navigation_state_only_for_defined_members() {
    let mut vm = new_storage_html_test_vm("https://request-init-navigation.test/");
    vm.eval("globalThis.nativeRequest = new Request('https://request-init-navigation.test/target', {headers: {'x-native':'original'}, referrer:'https://request-init-navigation.test/source', referrerPolicy:'strict-origin'});").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeRequest");
        let value = global.get(scope, key.into()).unwrap();
        let request = v8::Local::<v8::Object>::try_from(value).unwrap();
        crate::network_host::set_request_mode_for_service_worker_fetch_event(
            scope, request, "navigate",
        );
        crate::network_host::set_request_reload_navigation_for_service_worker_fetch_event(
            scope, request, true,
        );
        let history = v8::Boolean::new(scope, true);
        crate::util::set_private_value(
            scope,
            request,
            "__lmRequestIsHistoryNavigation",
            history.into(),
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/request-init-navigation.js"
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn window_request_like_fetch_validates_inherited_native_signal_after_init() {
    let mut vm = new_storage_html_test_vm("https://request-like-signal.test/");
    let source = format!(
        "globalThis.requestLikeSignalResult='pending'; ({}).then(value=>requestLikeSignalResult=value,error=>requestLikeSignalResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/request-like-signal.js")
    );
    vm.eval(&source).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("requestLikeSignalResult").unwrap(), "true");
}

#[test]
fn request_and_fetch_share_complete_dictionary_conversion_before_materialization() {
    let mut vm = new_storage_html_test_vm("https://request-init-dictionary.test/");
    let source = format!(
        "globalThis.requestInitResult='pending'; ({}).then(value=>requestInitResult=value,error=>requestInitResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/request-init-dictionary.js")
    );
    vm.eval(&source).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("requestInitResult").unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_null_body_init_preserves_and_consumes_the_inherited_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        let head = String::from_utf8(head).unwrap();
        let length: usize = head
            .split("\r\n")
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
        body
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader(&format!("http://{address}/"), &loader);
    vm.eval("globalThis.nullBodyResult = null; globalThis.bodySource = new Request('/post',{method:'POST',body:'inherited payload'}); fetch(bodySource,{body:null}).then(async response=>nullBodyResult=await response.text(),error=>nullBodyResult=String(error));").unwrap();
    assert_eq!(vm.eval("String(bodySource.bodyUsed)").unwrap(), "true");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(nullBodyResult !== null)",
        "true",
        "Fetch with null body override",
    )
    .await;
    assert_eq!(vm.eval("nullBodyResult").unwrap(), "inherited payload");
    assert_eq!(server.await.unwrap(), b"inherited payload");
}
