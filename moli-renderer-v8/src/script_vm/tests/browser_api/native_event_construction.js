(() => {
  const NativeObject = Object;
  const arrayIteratorPrototype=NativeObject.getPrototypeOf([][Symbol.iterator]());
  const names = ['Event','UIEvent','KeyboardEvent','MouseEvent','PointerEvent','WheelEvent',
    'DragEvent','TouchEvent','Touch','TouchList','FocusEvent','InputEvent','ClipboardEvent',
    'SubmitEvent','CommandEvent','ToggleEvent','InterestEvent','TrackEvent'];
  const constructors = NativeObject.create(null);
  for (let i=0;i<names.length;i++) constructors[names[i]]=globalThis[names[i]];
  globalThis.nativeTarget=document.body.appendChild(document.createElement('button'));
  globalThis.nativePeer=document.body.appendChild(document.createElement('button'));
  globalThis.nativeTransfer=new DataTransfer();
  const video=document.body.appendChild(document.createElement('video'));
  globalThis.nativeTrack=video.addTextTrack('subtitles','native','en');
  globalThis.nativeTrackList=video.textTracks;
  const rows=Array(15).fill(null), restore=[];
  let reads=0, rowIndex=0;
  const poison=()=>{reads++;throw Error('native initialization ran author code');};
  const cases=[
    ['copy','ClipboardEvent',e=>e.clipboardData===nativeTransfer],
    ['beforeinput','InputEvent',e=>e.data==='k'&&e.inputType==='insertText'],
    ['mousemove','MouseEvent',e=>e.clientX===11&&e.clientY===12&&e.detail===2],
    ['pointerdown','PointerEvent',e=>e.clientX===11&&e.clientY===12&&e.pointerType==='mouse'],
    ['dragstart','DragEvent',e=>e.dataTransfer===nativeTransfer],
    ['wheel','WheelEvent',e=>e.deltaX===4&&e.deltaY===5&&e.deltaZ===0],
    ['touchstart','TouchEvent',e=>e.touches instanceof constructors.TouchList &&
      e.touches.length===2&&e.targetTouches.length===1&&e.changedTouches.length===1&&
      e.touches[0] instanceof constructors.Touch&&e.touches[0].identifier===11&&
      e.touches[0].target===nativeTarget&&e.touches[0].clientX===11&&e.touches[0].force===0&&
      e.touches[1].identifier===12&&e.touches[1].target===nativePeer],
    ['keydown','KeyboardEvent',e=>e.key==='k'&&e.code==='KeyK'&&e.location===0&&e.keyCode===75],
    ['focusin','FocusEvent',e=>e.relatedTarget===nativePeer],
    ['change','Event',e=>e.bubbles&&!e.cancelable&&!e.composed],
    ['submit','SubmitEvent',e=>e.submitter===nativeTarget],
    ['command','CommandEvent',e=>e.command==='--test'&&e.source===nativePeer],
    ['beforetoggle','ToggleEvent',e=>e.oldState==='closed'&&e.newState==='open'&&e.source===nativePeer],
    ['interest','InterestEvent',e=>e.source===nativePeer],
    ['addtrack','TrackEvent',e=>e.track===nativeTrack]
  ];
  for(let i=0;i<cases.length;i++){
    const type=cases[i][0], name=cases[i][1], payload=cases[i][2];
    const target=type==='addtrack'?nativeTrackList:nativeTarget;
    target.addEventListener(type,e=>{rows[rowIndex++]={type,checks:{
      interface:e instanceof constructors[name],
      prototype:NativeObject.getPrototypeOf(e)===constructors[name].prototype,
      trusted:e.isTrusted===true,
      target:e.target===target&&e.currentTarget===target,
      payload:payload(e),
      view:!(e instanceof constructors.UIEvent)||e.view===window,
      inherited:!(e instanceof constructors.UIEvent)||!NativeObject.hasOwn(e,'view')
    }};});
  }
  function savePoison(object,key,descriptor){
    restore.push([object,key,NativeObject.getOwnPropertyDescriptor(object,key)]);
    NativeObject.defineProperty(object,key,descriptor);
  }
  globalThis.poisonNativeEventInputs=mode=>{
    if(mode==='ordinary')return;
    for(let i=0;i<names.length;i++){
      const name=names[i];
      if(mode==='deleted'){
        restore.push([globalThis,name,NativeObject.getOwnPropertyDescriptor(globalThis,name)]);
        delete globalThis[name];
      }else savePoison(globalThis,name,{configurable:true,...(mode==='getter'?{get:poison}:{value:poison,writable:true})});
    }
    const members=['view','detail','location','isComposing','screenX','screenY',
      'radiusX','radiusY','force','rotationAngle','deltaZ','altitudeAngle','azimuthAngle'];
    for(let i=0;i<members.length;i++)savePoison(NativeObject.prototype,members[i],{configurable:true,get:poison});
    savePoison(NativeObject.prototype,'0',{configurable:true,set:poison});
    savePoison(Array.prototype,Symbol.iterator,{configurable:true,get:poison});
    savePoison(arrayIteratorPrototype,'next',{configurable:true,get:poison});
    savePoison(globalThis,'Object',{configurable:true,get:poison});
  };
  globalThis.readNativeEventProducerProbe=()=>{
    for(let i=restore.length-1;i>=0;i--){
      const entry=restore[i];
      if(entry[2])NativeObject.defineProperty(entry[0],entry[1],entry[2]);
      else delete entry[0][entry[1]];
    }
    return {reads,rows};
  };
  return true;
})()
