function websocketCloseInfoProbe(realm) {
  const rows=[],failures=[],sentinel={};
  const check=(name,actual,expected)=>{rows.push({name,actual});if(JSON.stringify(actual)!==JSON.stringify(expected))failures.push({name,actual,expected});};
  const cases=[
    ['null closeCode',()=>({closeCode:null}),'InvalidAccessError',[]],
    ['fraction below 1000',()=>({closeCode:999.5}),'InvalidAccessError',[]],
    ['fraction above 1000',()=>({closeCode:1000.5}),'none',[]],
    ['overflow does not wrap',()=>({closeCode:68536}),'InvalidAccessError',[]],
    ['primitive dictionary',()=>true,'TypeError',[]],
    ['throwing code getter',log=>({get closeCode(){log.push('code');throw sentinel;},get reason(){log.push('reason');return '';}}),'sentinel',['code']],
    ['throwing numeric conversion',log=>({get closeCode(){log.push('code');return {valueOf(){log.push('number');throw sentinel;}};},get reason(){log.push('reason');return '';}}),'sentinel',['code','number']],
    ['dictionary before semantic validation',log=>({get closeCode(){log.push('code');return 999;},get reason(){log.push('reason');throw sentinel;}}),'sentinel',['code','reason']],
    ['throwing reason conversion',log=>({get closeCode(){log.push('code');return 3000;},get reason(){log.push('reason');return {toString(){log.push('string');throw sentinel;}};}}),'sentinel',['code','reason','string']],
    ['symbol code stops reason',log=>({get closeCode(){log.push('code');return Symbol();},get reason(){log.push('reason');return '';}}),'TypeError',['code']],
    ['dictionary Get order and inheritance',log=>new Proxy(Object.create({closeCode:3000,reason:'reason'}),{get(target,key,receiver){log.push(String(key));return Reflect.get(target,key,receiver);},ownKeys(){throw sentinel;}}),'none',['closeCode','reason']],
  ];
  for(const [label,make,expected,expectedLog] of cases){
    const log=[],controller=new realm.AbortController();
    const stream=new realm.WebSocketStream('wss://127.0.0.1:65534/',{signal:controller.signal});
    stream.opened.catch(()=>{});stream.closed.catch(()=>{});
    let caught;
    try{stream.close(make(log));}catch(error){caught=error;}finally{controller.abort();}
    const kind=caught===sentinel?'sentinel':caught===undefined?'none':caught instanceof realm.TypeError?'TypeError':caught instanceof realm.DOMException?caught.name:'foreign exception';
    check(label+': callee exception',kind,expected);
    check(label+': conversion order',log,expectedLog);
  }
  return {rows,failures};
}
