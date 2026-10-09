use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct BatchServer {
    pub(super) origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for BatchServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn batch_server() -> BatchServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            connections.spawn(async move {
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 2048];
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 { return; }
                    request.extend_from_slice(&buffer[..count]);
                }
                let head = String::from_utf8(request).unwrap();
                let target = head.split_whitespace().nth(1).unwrap();
                let url = url::Url::parse(&format!("http://fixture.test{target}")).unwrap();
                let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
                let status = query.get("status").map(|value| value.parse::<u16>().unwrap()).unwrap_or(200);
                let body = if status == 204 { "" } else {
                    head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("x-shape").then(|| value.trim())
                    }).unwrap_or("fixture")
                };
                let vary = query.get("vary").map(|value| format!("Vary: {value}\r\n")).unwrap_or_default();
                let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\n{vary}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                // Synchronously aborted requests may close before the reply.
                let _ = socket.write_all(response.as_bytes()).await;
            });
            while let Some(result) = connections.try_join_next() {
                result.unwrap();
            }
        }
    });
    BatchServer { origin, task }
}

#[tokio::test(flavor = "current_thread")]
async fn cache_add_native_fetch_batches_preserve_webidl_and_atomic_vary_semantics() {
    let server = batch_server().await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", server.origin), &loader);
    vm.exec(include_str!("cache_add.js"), None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete===true)",
        "true",
        "cache-add-frozen-fixture",
    )
    .await;
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "66");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}

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

#[tokio::test(flavor = "current_thread")]
async fn cache_add_accepts_registered_native_proxy_receivers_with_callee_realm_rejections() {
    let server = batch_server().await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{}/", server.origin), &loader);
    vm.exec("globalThis.cache=null;caches.open('native-proxy').then(value=>cache=value);document.body.innerHTML='<iframe></iframe>';globalThis.other=document.querySelector('iframe').contentWindow;", None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(cache!==null)",
        "true",
        "cache-proxy-open",
    )
    .await;
    vm.exec("globalThis.otherCache=null;other.caches.open('native-proxy').then(value=>otherCache=value);",None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(otherCache!==null)",
        "true",
        "cache-child-open",
    )
    .await;
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "cache");
        let object =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, object, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.exec(r#"globalThis.result='pending';(async()=>{
      for(const [owner,receiver] of [[window,nativeProxy],[other,otherCache]])for(const realm of [window,other]){
        const empty=realm.Cache.prototype.addAll.call(receiver,[]);
        if(!(empty instanceof owner.Promise)||await empty!==undefined)throw Error('native empty batch');
        for(const member of ['add','addAll']){
          for(const input of ['http://[','data:text/plain,x']){
            let error;const promise=realm.Cache.prototype[member].call(receiver,member==='add'?input:[input]);
            try{await promise;}catch(caught){error=caught;}
            if(!(promise instanceof realm.Promise)||Object.getPrototypeOf(error)!==realm.TypeError.prototype)throw Error('callee Request construction');
          }
          const reason={identity:true};let error;
          const item={toString(){throw reason;}};
          try{await realm.Cache.prototype[member].call(receiver,member==='add'?item:[item]);}catch(caught){error=caught;}
          if(error!==reason)throw Error('native proxy conversion');
          let traps=0,conversions=0;const wrapped=new Proxy(receiver,{get(){traps++;throw 43;},getPrototypeOf(){traps++;throw 44;}});
          const value={toString(){conversions++;throw 45;}};
          const promise=realm.Cache.prototype[member].call(wrapped,member==='add'?value:[value]);
          try{await promise;}catch(caught){error=caught;}
          if(!(promise instanceof realm.Promise)||Object.getPrototypeOf(error)!==realm.TypeError.prototype||traps||conversions)throw Error('callee binding');
        }
        const url=new URL('/batch-resource',location.href).href;
        for(const kind of ['status','duplicate']){
          const promise=kind==='status'?realm.Cache.prototype.add.call(receiver,url+'?status=404'):realm.Cache.prototype.addAll.call(receiver,[url,url]);
          let error;try{await promise;}catch(caught){error=caught;}
          const prototype=kind==='status'?owner.TypeError.prototype:owner.DOMException.prototype;
          if(!(promise instanceof owner.Promise)||Object.getPrototypeOf(error)!==prototype||kind==='duplicate'&&error.name!=='InvalidStateError')throw Error('owner cache job realm');
        }
      }return true;
    })().then(value=>result=String(value),error=>result='error:'+error);"#,None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "result",
        "true",
        "cache-native-proxy-and-callee-realm",
    )
    .await;
}
