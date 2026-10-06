(() => {
  const ns = 'http://www.w3.org/2000/svg', checks = [];
  const check = (name, fn) => { try { const passed = fn() === true; checks.push({name,passed}); }
    catch (e) { checks.push({name,passed:false,error:String(e)}); } };
  const throws = (fn, Type, name) => { try { fn(); } catch(e) { return (!Type || e instanceof Type) && (!name || e.name === name); } return false; };
  const child = document.querySelector('iframe').contentWindow;
  const definitions = [['g','SVGGraphicsElement','transform'],['text','SVGGraphicsElement','transform'],
    ['rect','SVGGraphicsElement','transform'],['pattern','SVGPatternElement','patternTransform'],
    ['linearGradient','SVGGradientElement','gradientTransform'],['radialGradient','SVGGradientElement','gradientTransform'],
    ['clipPath','SVGClipPathElement','transform']];
  globalThis.__svgTransformDefinitions = definitions;
  const matrices = m => ['a','b','c','d','e','f'].map(k=>m[k]);
  const near = (a,b) => a.length===b.length && a.every((v,i)=>Math.abs(v-b[i])<=1e-10*Math.max(1,Math.abs(b[i])));
  for (const [realmName, realm] of [['main',window],['child',child]]) {
    for (const [docName,doc] of [['live',realm.document],['html',realm.document.implementation.createHTMLDocument('')],
      ['xml',new realm.DOMParser().parseFromString('<root/>','application/xml')]]) {
      for (const [tag,interfaceName,attribute] of definitions) {
        const label=realmName+'/'+docName+'/'+tag;
        function owner() { const e=doc.createElementNS(ns,tag);e.setAttribute(attribute,'translate(2 3) scale(4 5)');return e; }
        function setup() {const e=owner(), animated=e[attribute];return {e,animated,base:animated.baseVal,anim:animated.animVal,root:doc.createElementNS(ns,'svg')};}
        const tests = {
          'owner/descriptor':()=>{const d=Object.getOwnPropertyDescriptor(realm[interfaceName].prototype,attribute);return typeof d.get==='function' && d.enumerable && d.configurable && !d.set;},
          'owner/cache':()=>{const {e,animated,base,anim}=setup();return e[attribute]===animated && animated.baseVal===base && animated.animVal===anim && base!==anim;},
          'owner/producer':()=>{const e=owner(), get=Object.getOwnPropertyDescriptor(window[interfaceName].prototype,attribute).get,v=get.call(e);return Object.getPrototypeOf(v)===realm.SVGAnimatedTransformList.prototype && Object.getPrototypeOf(v.baseVal)===realm.SVGTransformList.prototype;},
          'list/initial':()=>{const {base,anim}=setup();return base.length===2 && base.numberOfItems===2 && anim.length===2 && base.getItem(0).type===2 && base.getItem(1).type===3 && near(matrices(base.getItem(0).matrix),[1,0,0,1,2,3]);},
          'list/same-item':()=>{const {base}=setup(),t=base.getItem(0);return base.getItem(0)===t && base[0]===t;},
          'list/indices':()=>{const {base}=setup(),d=Object.getOwnPropertyDescriptor(base,'0');return Object.keys(base).filter(k=>/^\d+$/.test(k)).join(',')==='0,1' && 0 in base && !(2 in base) && base[2]===undefined && d.value===base.getItem(0) && d.enumerable && d.configurable && d.writable;},
          'list/delete':()=>{const {base}=setup();return Reflect.deleteProperty(base,'0')===false && base.length===2;},
          'list/sync':()=>{const {e,base,anim}=setup();e.setAttribute(attribute,'scale(6 7)');return base.length===1 && anim.length===1 && base[0].type===3 && base[0].matrix.a===6 && anim[0].matrix.d===7;},
          'list/item-sync':()=>{const {e,base}=setup(),t=base.getItem(0),m=t.matrix;e.setAttribute(attribute,'scale(6 7)');return t.type===3 && t.matrix===m && m.a===6 && m.d===7 && base.getItem(0)===t;},
          'list/matrix-sync-first':()=>{const {e,base}=setup(),m=base.getItem(0).matrix;e.setAttribute(attribute,'scale(6 7)');return m.a===6 && m.d===7;},
          'list/grow':()=>{const {e,base}=setup(),t=base.getItem(0);e.setAttribute(attribute,'translate(7 8) scale(2 3) skewX(45)');return base.length===3 && base.getItem(0)===t && t.matrix.e===7 && base[2].type===5;},
          'list/truncate':()=>{const {e,base}=setup(),t=base.getItem(1),m=t.matrix;e.setAttribute(attribute,'translate(8 9)');return base.length===1 && t.type===3 && m.a===4 && (t.setTranslate(20,30),base.length===1 && base[0].matrix.e===8);},
          'list/invalid-clears':()=>{const {e,base,anim}=setup();e.setAttribute(attribute,'nonsense');return base.length===0 && anim.length===0 && base[0]===undefined && throws(()=>base.getItem(0),realm.DOMException,'IndexSizeError');},
          'list/remove-attribute':()=>{const {e,base,anim}=setup();e.removeAttribute(attribute);return base.length===0 && anim.length===0;},
          'list/write-after-sync':()=>{const {e,base,root}=setup();e.setAttribute(attribute,'scale(6 7)');const t=root.createSVGTransform();t.setTranslate(20,30);base.appendItem(t);return base.length===2 && base[0].matrix.a===6 && base[1]===t && base[1].matrix.e===20;},
          'list/clear':()=>{const {e,base,anim}=setup();return base.clear()===undefined && base.length===0 && anim.length===0 && e.getAttribute(attribute)==='';},
          'list/initialize-self':()=>{const {base}=setup(),t=base.getItem(0);return base.initialize(t)===t && base.length===1 && base[0]===t && t.matrix.e===2;},
          'list/attached-copy':()=>{const a=setup(),b=setup(),t=a.base.getItem(0),copy=b.base.appendItem(t);t.setTranslate(20,30);return copy!==t && a.base.length===2 && b.base.length===3 && copy.matrix.e===2 && a.base[0].matrix.e===20;},
          'list/anim-copy':()=>{const a=setup(),b=setup(),t=a.anim.getItem(0),copy=b.base.appendItem(t);return copy!==t && copy.type===t.type && (copy.setTranslate(20,30),t.matrix.e===2 && b.base[2].matrix.e===20);},
          'list/self-append-copy':()=>{const {base}=setup(),t=base.getItem(0),copy=base.appendItem(t);t.setTranslate(20,30);return base.length===3 && copy!==t && copy.matrix.e===2 && base[0]===t;},
          'list/insert-clamp':()=>{const {base,root}=setup(),t=root.createSVGTransform();t.setTranslate(10,11);return base.insertItemBefore(t,0xffffffff)===t && base.length===3 && base[2]===t;},
          'list/replace-detaches':()=>{const {base,root}=setup(),old=base.getItem(0),t=root.createSVGTransform();t.setTranslate(10,11);base.replaceItem(t,0);old.setTranslate(30,40);return base.length===2 && base[0]===t && t.matrix.e===10;},
          'list/remove-detaches':()=>{const {base}=setup(),t=base.getItem(0);return base.removeItem(0)===t && base.length===1 && (t.setTranslate(30,40),base[0].type===3);},
          'list/index-replace':()=>{const {base,root}=setup(),t=root.createSVGTransform();t.setScale(10,11);base[0]=t;return base.length===2 && base.getItem(0)===t && base[0].matrix.a===10;},
          'list/index-out-of-range':()=>{const {base,root}=setup();return throws(()=>base[2]=root.createSVGTransform(),window.DOMException,'IndexSizeError') && base.length===2;},
          'list/index-producer-caller':()=>{const {base,root}=setup(),assign=realm.Function('list','item','list[2] = item');return throws(()=>assign(base,root.createSVGTransform()),realm.DOMException,'IndexSizeError') && base.length===2;},
          'list/consolidate':()=>{const {base,anim}=setup(),old=base.getItem(0),t=base.consolidate();return base.length===1 && t===base[0] && t.type===1 && near(matrices(t.matrix),[4,0,0,5,2,3]) && anim.length===1 && (old.setTranslate(30,40),t.matrix.e===2);},
          'list/consolidate-empty':()=>{const {base}=setup();base.clear();return base.consolidate()===null;},
          'list/factory':()=>{const {base,root}=setup(),t=base.createSVGTransformFromMatrix({a:2,d:3,e:4,f:5});return t.type===1 && near(matrices(t.matrix),[2,0,0,3,4,5]) && (t.setTranslate(20,30),base.length===2);},
          'transform/matrix-same-object':()=>{const {base}=setup(),t=base.getItem(0),m=t.matrix;t.setTranslate(10,11);t.setScale(2,3);return t.matrix===m && m.a===2 && m.d===3;},
          'transform/matrix-write':()=>{const {base,anim}=setup(),t=base.getItem(0),m=t.matrix;m.a=2;return t.type===1 && t.angle===0 && t.matrix===m && base[0].matrix.a===2 && anim[0].matrix.a===2;},
          'transform/set-matrix':()=>{const {base}=setup(),t=base.getItem(0),m=t.matrix;t.setMatrix({a:2,d:3,e:4,f:5});return t.type===1 && t.matrix===m && near(matrices(m),[2,0,0,3,4,5]);},
          'transform/float':()=>{const {base}=setup(),t=base.getItem(0);t.setTranslate(.1,16777217);return t.matrix.e===Math.fround(.1) && t.matrix.f===16777216 && throws(()=>t.setTranslate(1e40,0),realm.TypeError);},
          'transform/rotate':()=>{const {base,anim}=setup(),t=base.getItem(0);t.setRotate(90,3,4);return t.type===4 && t.angle===90 && near(matrices(t.matrix),[0,1,-1,0,7,1]) && anim[0].type===4 && near(matrices(anim[0].matrix),[0,1,-1,0,7,1]);},
          'transform/skew':()=>{const {base,anim}=setup(),t=base.getItem(0);t.setSkewX(45);return t.type===5 && t.angle===45 && Math.abs(t.matrix.c-1)<1e-10 && anim[0].type===5 && (t.setSkewY(45),t.type===6 && Math.abs(t.matrix.b-1)<1e-10);},
          'transform/clone-reflection':()=>{const {e,base}=setup(),t=base.getItem(0);t.setRotate(90,3,4);const clone=e.cloneNode(false);return clone[attribute].baseVal[0].type===4 && near(matrices(clone[attribute].baseVal[0].matrix),[0,1,-1,0,7,1]);},
          'readonly/clear':()=>{const {e,anim}=setup(),raw=e.getAttribute(attribute);return throws(()=>anim.clear(),realm.DOMException,'NoModificationAllowedError') && e.getAttribute(attribute)===raw && anim.length===2;},
          'readonly/initialize':()=>{const {anim,root}=setup();return throws(()=>anim.initialize(root.createSVGTransform()),realm.DOMException,'NoModificationAllowedError');},
          'readonly/append':()=>{const {anim,root}=setup();return throws(()=>anim.appendItem(root.createSVGTransform()),realm.DOMException,'NoModificationAllowedError');},
          'readonly/insert':()=>{const {anim,root}=setup();return throws(()=>anim.insertItemBefore(root.createSVGTransform(),0),realm.DOMException,'NoModificationAllowedError');},
          'readonly/replace':()=>{const {anim,root}=setup();return throws(()=>anim.replaceItem(root.createSVGTransform(),50),realm.DOMException,'NoModificationAllowedError');},
          'readonly/remove':()=>{const {anim}=setup();return throws(()=>anim.removeItem(50),realm.DOMException,'NoModificationAllowedError');},
          'readonly/consolidate':()=>{const {anim}=setup();return throws(()=>anim.consolidate(),realm.DOMException,'NoModificationAllowedError');},
          'readonly/index':()=>{const {anim,root}=setup();return throws(()=>anim[0]=root.createSVGTransform(),window.DOMException,'NoModificationAllowedError');},
          'readonly/index-producer-caller':()=>{const {anim,root}=setup(),assign=realm.Function('list','item','list[0] = item');return throws(()=>assign(anim,root.createSVGTransform()),realm.DOMException,'NoModificationAllowedError');},
          'readonly/argument-first':()=>{const {anim}=setup();return throws(()=>anim.appendItem({}),realm.TypeError);},
          'readonly/transform':()=>{const {anim}=setup(),t=anim.getItem(0);return throws(()=>t.setTranslate(3,4),realm.DOMException,'NoModificationAllowedError') && t.matrix.e===2;},
          'readonly/matrix':()=>{const {anim}=setup(),m=anim.getItem(0).matrix;return throws(()=>m.e=50,realm.DOMException,'NoModificationAllowedError') && m.e===2;},
          'readonly/detach-mutable':()=>{const {e,anim}=setup(),t=anim.getItem(1),m=t.matrix;e.setAttribute(attribute,'translate(7 8)');return anim.length===1 && (t.setTranslate(20,30),m.e===20) && (m.a=2,t.type===1 && anim[0].matrix.e===7);},
          'readonly/float-before-state':()=>{const {anim}=setup();return throws(()=>anim.getItem(0).setTranslate(1e40,0),realm.TypeError);},
          'owner/clone-isolated':()=>{const {e,base}=setup(),clone=e.cloneNode(false);clone[attribute].baseVal.clear();return base.length===2 && clone[attribute].baseVal.length===0;},
        };
        for (const [name,fn] of Object.entries(tests)) check(label+'/'+name,fn);
        for (const callee of [window,child]) {
          const which=callee===window?'main':'child';
          for (const member of ['length','numberOfItems']) check(label+'/receiver/'+which+'/'+member,()=>{
            const {base}=setup(),get=Object.getOwnPropertyDescriptor(callee.SVGTransformList.prototype,member).get;
            return get.call(base)===2 && throws(()=>get.call(new Proxy(base,{})),callee.TypeError) && throws(()=>get.call(Object.create(base)),callee.TypeError);
          });
          for (const method of ['clear','initialize','getItem','insertItemBefore','replaceItem','removeItem','appendItem','createSVGTransformFromMatrix','consolidate']) {
            for (const invalid of ['plain','prototype','inherits-real','proxy','revoked','wrong-brand']) check(label+'/receiver/'+which+'/'+method+'/'+invalid,()=>{
              const {base,root}=setup();let conversions=0,traps=0,receiver;
              const poisoned={valueOf(){conversions++;return 0},get a(){conversions++;return 1}};
              if(invalid==='plain')receiver={};
              else if(invalid==='prototype')receiver=Object.create(callee.SVGTransformList.prototype);
              else if(invalid==='inherits-real')receiver=Object.create(base);
              else if(invalid==='proxy')receiver=new Proxy(base,{get(){traps++;throw Error('trap')}});
              else if(invalid==='revoked'){const p=Proxy.revocable(base,{});p.revoke();receiver=p.proxy;}
              else receiver=root.createSVGTransform();
              const args=method==='getItem'||method==='removeItem'?[poisoned]:method==='createSVGTransformFromMatrix'?[poisoned]:[root.createSVGTransform(),poisoned];
              return throws(()=>callee.SVGTransformList.prototype[method].apply(receiver,args),callee.TypeError) && conversions===0 && traps===0;
            });
          }
          for (const method of ['setTranslate','setScale','setRotate','setSkewX','setSkewY','setMatrix']) check(label+'/transform-receiver/'+which+'/'+method,()=>{
            const {base}=setup(),t=base.getItem(0);let conversions=0,traps=0;const v={valueOf(){conversions++;return 0},get a(){conversions++;return 1}},p=new Proxy(t,{get(){traps++;throw Error('trap')}});
            return throws(()=>callee.SVGTransform.prototype[method].call(p,v,v,v),callee.TypeError) && conversions===0 && traps===0;
          });
        }
      }
    }
  }
  globalThis.__uiEventResults={complete:true,checks,total:checks.length,passed:checks.filter(r=>r.passed).length};
  return checks.every(r=>r.passed);
})();
