(() => {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({name, passed: run() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const SVG = 'http://www.w3.org/2000/svg', XLINK = 'http://www.w3.org/1999/xlink';
  const child = document.querySelector('iframe').contentWindow;
  const components = ['origin','protocol','username','password','host','hostname','port','pathname','search','hash'];
  const strings = ['download','rel','hreflang','type','referrerPolicy','ping'];
  for (const [realmName, realm] of [['top',globalThis],['child',child]]) {
    const proto = realm.SVGAElement.prototype;
    for (const property of ['target', ...strings, ...components]) {
      check(`${realmName} descriptor ${property}`, () => {
        const d = Object.getOwnPropertyDescriptor(proto, property);
        return d.enumerable && d.configurable && typeof d.get === 'function' &&
          (['target','origin'].includes(property) ? d.set === undefined : typeof d.set === 'function');
      });
      const wrong = realm.document.createElement('a');
      const genuine = realm.document.createElementNS(SVG,'a');
      const revoked = realm.Proxy.revocable(genuine,{}); revoked.revoke();
      let traps = 0;
      const proxy = new realm.Proxy(genuine,{get(){traps++;throw Error('trap');}});
      for (const [kind,receiver] of [['null',null],['undefined',undefined],['plain',{}],['forged',Object.create(proto)],['inherited',Object.create(genuine)],['HTML',wrong],['other SVG',realm.document.createElementNS(SVG,'rect')],['proxy',proxy],['revoked',revoked.proxy]]) {
        check(`${realmName} ${property} rejects ${kind} getter`, () => {
          const d = Object.getOwnPropertyDescriptor(proto,property); let error;
          try { d.get.call(receiver); } catch (e) { error=e; }
          return error instanceof realm.TypeError && traps===0;
        });
        if (!['target','origin'].includes(property)) check(`${realmName} ${property} rejects ${kind} before conversion`, () => {
          let converted=0,error; const d=Object.getOwnPropertyDescriptor(proto,property);
          try { d.set.call(receiver,{toString(){converted++;return 'changed';}}); } catch(e){error=e;}
          return error instanceof realm.TypeError && converted===0 && traps===0;
        });
      }
    }
    const documents = [['live',realm.document], ['windowless HTML',realm.document.implementation.createHTMLDocument('')], ['XML',new realm.DOMParser().parseFromString(`<svg xmlns="${SVG}"/>`,'image/svg+xml')]];
    for (const [documentName,doc] of documents) {
      const prefix=`${realmName} ${documentName}`;
      const anchor=doc.createElementNS(SVG,'a');
      for (const property of ['download','rel','hreflang','type']) {
        const attr=property;
        for(const value of ['', ' MiXeD value ', '\ud800x\udfff', 'null']) check(`${prefix} ${property} DOMString ${JSON.stringify(value)}`, () => {
          Object.getOwnPropertyDescriptor(proto,property).set.call(anchor,value);
          return anchor.getAttribute(attr)===value && Object.getOwnPropertyDescriptor(proto,property).get.call(anchor)===value;
        });
        check(`${prefix} ${property} removal`,()=>{anchor.removeAttribute(attr);return anchor[property]==='';});
        check(`${prefix} ${property} original conversion exception`,()=>{
          const marker=new realm.Error('conversion'); let error;
          try{Object.getOwnPropertyDescriptor(proto,property).set.call(anchor,{toString(){throw marker;}});}catch(e){error=e;}
          return error===marker && !anchor.hasAttribute(attr);
        });
      }
      check(`${prefix} ping USVString`,()=>{anchor.ping='\ud800 /two';return anchor.getAttribute('ping')==='\ufffd /two' && anchor.ping==='\ufffd /two';});
      check(`${prefix} ping attribute USVString`,()=>{anchor.setAttribute('ping','\ud800');return anchor.ping==='\ufffd' && anchor.getAttribute('ping')==='\ud800';});
      for(const policy of ['no-referrer','no-referrer-when-downgrade','same-origin','origin','strict-origin','origin-when-cross-origin','strict-origin-when-cross-origin','unsafe-url','invalid',' origin ','\ud800']) check(`${prefix} referrerPolicy ${JSON.stringify(policy)}`,()=>{
        anchor.referrerPolicy=policy.toUpperCase();
        return anchor.getAttribute('referrerpolicy')===policy.toUpperCase() && anchor.referrerPolicy===(['invalid',' origin ','\ud800'].includes(policy)?'':policy);
      });
      check(`${prefix} target native realm and identity`,()=>{
        const target=Object.getOwnPropertyDescriptor(proto,'target').get.call(anchor);
        return target===anchor.target && target instanceof realm.SVGAnimatedString && target.baseVal==='' && target.animVal==='';
      });
      check(`${prefix} target live UTF16`,()=>{
        const target=anchor.target; anchor.setAttribute('target','\ud800frame');
        if(target.baseVal!=='\ud800frame'||target.animVal!=='\ud800frame')return false;
        target.baseVal='_blank';
        return anchor.getAttribute('target')==='_blank' && target.animVal==='_blank' && target===anchor.target;
      });
      check(`${prefix} target removal`,()=>{const saved=anchor.target;anchor.removeAttribute('target');return saved.baseVal===''&&saved.animVal==='';});
      check(`${prefix} relList SameObject and token updates`,()=>{
        anchor.rel='one one two';const list=anchor.relList;
        if(list.length!==2||list[0]!=='one'||list[1]!=='two')return false;
        list.add('three');list.remove('one');list.toggle('four',true);list.replace('two','five');
        return anchor.rel==='five three four' && anchor.relList===list;
      });
      check(`${prefix} relList PutForwards UTF16`,()=>{anchor.relList='\ud800 token';return anchor.rel==='\ud800 token'&&anchor.relList.value==='\ud800 token';});
      check(`${prefix} relList supports`,()=>['noopener','noreferrer','opener'].every(t=>anchor.relList.supports(t)) && !anchor.relList.supports('stylesheet'));
      check(`${prefix} relList token exception`,()=>{
        anchor.rel='one';let error;try{anchor.relList.add('two','has space');}catch(e){error=e;}
        return error instanceof realm.DOMException && error.name==='InvalidCharacterError' && anchor.rel==='one';
      });
      for(const [kind,href,xlink] of [['missing',null,null],['href','https://user:pass@example.test:8443/a/b?q=1#part',null],['xlink',null,'https://legacy.test:8080/old?q=2#p'],['both','https://primary.test/a','https://legacy.test/old'],['empty','', 'https://legacy.test/old'],['invalid','http://[invalid',null],['opaque','mailto:user@example.test',null]]) {
        anchor.removeAttribute('href');anchor.removeAttributeNS(XLINK,'href');
        if(href!==null)anchor.setAttribute('href',href);
        if(xlink!==null)anchor.setAttributeNS(XLINK,'xlink:href',xlink);
        let expected=null;const raw=href===null?xlink:href;
        if(raw!==null)try{expected=new URL(raw,doc.baseURI);}catch{}
        for(const property of components) check(`${prefix} ${kind} ${property}`,()=>
          Object.getOwnPropertyDescriptor(proto,property).get.call(anchor)===(expected?expected[property]:property==='protocol'?':':''));
        check(`${prefix} ${kind} href stays SVGAnimatedString`,()=>anchor.href instanceof realm.SVGAnimatedString && anchor.href.baseVal===(raw===null?'':raw));
      }
      const initial='https://user:pass@example.test:8443/a/b?q=1#part';
      for(const [property,value] of [['protocol','http'],['host','other.test:9876'],['hostname','[::1]'],['port','443'],['port','invalid'],['username','other user'],['password','secret?'],['pathname','/new path'],['search','?q=two three'],['hash','#next part'],['protocol','invalid space:']]) {
        for(const mode of ['href','xlink','both'])check(`${prefix} ${mode} ${property} setter ${value}`,()=>{
          anchor.removeAttribute('href');anchor.removeAttributeNS(XLINK,'href');
          if(mode!=='xlink')anchor.setAttribute('href',initial);
          if(mode!=='href')anchor.setAttributeNS(XLINK,'xlink:href',mode==='both'?'https://ignored.test/':initial);
          const expected=new URL(initial);expected[property]=value;
          Object.getOwnPropertyDescriptor(proto,property).set.call(anchor,value);
          // SVG2 update-href writes xlink:href whenever that attribute is present.
          return (mode==='href'?anchor.getAttribute('href'):anchor.getAttributeNS(XLINK,'href'))===expected.href &&
            (mode!=='both'||anchor.getAttribute('href')===initial);
        });
      }
      for(const property of components.filter(p=>p!=='origin')) {
        check(`${prefix} ${property} propagates conversion exception`,()=>{
          anchor.setAttribute('href',initial);const marker=new realm.Error('convert');let error;
          try{Object.getOwnPropertyDescriptor(proto,property).set.call(anchor,{toString(){throw marker;}});}catch(e){error=e;}
          return error===marker && anchor.getAttribute('href')===initial;
        });
        check(`${prefix} ${property} converts absent URL`,()=>{
          anchor.removeAttribute('href');anchor.removeAttributeNS(XLINK,'href');let converted=0;
          Object.getOwnPropertyDescriptor(proto,property).set.call(anchor,{toString(){converted++;return 'value';}});
          return converted===1 && !anchor.hasAttribute('href');
        });
      }
      check(`${prefix} reentrant href mutation before URL parse`,()=>{
        anchor.removeAttributeNS(XLINK,'href');anchor.setAttribute('href',initial);
        anchor.pathname={toString(){anchor.setAttribute('href','https://changed.test/root');return '/converted';}};
        return anchor.getAttribute('href')==='https://changed.test/converted';
      });
      check(`${prefix} adoption during conversion`,()=>{
        const destination=realm.document.implementation.createHTMLDocument('');
        anchor.setAttribute('href','https://old.test/');anchor.removeAttributeNS(XLINK,'href');
        anchor.hash={toString(){destination.adoptNode(anchor);anchor.setAttribute('href','https://adopted.test/');return '#ok';}};
        return anchor.ownerDocument===destination && anchor.getAttribute('href')==='https://adopted.test/#ok';
      });
    }
    check(`${realmName} document base update`,()=>{
      const doc=realm.document,base=doc.createElement('base'),a=doc.createElementNS(SVG,'a');
      base.href='https://first.test/dir/';doc.head.prepend(base);a.setAttribute('href','item');
      try{
        if(a.host!=='first.test'||a.pathname!=='/dir/item')return false;
        base.href='https://second.test/next/';
        if(a.host!=='second.test'||a.pathname!=='/next/item')return false;
        a.hash={toString(){base.href='https://third.test/final/';return '#done';}};
        return a.getAttribute('href')==='https://third.test/final/item#done';
      }finally{base.remove();}
    });
  }
  globalThis.__uiEventResults={checks,complete:true,total:checks.length,passed:checks.filter(c=>c.passed).length};
})();
