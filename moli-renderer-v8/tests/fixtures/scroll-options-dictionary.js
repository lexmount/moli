(() => {
  const assert=(value,label)=>{if(!value)throw Error(label);};
  document.body.style.cssText='width:5000px;height:5000px;margin:0';
  const box=document.createElement('div');box.style.cssText='width:100px;height:100px;overflow:auto';
  box.innerHTML='<div style="width:2000px;height:2000px"></div>';document.body.append(box);
  const cases=[];
  for(const object of [window,box]) for(const method of ['scroll','scrollTo','scrollBy'])
    cases.push([object,method,['behavior','left','top']]);
  cases.push([box,'scrollIntoView',['behavior','block','container','inline']]);
  for(const [object,method,members] of cases) {
    for(const key of members) for(const sentinel of [{},null,undefined,7]) {
      const reads=[];let caught=false;
      try {object[method](new Proxy({}, {get(_,name) {
        reads.push(name); if(name===key) throw sentinel;return undefined;
      }, has() {throw Error('Get required');}}));} catch(e) {caught=true;}
      assert(!caught, method+' silently cancels conversion failure');
      assert(reads.join()===members.slice(0,members.indexOf(key)+1).join(),method+' conversion order/short circuit: '+reads);
    }
    const reads=[];
    object[method](new Proxy({}, {get(_,name) {reads.push(name);return 'invalid';}}));
    assert(reads.join()==='behavior',method+' invalid enum must stop conversion');
  }
  window.scrollTo(10,20);
  for(const init of [1,true,'init',1n,Symbol(),{behavior:'invalid',left:99,top:99}]) {
    window.scrollTo(init);
    assert(scrollX===10 && scrollY===20,'failed one-argument dictionary must not change position');
  }
  let later=0;
  window.scrollTo({get left(){throw 7;},get top(){later++;return 99;}});
  assert(later===0 && scrollX===10 && scrollY===20,'abrupt conversion cannot partially scroll');
  for(const object of [window,box]) for(const method of ['scrollTo','scrollBy']) {
    const reads=[];const options=function(){};
    for(const name of ['behavior','left','top']) Object.defineProperty(options,name,{get(){reads.push(name);return undefined;}});
    object[method](options);
    assert(reads.join()==='behavior,left,top',method+' callable dictionary branch');
    let first=0,second=0;
    object[method]({valueOf(){first++;throw 7;}},{valueOf(){second++;return 2;}});
    assert(first===1 && second===0,method+' numeric overload stops at first error');
  }
  window.scrollTo(10,20);
  window.scrollTo({get behavior(){window.scrollTo(30,40);return 'instant';}});
  assert(scrollX===30 && scrollY===40,'fallback position read after dictionary conversion');
  window.scrollTo(NaN,Infinity);
  assert(scrollX===0 && scrollY===0,'nonfinite coordinates normalize to zero');
  window.scrollTo(10,20);window.scrollBy(NaN,-Infinity);
  assert(scrollX===10 && scrollY===20,'nonfinite deltas normalize to zero');
  for(const method of ['scroll','scrollTo','scrollBy','scrollIntoView']) {
    const detached=document.createElement('div'),reads=[];
    detached[method](new Proxy({}, {get(_,name){reads.push(name);return undefined;}}));
    const expected=method==='scrollIntoView'?'behavior,block,container,inline':'behavior,left,top';
    assert(reads.join()===expected,method+' converts detached receiver options');
  }
  let prototypeReads=0;
  try {
    for(const name of ['behavior','left','top']) Object.defineProperty(Object.prototype,name,{configurable:true,get(){prototypeReads++;throw Error('nullish prototype');}});
    window.scrollTo();window.scrollTo(null);window.scrollBy(undefined);
    assert(prototypeReads===0,'nullish dictionary must not consult Object.prototype');
  } finally {for(const name of ['behavior','left','top']) delete Object.prototype[name];}
  box.remove();return true;
})()
