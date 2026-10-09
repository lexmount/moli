use super::*;

#[test]
fn midi_frontend_uses_shared_receivers_dictionaries_and_secure_exposure() {
    for url in [
        "https://midi-frontend.test/",
        "http://midi-frontend.test/",
        "http://localhost/",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        vm.eval("document.body.innerHTML = '<iframe></iframe>'")
            .unwrap();
        if url == "http://midi-frontend.test/" {
            assert_eq!(vm.eval("!isSecureContext && !('requestMIDIAccess' in navigator) && !('requestMIDIAccess' in document.querySelector('iframe').contentWindow.navigator)").unwrap(), "true");
            continue;
        }
        vm.eval(include_str!("../../../tests/fixtures/midi-frontend.js"))
            .unwrap();
        assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
                .unwrap(),
            "[]"
        );
        assert_eq!(vm.eval("__uiEventResults.total >= 100").unwrap(), "true");
    }
}

#[test]
fn native_midi_maps_are_live_lossless_and_support_registered_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-midi-maps.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (interface, name, backing_name, proxy_name) in [
            ("MIDIInputMap", "inputs", "inputBacking", "nativeInputs"),
            ("MIDIOutputMap", "outputs", "outputBacking", "nativeOutputs"),
        ] {
            let map = v8::Map::new(scope);
            let object = crate::context_bootstrap::midi_map_for_test(scope, interface, map);
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            for (name, value) in [
                (name, object.into()),
                (backing_name, map.into()),
                (proxy_name, proxy.into()),
            ] {
                assert_eq!(
                    global.create_data_property(
                        scope,
                        crate::util::v8str(scope, name).into(),
                        value
                    ),
                    Some(true)
                );
            }
        }
        for (interface, name) in [("MIDIInput", "input"), ("MIDIOutput", "output")] {
            let prototype =
                crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, interface)
                    .unwrap();
            let port = v8::Object::new(scope);
            crate::web_api_interfaces::initialize(scope, port, interface).unwrap();
            assert_eq!(port.set_prototype(scope, prototype.into()), Some(true));
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, name).into(),
                    port.into()
                ),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert=(ok,name)=>{if(!ok)throw Error(name)};
      const other=document.querySelector('iframe').contentWindow;
      const throws=(C,action,name)=>{let error;try{action()}catch(caught){error=caught}assert(error instanceof C,name)};
      for(const [name,map,backing,native,port] of [['MIDIInputMap',inputs,inputBacking,nativeInputs,input],['MIDIOutputMap',outputs,outputBacking,nativeOutputs,output]]) {
        backing.set('a\ud800',port);backing.set('b',port);
        for(const w of [window,other]) {
          const P=w[name].prototype;
          assert(Object.getOwnPropertyDescriptor(P,'size').get.call(native)===2,'registered native size');
          let conversions=0;assert(P.get.call(native,{toString(){conversions++;return 'a\ud800'}})===port&&conversions===1,'lossless DOMString identity');
          assert(P.has.call(native,'a\ud800')&&!P.has.call(native,'a\ufffd')&&P.get.call(native,'absent')===undefined,'native lookup');
          backing.set('undefined',port);assert(P.get.call(map)===port&&P.has.call(map),'missing keys convert undefined');backing.delete('undefined');
          const iterator=P.entries.call(native),proto=Object.getPrototypeOf(iterator);
          assert(Object.prototype.toString.call(iterator)==='[object Map Iterator]'&&proto===Object.getPrototypeOf(new w.Map().entries()),'callee maplike iterator prototype');
          const first=iterator.next();assert(first instanceof w.Object&&first.value instanceof w.Array&&first.value[0]==='a\ud800','callee iterator result');
          backing.delete('b');backing.set('c',port);
          assert(iterator.next().value[0]==='c'&&iterator.next().done,'live iteration observes deletion and append');
          backing.set('afterDone',port);assert(iterator.next().done,'exhaustion stays exhausted');backing.delete('afterDone');
          const keys=P.keys.call(map),values=P.values.call(map);assert(keys.next().value==='a\ud800'&&values.next().value===port,'key/value iteration');
          const marker={},log=[],thisArg={};
          assert(P.forEach.call(native,function(value,key,owner){assert(this===thisArg&&value===port&&owner===native,'forEach arguments');log.push(key);if(key==='a\ud800'){backing.delete('c');backing.set('added',port)}},thisArg)===undefined&&log.join()==='a\ud800,added','live forEach');
          let error;try{P.forEach.call(map,()=>{throw marker})}catch(caught){error=caught}assert(error===marker,'callback exception identity');
          for(const callback of [null,{},1])throws(w.TypeError,()=>P.forEach.call(map,callback),'forEach callable validation');
          for(const key of [Symbol()])throws(w.TypeError,()=>P.get.call(map,key),'DOMString failure');
          for(const bad of [{},Object.create(map),new Proxy(map,{})])throws(w.TypeError,()=>P.get.call(bad,{toString(){throw Error('conversion before brand')}}),'map native brand');
          throws(w.TypeError,()=>proto.next.call({}),'iterator native brand');
          const saved=w.Map.prototype.entries,savedNext=Object.getPrototypeOf(new w.Map().entries()).next;
          try {
            w.Map.prototype.entries=()=>{throw marker};Object.getPrototypeOf(new w.Map().keys()).next=()=>{throw marker};
            assert(savedNext.call(P.entries.call(map)).value[0]==='a\ud800','captured map creation intrinsic');
            const keys=[];P.forEach.call(map,(_,key)=>keys.push(key));assert(keys[0]==='a\ud800','captured iterator next for internal forEach');
          } finally {w.Map.prototype.entries=saved;Object.getPrototypeOf(new w.Map().entries()).next=savedNext}
          backing.delete('added');backing.set('b',port);
        }
      }
      const log=[];input.onstatechange=()=>log.push('handler');input.addEventListener('statechange',()=>log.push('listener'));
      input.onstatechange=()=>log.push('replacement');input.dispatchEvent(new Event('statechange'));
      assert(log.join()==='replacement,listener','ordered port event handlers');input.onstatechange=1;assert(input.onstatechange===null,'non-object event handler');
      let reads=0,error,marker={};const iterable={get [Symbol.iterator](){reads++;return function*(){yield -1;yield 257}}};
      try{other.MIDIOutput.prototype.send.call(output,iterable,{valueOf(){reads++;return 1}})}catch(caught){error=caught}
      assert(error instanceof other.DOMException&&error.name==='NotSupportedError'&&reads===2,'shared sequence and timestamp before backend error');
      reads=0;try{other.MIDIOutput.prototype.send.call(output,[Symbol()],{valueOf(){reads++;return 0}})}catch(caught){error=caught}
      assert(error instanceof other.TypeError&&reads===0,'sequence failure stops timestamp conversion');
      try{other.MIDIOutput.prototype.send.call(output,[],Infinity)}catch(caught){error=caught}assert(error instanceof other.TypeError,'finite timestamp');
      try{other.MIDIOutput.prototype.send.call(output,{get [Symbol.iterator](){throw marker}})}catch(caught){error=caught}assert(error===marker,'iterator exception identity');
      return true;
    })()"#).unwrap(),"true");
}
