use super::*;

fn native_status_map_vm() -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-eme-map.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let map = v8::Map::new(scope);
        let object = crate::context_bootstrap::media_key_status_map::map_for_test(scope, map);
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, object, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        for (name, value) in [
            ("statuses", object.into()),
            ("backing", map.into()),
            ("nativeStatuses", proxy.into()),
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
fn media_key_status_map_public_members_use_derived_receivers_and_window_exposure() {
    for url in ["https://eme-members.test/", "http://localhost/"] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval(include_str!("media_key_status_map.js")).unwrap();
        assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]"
        );
        assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "171");
    }
    let mut vm = new_storage_page_task_executor_test_vm("http://eme-insecure.test/");
    assert_eq!(
        vm.eval("!isSecureContext && !('MediaKeyStatusMap' in globalThis)")
            .unwrap(),
        "true"
    );
}

#[test]
fn media_key_status_map_compares_binary_keys_and_preserves_sorted_key_identities() {
    let mut vm = native_status_map_vm();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      const bytes=b=>Array.from(new Uint8Array(b)).join('-');
      const entries=[[[2],'expired'],[[1,0],'usable'],[[1],'status-pending'],[[255],'released']];
      for(const [key,value] of entries)backing.set(new Uint8Array(key).buffer,value);
      assert(statuses.size===4 && nativeStatuses.size===4,'native size identity');
      const padded=new Uint8Array([7,1,0,8]);
      assert(statuses.get(padded.subarray(1,3))==='usable','typed array byte range');
      assert(statuses.has(new DataView(padded.buffer,1,2)),'DataView byte range');
      assert(statuses.get(new Uint16Array(new Uint8Array([1,0]).buffer))==='usable','bytes rather than elements');
      assert(statuses.get(new Uint8Array([1]))==='status-pending'&&!statuses.has(new Uint8Array([0,1])),'prefix and order distinct');
      assert(statuses.get(new Uint8Array([3]))===undefined&&!statuses.has(new ArrayBuffer(0)),'unknown keys');
      assert([...statuses.keys()].map(bytes).join()==='1,1-0,2,255','octet lexicographic and prefix sorting');
      assert([...statuses.values()].join()==='status-pending,usable,expired,released','associated statuses');
      const pair=statuses.entries().next().value;
      assert(pair instanceof Array&&pair[0] instanceof ArrayBuffer&&pair[1]==='status-pending','pair representation');
      const key1=statuses.keys().next().value,key2=statuses.keys().next().value;
      assert(key1===key2&&key1===pair[0],'BufferSource keys preserve object identity');
      assert(Object.getPrototypeOf(statuses.entries())===Object.getPrototypeOf(statuses.keys()),'shared interface iterator prototype');
      const iterator=statuses.entries(),proto=Object.getPrototypeOf(iterator);
      assert(Object.prototype.toString.call(iterator)==='[object MediaKeyStatusMap Iterator]','iterator tag');
      assert(!Object.hasOwn(iterator,Symbol.toStringTag)&&iterator[Symbol.iterator]()===iterator,'iterator inherited surface');
      assert(Object.getPrototypeOf(proto)===Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())),'IteratorPrototype parent');
      assert(iterator.next().value[0] instanceof ArrayBuffer,'first item');
      backing.clear();backing.set(new Uint8Array([0]).buffer,'released');backing.set(new Uint8Array([9]).buffer,'expired');
      assert(bytes(iterator.next().value[0])==='9','next reads the current sorted list at its index');
      assert(iterator.next().done,'current list exhausted');
      backing.set(new Uint8Array([10]).buffer,'usable');
      assert(bytes(iterator.next().value[0])==='10','IDL pair iterator reads a later list after done');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn media_key_status_map_preserves_callback_realms_order_and_conversion_boundaries() {
    let mut vm = native_status_map_vm();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      const other=document.querySelector('iframe').contentWindow;
      const marker={},thisArg={};
      const expect=(C,action,name)=>{let error;try{action()}catch(caught){error=caught}assert(error instanceof C,name)};
      for(const w of [window,other]) {
        const P=w.MediaKeyStatusMap.prototype;
        backing.clear();backing.set(new Uint8Array([2]).buffer,'expired');backing.set(new Uint8Array([1]).buffer,'usable');
        assert(P.get.call(nativeStatuses,new other.Uint8Array([1]))==='usable','cross-realm native key lookup');
        const iterator=P.entries.call(nativeStatuses),first=iterator.next();
        assert(first instanceof w.Object&&first.value instanceof w.Array&&first.value[0] instanceof ArrayBuffer,'iterator result realm and existing key realm');
        const keys=[];
        assert(P.forEach.call(nativeStatuses,function(value,key,owner){
          assert(this===thisArg&&owner===nativeStatuses,'callback receiver and owner');
          keys.push(new Uint8Array(key)[0]);
          if(keys.length===1){backing.clear();backing.set(new Uint8Array([0]).buffer,'expired');backing.set(new Uint8Array([9]).buffer,'usable');}
        },thisArg)===undefined&&keys.join()==='1,9','forEach refreshes the sorted pairs after callback');
        let caught;try{P.forEach.call(statuses,()=>{throw marker})}catch(error){caught=error}assert(caught===marker,'callback exception identity');
        for(const bad of [null,{},1])expect(w.TypeError,()=>P.forEach.call(statuses,bad),'callback callable check');
        for(const name of ['get','has']) {
          expect(w.TypeError,()=>P[name].call(statuses),'required key argument');
          for(const value of [null,undefined,{},[],1,'1',Symbol(),new Proxy(new Uint8Array([1]),{}),new SharedArrayBuffer(0),new Uint8Array(new SharedArrayBuffer(0)),new SharedArrayBuffer(1),new Uint8Array(new SharedArrayBuffer(1)),new ArrayBuffer(1,{maxByteLength:2}),new Uint8Array(new ArrayBuffer(1,{maxByteLength:2}))])expect(w.TypeError,()=>P[name].call(statuses,value),'fixed nonshared BufferSource');
          const detached=new ArrayBuffer(1),detachedView=new Uint8Array(detached);
          structuredClone(detached,{transfer:[detached]});
          assert(P[name].call(statuses,detached)===(name==='has'?false:undefined)&&P[name].call(statuses,detachedView)===(name==='has'?false:undefined),'detached fixed buffers are empty lookups');
          const resizable=new ArrayBuffer(1,{maxByteLength:2}),resizableView=new Uint8Array(resizable);
          structuredClone(resizable,{transfer:[resizable]});
          for(const value of [resizable,resizableView])expect(w.TypeError,()=>P[name].call(statuses,value),'detached resizable BufferSource');
          let traps=0;const proxy=new Proxy(new Uint8Array([1]),{get(){traps++;throw marker}});
          expect(w.TypeError,()=>P[name].call(statuses,proxy),'author buffer Proxy');assert(traps===0,'no author buffer traps');
          const revoked=Proxy.revocable(statuses,{});revoked.revoke();
          for(const receiver of [{},Object.create(statuses),new Proxy(nativeStatuses,{}),revoked.proxy])expect(w.TypeError,()=>P[name].call(receiver,proxy),'native brand before argument conversion');
          assert(traps===0,'receiver and argument Proxies do not trigger traps');
        }
        const p=Object.getPrototypeOf(iterator);
        for(const receiver of [{},Object.create(iterator),new Proxy(iterator,{})])expect(w.TypeError,()=>p.next.call(receiver),'iterator native brand');
      }
      const foreign=other.Function('value','key','owner', 'globalThis.callbackKey = key; globalThis.callbackOwner = owner;');
      statuses.forEach(foreign);
      assert(other.callbackKey instanceof ArrayBuffer&&!(other.callbackKey instanceof other.ArrayBuffer)&&other.callbackOwner===statuses,'callback preserves existing BufferSource realm');
      Object.setPrototypeOf(statuses,null);
      assert(other.MediaKeyStatusMap.prototype.get.call(nativeStatuses,new Uint8Array([9]))==='usable','brand survives prototype replacement');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn media_key_status_map_iterators_support_native_proxies_and_borrowed_next_realms() {
    let mut vm = native_status_map_vm();
    vm.eval(r#"
      backing.set(new Uint8Array([1]).buffer,'usable');
      backing.set(new Uint8Array([2]).buffer,'expired');
      Object.defineProperty(globalThis,'MediaKeyStatusMap Iterator',{get(){throw Error('author hidden constructor getter')}});
      globalThis.iterator=statuses.entries();
    "#).unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let object = v8::Local::<v8::Object>::try_from(
            global
                .get(scope, crate::util::v8str(scope, "iterator").into())
                .unwrap(),
        )
        .unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, object, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeIterator").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      const other=document.querySelector('iframe').contentWindow;
      const foreignNext=Object.getPrototypeOf(other.MediaKeyStatusMap.prototype.entries.call(statuses)).next;
      const row=foreignNext.call(nativeIterator);
      assert(row instanceof other.Object&&row.value instanceof other.Array&&row.value[0] instanceof ArrayBuffer,'borrowed next creates result containers in its callee realm and retains the key realm');
      assert(row.value[1]==='usable','registered iterator native identity');
      let traps=0,error;try{foreignNext.call(new Proxy(nativeIterator,{get(){traps++;}}))}catch(caught){error=caught}
      assert(error instanceof other.TypeError&&traps===0,'author wrapper is rejected without traps');
      Object.setPrototypeOf(iterator,null);
      assert(foreignNext.call(nativeIterator).value[1]==='expired'&&foreignNext.call(nativeIterator).done,'native iterator brand survives prototype changes');
      return true;
    })()"#).unwrap(),"true");
}
