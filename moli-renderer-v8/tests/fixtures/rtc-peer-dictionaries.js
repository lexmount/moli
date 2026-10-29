(async () => {
  const assert=(value,label)=>{if(!value)throw Error(label);};
  const peer=new RTCPeerConnection();peer.close();
  const cases=[
    ['createOffer',x=>peer.createOffer(x),['voiceActivityDetection','iceRestart','offerToReceiveAudio','offerToReceiveVideo'],true],
    ['setLocalDescription',x=>peer.setLocalDescription(x),['sdp','type'],true],
    ['createDataChannel',x=>peer.createDataChannel('label',x),['id','maxPacketLifeTime','maxRetransmits','negotiated','ordered','protocol'],false]
  ];
  for(const [name,call,members,promised] of cases) {
    for(const key of members)for(const sentinel of [{},null,undefined,7]){
      const reads=[];let caught=Symbol(),returned=false;
      try{const value=call(new Proxy({}, {get(_,member){reads.push(member);if(member===key)throw sentinel;return undefined;},has(){throw Error('Get required');}}));returned=value instanceof Promise;await value;}catch(e){caught=e;}
      assert(returned===promised && caught===sentinel,name+'/'+key+' error boundary');
      assert(reads.join()===members.slice(0,members.indexOf(key)+1).join(),name+'/'+key+' conversion order');
    }
    const reads=[];let caught;
    try{await call(new Proxy({}, {get(_,key){reads.push(key);return undefined;}}));}catch(e){caught=e;}
    assert(caught?.name==='InvalidStateError' && reads.join()===members.join(),name+' full conversion before closed state');
    for(const value of [1,true,'options',1n,Symbol()]){
      let caught;try{await call(value);}catch(e){caught=e;}
      assert(caught instanceof TypeError,name+' primitive dictionary');
    }
  }
  for(const key of ['offerToReceiveAudio','offerToReceiveVideo']){
    const sentinel={};let caught;
    try{await peer.createOffer({[key]:{valueOf(){throw sentinel;}}});}catch(e){caught=e;}
    assert(caught===sentinel,'offer long conversion');
  }
  const live=new RTCPeerConnection();
  try{
    const channel=live.createDataChannel('\ud800',{ordered:false,negotiated:true,id:7,protocol:'\ud800',maxRetransmits:null});
    assert(channel.label==='\ufffd' && channel.protocol==='\ufffd' && !channel.ordered && channel.negotiated && channel.id===7 && channel.maxRetransmits===0 && channel.maxPacketLifeTime===null,'native channel options');
    channel.close();
    const wrapped=live.createDataChannel('wrapped',{maxPacketLifeTime:65536});
    assert(wrapped.maxPacketLifeTime===0,'ordinary ushort wraps');wrapped.close();
    for(const init of [{},{id:undefined}]) {
      const missing=live.createDataChannel('missing',init);
      assert(missing.id===null,'missing or undefined id snapshot default');missing.close();
    }
    for(const [id,expected] of [[null,0],[false,0],[true,1],['7',7]]) {
      const converted=live.createDataChannel('converted',{negotiated:true,id});
      assert(converted.id===expected,'non-nullable id numeric conversion');converted.close();
    }
    let idReads=0;
    const coerced=live.createDataChannel('coerced',{negotiated:true,id:{valueOf(){idReads++;return 7;}}});
    assert(coerced.id===7 && idReads===1,'id conversion once');coerced.close();
    for(const init of [{id:-1},{id:65536},{id:65535},{negotiated:true},{negotiated:true,id:undefined},{maxPacketLifeTime:1,maxRetransmits:1}]){
      let caught;try{live.createDataChannel('bad',init);}catch(e){caught=e;}
      assert(caught instanceof TypeError,'invalid channel options');
    }
    let caught;try{await live.createOffer({offerToReceiveAudio:2});}catch(e){caught=e;}
    assert(caught?.name==='InvalidAccessError','unsupported offer receive count');
  }finally{live.close();}
  return true;
})()
