(async () => {
  const checks = [];
  const run = (name, callback) => {
    try {checks.push({name, passed: !!callback(), detail: null});}
    catch (error) {checks.push({name, passed: false, detail: String(error)});}
  };
  const frame = document.querySelector('iframe');
  if (frame.contentDocument.readyState !== 'complete') await new Promise(resolve => frame.addEventListener('load', resolve, {once: true}));
  const other = frame.contentWindow, ns = 'http://www.w3.org/2000/svg';
  const cases = [
    ['feFuncR','tableValues'], ['feFuncG','tableValues'], ['feFuncB','tableValues'], ['feFuncA','tableValues'],
    ['feColorMatrix','values'], ['feConvolveMatrix','kernelMatrix'], ['text','rotate'],
  ];
  const mutations = [
    ['setAttribute', (e, name, value) => e.setAttribute(name,value), (e,name) => e.removeAttribute(name)],
    ['namespace-null', (e,name,value) => e.setAttributeNS(null,name,value), (e,name) => e.removeAttributeNS(null,name)],
    ['Attr.value', (e,name,value) => {e.getAttributeNode(name).value=value;}, (e,name) => e.removeAttributeNode(e.getAttributeNode(name))],
    ['Attr.nodeValue', (e,name,value) => {e.getAttributeNode(name).nodeValue=value;}, (e,name) => e.removeAttributeNode(e.getAttributeNode(name))],
    ['Attr.textContent', (e,name,value) => {e.getAttributeNode(name).textContent=value;}, (e,name) => e.removeAttributeNode(e.getAttributeNode(name))],
  ];
  const same = (list, expected) => JSON.stringify(Array.from({length:list.length}, (_,i) => list.getItem(i).value)) === JSON.stringify(expected);
  for (const w of [window,other]) {
    const realm = w === window ? 'main' : 'child';
    for (const [docIndex,doc] of [w.document,w.document.implementation.createHTMLDocument(''),
      new w.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')].entries()) {
      for (const [tag,name] of cases) {
        const label = realm+'/'+docIndex+'/'+tag;
        const create = () => {const e=doc.createElementNS(ns,tag);e.setAttribute(name,'1 2');return e;};
        for (const [path,set,remove] of mutations) run(label+'/'+path+'/shrink-regrow', () => {
          const e=create(), a=e[name], base=a.baseVal, anim=a.animVal, first=base.getItem(0), last=base.getItem(1), oldAnim=anim.getItem(1);
          set(e,name,'1');set(e,name,'1 2');
          if (base.getItem(0)!==first || base.getItem(1)===last || anim.getItem(1)===oldAnim || last.value!==2 || oldAnim.value!==2) return false;
          last.value=7;oldAnim.value=8;
          return last.value===7 && oldAnim.value===8 && same(base,[1,2]) && same(anim,[1,2]);
        });
        for (const [path,set,remove] of mutations) run(label+'/'+path+'/remove-regrow', () => {
          const e=create(), a=e[name], base=a.baseVal, anim=a.animVal, first=base.getItem(0), oldAnim=anim.getItem(0);
          remove(e,name);e.setAttribute(name,'1 2');
          if(base.getItem(0)===first || anim.getItem(0)===oldAnim || first.value!==1 || oldAnim.value!==1) return false;
          first.value=7;oldAnim.value=8;
          return same(base,[1,2]) && same(anim,[1,2]);
        });
        run(label+'/setter-conversion-detaches-item', () => {
          const e=create(), a=e[name], first=a.baseVal.getItem(0);
          first.value={valueOf() {e.removeAttribute(name);e.setAttribute(name,'1 2');return 7;}};
          return first.value===7 && same(a.baseVal,[1,2]) && a.baseVal.getItem(0)!==first;
        });
        run(label+'/readonly-setter-conversion-detaches-item', () => {
          const e=create(), a=e[name], old=a.animVal.getItem(0);
          old.value={valueOf() {e.removeAttribute(name);e.setAttribute(name,'1 2');return 7;}};
          return old.value===7 && same(a.baseVal,[1,2]) && same(a.animVal,[1,2]) && a.animVal.getItem(0)!==old;
        });
        run(label+'/base-list-edits-synchronize-animation', () => {
          const e=create(), a=e[name], old=a.animVal.getItem(0);
          const n=value => {const x=doc.createElementNS(ns,'svg').createSVGNumber();x.value=value;return x;};
          a.baseVal.clear();a.baseVal.appendItem(n(1));a.baseVal.appendItem(n(2));
          old.value=7;
          return old.value===7 && a.animVal.getItem(0)!==old && same(a.baseVal,[1,2]) && same(a.animVal,[1,2]);
        });
        run(label+'/unrelated-namespace-and-case', () => {
          const e=create(), a=e[name], first=a.baseVal.getItem(0), animated=a.animVal.getItem(0);
          e.setAttributeNS('urn:unrelated','p:'+name,'');e.removeAttributeNS('urn:unrelated',name);
          e.setAttribute(name.toUpperCase(),'');e.removeAttribute(name.toUpperCase());
          return a.baseVal.getItem(0)===first && a.animVal.getItem(0)===animated && first.value===1 && animated.value===1;
        });
        run(label+'/borrowed-mutation-method', () => {
          const e=create(), a=e[name], old=a.animVal.getItem(0), source=w===window?other:window;
          source.Element.prototype.removeAttribute.call(e,name);
          source.Element.prototype.setAttribute.call(e,name,'1 2');
          old.value=7;
          return old.value===7 && a.animVal.getItem(0)!==old && same(a.baseVal,[1,2]);
        });
        run(label+'/adopted-owner', () => {
          const e=create(), a=e[name], old=a.animVal.getItem(0), recipient=w===window?other.document:document;
          recipient.adoptNode(e);e.removeAttribute(name);e.setAttribute(name,'1 2');
          old.value=7;
          return old.value===7 && a.animVal.getItem(0)!==old && same(a.baseVal,[1,2]);
        });
      }
    }
  }
  run('retained-owner-after-iframe-removal', () => {
    const f=document.createElement('iframe');document.body.appendChild(f);
    const e=f.contentDocument.createElementNS(ns,'feColorMatrix');e.setAttribute('values','1 2');
    const a=e.values,old=a.animVal.getItem(0);f.remove();e.removeAttribute('values');e.setAttribute('values','1 2');old.value=7;
    return old.value===7 && a.animVal.getItem(0)!==old && same(a.baseVal,[1,2]);
  });
  globalThis.__uiEventResults={complete:true,checks,total:checks.length,passed:checks.filter(row=>row.passed).length};
  return true;
})()
