(() => {
  const names = ['onanimationcancel','onanimationend','onanimationiteration','onanimationstart','onauxclick','onbeforeinput','onbeforetoggle','oncontextlost','oncontextrestored','oncuechange','onformdata','ongamepadconnected','ongamepaddisconnected','ongotpointercapture','onlanguagechange','onlostpointercapture','onmouseenter','onmouseleave','onpointercancel','onpointerdown','onpointerenter','onpointerleave','onpointermove','onpointerout','onpointerover','onpointerup','onscrollend','onsecuritypolicyviolation','onselectionchange','onselectstart','onslotchange','ontoggle','ontransitioncancel','ontransitionend','ontransitionrun','ontransitionstart','onwebkitanimationend','onwebkitanimationiteration','onwebkitanimationstart','onwebkittransitionend','onwheel'];
  for (const w of [window, document.getElementById('child').contentWindow]) {
    for (const name of names) {
      const d=Object.getOwnPropertyDescriptor(w,name);
      if (!d || !d.enumerable || !d.configurable || typeof d.get !== 'function' || typeof d.set !== 'function' || w[name] !== null) throw Error(name+': descriptor');
      const type=name.slice(2);let calls=0;
      w[name]=function(e){if(this!==w || e.type!==type) throw Error(name+': callback');calls++;return false;};
      if(typeof w[name] !== 'function') throw Error(name+': native handler storage');
      if(!name.startsWith('onwebkit') && (w.dispatchEvent(new Event(type,{cancelable:true})) || calls!==1)) throw Error(name+': dispatch');
      w[name]=1;if(w[name]!==null) throw Error(name+': primitive');
    }
    for (const target of [w.document,w.document.createElement('div'),w.document.createElementNS('http://www.w3.org/2000/svg','svg')]) {
      if(target.onselectstart!==null) throw Error('DOM initial handler');let calls=0;
      target.onselectstart=function(){if(this!==target)throw Error('DOM receiver');calls++;return false;};
      if(target.dispatchEvent(new Event('selectstart',{cancelable:true})) || calls!==1)throw Error('DOM dispatch');target.onselectstart=null;
    }
  }
  return true;
})()
