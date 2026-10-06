use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cache_queries_preserve_webidl_conversion_order_and_dom_string_identity() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://cache-query.test/", &loader);
    vm.exec(include_str!("cache_query.js"), None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete===true)",
        "true",
        "cache-query-webidl-fixture",
    )
    .await;
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "85");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cache_dom_string_names_survive_pending_put_body_reactions() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://cache-query.test/", &loader);
    vm.exec(r#"globalThis.result='pending';(async()=>{
      const name='\ud800';const cache=await caches.open(name);
      let controller;const body=new ReadableStream({start(value){controller=value;}});
      const promise=cache.put('/pending',new Response(body));
      controller.enqueue(new Uint8Array([65,0,66]));controller.close();await promise;
      const response=await caches.match('/pending',{cacheName:name});
      const bytes=new Uint8Array(await response.arrayBuffer());
      return bytes.join()==='65,0,66'&&(await caches.keys()).includes(name)&&!(await caches.has('\ufffd'));
    })().then(value=>result=String(value),error=>result='error:'+error);"#, None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "result",
        "true",
        "cache-name-pending-body",
    )
    .await;
}
