(() => {
  const ns = 'http://www.w3.org/2000/svg', xlink = 'http://www.w3.org/1999/xlink';
  const cases = [['animate', 'SVGAnimateElement'], ['set', 'SVGSetElement'],
    ['animateMotion', 'SVGAnimateMotionElement'], ['animateTransform', 'SVGAnimateTransformElement']];
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = (name, run) => {
    try { run(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, error: error.name, message: error.message}); }
  };
  const throws = (run, Constructor, name = Constructor.name) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    assert(error instanceof Constructor && error.name === name, name + ' required');
  };
  const child = document.getElementById('child').contentWindow;
  const realms = [['main', window], ['child', child]];
  for (const [owner, realm] of realms) {
    const Base = realm.SVGAnimationElement;
    check(owner + '/base', () => {
      assert(Object.getPrototypeOf(Base) === realm.SVGElement, 'base constructor inheritance');
      assert(Object.getPrototypeOf(Base.prototype) === realm.SVGElement.prototype, 'base prototype inheritance');
      throws(() => new Base(), realm.TypeError);
      throws(() => Base(), realm.TypeError);
      assert(!('onbegin' in realm.document.createElementNS(ns, 'rect')), 'only animation interfaces expose onbegin');
    });
    const documents = [['live', realm.document],
      ['html', realm.document.implementation.createHTMLDocument('')],
      ['xml', realm.document.implementation.createDocument(ns, 'svg')],
      ['parser', new realm.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>', 'image/svg+xml')]];
    for (const [tag, name] of cases) {
      const Constructor = realm[name];
      check(owner+'/'+tag+'/constructor', () => {
        assert(Constructor.name === name && Constructor.length === 0, 'constructor metadata');
        assert(Object.getPrototypeOf(Constructor) === Base, 'constructor inheritance');
        assert(Object.getPrototypeOf(Constructor.prototype) === Base.prototype, 'prototype inheritance');
        throws(() => new Constructor(), realm.TypeError);
        throws(() => Constructor(), realm.TypeError);
      });
      for (const [kind, doc] of documents) {
        const label=owner+'/'+kind+'/'+tag, element=doc.createElementNS(ns, tag);
        check(label+'/factory', () => {
          assert(Object.getPrototypeOf(element) === Constructor.prototype, 'native prototype');
          assert(element instanceof Base && element instanceof realm.SVGElement && !(element instanceof realm.SVGGraphicsElement), 'native inheritance');
          assert(Object.prototype.toString.call(element)==='[object '+name+']', 'native tag');
          assert(Object.getPrototypeOf(element.cloneNode())===Constructor.prototype, 'clone identity');
          assert(Object.getPrototypeOf(realm.document.importNode(element))===Constructor.prototype, 'import identity');
          assert(!(doc.createElementNS('urn:other', tag) instanceof Constructor), 'namespace-sensitive factory');
          assert(!(doc.createElementNS(ns, tag.toUpperCase()) instanceof Constructor), 'case-sensitive factory');
        });
        check(label+'/SVGTests', () => {
          for (const attr of ['requiredExtensions', 'systemLanguage']) {
            const list = element[attr];
            assert(list instanceof realm.SVGStringList && element[attr] === list, 'stable native list');
            list.appendItem('test');
            assert(element.getAttribute(attr)==='test', 'list writes native attribute');
            element.setAttribute(attr, 'replacement');
            assert(list.numberOfItems===1 && list.getItem(0)==='replacement', 'attribute refreshes cached list');
            element.removeAttribute(attr);
            assert(list.numberOfItems===0, 'removal clears list');
          }
        });
        check(label+'/target', () => {
          const svg=doc.createElementNS(ns,'svg'), parent=doc.createElementNS(ns,'g'), target=doc.createElementNS(ns,'rect');
          target.id='svg-animation-target id'; parent.append(element); svg.append(parent,target);
          assert(element.targetElement===null, 'detached subtree has no target');
          (doc.body || doc.documentElement).append(svg);
          try {
            assert(element.targetElement===parent, 'parent target');
            element.setAttributeNS(xlink,'other:href','#svg-animation-target%20id');
            assert(element.targetElement===target, 'namespaced fallback and decoded fragment');
            element.setAttribute('href','#missing');
            assert(element.targetElement===null, 'href takes precedence');
            element.setAttribute('href','');
            assert(element.targetElement===parent, 'empty href targets parent');
            element.removeAttribute('href');
            assert(element.targetElement===target, 'remove href restores xlink');
            element.setAttribute('href','https://external-svg.test/#svg-animation-target%20id');
            assert(element.targetElement===null, 'external URL does not resolve locally');
            element.setAttribute('href','#svg-animation-target%20id');
            target.id='renamed'; assert(element.targetElement===null, 'ID mutations refresh target');
            target.id='svg-animation-target id'; assert(element.targetElement===target, 'target restored');
            target.remove(); assert(element.targetElement===null, 'removed reference target');
            element.removeAttribute('href'); element.removeAttributeNS(xlink,'href');
            assert(element.targetElement===parent, 'fallback parent restored');
          } finally {svg.remove();}
          assert(element.targetElement===null, 'removed animation loses target');
        });
        check(label+'/handlers', () => {
          for (const [name,type] of [['onbegin','beginEvent'],['onend','endEvent'],['onrepeat','repeatEvent']]) {
            const log=[], before=()=>log.push('before'),after=()=>log.push('after');
            const first=function(e){assert(this===element && e.type===type,'handler receiver and event');log.push('first');return false;};
            const replacement=function(){log.push('replacement');};
            assert(element[name]===null,'initial null');
            element.addEventListener(type,before); element[name]=first; element.addEventListener(type,after); element[name]=replacement;
            element.dispatchEvent(new realm.Event(type));
            assert(log.join(',')==='before,replacement,after','replacement keeps registration position');log.length=0;
            element.dispatchEvent(new realm.Event(name.slice(2)));
            assert(log.length===0,'short event name does not invoke SVG handler');
            element[name]=null;element.dispatchEvent(new realm.Event(type));
            assert(log.join(',')==='before,after','null clears handler');log.length=0;
            element[name]=first;const event=new realm.Event(type,{cancelable:true});
            assert(!element.dispatchEvent(event) && event.defaultPrevented,'return false cancels event');
            assert(log.join(',')==='before,after,first','new registration goes last');
            element[name]=null;element.removeEventListener(type,before);element.removeEventListener(type,after);
          }
        });
      }
    }
    check(owner+'/parser', () => {
      const svg=realm.document.createElementNS(ns,'svg');
      svg.innerHTML='<animate/><set/><animateMotion/><animateTransform/>';
      for (let i=0;i<cases.length;i++) assert(svg.children[i] instanceof realm[cases[i][1]],'HTML SVG parser interface');
    });
    check(owner+'/content-handlers', () => {
      const svg=realm.document.createElementNS(ns,'svg'), element=realm.document.createElementNS(ns,'animate');
      svg.append(element);realm.document.body.append(svg);
      try {
        for(const [name,type] of [['onbegin','beginEvent'],['onend','endEvent'],['onrepeat','repeatEvent']]) {
          element.setAttribute(name,'this.setAttribute("data-event", event.type)');
          element.dispatchEvent(new realm.Event(name.slice(2)));
          assert(!element.hasAttribute('data-event'),'short name does not compile content handler');
          element.dispatchEvent(new realm.Event(type));
          assert(element.getAttribute('data-event')===type,'content handler observes canonical event type');
          element.removeAttribute(name);element.removeAttribute('data-event');
          element.dispatchEvent(new realm.Event(type));
          assert(!element.hasAttribute('data-event'),'removal unregisters content handler');
        }
      } finally {svg.remove();}
    });
  }
  for(const [sourceName,source] of realms) for(const [ownerName,owner] of realms) {
    check(sourceName+'->'+ownerName+'/receiver-and-realm', () => {
      const element=owner.document.createElementNS(ns,'animate');
      const Base=source.SVGAnimationElement;
      const revoked=Proxy.revocable(element,{});revoked.revoke();
      let traps=0,conversions=0;
      const trap=()=>{traps++;throw Error('trap');};
      const bad=[{},Base.prototype,Object.create(element),new Proxy(element,{get:trap,getPrototypeOf:trap}),revoked.proxy,owner.document.createElementNS(ns,'rect')];
      for(const name of ['targetElement','requiredExtensions','systemLanguage','onbegin','onend','onrepeat']) {
        const descriptor=Object.getOwnPropertyDescriptor(Base.prototype,name);
        assert(descriptor.enumerable && descriptor.configurable && descriptor.get.length===0,'accessor descriptor');
        for(const value of bad) {
          throws(()=>descriptor.get.call(value),source.TypeError);
          if(descriptor.set) throws(()=>descriptor.set.call(value,()=>{}),source.TypeError);
        }
      }
      for(const [name,length] of [['getStartTime',0],['getCurrentTime',0],['getSimpleDuration',0],['beginElement',0],['beginElementAt',1],['endElement',0],['endElementAt',1]]) {
        const descriptor=Object.getOwnPropertyDescriptor(Base.prototype,name), method=descriptor.value;
        assert(descriptor.enumerable && descriptor.configurable && descriptor.writable && method.length===length && method.name===name,'method descriptor');
        for(const value of bad) throws(()=>method.call(value,{valueOf(){conversions++;return 1;}}),source.TypeError);
        if(length) {
          throws(()=>method.call(element),source.TypeError);
          for(const value of [NaN,Infinity,-Infinity,1e100,Symbol(),1n]) throws(()=>method.call(element,value),source.TypeError);
          const sentinel={};let caught;
          try {method.call(element,{valueOf(){throw sentinel;}});} catch(error){caught=error;}
          assert(caught===sentinel,'conversion exception propagated');
        }
      }
      assert(traps===0 && conversions===0,'brand checked before any author code');
      assert(Base.prototype.getCurrentTime.call(element)===0,'detached current time');
      throws(()=>Base.prototype.getStartTime.call(element),source.DOMException,'InvalidStateError');
      throws(()=>Base.prototype.getSimpleDuration.call(element),source.DOMException,'NotSupportedError');
      const listGet=Object.getOwnPropertyDescriptor(Base.prototype,'systemLanguage').get;
      assert(listGet.call(element) instanceof owner.SVGStringList,'list belongs to element realm');
      const graphicsGet=Object.getOwnPropertyDescriptor(source.SVGGraphicsElement.prototype,'systemLanguage').get;
      throws(()=>graphicsGet.call(element),source.TypeError);
      const handler=Object.getOwnPropertyDescriptor(Base.prototype,'onbegin'),callback=()=>{};
      handler.set.call(element,callback);assert(handler.get.call(element)===callback,'genuine cross-realm receiver');handler.set.call(element,null);
    });
  }
  globalThis.__nodeReplacementResults={rows,failures:rows.filter(row=>!row.pass),passed:rows.filter(row=>row.pass).length,total:rows.length};
  return rows.every(row=>row.pass);
})()
