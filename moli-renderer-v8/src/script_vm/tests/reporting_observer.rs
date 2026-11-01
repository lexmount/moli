use super::*;

fn report(vm: &mut ScriptVm, report_type: &str, value: i32) {
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        crate::context_bootstrap::notify_reporting_observers(
            scope,
            report_type,
            "https://reporting.test/",
            serde_json::json!({"value":value}),
        );
        Ok(())
    })
    .unwrap();
}

fn drain_reporting_tasks(vm: &mut ScriptVm) {
    for _ in 0..400 {
        if !vm.has_ready_callback_timer() {
            return;
        }
        vm.run_next_timeout_for_test().unwrap();
    }
    panic!("reporting tasks did not settle");
}

#[test]
fn reporting_observer_uses_native_cross_realm_receivers_and_dictionary_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        "(() => {{const probe={}; const realms=[globalThis,child.contentWindow];globalThis.checks=[];for(let a=0;a<2;a++)for(let b=0;b<2;b++)checks.push(...probe(realms[a],realms[b],a+'-'+b));return JSON.stringify(checks.filter(row=>!row.passed));}})()",
        include_str!("reporting_observer.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "[]");
    assert_eq!(vm.eval("checks.length").unwrap(), "248");
}

#[test]
fn reporting_observer_disconnect_keeps_queued_reports_and_take_records_drains() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval("globalThis.calls=[];globalThis.observer=new ReportingObserver(function(reports,self){calls.push([this===observer,self===observer,reports.map(r=>r.body.value)]);});observer.observe();observer.observe();").unwrap();
    report(&mut vm, "test", 1);
    assert_eq!(
        vm.eval("observer.disconnect();JSON.stringify(calls)")
            .unwrap(),
        "[]"
    );
    drain_reporting_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(calls)").unwrap(),
        "[[true,true,[1]]]"
    );
    report(&mut vm, "test", 2);
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("calls.length").unwrap(), "1");
    vm.eval("observer.observe()").unwrap();
    report(&mut vm, "test", 3);
    assert_eq!(
        vm.eval("observer.takeRecords().map(r=>r.body.value).join()")
            .unwrap(),
        "3"
    );
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("calls.length").unwrap(), "1");
}

#[test]
fn reporting_observer_buffer_is_once_only_ordered_and_limited_per_type() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    for index in 0..110 {
        report(&mut vm, "first", index);
        report(&mut vm, "second", index);
    }
    vm.eval("globalThis.reports=[];globalThis.observer=new ReportingObserver(values=>reports.push(...values),{buffered:true});observer.observe();observer.observe();").unwrap();
    assert_eq!(vm.eval("reports.length").unwrap(), "0");
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("JSON.stringify([reports.length,reports.slice(0,4).map(r=>[r.type,r.body.value]),reports.slice(-2).map(r=>[r.type,r.body.value])])").unwrap(),r#"[200,[["first",10],["second",10],["first",11],["second",11]],[["first",109],["second",109]]]"#);
    vm.eval("observer.disconnect();observer.observe()").unwrap();
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("reports.length").unwrap(), "200");
}

#[test]
fn reporting_observer_filters_copy_reports_and_follow_registration_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval(r#"globalThis.calls=[];
      globalThis.first=new ReportingObserver(reports=>{calls.push('first:'+reports[0].body.value);reports[0].body.value=999;});
      globalThis.second=new ReportingObserver(reports=>calls.push('second:'+reports[0].body.value),{types:['test']});
      globalThis.unknown=new ReportingObserver(()=>calls.push('unknown'),{types:['unknown']});
      second.observe();first.observe();unknown.observe();second.disconnect();second.observe();"#).unwrap();
    report(&mut vm, "test", 42);
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("calls.join()").unwrap(), "first:42,second:42");
    vm.eval("globalThis.buffered=new ReportingObserver(reports=>calls.push('buffer:'+reports[0].body.value),{types:['test'],buffered:true});buffered.observe()").unwrap();
    drain_reporting_tasks(&mut vm);
    assert_eq!(
        vm.eval("calls.join()").unwrap(),
        "first:42,second:42,buffer:42"
    );
}

