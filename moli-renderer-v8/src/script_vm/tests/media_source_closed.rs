use super::*;

#[test]
fn media_source_closed_frontend_validates_receivers_realms_and_conversion_order() {
    for url in ["https://mse-closed.test/", "http://mse-closed.test/"] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval(include_str!("media_source_closed.js")).unwrap();
        assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]"
        );
        assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "477");
    }
}

fn native_source_buffer_list_vm() -> crate::runtime::PageVmTaskExecutorTestHarness {
    use moli_webapi_declare::WebApiObject;

    #[derive(WebApiObject)]
    #[webapi(interface = crate::web_api_interfaces::SourceBuffer, require_prototype)]
    struct TestSourceBuffer {}

    let mut vm = new_storage_page_task_executor_test_vm("https://mse-list.test/");
    vm.eval("globalThis.source = new MediaSource(); document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let owner = v8::Local::<v8::Object>::try_from(
            global
                .get(scope, crate::util::v8str(scope, "source").into())
                .unwrap(),
        )
        .unwrap();
        let first = TestSourceBuffer::new().bind(scope).unwrap();
        let second = TestSourceBuffer::new().bind(scope).unwrap();
        let backing = v8::Array::new_with_elements(scope, &[first.into(), second.into()]);
        let list = crate::context_bootstrap::media_source::source_buffer_list::build(
            scope, owner, backing,
        );
        assert!(list.is_proxy(), "native lists use a registered Proxy");
        assert!(crate::web_api_interfaces::SourceBufferList::is_instance(
            scope, list
        ));
        for (name, value) in [
            ("backing", backing.into()),
            ("buffers", list.into()),
            ("nativeBuffers", list.into()),
            ("first", first.into()),
            ("second", second.into()),
        ] {
            assert_eq!(
                global.create_data_property(scope, crate::util::v8str(scope, name).into(), value),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    vm
}

#[test]
fn media_source_buffer_list_indexes_and_array_iterator_observe_native_backing() {
    let mut vm = native_source_buffer_list_vm();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      assert(buffers.length===2&&buffers[0]===first&&buffers[1]===second,'native indexed identity');
      assert(Object.keys(buffers).join()==='0,1','supported indices are enumerable');
      const d=Object.getOwnPropertyDescriptor(buffers,'0');
      assert(d.value===first&&d.writable===false&&d.enumerable&&d.configurable,'indexed descriptor');
      for(const key of ['0','1','2','4294967294']) {
        assert(Reflect.set(buffers,key,{})===false,'readonly array index set '+key);
        assert(Reflect.defineProperty(buffers,key,{value:first,configurable:true})===false,'readonly define '+key);
        assert(Reflect.defineProperty(buffers,key,{get(){throw Error('getter')}})===false,'readonly accessor define '+key);
      }
      assert(Reflect.deleteProperty(buffers,'0')===false&&Reflect.deleteProperty(buffers,'2')===true,'delete supported and unsupported indices');
      const iterator=buffers[Symbol.iterator]();
      assert(iterator.next().value===first,'ArrayProtoValues first member');
      backing[1]=first;backing.push(second);
      assert(iterator.next().value===first&&iterator.next().value===second&&iterator.next().done,'iterator observes the live backing');
      backing.length=0;
      assert(buffers.length===0&&!Object.hasOwn(buffers,0)&&Object.keys(buffers).length===0,'shrinking removes supported indices');
      const p=Object.create(SourceBufferList.prototype);
      Object.defineProperty(p,'0',{get(){return 'prototype'},set(value){this.expando=value},configurable:true});
      Object.setPrototypeOf(buffers,p);
      assert(buffers[0]==='prototype'&&Reflect.set(buffers,'0','setter')&&buffers.expando==='setter','unsupported index uses the prototype');
      backing.push(first);
      assert(buffers[0]===first&&!Reflect.set(buffers,'0',second),'supported index masks the prototype');
      const other={};assert(Reflect.set(buffers,'4','ordinary',other)&&other[4]==='ordinary','ordinary set receiver');
      const reflectSet=Reflect.set;
      try {
        Reflect.set=()=>{throw Error('author Reflect.set')};
        assert(reflectSet(buffers,'4','ignored')===false,'captured intrinsic survives author replacement');
      } finally {Reflect.set=reflectSet}
      Object.setPrototypeOf(buffers,new Proxy({}, {set(target,key,value,receiver){
        assert(key==='4'&&value==='primitive'&&receiver===42,'prototype Proxy receives the original arguments');return true;
      }}));
      assert(Reflect.set(buffers,'4','primitive',42),'prototype Proxy with a primitive receiver');
      Object.setPrototypeOf(buffers,null);
      assert(Reflect.set(buffers,'4',1)===false&&Reflect.set(buffers,'4',1,42)===false,'null prototype readonly creation');
      assert(Reflect.preventExtensions(buffers)===false&&Object.isExtensible(buffers),'legacy object remains extensible');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn media_source_buffer_list_native_proxies_and_foreign_source_buffers_use_brands() {
    let mut vm = native_source_buffer_list_vm();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      const other=document.querySelector('iframe').contentWindow;
      const log=[],controller=new AbortController();
      const listener=function(e){assert(this===buffers&&e.target===buffers,'native wrapper event identity');log.push('listener')};
      buffers.addEventListener('custom',listener,{once:true});
      buffers.dispatchEvent(new Event('custom'));buffers.dispatchEvent(new Event('custom'));
      assert(log.join()==='listener','once across the registered native Proxy');
      buffers.addEventListener('custom',listener,{signal:controller.signal});controller.abort();
      buffers.dispatchEvent(new Event('custom'));assert(log.length===1,'aborting removes the native proxy listener');
      buffers.addEventListener('custom',listener);buffers.removeEventListener('custom',listener);
      buffers.dispatchEvent(new Event('custom'));assert(log.length===1,'remove uses the same backing registry');
      try {
        Object.prototype.get=()=>{throw Error('polluted get trap')};
        Object.prototype.ownKeys=()=>{throw Error('polluted ownKeys trap')};
        assert(buffers[0]===first&&Object.keys(buffers).join()==='0,1','null prototype native handler');
      } finally {delete Object.prototype.get;delete Object.prototype.ownKeys}
      for(const w of [window,other]) {
        const getLength=Object.getOwnPropertyDescriptor(w.SourceBufferList.prototype,'length').get;
        assert(getLength.call(nativeBuffers)===2,'registered native Proxy');
        let traps=0;
        const proxy=new Proxy(nativeBuffers,{get(){traps++;throw Error('author trap')}});
        const revoked=Proxy.revocable(buffers,{});revoked.revoke();
        for(const receiver of [{},Object.create(buffers),proxy,revoked.proxy]) {
          let error;try{getLength.call(receiver)}catch(caught){error=caught}
          assert(error instanceof w.TypeError&&traps===0,'callee realm brand check without traps');
        }
        let error;try{w.MediaSource.prototype.removeSourceBuffer.call(source,first)}catch(caught){error=caught}
        assert(error instanceof w.DOMException&&error.name==='NotFoundError','genuine buffer missing from closed source');
        for(const argument of [Object.create(first),new Proxy(first,{})]) {
          let error;try{w.MediaSource.prototype.removeSourceBuffer.call(source,argument)}catch(caught){error=caught}
          assert(error instanceof w.TypeError,'buffer argument uses native interface conversion');
        }
      }
      Object.setPrototypeOf(buffers,null);
      assert(Object.getOwnPropertyDescriptor(other.SourceBufferList.prototype,'length').get.call(nativeBuffers)===2,'brand survives prototype changes');
      return true;
    })()"#).unwrap(), "true");
}
