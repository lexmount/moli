use super::*;

#[tokio::test]
async fn worker_reporting_observer_uses_native_receivers_and_dictionary_conversion() {
    ensure_v8();
    let source = format!(
        "const probe={};const checks=probe(globalThis,globalThis,'worker');postMessage({{total:checks.length,failures:checks.filter(row=>!row.passed)}});close();",
        include_str!("../../../script_vm/tests/reporting_observer.js")
    );
    let mut worker = spawn_worker(source, "https://reporting.test/worker.js".into());
    let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
    assert_eq!(expect_post_json(message), r#"{"total":62,"failures":[]}"#);
}

#[tokio::test]
async fn worker_reporting_observer_delivers_native_csp_reports_in_a_task() {
    ensure_v8();
    let source = r#"
      globalThis.order=[];
      const observer=new ReportingObserver(function(reports,self){
        postMessage({order,receiver:this===observer,observer:self===observer,
          reports:reports.map(r=>[r.type,r.url,r.body.effectiveDirective,r.body.disposition]),
          remaining:observer.takeRecords().length});close();
      },{types:['csp-violation']});
      observer.observe();
      try{eval('1')}catch{}
      order.push('sync');Promise.resolve().then(()=>order.push('microtask'));
      addEventListener('securitypolicyviolation',()=>{order.push('event');observer.disconnect();});
    "#;
    let mut worker = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source.into(), "https://reporting.test/worker.js".into())
            .with_content_security_policies(vec!["script-src 'self'".into()]),
    );
    let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
    assert_eq!(
        expect_post_json(message),
        r#"{"order":["sync","microtask","event"],"receiver":true,"observer":true,"reports":[["csp-violation","https://reporting.test/worker.js","script-src","enforce"]],"remaining":0}"#
    );
}