#[test]
fn reporting_observer_registered_native_proxy_shares_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval("globalThis.calls=0;globalThis.observer=new ReportingObserver(()=>calls++)")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "observer");
        let observer =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, observer, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeProxy");
        global.create_data_property(scope, key.into(), proxy.into());
        Ok(())
    })
    .unwrap();
    vm.eval("ReportingObserver.prototype.observe.call(nativeProxy)")
        .unwrap();
    report(&mut vm, "test", 7);
    assert_eq!(
        vm.eval("ReportingObserver.prototype.takeRecords.call(nativeProxy)[0].body.value")
            .unwrap(),
        "7"
    );
    vm.eval("ReportingObserver.prototype.disconnect.call(nativeProxy)")
        .unwrap();
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("calls").unwrap(), "0");
}

#[test]
fn reporting_observer_converts_callback_and_returned_records_in_the_consumer_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval(
        r#"
      document.body.innerHTML='<iframe id=frame></iframe>';
      globalThis.other=frame.contentWindow;globalThis.calls=[];
      globalThis.observer=new ReportingObserver(other.Function('reports,self',`
        parent.calls.push([reports instanceof Array,
          Object.getPrototypeOf(Object.getPrototypeOf(reports[0]))===Object.prototype&&Object.prototype.toString.call(reports[0])==='[object Report]',
          Object.getPrototypeOf(reports[0].body)===Object.prototype,
          this===self,reports[0].body.value]);
      `));
      observer.observe();
    "#,
    )
    .unwrap();
    report(&mut vm, "test", 1);
    drain_reporting_tasks(&mut vm);
    assert_eq!(
        vm.eval("JSON.stringify(calls)").unwrap(),
        "[[true,true,true,true,1]]"
    );
    report(&mut vm, "test", 2);
    assert_eq!(
        vm.eval(
            r#"(() => {
          const records=other.ReportingObserver.prototype.takeRecords.call(observer);
          return JSON.stringify([records instanceof other.Array,
            Object.getPrototypeOf(Object.getPrototypeOf(records[0]))===other.Object.prototype&&Object.prototype.toString.call(records[0])==='[object Report]',
            Object.getPrototypeOf(records[0].body)===other.Object.prototype,
            records[0].body.value,observer.takeRecords().length]);
        })()"#
        )
        .unwrap(),
        "[true,true,true,2,0]"
    );
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("calls.length").unwrap(), "1");
}

#[test]
fn reporting_observer_callbacks_survive_exceptions_and_indexed_prototype_pollution() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval(r#"
      globalThis.calls='';globalThis.traps=0;
      globalThis.first=new ReportingObserver(()=>{calls+='first|';throw Error('observer failure');});
      globalThis.second=new ReportingObserver(reports=>{calls+='second:'+reports[0].body.value;});
      first.observe();second.observe();
      Object.defineProperty(Array.prototype,'0',{configurable:true,get(){traps++;throw Error('get');},set(){traps++;throw Error('set');}});
      JSON.parse=()=>{throw Error('author JSON.parse');};
    "#).unwrap();
    report(&mut vm, "test", 9);
    drain_reporting_tasks(&mut vm);
    assert_eq!(
        vm.eval("delete Array.prototype[0];calls+'|'+traps")
            .unwrap(),
        "first|second:9|0"
    );
}

#[test]
fn reporting_observer_callback_and_owner_realm_retirement_suppresses_pending_delivery() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting.test/");
    vm.eval(
        r#"
      document.body.innerHTML='<iframe id=frame></iframe>';
      globalThis.other=frame.contentWindow;globalThis.calls=0;
      globalThis.observer=new ReportingObserver(other.Function('parent.calls++'));
      globalThis.childObserver=new other.ReportingObserver(()=>calls++);
      observer.observe();childObserver.observe();
    "#,
    )
    .unwrap();
    report(&mut vm, "test", 1);
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "other");
        let other =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let context = other.get_creation_context(scope).unwrap();
        let scope = &mut v8::ContextScope::new(scope, context);
        crate::context_bootstrap::notify_reporting_observers(
            scope,
            "test",
            "https://reporting.test/child",
            serde_json::json!({"value":2}),
        );
        Ok(())
    })
    .unwrap();
    vm.eval("frame.remove()").unwrap();
    drain_reporting_tasks(&mut vm);
    assert_eq!(vm.eval("calls").unwrap(), "0");
}

