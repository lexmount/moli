use super::*;
use crate::cross_origin_isolation::{
    CrossOriginEmbedderPolicy as Coep, DocumentIsolationPolicy as Dip,
};

fn install_responses(vm: &mut ScriptVm, origin: &str) {
    let context = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let mut values = Vec::new();
        for (name, credentials, redirect, corp) in [
            ("included", true, false, false),
            ("omitted", false, false, false),
            ("corp", true, false, true),
            ("redirect", true, true, false),
        ] {
            let mut headers = Vec::new();
            if corp {
                headers.push((
                    "Cross-Origin-Resource-Policy".to_owned(),
                    b"cross-origin".to_vec(),
                ));
            }
            if redirect {
                headers.push((
                    "Location".to_owned(),
                    b"https://opaque.test/target".to_vec(),
                ));
            }
            let head = moli_fetch::ResponseHead {
                final_url: url::Url::parse(&format!("https://opaque.test/{name}")).unwrap(),
                status: if redirect { 302 } else { 200 },
                status_text: None,
                headers,
                request_cookie_report: None,
                request_include_credentials: credentials,
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
                    moli_url::WebOrigin::from_url(&url::Url::parse(origin).unwrap()),
                    crate::network_host::FetchResponseRequest {
                        method: "GET",
                        mode: moli_fetch::RequestMode::NoCors,
                        redirect_mode: if redirect {
                            moli_fetch::RequestRedirectMode::Manual
                        } else {
                            moli_fetch::RequestRedirectMode::Follow
                        },
                    },
                    moli_fetch::Response::from_head_and_text_body(head, "body".to_owned()),
                    None,
                );
            values.push(response.into());
        }
        let values = v8::Array::new_with_elements(scope, &values);
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "inputs");
        global.create_data_property(scope, key.into(), values.into());
        Ok(())
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn cache_read_policy_preserves_credentials_and_reports_only_coep_blocks() {
    for (name, coep, dip, report_only, blocked, enforce_reports, reporting_reports) in [
        (
            "none",
            Coep::None,
            Dip::None,
            Coep::None,
            [false, false],
            0,
            0,
        ),
        (
            "require",
            Coep::RequireCorp,
            Dip::None,
            Coep::None,
            [true, true],
            6,
            0,
        ),
        (
            "credentialless",
            Coep::Credentialless,
            Dip::None,
            Coep::None,
            [true, false],
            3,
            0,
        ),
        (
            "dip-require",
            Coep::None,
            Dip::IsolateAndRequireCorp,
            Coep::None,
            [true, true],
            0,
            0,
        ),
        (
            "dip-credentialless",
            Coep::None,
            Dip::IsolateAndCredentialless,
            Coep::None,
            [true, false],
            0,
            0,
        ),
        (
            "report-only",
            Coep::None,
            Dip::None,
            Coep::RequireCorp,
            [false, false],
            0,
            6,
        ),
        (
            "both",
            Coep::RequireCorp,
            Dip::None,
            Coep::RequireCorp,
            [true, true],
            6,
            6,
        ),
    ] {
        let loader = static_http_loader(std::iter::empty::<String>());
        let url = format!("https://cache-policy.test/{name}");
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(&url, &loader);
        vm.set_main_navigation_policy_container(crate::document_runtime::DocumentPolicyContainer {
            cross_origin_embedder_policy: coep,
            cross_origin_embedder_policy_report_only: report_only,
            document_isolation_policy: dip,
            ..Default::default()
        });
        install_responses(&mut vm, &url);
        vm.exec(&format!(r#"
          globalThis.reports=[];globalThis.done=false;globalThis.results=[];
          globalThis.observer=new ReportingObserver(list=>reports.push(...list.map(report=>report.toJSON())),{{types:['coep']}});
          observer.observe();
          (async()=>{{
            const cache=await caches.open('policy');
            const blocked={blocked};
            const entries=[...inputs,new Response('basic')];
            const types=['opaque','opaque','opaque','opaqueredirect','default'];
            for(let index=0;index<entries.length;index++){{
              const key=new URL('entry-'+index,location.href).href;
              await cache.put(key,entries[index]);
              for(const [api,read] of [
                ['match',()=>cache.match(key)],
                ['matchAll',async()=>(await cache.matchAll(key))[0]],
                ['storage',()=>caches.match(key,{{cacheName:'policy'}})],
              ]){{
                const shouldBlock=index<2&&blocked[index];
                try{{
                  const response=await read();
                  if(shouldBlock||response.type!==types[index])throw new Error('unexpected response '+index+' '+api);
                }}catch(error){{
                  if(!shouldBlock||!(error instanceof TypeError))throw error;
                }}
                results.push([index,api]);
              }}
            }}
            done=true;
          }})().catch(error=>done='error:'+error);
        "#, blocked=serde_json::to_string(&blocked).unwrap()),None).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(done)",
            "true",
            name,
        )
        .await;
        let count = enforce_reports + reporting_reports;
        if count != 0 {
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(reports.length)",
                &count.to_string(),
                name,
            )
            .await;
        }
        let reports: Vec<serde_json::Value> =
            serde_json::from_str(&vm.eval("JSON.stringify(reports)").unwrap()).unwrap();
        assert_eq!(vm.eval("results.length").unwrap(), "15", "{name}");
        assert_eq!(reports.len(), count, "{name}: {reports:?}");
        assert_eq!(
            reports
                .iter()
                .filter(|report| report["body"]["disposition"] == "enforce")
                .count(),
            enforce_reports,
            "{name}"
        );
        assert_eq!(
            reports
                .iter()
                .filter(|report| report["body"]["disposition"] == "reporting")
                .count(),
            reporting_reports,
            "{name}"
        );
        for report in reports {
            assert_eq!(report["type"], "coep", "{name}");
            assert_eq!(report["url"], url, "{name}");
            assert_eq!(report["body"]["type"], "corp", "{name}");
            assert_eq!(report["body"]["destination"], "", "{name}");
            assert!(
                matches!(
                    report["body"]["blockedURL"].as_str(),
                    Some("https://opaque.test/included" | "https://opaque.test/omitted")
                ),
                "{name}: {report}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn borrowed_cache_reads_use_owner_policy_reports_and_error_realm_after_removal() {
    let server = StaticHttpServer::spawn_with_raw_responder(1, |_, _| {
        let body="<!doctype html><body><script>parent.postMessage('cache-child-ready','*')</script>";
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCross-Origin-Embedder-Policy: require-corp\r\nCross-Origin-Embedder-Policy-Report-Only: require-corp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
    }).await;
    let loader = static_http_loader([server.resolve_entry("localhost")]);
    let root_url = server.url_for_host("localhost", "/root");
    let child_url = server.url_for_host("localhost", "/child");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(root_url.as_str(), &loader);
    install_responses(&mut vm, root_url.as_str());
    vm.exec(&format!(r#"
      globalThis.frameReady=false;globalThis.parentReports=[];globalThis.childReports=[];
      addEventListener('message',event=>{{if(event.data==='cache-child-ready')frameReady=true;}});
      const frame=document.createElement('iframe');frame.id='target';frame.src={child};document.body.append(frame);
    "#,child=serde_json::to_string(child_url.as_str()).unwrap()),None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(frameReady)",
        "true",
        "cache child response policy",
    )
    .await;
    vm.exec(r#"
      globalThis.done=false;
      (async()=>{
        const child=document.getElementById('target').contentWindow;
        globalThis.parentObserver=new ReportingObserver(list=>parentReports.push(...list.map(report=>report.toJSON())),{types:['coep']});
        globalThis.childObserver=new child.ReportingObserver(list=>childReports.push(...list.map(report=>report.toJSON())),{types:['coep']});
        parentObserver.observe();childObserver.observe();
        const ownCache=await caches.open('policy');const otherCache=await child.caches.open('policy');
        const key=new URL('/entry',location.href).href;await ownCache.put(key,inputs[0]);
        child.origin='https://opaque.test';
        const base=child.document.createElement('base');base.href='https://opaque.test/';child.document.head.append(base);
        globalThis.completed=0;
        for(const [owner,cache] of [[window,ownCache],[child,otherCache]]){
          for(const callee of [window,child]){
            for(const [api,read] of [
              ['match',()=>callee.Cache.prototype.match.call(cache,key)],
              ['all',()=>callee.Cache.prototype.matchAll.call(cache,key)],
              ['storage',()=>callee.CacheStorage.prototype.match.call(owner.caches,key,{cacheName:'policy'})],
            ]){
              const promise=read();
              if(!(promise instanceof owner.Promise))throw new Error('wrong Promise realm');
              try{
                const value=await promise;
                if(owner===child)throw new Error('owner COEP was ignored');
                const response=api==='all'?value[0]:value;
                if(Object.getPrototypeOf(response)!==owner.Response.prototype)throw new Error('wrong Response realm');
                if(api==='all'&&(!Object.isFrozen(value)||Object.getPrototypeOf(value)!==owner.Array.prototype))throw new Error('wrong result array');
              }catch(error){
                if(owner!==child||!(error instanceof child.TypeError)||error instanceof TypeError)throw error;
                if(!error.message.includes('blocked by Cross-Origin-Resource-Policy'))throw error;
              }
              completed++;
            }
          }
        }
        globalThis.savedCache=otherCache;
        done=true;
      })().catch(error=>done='error:'+error);
    "#,None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "borrowed Cache read policies",
    )
    .await;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(childReports.length)",
        "12",
        "owner Cache COEP reports",
    )
    .await;
    assert_eq!(vm.eval("completed").unwrap(), "12");
    assert_eq!(vm.eval("parentReports.length").unwrap(), "0");
    let reports: Vec<serde_json::Value> =
        serde_json::from_str(&vm.eval("JSON.stringify(childReports)").unwrap()).unwrap();
    for report in reports {
        assert_eq!(report["url"], child_url.as_str());
        assert_eq!(report["body"]["blockedURL"], "https://opaque.test/included");
    }
    vm.exec(r#"
      globalThis.done=false;document.getElementById('target').remove();
      const key=new URL('/entry',location.href).href;
      const promise=Cache.prototype.match.call(savedCache,key);
      // Retirement is rejected by the pre-existing lifecycle guard before
      // result-realm selection or CORP evaluation.
      if(!(promise instanceof Promise))throw new Error('retired rejection Promise realm changed');
      promise.then(()=>done='unexpected response',error=>{
        done=error instanceof DOMException&&error.name==='InvalidStateError'&&error.message.includes('execution context has been destroyed');
      });
    "#,None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "retained Cache response policy",
    )
    .await;
    assert_eq!(vm.eval("parentReports.length").unwrap(), "0");
    assert_eq!(server.finish_targets().await, ["/child"]);
}
