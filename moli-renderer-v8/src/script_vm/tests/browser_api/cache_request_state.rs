use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cache_request_metadata_respects_quota_without_charging_implicit_defaults() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-metadata-quota.test/",
        &loader,
    );
    vm.exec(
        r#"globalThis.result='pending';(async()=>{
          const bucket=await navigator.storageBuckets.open('metadata-quota',{quota:128});
          const cache=await bucket.caches.open('entries');
          await cache.put('small',new Response('ok'));
          const before=await bucket.estimate();
          const request=new Request('large',{integrity:'sha256-'+'a'.repeat(128)});
          let rejected=false;try{await cache.put(request,new Response('ok'));}
          catch(error){rejected=error instanceof DOMException&&error.name==='QuotaExceededError';}
          const after=await bucket.estimate();
          return before.usage>0&&before.quota===128&&rejected&&after.usage===before.usage&&
            (await cache.keys()).length===1&&(await cache.match('large'))===undefined&&
            await(await cache.match('small')).text()==='ok';
        })().then(value=>result=String(value),error=>result='error:'+error);"#,
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "result",
        "true",
        "cache-metadata-quota",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn cache_preserves_request_metadata_and_put_owner_realm() {
    let server = super::cache_add::batch_server().await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", server.origin), &loader);
    vm.exec(include_str!("cache_request_state.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete===true)",
        "true",
        "cache-request-state-frozen-fixture",
    )
    .await;
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "151");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cache_keys_preserve_native_navigation_metadata_and_internal_headers() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-navigation.test/",
        &loader,
    );
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "Request").unwrap();
        let request = moli_storage_service::StorageBucketCachedRequest {
            method: "GET".to_owned(),
            headers: vec![
                ("cookie".to_owned(), "session=original".to_owned()),
                ("host".to_owned(), "native.test".to_owned()),
            ],
            metadata: moli_storage_service::StorageBucketCachedRequestMetadata {
                destination: "document".to_owned(),
                mode: "navigate".to_owned(),
                referrer: "https://other.test/source".to_owned(),
                referrer_policy: "strict-origin".to_owned(),
                credentials: "include".to_owned(),
                priority: "high".to_owned(),
                is_history_navigation: true,
                is_reload_navigation: true,
                ..Default::default()
            },
        };
        let native = crate::network_host::build_cached_request_object(
            scope,
            "https://cache-navigation.test/native",
            &request,
        )
        .unwrap();
        assert_eq!(
            crate::network_host::cached_request_from_native(scope, native),
            request
        );
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nativeRequest");
        assert_eq!(
            global.create_data_property(scope, key.into(), native.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.exec(r#"globalThis.result='pending';(async()=>{
      const cache=await caches.open('navigation-state');await cache.put(nativeRequest,new Response('navigation'));
      const [request]=await cache.keys();globalThis.storedRequest=request;let immutable=false;try{request.headers.set('x-state','changed');}catch(error){immutable=error instanceof TypeError;}
      return request instanceof Request&&request.mode==='navigate'&&request.destination==='document'&&
        request.referrer==='https://other.test/source'&&request.referrerPolicy==='strict-origin'&&
        request.credentials==='include'&&request.isHistoryNavigation&&request.isReloadNavigation&&
        request.headers.get('cookie')==='session=original'&&request.headers.get('host')==='native.test'&&immutable&&
        request.signal instanceof AbortSignal&&!request.signal.aborted&&request.signal!==nativeRequest.signal&&
        await(await cache.match(request)).text()==='navigation';
    })().then(value=>result=String(value),error=>result='error:'+error);"#,None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "result",
        "true",
        "cache-navigation-state",
    )
    .await;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let source_key = crate::util::v8str(scope, "nativeRequest");
        let source =
            v8::Local::<v8::Object>::try_from(global.get(scope, source_key.into()).unwrap())
                .unwrap();
        let stored_key = crate::util::v8str(scope, "storedRequest");
        let stored =
            v8::Local::<v8::Object>::try_from(global.get(scope, stored_key.into()).unwrap())
                .unwrap();
        let original = crate::network_host::cached_request_from_native(scope, source);
        let restored = crate::network_host::cached_request_from_native(scope, stored);
        assert_eq!(restored, original);
        assert_eq!(restored.metadata.priority, "high");
        Ok(())
    })
    .unwrap();
}
