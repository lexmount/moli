use super::*;

fn response_flags(vm: &mut ScriptVm) -> Vec<bool> {
    let context = &vm.page_default_runtime.context as *const _;
    let mut flags = Vec::new();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "samples");
        let values =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        for index in 0..values.length() {
            let value = values.get_index(scope, index).unwrap();
            let (head, _) =
                crate::network_host::materialize_cache_response_object_head(scope, value).unwrap();
            flags.push(head.request_include_credentials);
        }
        Ok(())
    })
    .unwrap();
    flags
}

#[tokio::test(flavor = "current_thread")]
async fn cache_response_credentials_survive_opaque_clones_and_repeated_put() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-response-credentials.test/",
        &loader,
    );
    let context = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let mut values = Vec::new();
        for flag in [true, false] {
            let head = moli_fetch::ResponseHead {
                final_url: url::Url::parse("https://opaque.test/body").unwrap(),
                status: 200,
                status_text: None,
                headers: Vec::new(),
                request_cookie_report: None,
                request_include_credentials: flag,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
                negotiated_http_version: None,
            };
            let response =
                crate::network_host::build_fetch_response_object_for_request_mode_with_filter(
                    scope,
                    moli_url::WebOrigin::from_url(
                        &url::Url::parse("https://cache-response-credentials.test/").unwrap(),
                    ),
                    crate::network_host::FetchResponseRequest {
                        method: "GET",
                        mode: moli_fetch::RequestMode::NoCors,
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    },
                    moli_fetch::Response::from_head_and_text_body(head, "body".to_owned()),
                    None,
                );
            values.push(response.into());
        }
        let inputs = v8::Array::new_with_elements(scope, &values);
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "inputs");
        global.create_data_property(scope, key.into(), inputs.into());
        Ok(())
    })
    .unwrap();
    vm.exec(
        r#"globalThis.samples=[];globalThis.done=false;
      (async()=>{
        const cache=await caches.open('credentials');
        for(let index=0;index<inputs.length;index++){
          const original=inputs[index];
          if(original.type!=='opaque')throw new Error('expected opaque');
          samples.push(original,original.clone());
          await cache.put('entry'+index,original.clone());
          const cached=await cache.match('entry'+index);
          samples.push(cached,cached.clone());
          await cache.put('copy'+index,cached.clone());
          samples.push(await caches.match('copy'+index));
        }
        done=true;
      })().catch(error=>done='error:'+error);"#,
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "cache-response-credentials",
    )
    .await;
    assert_eq!(
        response_flags(&mut vm),
        [vec![true; 5], vec![false; 5]].concat()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cache_response_credentials_are_snapshotted_before_pending_body_completion() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-pending-credentials.test/",
        &loader,
    );
    vm.exec("globalThis.slow=new Response(new ReadableStream({start(controller){globalThis.writer=controller;}}));",None).unwrap();
    let context = &vm.page_default_runtime.context as *const _;
    let set_flag = |vm: &mut ScriptVm, flag| {
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "slow");
            let response =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            crate::network_host::set_response_slot_bool(
                scope,
                response,
                crate::network_host::RESPONSE_REQUEST_INCLUDE_CREDENTIALS_SLOT,
                flag,
            );
            Ok(())
        })
        .unwrap();
    };
    set_flag(&mut vm, true);
    vm.exec("globalThis.done=false;(async()=>{const cache=await caches.open('credentials');await cache.put('pending',slow);globalThis.samples=[await cache.match('pending')];done=true;})().catch(error=>done='error:'+error);",None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(slow.body.locked)",
        "true",
        "cache-pending-credentials-locked",
    )
    .await;
    assert_eq!(vm.eval("done").unwrap(), "false");
    // The head belongs to the accepted Cache.put operation. A later native
    // change to the source Response must not replace that captured state.
    set_flag(&mut vm, false);
    vm.exec(
        "writer.enqueue(new Uint8Array([1,2,3]));writer.close();",
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "cache-pending-credentials-complete",
    )
    .await;
    assert_eq!(response_flags(&mut vm), [true]);
}
