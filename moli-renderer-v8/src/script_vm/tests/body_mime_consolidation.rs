use super::*;

#[tokio::test(flavor = "current_thread")]
async fn body_methods_share_mime_extraction_in_windows_and_workers() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://body-mime-consolidation.test/",
        &loader,
    );
    let probe = r#"(async () => {
      for (const C of [Request, Response]) {
        const create = (headers, body = 'a=1') => C === Request
          ? new C('https://body-mime.test/', {method:'POST',body,headers})
          : new C(body, {headers});
        const headers=[['Content-Type','text/plain'],['Content-Type','application/x-www-form-urlencoded; charset=UTF-8']];
        const form=await create(headers).formData();
        if(form.get('a')!=='1')throw Error('duplicate MIME extraction '+C.name);
        const blob=await create(headers).blob();
        if(blob.type!=='application/x-www-form-urlencoded;charset=UTF-8')throw Error('shared Blob extraction '+blob.type);
        const multipart=[['Content-Type','text/plain'],['Content-Type','multipart/form-data; boundary=AaB']];
        const bytes='--AaB\r\nContent-Disposition: form-data; name="field"\r\n\r\nvalue\r\n--AaB--\r\n';
        if((await create(multipart,bytes).formData()).get('field')!=='value')throw Error('multipart boundary case');
        const malformed=[['Content-Type','application/x-www-form-urlencoded'],['Content-Type','not-a-mime']];
        if((await create(malformed).formData()).get('a')!=='1')throw Error('invalid later MIME masks valid entry');
        let caught;try{await create({'Content-Type':'text/plain'}).formData();}catch(error){caught=error;}
        if(!(caught instanceof TypeError))throw Error('unsupported MIME');
      }
      return 'ok';
    })()"#;
    for worker in [false, true] {
        vm.eval("globalThis.bodyMimeConsolidation = null;").unwrap();
        let script = if worker {
            let source =
                format!("({probe}).then(postMessage, error => postMessage(String(error)));");
            format!(
                r#"(() => {{
                const url=URL.createObjectURL(new Blob([{}], {{type:'text/javascript'}}));
                const worker=new Worker(url);
                const finish=value=>{{bodyMimeConsolidation=value;worker.terminate();URL.revokeObjectURL(url);}};
                worker.onmessage=event=>finish(event.data);
                worker.onerror=event=>{{finish(event.message);event.preventDefault();}};
            }})()"#,
                serde_json::to_string(&source).unwrap()
            )
        } else {
            format!(
                "({probe}).then(value => bodyMimeConsolidation=value, error=>bodyMimeConsolidation=String(error));"
            )
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(bodyMimeConsolidation !== null)",
            "true",
            "shared Body MIME",
        )
        .await;
        assert_eq!(
            vm.eval("bodyMimeConsolidation").unwrap(),
            "ok",
            "worker={worker}"
        );
    }
}
