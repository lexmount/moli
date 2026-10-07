(() => {
  const check=(value,name)=>{if(!value)throw new Error(name)};
  const event=nativeTimeEvent;
  const realm=document.querySelector('iframe').contentWindow;
  const init=realm.TimeEvent.prototype.initTimeEvent;
  check(event instanceof TimeEvent && event instanceof Event,'native interfaces');
  check(event.isTrusted && event.type==='repeatEvent' && event.detail===-1 && event.view===window,'native payload and long conversion');
  check(Object.getPrototypeOf(event)===TimeEvent.prototype,'native prototype');
  check(!Object.hasOwn(event,'view') && !Object.hasOwn(event,'detail'),'prototype attribute placement');
  check(Object.getPrototypeOf(TimeEvent.prototype)===Event.prototype,'Event inheritance');
  for(const R of [window,realm]){
    try { new R.TimeEvent(); throw 42; } catch(error){check(Object.getPrototypeOf(error)===R.TypeError.prototype,'illegal constructor realm')}
  }
  let conversions=0,traps=0;
  const text={toString(){conversions++;return 'changed'}};
  const numeric={valueOf(){conversions++;return 7}};
  const proxy=new Proxy(event,{get(){traps++;throw 42},getPrototypeOf(){traps++;throw 42}});
  const revoked=Proxy.revocable(event,{});revoked.revoke();
  for(const receiver of [{},Object.create(TimeEvent.prototype),Object.create(event),proxy,revoked.proxy]){
    try { init.call(receiver,text,window,numeric); throw 42; }
    catch(error){check(Object.getPrototypeOf(error)===realm.TypeError.prototype,'receiver error realm')}
  }
  check(conversions===0 && traps===0,'receiver validation before argument conversion');
  const original={marker:1};
  try { init.call(event,{toString(){throw original}},window,numeric);throw 42 }
  catch(error){check(error===original,'original conversion exception')}
  check(conversions===0 && event.type==='repeatEvent','conversion failure leaves state intact');
  const stamp=event.timeStamp;
  init.call(event,'x\ud800',realm,4294967295);
  check(event.type==='x\ud800' && event.detail===-1 && event.view===realm,'UTF-16 and cross realm Window');
  check(!event.isTrusted && event.timeStamp===stamp,'legacy initialization preserves creation time');
  init.call(event,'again');
  check(event.view===null && event.detail===0 && !event.bubbles && !event.cancelable,'optional argument defaults');
  init.call(nativeTimeEventProxy,'native-proxy',realm,9);
  check(event.type==='native-proxy' && event.view===realm && event.detail===9,'registered native Proxy identity');
  const detail=Object.getOwnPropertyDescriptor(realm.TimeEvent.prototype,'detail').get;
  check(detail.call(nativeTimeEventProxy)===9,'registered native Proxy accessor');
  init.call(event,'again');
  event.added=1;
  const target=new EventTarget();
  target.addEventListener('again',()=>{
    init.call(event,text,window,numeric);
    check(event.type==='again' && event.view===null && event.detail===0,'dispatching initialization is ignored');
  });
  target.dispatchEvent(event);
  check(conversions===2,'dispatching events still convert arguments');
  const Original=TimeEvent;
  globalThis.TimeEvent=function(){throw 42};
  try { check(event instanceof Original,'author replacement retains native identity') }
  finally { globalThis.TimeEvent=Original }
  try { document.createEvent('TimeEvent');throw 42 }
  catch(error){check(error.name==='NotSupportedError','TimeEvent is not a legacy createEvent alias')}
  return 'time-event:ok';
})()
