(function path2dContract(owner, callee, kind, tag) {
  const checks = [];
  const check = (name, run) => {
    try {checks.push({name: 'path2d/' + tag + '/' + name, passed: run() === true});}
    catch(error) {checks.push({name: 'path2d/' + tag + '/' + name, passed: false, error: String(error)});}
  };
  const P = owner.Path2D, proto = callee.Path2D.prototype;
  const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(96,96);
  canvas.width = canvas.height = 96;
  const ctx = canvas.getContext('2d');
  const cp = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
  const typeError = fn => {try {fn();} catch(e) {return e instanceof callee.TypeError;} return false;};
  const rect = () => {const p = new P(); p.rect(10,10,20,20); return p;};
  const clear = () => {ctx.resetTransform(); ctx.clearRect(0,0,96,96); ctx.beginPath(); ctx.fillStyle = '#00f'; ctx.strokeStyle = '#00f'; ctx.lineWidth = 4; ctx.setLineDash([]);};
  const pixel = (x,y) => ctx.getImageData(x,y,1,1).data[3];
  const fill = (p, rule='nonzero') => cp.fill.call(ctx,p,rule);
  const methods = [['closePath',0],['moveTo',2],['lineTo',2],['quadraticCurveTo',4],['bezierCurveTo',6],['arcTo',5],['arc',5],['ellipse',7],['rect',4],['addPath',1]];

  check('constructor metadata', () => P.length === 0 && P.name === 'Path2D' && proto.constructor === callee.Path2D);
  check('native empty construction', () => {const p = new P(); return Object.getPrototypeOf(p) === P.prototype && Object.keys(p).length === 0;});
  check('constructor without new', () => typeError(() => callee.Path2D()));
  check('empty fill leaves default path alone', () => {clear(); ctx.rect(50,50,20,20); fill(new P()); return pixel(60,60) === 0;});
  check('rect fill', () => {clear(); fill(rect()); return pixel(20,20) === 255 && pixel(40,40) === 0;});
  check('copy independent source mutation', () => {const p=rect(), copy=new P(p); p.rect(50,50,20,20); clear(); fill(copy); return pixel(20,20) === 255 && pixel(60,60) === 0;});
  check('copy independent destination mutation', () => {const p=rect(), copy=new P(p); copy.rect(50,50,20,20); clear(); fill(p); return pixel(20,20) === 255 && pixel(60,60) === 0;});
  check('cross realm constructor copy', () => {const p=rect(),copy=new callee.Path2D(p); clear(); fill(copy); return pixel(20,20) === 255;});
  check('paint applies current transform', () => {const p=rect(); clear(); ctx.translate(40,0); fill(p); return pixel(60,20)===255 && pixel(20,20)===0;});
  check('painting does not mutate path', () => {const p=rect(); clear(); ctx.translate(40,0); fill(p); clear(); fill(p); return pixel(20,20)===255 && pixel(60,20)===0;});
  check('Path2D painting preserves current default path', () => {clear(); ctx.rect(50,50,20,20); fill(rect()); clear(); ctx.rect(50,50,20,20); fill(rect()); ctx.clearRect(0,0,96,96); cp.fill.call(ctx); return pixel(60,60)===255 && pixel(20,20)===0;});
  check('stroke explicit path', () => {clear(); cp.stroke.call(ctx,rect()); return pixel(10,20)===255 && pixel(20,20)===0;});
  check('stroke transform and line width', () => {clear(); ctx.scale(2,1); cp.stroke.call(ctx,rect()); return pixel(18,20)===255 && pixel(40,20)===0;});
  check('nested path nonzero', () => {const p=rect(); p.rect(15,15,10,10); clear(); fill(p); return pixel(20,20)===255;});
  check('nested path evenodd', () => {const p=rect(); p.rect(15,15,10,10); clear(); fill(p,'evenodd'); return pixel(12,12)===255 && pixel(20,20)===0;});
  check('default path evenodd', () => {clear(); ctx.rect(10,10,20,20); ctx.rect(15,15,10,10); cp.fill.call(ctx,'evenodd'); return pixel(12,12)===255 && pixel(20,20)===0;});
  check('fill enum object conversion', () => {clear(); ctx.rect(10,10,20,20); let n=0; cp.fill.call(ctx,{toString(){n++;return 'evenodd';}}); return n===1 && pixel(20,20)===255;});
  check('fill invalid enum', () => typeError(() => cp.fill.call(ctx,'invalid')));
  check('fill invalid second argument', () => typeError(() => cp.fill.call(ctx,rect(),'invalid')));
  check('stroke invalid interface', () => typeError(() => cp.stroke.call(ctx,{})));

  for (const [name,arity] of methods) {
    check(name+' descriptor', () => {const d=Object.getOwnPropertyDescriptor(proto,name); return d && typeof d.value==='function' && d.enumerable && d.writable && d.configurable && d.value.name===name && d.value.length===arity;});
    for (const label of ['plain','forged','inherited','proxy','revoked']) check(name+' receiver '+label, () => {
      const p=rect(), revoked=owner.Proxy.revocable(p,{}); revoked.revoke();
      const fake={plain:{},forged:Object.create(proto),inherited:Object.create(p),proxy:new owner.Proxy(p,{}),revoked:revoked.proxy}[label];
      let conversions=0,traps=0;
      const poison=new Proxy({}, {get(){traps++;throw Error('conversion trap');}});
      const number={valueOf(){conversions++;return 1;}};
      const args=name==='addPath'?[p,poison]:Array(arity).fill(number);
      return typeError(() => proto[name].call(fake,...args)) && conversions===0 && traps===0;
    });
  }
  for (const [name,args] of [['moveTo',[10,10]],['lineTo',[10,10]],['quadraticCurveTo',[10,10,20,20]],['bezierCurveTo',[10,10,20,20,30,30]],['arcTo',[10,10,20,20,5]],['arc',[20,20,5,0,Math.PI]],['ellipse',[20,20,5,10,0,0,Math.PI]],['rect',[10,10,20,20]]]) {
    check(name+' arity', () => typeError(() => proto[name].call(new P(),...args.slice(0,-1))));
    check(name+' context receiver rejected', () => typeError(() => proto[name].call(ctx,...args)));
    check(name+' path receiver rejected by context method', () => typeError(() => cp[name].call(new P(),...args)));
    check(name+' cross realm genuine path', () => proto[name].call(new P(),...args) === undefined);
  }
  check('line commands', () => {const p=new P(); proto.moveTo.call(p,10,10); proto.lineTo.call(p,30,10); proto.lineTo.call(p,30,30); proto.lineTo.call(p,10,30); proto.closePath.call(p); clear(); fill(p); return pixel(20,20)===255;});
  check('quadratic curve', () => {const p=new P(); p.moveTo(10,30); p.quadraticCurveTo(20,0,30,30); p.closePath(); clear(); fill(p); return pixel(20,25)===255 && pixel(20,10)===0;});
  check('cubic curve', () => {const p=new P(); p.moveTo(10,30); p.bezierCurveTo(10,10,30,10,30,30); p.closePath(); clear(); fill(p); return pixel(20,25)===255 && pixel(20,10)===0;});
  check('circle', () => {const p=new P(); p.arc(20,20,10,0,Math.PI*2); clear(); fill(p); return pixel(20,20)===255 && pixel(40,20)===0;});
  check('ellipse', () => {const p=new P(); p.ellipse(30,30,20,10,0,0,Math.PI*2); clear(); fill(p); return pixel(30,30)===255 && pixel(30,45)===0;});
  for (const [name,args] of [['arc',[20,20,-1,0,1]],['arcTo',[10,10,20,20,-1]],['ellipse',[20,20,-1,2,0,0,1]]])
    check(name+' negative radius', () => {try {proto[name].call(new P(),...args);} catch(e) {return e instanceof callee.DOMException && e.name==='IndexSizeError';} return false;});
  check('nonfinite path command ignored', () => {const p=rect(); p.rect(Infinity,50,20,20); clear(); fill(p); return pixel(20,20)===255 && pixel(60,60)===0;});
  for (const [name,svg] of [['absolute','M10 10H30V30H10Z'],['relative','m10 10h20v20h-20z'],['implicit','M10 10 30 10 30 30 10 30z'],['invalid suffix','M10 10H30V30H10Z X'],['incomplete suffix','M10 10H30V30H10Z M50'],['arc','M10 20A10 10 0 0 1 30 20A10 10 0 0 1 10 20Z']])
    check('SVG '+name, () => {clear(); fill(new P(svg)); return pixel(20,20)===255 && pixel(50,50)===0;});
  check('SVG conversion once and inherited method', () => {let n=0; clear(); fill(new P(Object.create({toString(){n++;return 'M10 10H30V30H10Z';}}))); return n===1 && pixel(20,20)===255;});
  check('SVG original conversion exception', () => {const sentinel={}; try {new P({toString(){throw sentinel;}});} catch(e) {return e===sentinel;} return false;});
  check('SVG constructor Symbol rejected', () => typeError(() => new callee.Path2D(Symbol())));
  for (const value of [undefined,null,'', 'invalid']) check('empty SVG '+String(value), () => {clear(); fill(new P(value)); return pixel(20,20)===0;});
  check('SVG starts a new subpath at last point', () => {const p=new P('M10 10L30 10L30 30'); p.lineTo(10,30); clear(); cp.stroke.call(ctx,p); return pixel(20,30)===255 && pixel(20,20)===0;});

  for (const [name,transform] of [['omitted',undefined],['null',null],['empty',{}],['identity',{a:1,b:0,c:0,d:1,e:0,f:0}]])
    check('addPath '+name, () => {const p=new P(); proto.addPath.call(p,rect(),transform); clear(); fill(p); return pixel(20,20)===255 && pixel(50,50)===0;});
  check('addPath translation', () => {const p=new P(); proto.addPath.call(p,rect(),{e:40}); clear(); fill(p); return pixel(60,20)===255 && pixel(20,20)===0;});
  check('addPath aliases', () => {const p=new P(); proto.addPath.call(p,rect(),{m41:40,m42:20}); clear(); fill(p); return pixel(60,40)===255 && pixel(20,20)===0;});
  check('addPath snapshot source', () => {const source=rect(),p=new P(); p.addPath(source); source.rect(50,50,20,20); clear(); fill(p); return pixel(20,20)===255 && pixel(60,60)===0;});
  check('addPath self snapshot', () => {const p=rect(); p.addPath(p,{e:40}); clear(); fill(p); return pixel(20,20)===255 && pixel(60,20)===255;});
  check('addPath transform composition', () => {const p=new P(),q=new P(); p.addPath(rect(),{e:20}); q.addPath(p,{a:2,d:2}); clear(); fill(q); return pixel(70,40)===255 && pixel(40,40)===0;});
  check('addPath new subpath endpoint', () => {const source=new P(); source.moveTo(10,10); source.lineTo(30,10); source.lineTo(30,30); const p=new P(); p.addPath(source); p.lineTo(10,30); clear(); cp.stroke.call(ctx,p); return pixel(20,30)===255 && pixel(20,20)===0;});
  for (const [short,long] of [['a','m11'],['b','m12'],['c','m21'],['d','m22'],['e','m41'],['f','m42']])
    check('addPath aliases atomic '+short, () => {const p=rect(); const thrown=typeError(() => proto.addPath.call(p,rect(),{[short]:1,[long]:2})); clear(); fill(p); return thrown && pixel(20,20)===255 && pixel(60,60)===0;});
  for (const value of [NaN,Infinity,-Infinity]) check('addPath nonfinite '+String(value), () => {const p=rect(); proto.addPath.call(p,rect(),{e:value,m41:value}); clear(); fill(p); return pixel(20,20)===255 && pixel(60,20)===0;});
  check('addPath 3D fields ignored', () => {let reads=0; const p=new P(); p.addPath(rect(),{get is2D(){reads++;throw Error('3D');},get m33(){reads++;throw Error('3D');}}); clear(); fill(p); return reads===0 && pixel(20,20)===255;});
  check('addPath ordered dictionary reads', () => {const order=[], init={}; for(const name of ['m42','m41','m22','m21','m12','m11','f','e','d','c','b','a']) Object.defineProperty(init,name,{get(){order.push(name);return undefined;}}); new P().addPath(rect(),init); return order.join() === 'a,b,c,d,e,f,m11,m12,m21,m22,m41,m42';});
  check('addPath getter exception identity', () => {const sentinel={}; try {new P().addPath(rect(),{get e(){throw sentinel;}});} catch(e) {return e===sentinel;} return false;});
  check('addPath invalid source before dictionary reads', () => {let reads=0; return typeError(() => proto.addPath.call(new P(),{}, {get a(){reads++;return 1;}})) && reads===0;});
  check('addPath missing source', () => typeError(() => proto.addPath.call(new P())));
  check('prototype null and frozen native state', () => {const p=rect(); p.__moliCanvasPathState=0; Object.setPrototypeOf(p,null); Object.freeze(p); proto.rect.call(p,50,50,20,20); clear(); fill(p); return pixel(20,20)===255 && pixel(60,60)===255;});
  return checks;
})
