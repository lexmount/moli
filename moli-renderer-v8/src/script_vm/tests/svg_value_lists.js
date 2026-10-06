(() => {
  const ns='http://www.w3.org/2000/svg', child=document.querySelector('iframe').contentWindow;
  const checks=[];
  function check(name,fn){try{checks.push({name,passed:fn()===true});}catch(e){checks.push({name,passed:false,error:String(e)});}}
  function throws(fn,C,name){try{fn();return false;}catch(e){return e instanceof C && (!name || e.name===name);}}
  const methods={clear:0,initialize:1,getItem:1,insertItemBefore:2,replaceItem:2,removeItem:1,appendItem:1};
  for (const realm of [window,child]) {
    for (const kind of ['live','html','xml']) {
      const doc=kind==='live'?realm.document:kind==='html'?realm.document.implementation.createHTMLDocument(''):new realm.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml');
      for (const [tag,attr,type] of [['text','x','Length'],['text','y','Length'],['text','dx','Length'],['text','dy','Length'],['text','rotate','Number'],['feFuncR','tableValues','Number'],['polygon','points','Point'],['polyline','points','Point']]) {
        const I='SVG'+type+'List', V=type==='Point'?'DOMPoint':'SVG'+type;
        const label=(realm===window?'main':'child')+'/'+kind+'/'+tag+'/'+attr;
        const raw=type==='Length'?'10px 20px':type==='Point'?'10 20 30 40':'10 20';
        const replacement=type==='Length'?'30cm 40mm':type==='Point'?'50 60 70 80':'30 40';
        const value=item=>type==='Point'?item.x:type==='Length'?item.valueInSpecifiedUnits:item.value;
        const write=(item,v)=>{if(type==='Point')item.x=v;else if(type==='Length')item.valueInSpecifiedUnits=v;else item.value=v;};
        function setup(){
          const root=doc.createElementNS(ns,'svg'), e=doc.createElementNS(ns,tag);
          root.setAttribute('width','100');root.setAttribute('height','200');root.appendChild(e);e.setAttribute(attr,raw);
          const animated=type==='Point'?null:e[attr];
          return {root,e,animated,base:animated?animated.baseVal:e.points,anim:animated?animated.animVal:e.animatedPoints};
        }
        function detached(root,v=5){const item=type==='Length'?root.createSVGLength():type==='Number'?root.createSVGNumber():new realm.DOMPoint(v,6,7,8);write(item,v);return item;}
        const tests={
          'identity':()=>{const {e,base,anim}=setup();return (type==='Point'?e.points===base&&e.animatedPoints===anim:e[attr].baseVal===base&&e[attr].animVal===anim) && base!==anim;},
          'brands':()=>{const {base,anim}=setup();return base instanceof realm[I] && anim instanceof realm[I] && base[0] instanceof realm[V] && anim[0] instanceof realm[V];},
          'items':()=>{const {base}=setup();return base.length===2 && base.numberOfItems===2 && base[0]===base.getItem(0) && value(base[0])===10 && value(base[1])===(type==='Point'?30:20);},
          'index/descriptor':()=>{const {base}=setup(),d=Object.getOwnPropertyDescriptor(base,'0');return d.value===base.getItem(0)&&d.enumerable&&d.writable&&d.configurable;},
          'index/outside':()=>{const {base}=setup();return base[2]===undefined && !Object.hasOwn(base,'2') && throws(()=>base.getItem(2),realm.DOMException,'IndexSizeError');},
          'sync/identity':()=>{const {e,base,anim}=setup(),a=base[0],b=base[1],aa=anim[0];e.setAttribute(attr,replacement);return base[0]===a&&base[1]===b&&anim[0]===aa&&value(a)===(type==='Point'?50:30)&&value(b)===(type==='Point'?70:40);},
          'sync/retained-direct':()=>{const {e,base,anim}=setup(),a=base[0],aa=anim[0];e.setAttribute(attr,replacement);return value(a)===(type==='Point'?50:30)&&value(aa)===(type==='Point'?50:30);},
          'sync/write-current':()=>{const {e,base,anim}=setup(),a=base[0];e.setAttribute(attr,replacement);write(a,9);return value(base[0])===9 && value(base[1])===(type==='Point'?70:40) && value(anim[0])===9 && (type!=='Point'||a.y===60);},
          'sync/grow':()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,type==='Point'?'50 60 70 80 90 100':'30 40 50');return base.length===3&&base[0]===a&&value(base[2])===(type==='Point'?90:50);},
          'sync/truncate':()=>{const {e,base}=setup(),a=base[1];e.setAttribute(attr,type==='Point'?'50 60':'30');return base.length===1 && (write(a,9),value(base[0])===(type==='Point'?50:30));},
          'sync/invalid':()=>{const {e,base,anim}=setup(),a=base[0];e.setAttribute(attr,'invalid');return base.length===0&&anim.length===0 && (write(a,9),base.length===0);},
          'sync/remove-attribute':()=>{const {e,base,anim}=setup();e.removeAttribute(attr);return base.length===0&&anim.length===0;},
          'mutation/item-reflection':()=>{const {e,base,anim}=setup(),a=base[0];write(a,9);const clone=e.cloneNode(false);return value(base[0])===9&&value(anim[0])===9&&value(type==='Point'?clone.points[0]:clone[attr].baseVal[0])===9;},
          'mutation/clear':()=>{const {base,anim}=setup(),a=base[0];return base.clear()===undefined&&base.length===0&&anim.length===0&&(write(a,9),base.length===0);},
          'mutation/initialize-self':()=>{const {base}=setup(),a=base[0];return base.initialize(a)===a&&base.length===1&&base[0]===a;},
          'mutation/attached-copy':()=>{const a=setup(),b=setup(),item=a.base[0],copy=b.base.appendItem(item);write(item,9);return copy!==item&&a.base.length===2&&b.base.length===3&&value(copy)===10&&value(item)===9;},
          'mutation/readonly-copy':()=>{const a=setup(),b=setup(),item=a.anim[0],copy=b.base.appendItem(item);write(copy,9);return copy!==item&&value(item)===10&&value(b.base[2])===9;},
          'mutation/self-append-copy':()=>{const {base}=setup(),a=base[0],copy=base.appendItem(a);write(a,9);return copy!==a&&base.length===3&&base[0]===a&&value(base[2])===10;},
          'mutation/append-detached':()=>{const {root,base}=setup(),a=detached(root);return base.appendItem(a)===a&&base[2]===a;},
          'mutation/insert-clamp':()=>{const {root,base}=setup(),a=detached(root);return base.insertItemBefore(a,0xffffffff)===a&&base[2]===a;},
          'mutation/replace-detaches':()=>{const {root,base}=setup(),old=base[0],a=detached(root);return base.replaceItem(a,0)===a&&base[0]===a&&(write(old,9),value(a)===5);},
          'mutation/remove-detaches':()=>{const {base}=setup(),a=base[0];return base.removeItem(0)===a&&base.length===1&&(write(a,9),value(base[0])===(type==='Point'?30:20));},
          'mutation/index-replace':()=>{const {root,base}=setup(),a=detached(root);base[0]=a;return base[0]===a&&base.length===2;},
          'mutation/index-range':()=>{const {root,base}=setup();return throws(()=>base[2]=detached(root),window.DOMException,'IndexSizeError');},
          'readonly/list':()=>{const {anim}=setup();return throws(()=>anim.clear(),realm.DOMException,'NoModificationAllowedError')&&anim.length===2;},
          'readonly/item':()=>{const {anim}=setup();return throws(()=>write(anim[0],9),realm.DOMException,'NoModificationAllowedError')&&value(anim[0])===10;},
          'readonly/argument-before-state':()=>{const {anim}=setup();return throws(()=>anim.appendItem({}),realm.TypeError);},
          'readonly/index':()=>{const {root,anim}=setup();return throws(()=>anim[0]=detached(root),window.DOMException,'NoModificationAllowedError');},
          'readonly/detach':()=>{const {e,anim}=setup(),a=anim[1];e.setAttribute(attr,type==='Point'?'50 60':'30');return anim.length===1 && (write(a,9),value(a)===9) && value(anim[0])===(type==='Point'?50:30);},
          'cross-realm/attached-producer':()=>{const {base}=setup(),item=base[0],other=(realm===window?child:window).document,owner=other.createElementNS(ns,tag);owner.setAttribute(attr,raw);const dest=type==='Point'?owner.points:owner[attr].baseVal;const copy=dest.appendItem(item),otherRealm=realm===window?child:window;return copy!==item&&Object.getPrototypeOf(copy)===otherRealm[V].prototype&&value(copy)===10;},
          'cross-realm/detached-reuse':()=>{const {base}=setup(),otherRealm=realm===window?child:window,root=otherRealm.document.createElementNS(ns,'svg');const item=type==='Length'?root.createSVGLength():type==='Number'?root.createSVGNumber():new otherRealm.DOMPoint(1,2);return base.appendItem(item)===item&&Object.getPrototypeOf(item)===otherRealm[V].prototype;},
          'owner/clone-isolation':()=>{const {e,base}=setup(),clone=e.cloneNode(false),copy=type==='Point'?clone.points:clone[attr].baseVal;copy.clear();return base.length===2&&copy.length===0;},
        };
        if(type==='Length'){
          tests['length/sync-units']=()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,replacement);return a.unitType===6&&a.valueAsString==='30cm'&&base[1].unitType===7;};
          tests['length/float']=()=>{const {base}=setup(),a=base[0];a.valueInSpecifiedUnits=.1;return a.valueInSpecifiedUnits===Math.fround(.1)&&throws(()=>a.valueInSpecifiedUnits=1e40,realm.TypeError);};
          tests['length/new-units']=()=>{const {base,anim}=setup(),a=base[0];a.newValueSpecifiedUnits(6,.1);return a.valueInSpecifiedUnits===Math.fround(.1)&&a.unitType===6&&anim[0].unitType===6;};
          tests['length/convert-after-sync']=()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,'1in 2in');a.convertToSpecifiedUnits(5);return a.unitType===5&&a.valueInSpecifiedUnits===96&&base[1].valueInSpecifiedUnits===2;};
          tests['length/readonly-float-first']=()=>{const {anim}=setup();return throws(()=>anim[0].value=1e40,realm.TypeError)&&throws(()=>anim[0].newValueSpecifiedUnits(5,1e40),realm.TypeError);};
          tests['length/string-after-sync']=()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,'1in 2in');a.valueAsString='9mm';return a.valueAsString==='9mm'&&base[1].valueAsString==='2in';};
        }
        if(type==='Point'){
          tests['point/sync-all-coordinates']=()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,replacement);return a.x===50&&a.y===60&&a.z===0&&a.w===1;};
          tests['point/json-sync']=()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,replacement);const json=a.toJSON();return json.x===50&&json.y===60;};
          tests['point/matrix-sync']=()=>{const {e,base}=setup(),a=base[0];e.setAttribute(attr,replacement);const copy=a.matrixTransform();return copy.x===50&&copy.y===60&&copy instanceof realm.DOMPoint;};
          tests['point/extra-coordinate-copy']=()=>{const a=setup(),b=setup(),p=a.base[0];p.z=7;p.w=8;const copy=b.base.appendItem(p);return copy!==p&&copy.x===10&&copy.y===20&&copy.z===7&&copy.w===8;};
        }
        for(const [name,fn] of Object.entries(tests))check(label+'/'+name,fn);
        for(const [method,length] of Object.entries(methods))check(label+'/descriptor/'+method,()=>{
          const {base,anim}=setup(),d=Object.getOwnPropertyDescriptor(realm[I].prototype,method);
          return !Object.hasOwn(base,method)&&!Object.hasOwn(anim,method)&&base[method]===d.value&&d.value.name===method&&d.value.length===length&&d.enumerable&&d.configurable&&d.writable;
        });
        for(const callee of [window,child]){
          const which=callee===window?'main':'child';
          for(const member of ['length','numberOfItems'])check(label+'/receiver/'+which+'/'+member,()=>{
            const {base}=setup(),get=Object.getOwnPropertyDescriptor(callee[I].prototype,member).get;
            return get.call(base)===2&&throws(()=>get.call(new Proxy(base,{})),callee.TypeError)&&throws(()=>get.call(Object.create(base)),callee.TypeError);
          });
          for(const method of Object.keys(methods)){
            for(const invalid of ['plain','prototype','inherits-real','proxy','revoked','wrong-brand'])check(label+'/receiver/'+which+'/'+method+'/'+invalid,()=>{
              const {root,base}=setup();let conversions=0,traps=0,receiver;
              const poisoned={valueOf(){conversions++;return 0;}};
              if(invalid==='plain')receiver={};else if(invalid==='prototype')receiver=Object.create(callee[I].prototype);else if(invalid==='inherits-real')receiver=Object.create(base);else if(invalid==='proxy')receiver=new Proxy(base,{get(){traps++;throw Error('trap')}});else if(invalid==='revoked'){const p=Proxy.revocable(base,{});p.revoke();receiver=p.proxy;}else receiver=detached(root);
              const args=method==='getItem'||method==='removeItem'?[poisoned]:[detached(root),poisoned];
              return throws(()=>callee[I].prototype[method].apply(receiver,args),callee.TypeError)&&conversions===0&&traps===0;
            });
          }
          for(const invalid of ['plain','inherits-real','proxy','revoked','readonly-point','wrong-brand'])check(label+'/argument/'+which+'/'+invalid,()=>{
            const {root,base}=setup(),real=detached(root);let input,traps=0,conversions=0;
            if(invalid==='plain')input={};else if(invalid==='inherits-real')input=Object.create(real);else if(invalid==='proxy')input=new Proxy(real,{get(){traps++;throw Error('trap')}});else if(invalid==='revoked'){const p=Proxy.revocable(real,{});p.revoke();input=p.proxy;}else if(invalid==='readonly-point')input=new realm.DOMPointReadOnly(1,2);else input=root.createSVGTransform();
            return throws(()=>callee[I].prototype.insertItemBefore.call(base,input,{valueOf(){conversions++;return 0;}}),callee.TypeError)&&conversions===0&&traps===0&&base.length===2;
          });
        }
      }
    }
  }
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(r=>r.passed).length,checks};
  return checks.every(r=>r.passed);
})();
