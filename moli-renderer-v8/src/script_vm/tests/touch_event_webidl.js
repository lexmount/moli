(() => {
  const checks = [];
  const check = (name, callback) => {
    try { checks.push({name, passed: callback() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const raises = (realm, callback) => {
    try { callback(); } catch (error) {
      return Object.getPrototypeOf(error) === realm.TypeError.prototype;
    }
    return false;
  };
  const realms = [window, document.querySelector('iframe').contentWindow];
  const touchProperties = ['identifier','target','screenX','screenY','clientX','clientY',
    'pageX','pageY','radiusX','radiusY','rotationAngle','force','altitudeAngle','azimuthAngle','touchType'];
  const lists = ['touches','targetTouches','changedTouches'];
  const flags = ['altKey','ctrlKey','metaKey','shiftKey'];
  const modifiers = ['Alt','Control','Meta','AltGraph','CapsLock','Fn','FnLock','Hyper',
    'NumLock','ScrollLock','Super','Symbol','SymbolLock','Shift'];
  const modifierMembers = ['altKey','ctrlKey','metaKey','modifierAltGraph','modifierCapsLock',
    'modifierFn','modifierFnLock','modifierHyper','modifierNumLock','modifierScrollLock',
    'modifierSuper','modifierSymbol','modifierSymbolLock','shiftKey'];
  const eventOrder = ['bubbles','cancelable','composed','detail','view','which',
    ...modifierMembers,'changedTouches','targetTouches','touches'];
  const touchOrder = ['altitudeAngle','azimuthAngle','clientX','clientY','force','identifier',
    'pageX','pageY','radiusX','radiusY','rotationAngle','screenX','screenY','target','touchType'];

  for (const [ownerIndex, owner] of realms.entries()) {
    const target = owner.document.createElement('div');
    const t = new owner.Touch({identifier: 12, target, clientX: 2.5, force: .3});
    const other = new realms[1-ownerIndex].Touch({identifier: 13, target: document});
    const make = init => new owner.TouchEvent('touchstart', init);
    const p = 'owner ' + ownerIndex + ' ';
    check(p+'default event', () => {
      const e=make();
      return e.type==='touchstart' && !e.bubbles && !e.cancelable && !e.composed &&
        !e.isTrusted && e.view===null && e.detail===0 && e.which===0 &&
        lists.every(key=>e[key] instanceof owner.TouchList && e[key].length===0 && e[key].item(0)===null) &&
        flags.every(key=>e[key]===false);
    });
    check(p+'Touch defaults and float rounding', () => t.identifier===12 && t.target===target &&
      t.force===Math.fround(.3) && t.radiusX===0 && t.radiusY===0 && t.rotationAngle===0 &&
      t.altitudeAngle===0 && t.azimuthAngle===0 && t.touchType==='direct');
    check(p+'complete TouchInit', () => {
      const x = new owner.Touch({identifier:4294967297.9,target,clientX:-0,clientY:3.25,
        pageX:4,pageY:5,screenX:6,screenY:7,radiusX:.1,radiusY:.2,rotationAngle:.3,
        force:.4,altitudeAngle:.5,azimuthAngle:.6,touchType:'stylus'});
      return x.identifier===1 && x.target===target && Object.is(x.clientX,-0) &&
        x.clientY===3.25 && x.pageX===4 && x.pageY===5 && x.screenX===6 && x.screenY===7 &&
        x.radiusX===Math.fround(.1) && x.radiusY===Math.fround(.2) &&
        x.rotationAngle===Math.fround(.3) && x.force===Math.fround(.4) &&
        x.altitudeAngle===.5 && x.azimuthAngle===.6 && x.touchType==='stylus';
    });
    check(p+'Touch getter order', () => {
      const seen=[];
      new owner.Touch(new Proxy({identifier:1,target},{get(o,k){seen.push(String(k));return o[k];}}));
      return same(seen,touchOrder);
    });
    for (const key of ['clientX','clientY','pageX','pageY','screenX','screenY',
      'radiusX','radiusY','rotationAngle','force','altitudeAngle','azimuthAngle']) {
      for (const [label,value] of [['nan',NaN],['infinite',Infinity],['negative infinite',-Infinity],
        ['symbol',Symbol()],['bigint',1n]]) {
        check(p+'Touch '+key+' rejects '+label,()=>raises(owner,()=>new owner.Touch({identifier:1,target,[key]:value})));
      }
      check(p+'Touch '+key+' preserves signed zero',()=>Object.is(new owner.Touch({identifier:1,target,[key]:-0})[key],-0));
      check(p+'Touch '+key+' getter exception identity',()=>{
        const sentinel=new RangeError('touch value'), init={identifier:1,target};
        Object.defineProperty(init,key,{get(){throw sentinel;}});
        try { new owner.Touch(init); } catch(error) {return error===sentinel;}
        return false;
      });
    }
    for (const key of ['force','radiusX','radiusY','rotationAngle']) {
      check(p+'Touch '+key+' float overflow',()=>raises(owner,()=>new owner.Touch({identifier:1,target,[key]:1e100})));
    }
    for (const value of ['', 'Direct', 'mouse', null, 1, Symbol()]) {
      check(p+'TouchType rejects '+String(value),()=>raises(owner,()=>new owner.Touch({identifier:1,target,touchType:value})));
    }
    check(p+'TouchType coercion once',()=>{
      let count=0;const x=new owner.Touch({identifier:1,target,touchType:{toString(){count++;return 'stylus';}}});
      return count===1 && x.touchType==='stylus';
    });
    for (const value of [0,1,'text',true,Symbol(),1n]) {
      check(p+'event dictionary rejects '+String(value),()=>raises(owner,()=>make(value)));
    }
    for (const value of [null,undefined]) {
      check(p+'event nullish dictionary '+String(value),()=>make(value).touches.length===0);
    }
    check(p+'event getter order',()=>{
      const seen=[];
      make(new Proxy({},{get(o,k){seen.push(String(k));return undefined;}}));
      return same(seen,eventOrder);
    });
    check(p+'event type before dictionary',()=>{
      const seen=[];new owner.TouchEvent({toString(){seen.push('type');return 'x';}},
        new Proxy({},{get(o,k){seen.push(String(k));return undefined;}}));
      return same(seen,['type',...eventOrder]);
    });
    for (const key of eventOrder) {
      check(p+'event getter throws '+key,()=>{
        const sentinel=new RangeError(key),seen=[],init=new Proxy({},
          {get(o,k){seen.push(String(k));if(k===key)throw sentinel;return undefined;}});
        try {make(init);} catch(error) {return error===sentinel && same(seen,eventOrder.slice(0,eventOrder.indexOf(key)+1));}
        return false;
      });
    }
    for (const key of lists) {
      for (const [label, source] of [
        ['array',()=>[t,other]],['set',()=>new Set([t,other])],
        ['generator',()=>function*(){yield t;yield other;}()],
        ['TouchList',()=>make({touches:[t,other]}).touches],
        ['custom iterator',()=>({*[Symbol.iterator](){yield t;yield other;}})]]) {
        check(p+key+' sequence '+label,()=>{
          const e=make({[key]:source()});
          return e[key].length===2 && e[key][0]===t && e[key][1]===other &&
            e[key].item(1)===other && same(Array.from(e[key]),[t,other]);
        });
      }
      for (const [label,value] of [['null',null],['number',1],['string',''],['array-like',{0:t,length:1}],
        ['non-callable iterator',{[Symbol.iterator]:1}],['bad touch',[{}]],
        ['forged touch',[Object.create(owner.Touch.prototype)]],['inherited brand',[Object.create(t)]],
        ['author proxy',[new Proxy(t,{})]]]) {
        check(p+key+' rejects '+label,()=>raises(owner,()=>make({[key]:value})));
      }
      check(p+key+' revoked Touch rejects',()=>{
        const x=Proxy.revocable(t,{});x.revoke();return raises(owner,()=>make({[key]:[x.proxy]}));
      });
      check(p+key+' rejects Touch without invoking traps',()=>{
        let traps=0;const x=new Proxy(t,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
        return raises(owner,()=>make({[key]:[x]})) && traps===0;
      });
      check(p+key+' preserves iterator getter exception',()=>{
        const sentinel=new RangeError('iterator'),source={get [Symbol.iterator](){throw sentinel;}};
        try {make({[key]:source});}catch(error){return error===sentinel;}return false;
      });
      check(p+key+' iterator completion order',()=>{
        const log=[];let n=0;
        const source={get length(){throw 42;},get [Symbol.iterator](){log.push('iterator');
          return function(){log.push('call');return {get next(){log.push('next');return function(){
            log.push('step');return n++===0 ? {get done(){log.push('done');return false;},get value(){log.push('value');return t;}} : {done:true};
          };}};};}};
        const list=make({[key]:source})[key];
        return list.length===1 && list[0]===t && same(log,['iterator','call','next','step','done','value','step']);
      });
      check(p+key+' invalid item stops iterator without closing',()=>{
        let steps=0,closed=0;const source={[Symbol.iterator](){return {
          next(){steps++;return {done:false,value:steps===1 ? t : {}};},return(){closed++;return {};}};}};
        return raises(owner,()=>make({[key]:source})) && steps===2 && closed===0;
      });
      check(p+key+' iterator next exception identity',()=>{
        const sentinel=new RangeError('next'),source={[Symbol.iterator](){return {next(){throw sentinel;}};}};
        try {make({[key]:source});}catch(error){return error===sentinel;}return false;
      });
      check(p+key+' snapshot source',()=>{
        const source=[t,other],e=make({[key]:source});source[0]=other;source.length=0;
        return e[key].length===2 && e[key][0]===t && e[key][1]===other;
      });
    }
    check(p+'sequences convert before later getters',()=>{
      const log=[],init={get changedTouches(){log.push('changed');return {get [Symbol.iterator](){log.push('iterator');throw 42;}};},
        get targetTouches(){log.push('target');return [];},get touches(){log.push('touches');return [];}};
      try{make(init);}catch(error){return error===42 && same(log,['changed','iterator']);}return false;
    });
    check(p+'typed UIEvent state',()=>{
      const e=make({view:realms[1-ownerIndex],detail:4294967297.9,which:-1,bubbles:1,cancelable:1,composed:1});
      return e.view===realms[1-ownerIndex] && e.detail===1 && e.which===4294967295 &&
        e.bubbles && e.cancelable && e.composed;
    });
    for (const view of [{},target,new Proxy(owner,{})]) {
      check(p+'invalid view '+String(view),()=>raises(owner,()=>make({view})));
    }
    for (const key of ['detail','which']) {
      check(p+key+' symbol rejects',()=>raises(owner,()=>make({[key]:Symbol()})));
    }
    check(p+'modifier states',()=>{
      const init=Object.fromEntries(modifierMembers.map(key=>[key,true]));const e=make(init);
      return flags.every(key=>e[key]) && modifiers.every(key=>e.getModifierState(key)) &&
        !e.getModifierState('unknown') && !e.getModifierState('alt');
    });
    check(p+'modifier defaults',()=>modifiers.every(key=>make().getModifierState(key)===false));
    for (const [calleeIndex,callee] of realms.entries()) {
      const q=p+'callee '+calleeIndex+' ',e=make({touches:[t,other],altKey:true,modifierAltGraph:true});
      const list=e.touches;
      for (const [interfaceName,receiver,properties] of [['Touch',t,touchProperties],
        ['TouchEvent',e,[...lists,...flags]],['TouchList',list,['length']]]) {
        for (const key of properties) {
          check(q+interfaceName+'.'+key+' descriptor and native state',()=>{
            const d=Object.getOwnPropertyDescriptor(callee[interfaceName].prototype,key);
            return d && d.enumerable && d.configurable && d.set===undefined &&
              d.get.length===0 && d.get.name==='get '+key && !Object.hasOwn(receiver,key) &&
              Object.is(d.get.call(receiver),receiver[key]);
          });
          for (const [label,fake] of [['ordinary',{}],['prototype',Object.create(callee[interfaceName].prototype)],
            ['inherited',Object.create(receiver)],['proxy',new Proxy(receiver,{})]]) {
            check(q+interfaceName+'.'+key+' rejects '+label,()=>raises(callee,
              ()=>Object.getOwnPropertyDescriptor(callee[interfaceName].prototype,key).get.call(fake)));
          }
          check(q+interfaceName+'.'+key+' revoked',()=>{
            const x=Proxy.revocable(receiver,{});x.revoke();return raises(callee,
              ()=>Object.getOwnPropertyDescriptor(callee[interfaceName].prototype,key).get.call(x.proxy));
          });
        }
      }
      for (const [interfaceName,method,receiver] of [['TouchList','item',list],['TouchEvent','getModifierState',e]]) {
        check(q+method+' descriptor',()=>{
          const d=Object.getOwnPropertyDescriptor(callee[interfaceName].prototype,method);
          return d && d.enumerable && d.configurable && d.writable && d.value.length===1 && d.value.name===method;
        });
        for (const [label,fake] of [['ordinary',{}],['inherited',Object.create(receiver)],
          ['proxy',new Proxy(receiver,{})]]) {
          check(q+method+' receiver before coercion '+label,()=>{
            let conversions=0;return raises(callee,()=>callee[interfaceName].prototype[method].call(fake,
              {valueOf(){conversions++;throw 42;},toString(){conversions++;throw 42;}})) && conversions===0;
          });
        }
        check(q+method+' requires argument',()=>raises(callee,()=>callee[interfaceName].prototype[method].call(receiver)));
      }
      check(q+'TouchList real cross-realm',()=>callee.TouchList.prototype.item.call(list,1)===other &&
        callee.TouchList.prototype.item.call(list,4294967296.9)===t &&
        callee.TouchList.prototype.item.call(list,-1)===null && list[2]===undefined);
      check(q+'TouchList indexed descriptors',()=>[0,1].every(index=>{
        const d=Object.getOwnPropertyDescriptor(list,String(index));
        return d.value===(index===0 ? t : other) && !d.writable && d.enumerable && d.configurable;
      }));
      check(q+'TouchList readonly indexed set',()=>!Reflect.set(list,0,other) && list[0]===t && list.item(0)===t);
      check(q+'TouchList cannot delete supported index',()=>!Reflect.deleteProperty(list,0) && list[0]===t);
      check(q+'TouchList refuses index definition',()=>!Reflect.defineProperty(list,0,{value:other}) &&
        !Reflect.defineProperty(list,4,{value:t}) && !Reflect.defineProperty(list,0,{get(){return other;}}) && list[0]===t);
      check(q+'TouchList unsupported index remains absent',()=>!Reflect.set(list,4,t) &&
        Reflect.deleteProperty(list,4) && list[4]===undefined && !Object.hasOwn(list,4));
      check(q+'TouchList own keys',()=>same(Object.keys(list),['0','1']));
      check(q+'TouchList stays extensible',()=>!Reflect.preventExtensions(list) && Object.isExtensible(list));
      check(q+'TouchList ordinary expandos',()=>{
        list.extra=9;list['01']=10;list['4294967295']=11;
        return list.extra===9 && list['01']===10 && list['4294967295']===11 && list.length===2;
      });
      check(q+'TouchList item ignores author length',()=>{
        Object.defineProperty(list,'length',{value:0,configurable:true});
        return callee.TouchList.prototype.item.call(list,1)===other &&
          Object.getOwnPropertyDescriptor(callee.TouchList.prototype,'length').get.call(list)===2;
      });
    }
    check(p+'intrinsic TouchList after public replacement',()=>{
      const original=owner.TouchList;try {owner.TouchList=function(){throw 42;};
        return make({touches:[t]}).touches instanceof original;
      }finally{owner.TouchList=original;}
    });
    check(p+'event native dispatch',()=>{
      const e=make({touches:[t],cancelable:true});let count=0;
      target.addEventListener('touchstart',event=>{count++;if(event===e && event.touches[0]===t)event.preventDefault();});
      return target.dispatchEvent(e)===false && count===1 && e.defaultPrevented;
    });
  }
  globalThis.__touchEventWebIdlResults={complete:true,total:checks.length,
    passed:checks.filter(row=>row.passed).length,checks};
  globalThis.__uiEventResults=__touchEventWebIdlResults;
  return true;
})()
