use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cache_put_preserves_stream_errors_without_stringifying_author_reasons() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://cache-error.test/", &loader);
    vm.exec(r#"globalThis.result='pending';
      (async()=>{
        const cache=await caches.open('stream-errors');let reads=0;
        const reason={toString(){reads++;throw 42;}};
        const errored=new ReadableStream({start(controller){controller.error(reason);}});
        let immediate;try{await cache.put('/immediate',new Response(errored));}catch(error){immediate=error;}
        let controller;const pending=new ReadableStream({start(value){controller=value;}});
        const put=cache.put('/pending',new Response(pending));controller.error(reason);
        let asynchronous;try{await put;}catch(error){asynchronous=error;}
        return immediate===reason&&asynchronous===reason&&reads===0&&(await cache.keys()).length===0;
      })().then(value=>result=String(value),error=>result='error:'+error);
    "#,None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "result",
        "true",
        "cache-put-stream-error-identity",
    )
    .await;
}