#[test]
fn reporting_records_use_hidden_native_prototypes_and_immutable_json_snapshots() {
    let mut vm = new_storage_page_task_executor_test_vm("https://reporting-records.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>';globalThis.other=child.contentWindow;globalThis.observer=new ReportingObserver(()=>{});observer.observe();").unwrap();
    let context = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context,|scope,_|{
        crate::context_bootstrap::notify_reporting_observers(scope,"coep","https://reporting-records.test/",serde_json::json!({"type":"corp","blockedURL":"https://remote.test/image","destination":"","disposition":"enforce"}));
        Ok(())
    }).unwrap();
    assert_eq!(vm.eval(r#"(() => {
      if(typeof Report!=='undefined'||typeof other.Report!=='undefined')return 'global Report';
      let constructorLookups=0;
      Object.defineProperty(other,'Report',{configurable:true,get(){constructorLookups++;throw Error('public Report getter');}});
      const record=other.ReportingObserver.prototype.takeRecords.call(observer)[0];
      delete other.Report;
      if(constructorLookups!==0)return 'constructor lookup';
      const proto=Object.getPrototypeOf(record);const body=record.body;
      if(Reflect.ownKeys(record).length!==0||Reflect.ownKeys(body).length!==0)return 'own properties';
      if(Object.getPrototypeOf(proto)!==other.Object.prototype||Object.hasOwn(proto,'constructor'))return 'Report prototype';
      if(Object.prototype.toString.call(record)!=='[object Report]'||Object.prototype.toString.call(body)!=='[object ReportBody]')return 'tags';
      if(Object.getPrototypeOf(body)!==other.ReportBody.prototype)return 'body prototype';
      const descriptor=Object.getOwnPropertyDescriptor(proto,'type');
      if(!descriptor.enumerable||!descriptor.configurable||descriptor.set!==undefined)return 'readonly attribute';
      if(record.type!=='coep'||record.url!=='https://reporting-records.test/')return 'attributes';
      const serialize=proto.toJSON;const serializeBody=other.ReportBody.prototype.toJSON;
      const expected={type:'corp',blockedURL:'https://remote.test/image',destination:'',disposition:'enforce'};
      const first=serialize.call(record);first.body.blockedURL='changed';
      if(serialize.call(record).body.blockedURL!==expected.blockedURL)return 'shared snapshot';
      if(serializeBody.call(body).blockedURL!==expected.blockedURL)return 'body snapshot';
      Object.defineProperties(record,{type:{get(){throw Error('public type getter')}},body:{get(){throw Error('public body getter')}},url:{value:'https://forged.test/'}});
      const snapshot=serialize.call(record);
      if(Object.keys(snapshot).join(',')!=='type,url,body')return 'snapshot keys';
      if(snapshot.type!=='coep'||snapshot.url!=='https://reporting-records.test/'||snapshot.body.blockedURL!==expected.blockedURL)return 'public override';
      if(Object.getPrototypeOf(snapshot)!==other.Object.prototype||Object.getPrototypeOf(snapshot.body)!==other.Object.prototype)return 'snapshot realm';
      for(const [real,method] of [[record,serialize],[body,serializeBody]]){
        let traps=0;const proxy=new Proxy(real,{get(){traps++;throw Error('proxy get')}});
        for(const fake of [{},Object.create(real),proxy]){
          try{method.call(fake);return 'accepted forged receiver';}
          catch(error){if(!(error instanceof other.TypeError)||error instanceof TypeError)return 'error realm';}
        }
        if(traps!==0)return 'proxy traps';
      }
      return 'true';
    })()"#).unwrap(),"true");
}
