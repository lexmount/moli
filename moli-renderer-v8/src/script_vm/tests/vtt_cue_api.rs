use super::*;

#[test]
fn vtt_cue_api_preserves_native_identity_utf16_and_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://vtt-cue-api.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.foreign=document.querySelector('iframe').contentWindow;globalThis.cue=new foreign.VTTCue(0,1,'text');globalThis.region=new foreign.VTTRegion();").unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("cue", "nativeCue"), ("region", "nativeRegion")] {
            let key = crate::util::v8str(scope, name);
            let object = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(object).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name);};
      const throws = (callback, ctor, name) => {
        let error; try {callback();} catch (caught) {error=caught;}
        assert(error && Object.getPrototypeOf(error) === ctor.prototype, name);
      };
      const base = ['track','id','startTime','endTime','pauseOnExit','onenter','onexit'];
      const own = ['region','vertical','snapToLines','line','lineAlign','position','positionAlign','size','align','text'];
      const regionNames = ['id','width','lines','regionAnchorX','regionAnchorY','viewportAnchorX','viewportAnchorY','scroll'];
      for (const realm of [window,foreign]) {
        for (const [prototype, names, real] of [[realm.TextTrackCue.prototype,base,cue],[realm.VTTCue.prototype,own,cue],[realm.VTTRegion.prototype,regionNames,region]]) {
          for (const name of names) {
            const descriptor=Object.getOwnPropertyDescriptor(prototype,name);
            assert(descriptor.enumerable && descriptor.configurable && descriptor.get.length===0, name+'/descriptor');
            let conversions=0,traps=0;
            const value={toString(){conversions++;return 'auto';},valueOf(){conversions++;return 10;}};
            const proxy=new Proxy(real,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
            const revoked=Proxy.revocable(real,{});revoked.revoke();
            for (const receiver of [{},Object.create(real),Object.create(prototype),proxy,revoked.proxy]) {
              throws(() => descriptor.get.call(receiver),realm.TypeError,name+'/getter brand');
              if (descriptor.set) {
                assert(descriptor.set.length===1,name+'/setter length');
                throws(() => descriptor.set.call(receiver,value),realm.TypeError,name+'/setter brand');
              }
              assert(conversions===0 && traps===0,name+'/brand before conversion');
            }
          }
        }
        const calls=[];
        const made=new realm.VTTCue({valueOf(){calls.push('start');return 2;}},{valueOf(){calls.push('end');return Infinity;}},{toString(){calls.push('text');return '\uD800';}});
        assert(calls.join()==='start,end,text' && made.endTime===Infinity && made.text==='\uD800','constructor conversion order');
        calls.length=0;
        throws(() => new realm.VTTCue(0,NaN,{toString(){calls.push('text');return '';}}),realm.TypeError,'invalid end time');
        assert(calls.join()==='text','end validation follows all argument conversions');
        calls.length=0;
        throws(() => new realm.VTTCue(NaN,{valueOf(){calls.push('end');return 0;}},{toString(){calls.push('text');return '';}}),realm.TypeError,'invalid start time');
        assert(calls.length===0,'restricted start short circuits later arguments');
        class Subclass extends realm.VTTCue {}
        assert(Object.getPrototypeOf(new Subclass(0,1,''))===Subclass.prototype,'newTarget prototype');
        const end=Object.getOwnPropertyDescriptor(realm.TextTrackCue.prototype,'endTime');
        end.set.call(nativeCue,Infinity);assert(end.get.call(nativeCue)===Infinity && cue.endTime===Infinity,'native infinity setter');
        for (const bad of [NaN,-Infinity]) throws(() => end.set.call(nativeCue,bad),realm.TypeError,'invalid end setter');
        assert(cue.endTime===Infinity,'failed end setter preserves state');
        for (const name of ['line','position']) {
          const descriptor=Object.getOwnPropertyDescriptor(realm.VTTCue.prototype,name);
          descriptor.set.call(nativeCue,12.5);assert(cue[name]===12.5,'native numeric setting');
          for (const bad of ['12.5',null,undefined,false]) throws(() => descriptor.set.call(nativeCue,bad),realm.TypeError,'union string member');
          let hint;
          descriptor.set.call(nativeCue,{[Symbol.toPrimitive](value){hint=value;return 'auto';}});
          assert(hint==='string' && cue[name]==='auto','union object string conversion');
        }
        for (const [name,valid] of [['lineAlign','end'],['positionAlign','line-right'],['vertical','rl'],['align','right']]) {
          const descriptor=Object.getOwnPropertyDescriptor(realm.VTTCue.prototype,name);
          descriptor.set.call(nativeCue,valid);descriptor.set.call(nativeCue,'invalid');
          assert(cue[name]===valid,'invalid enum attribute ignored');
          const sentinel={};let thrown;
          try {descriptor.set.call(nativeCue,{toString(){throw sentinel;}});} catch (error) {thrown=error;}
          assert(thrown===sentinel && cue[name]===valid,'enum conversion exception');
        }
        const r=Object.getOwnPropertyDescriptor(realm.VTTCue.prototype,'region');
        r.set.call(nativeCue,nativeRegion);assert(r.get.call(cue)===nativeRegion,'nullable interface preserves identity');
        throws(() => r.set.call(cue,new Proxy(region,{})),realm.TypeError,'author region proxy');
        assert(cue.region===nativeRegion,'invalid region preserves state');
        r.set.call(cue,undefined);assert(cue.region===null,'nullable undefined');
        const width=Object.getOwnPropertyDescriptor(realm.VTTRegion.prototype,'width');
        width.set.call(nativeRegion,12.5);assert(region.width===12.5,'native region state');width.set.call(nativeRegion,100);
        const lines=Object.getOwnPropertyDescriptor(realm.VTTRegion.prototype,'lines');
        lines.set.call(nativeRegion,-1);assert(region.lines===4294967295,'unsigned long wrapping');
      }
      nativeCue.text='\uD800x\uDC00';assert(cue.text==='\uD800x\uDC00','text UTF-16');
      nativeCue.id='\uD800';assert(cue.id==='\uD800','id UTF-16');
      nativeRegion.id='\uDC00';assert(region.id==='\uDC00','region id UTF-16');
      const track=document.createElement('video').addTextTrack('subtitles');
      const other=new VTTCue(0,1,'\uDC00');other.id='\uDC00';
      track.addCue(nativeCue);track.addCue(other);
      assert(track.cues.getCueById('\uD800')===nativeCue && track.cues.getCueById('\uDC00')===other,'distinct surrogate identifiers');
      throws(() => track.cues.getCueById(),TypeError,'required cue id');
      let events=0;EventTarget.prototype.addEventListener.call(nativeCue,'custom',()=>events++);
      EventTarget.prototype.dispatchEvent.call(nativeCue,new Event('custom'));assert(events===1,'native cue EventTarget');
      const htmlCue=new VTTCue(0,1,'<b>private text</b>');
      Object.defineProperty(htmlCue,'text',{get(){throw Error('author text getter');}});
      assert(VTTCue.prototype.getCueAsHTML.call(htmlCue).textContent==='private text','getCueAsHTML uses native state');
      const foreignCue=new foreign.VTTCue(0,1,'<b>realm text</b>'), foreignDocument=foreign.document;
      for (const method of [VTTCue.prototype.getCueAsHTML,foreign.VTTCue.prototype.getCueAsHTML]) {
        const fragment=method.call(foreignCue);
        assert(fragment.ownerDocument===foreignDocument && fragment.firstChild.ownerDocument===foreignDocument,'fragment relevant Document');
        assert(Object.getPrototypeOf(fragment)===foreign.DocumentFragment.prototype,'fragment relevant prototype');
      }
      document.querySelector('iframe').remove();
      const retainedFragment=VTTCue.prototype.getCueAsHTML.call(foreignCue);
      assert(retainedFragment.ownerDocument===foreignDocument && retainedFragment.textContent==='realm text','retired cue realm Document');
      return true;
    })()"#).unwrap(), "true");
}
