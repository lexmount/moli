(() => {
 const checks=[], ns='http://www.w3.org/2000/svg', realms=[window,document.querySelector('iframe').contentWindow];
 const check=(name,fn)=>{try{checks.push({name,passed:fn()===true});}catch(error){checks.push({name,passed:false,error:String(error)});}};
 const raises=(realm,fn,name='TypeError')=>{try{fn();}catch(e){return Object.getPrototypeOf(e)===realm[name].prototype;}return false;};
 const values='M0 0L10 10;M20 40L50 70';
 const close=(a,b)=>Math.abs(a-b)<1e-4;
 const coordinates=path=>(getComputedStyle(path).getPropertyValue('d').match(/-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/gi)||[]).map(Number);
 const equals=(a,b)=>a.length===b.length&&a.every((v,i)=>close(v,b[i]));
 const make=(owner,attributes={})=>{
  const doc=owner.document, svg=doc.createElementNS(ns,'svg'),path=doc.createElementNS(ns,'path'),animation=doc.createElementNS(ns,'animate');
  path.setAttribute('d','M0 0L10 10');
  for(const [k,v] of Object.entries({attributeName:'d',dur:'10s',values,...attributes}))animation.setAttribute(k,v);
  path.append(animation);svg.append(path);doc.body.append(svg);svg.pauseAnimations();svg.setCurrentTime(0);return {svg,path,animation};
 };
 for(const [ci,callee] of realms.entries()){
  const methods=['pauseAnimations','unpauseAnimations','animationsPaused','getCurrentTime','setCurrentTime'];
  const invoke=(name,receiver,args=[])=>{
   const fn=Object.getOwnPropertyDescriptor(callee.SVGSVGElement.prototype,name)?.value;
   if(typeof fn!=='function')throw Error('Missing SVGSVGElement.'+name);
   return Reflect.apply(fn,receiver,args);
  };
  for(const name of methods)check(`${ci}/smil/metadata/${name}`,()=>{
   const d=Object.getOwnPropertyDescriptor(callee.SVGSVGElement.prototype,name);
   return typeof d?.value==='function'&&d.enumerable&&d.configurable&&d.writable&&d.value.name===name&&
    d.value.length===(name==='setCurrentTime'?1:0)&&Object.getPrototypeOf(d.value)===callee.Function.prototype;
  });
  for(const [oi,owner] of realms.entries()){
   const prefix=`${ci}/${oi}/smil/`, sample=(f,t)=>{invoke('setCurrentTime',f.svg,[t]);return coordinates(f.path);};
   check(prefix+'clock-pause-seek-resume',()=>{
    const f=make(owner);if(invoke('animationsPaused',f.svg)!==true)return false;
    invoke('setCurrentTime',f.svg,[7]);invoke('pauseAnimations',f.svg);
    if(invoke('getCurrentTime',f.svg)!==7||f.animation.getCurrentTime()!==7)return false;
    invoke('unpauseAnimations',f.svg);if(invoke('animationsPaused',f.svg)!==false||invoke('getCurrentTime',f.svg)<7)return false;
    invoke('pauseAnimations',f.svg);const t=invoke('getCurrentTime',f.svg);return invoke('getCurrentTime',f.svg)===t;
   });
   check(prefix+'negative-seek-clamped',()=>{const f=make(owner);invoke('setCurrentTime',f.svg,[-5]);return invoke('getCurrentTime',f.svg)===0;});
   check(prefix+'binary32-seek-conversion',()=>{const f=make(owner);invoke('setCurrentTime',f.svg,[1.1]);return invoke('getCurrentTime',f.svg)===Math.fround(1.1);});
   check(prefix+'pending-seek-before-connection',()=>{const s=owner.document.createElementNS(ns,'svg');invoke('pauseAnimations',s);invoke('setCurrentTime',s,[17]);if(invoke('getCurrentTime',s)!==0)return false;owner.document.body.append(s);return invoke('getCurrentTime',s)===17;});
   check(prefix+'independent-fragments',()=>{const a=make(owner),b=make(owner);invoke('setCurrentTime',a.svg,[4]);invoke('setCurrentTime',b.svg,[9]);return invoke('getCurrentTime',a.svg)===4&&invoke('getCurrentTime',b.svg)===9;});
   for(const [time,expected] of [[0,[0,0,10,10]],[2.5,[5,10,20,25]],[5,[10,20,30,40]],[7.5,[15,30,40,55]]])check(prefix+'typed-path-at-'+time,()=>{
    const f=make(owner);const actual=sample(f,time),end=f.path.getPointAtLength(1e6);
    return equals(actual,expected)&&close(end.x,expected[2])&&close(end.y,expected[3])&&
     f.path.getAttribute('d')==='M0 0L10 10'&&equals(f.path.getPathData().flatMap(s=>s.values),[0,0,10,10]);
   });
   check(prefix+'setPathData-updates-base-during-animation',()=>{
    const f=make(owner);sample(f,5);f.path.setPathData([{type:'M',values:[0,0]},{type:'L',values:[200,200]}]);
    return f.path.getAttribute('d')==='M 0 0 L 200 200'&&equals(coordinates(f.path),[10,20,30,40])&&
     equals(f.path.getPathData().flatMap(s=>s.values),[0,0,200,200]);
   });
   check(prefix+'freeze-final-sample',()=>{const f=make(owner,{fill:'freeze'});return equals(sample(f,20),[20,40,50,70]);});
   check(prefix+'remove-restores-base',()=>{const f=make(owner);return equals(sample(f,20),[0,0,10,10]);});
   check(prefix+'fractional-repeat-freeze',()=>{const f=make(owner,{fill:'freeze',repeatCount:'2.5'});return equals(sample(f,25),[10,20,30,40]);});
   check(prefix+'repeat-interior-and-final',()=>{const f=make(owner,{fill:'freeze',repeatCount:'2'});return equals(sample(f,15),[10,20,30,40])&&equals(sample(f,20),[20,40,50,70]);});
   check(prefix+'values-keyTimes',()=>{const f=make(owner,{values:'M0 0L10 10;M20 40L50 70;M40 80L90 130',keyTimes:'0;0.25;1'});return equals(sample(f,2.5),[20,40,50,70])&&equals(sample(f,6.25),[30,60,70,100]);});
   check(prefix+'discrete-sampling',()=>{const f=make(owner,{calcMode:'discrete'});return equals(sample(f,4),[0,0,10,10])&&equals(sample(f,6),[20,40,50,70]);});
   check(prefix+'from-to-sampling',()=>{const f=make(owner,{from:'M10 20L30 40',to:'M30 40L50 60'});f.animation.removeAttribute('values');return equals(sample(f,5),[20,30,40,50]);});
   check(prefix+'to-only-uses-base',()=>{const f=make(owner,{to:'M20 40L50 70'});f.animation.removeAttribute('values');return equals(sample(f,5),[10,20,30,40]);});
   check(prefix+'DOM-begin-and-end-instance-times',()=>{
    const f=make(owner,{begin:'indefinite',fill:'freeze'});if(!raises(callee,()=>Reflect.apply(callee.SVGAnimationElement.prototype.getStartTime,f.animation,[]),'DOMException'))return false;
    invoke('setCurrentTime',f.svg,[10]);Reflect.apply(callee.SVGAnimationElement.prototype.beginElementAt,f.animation,[2]);
    if(f.animation.getStartTime()!==12)return false;
    if(!equals(sample(f,17),[10,20,30,40]))return false;
    Reflect.apply(callee.SVGAnimationElement.prototype.endElementAt,f.animation,[1]);return equals(sample(f,19),[12,24,34,46]);
   });
   check(prefix+'immediate-begin-restarts',()=>{const f=make(owner,{begin:'indefinite'});invoke('setCurrentTime',f.svg,[3]);f.animation.beginElement();if(f.animation.getStartTime()!==3)return false;sample(f,8);f.animation.beginElement();invoke('setCurrentTime',f.svg,[8]);return f.animation.getStartTime()===8&&equals(coordinates(f.path),[0,0,10,10]);});
   check(prefix+'restart-never',()=>{const f=make(owner,{restart:'never'});sample(f,5);f.animation.beginElement();return f.animation.getStartTime()===0&&equals(coordinates(f.path),[10,20,30,40]);});
   check(prefix+'restart-whenNotActive',()=>{const f=make(owner,{restart:'whenNotActive'});sample(f,5);f.animation.beginElement();return f.animation.getStartTime()===0&&equals(coordinates(f.path),[10,20,30,40]);});
   check(prefix+'animated-href-target',()=>{const f=make(owner);f.path.id='smil-target-'+ci+'-'+oi;f.svg.append(f.animation);f.animation.setAttribute('href','#'+f.path.id);return equals(sample(f,5),[10,20,30,40]);});
   check(prefix+'attribute-removal-stops-sampling',()=>{const f=make(owner);sample(f,5);f.animation.remove();return equals(coordinates(f.path),[0,0,10,10]);});
   for(const [dur,expected] of [['250ms',.25],['2min',120],['1h',3600],['01:02:03.5',3723.5],['02:03.5',123.5],[' 3.5s ',3.5]])check(prefix+'duration-'+dur,()=>{const f=make(owner,{dur});return f.animation.getSimpleDuration()===Math.fround(expected);});
   for(const dur of ['indefinite','media','NaN','-1s','garbage'])check(prefix+'indefinite-duration-'+dur,()=>{const f=make(owner,{dur});try{f.animation.getSimpleDuration();}catch(e){return e.name==='NotSupportedError';}return false;});
   for(const value of [NaN,Infinity,-Infinity,1e100,Symbol(),1n])check(prefix+'invalid-time-'+String(value),()=>{const f=make(owner);invoke('setCurrentTime',f.svg,[4]);return raises(callee,()=>invoke('setCurrentTime',f.svg,[value]))&&invoke('getCurrentTime',f.svg)===4;});
   check(prefix+'conversion-before-seek-and-original-exception',()=>{const f=make(owner),sentinel={};invoke('setCurrentTime',f.svg,[4]);let old;invoke('setCurrentTime',f.svg,[{valueOf(){old=invoke('getCurrentTime',f.svg);return 7;}}]);try{invoke('setCurrentTime',f.svg,[{valueOf(){throw sentinel;}}]);}catch(e){return e===sentinel&&old===4&&invoke('getCurrentTime',f.svg)===7;}return false;});
   const f={svg:owner.document.createElementNS(ns,'svg'),path:owner.document.createElementNS(ns,'path')};f.svg.append(f.path);owner.document.body.append(f.svg);const revoked=Proxy.revocable(f.svg,{});revoked.revoke();let traps=0;
   const invalid=[{},Object.create(callee.SVGSVGElement.prototype),Object.create(f.svg),new Proxy(f.svg,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}}),revoked.proxy,f.path,owner.document.createElement('div'),null,undefined];
   for(const [ri,receiver] of invalid.entries())for(const name of methods)check(prefix+'brand-'+ri+'-'+name,()=>{let conversion=0;return raises(callee,()=>invoke(name,receiver,[{valueOf(){conversion++;return 5;}}]))&&conversion===0&&traps===0;});
  }
 }
 globalThis.__svgSmilPathTimingResults={complete:true,total:checks.length,passed:checks.filter(c=>c.passed).length,checks};globalThis.__uiEventResults=__svgSmilPathTimingResults;return true;
})()
