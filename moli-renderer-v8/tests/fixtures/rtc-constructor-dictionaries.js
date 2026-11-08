(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const cases = [[RTCSessionDescription, ['sdp', 'type'], {type:'offer'}],
    [RTCIceCandidate, ['candidate', 'sdpMLineIndex', 'sdpMid', 'usernameFragment'], {sdpMid:'0'}]];
  for (const [C,members,values] of cases) {
    for (const value of [1, true, 'init', 1n, Symbol()]) {
      let caught; try {new C(value);} catch(e) {caught=e;}
      assert(caught instanceof TypeError, C.name + ' primitive dictionary');
    }
    const reads = [];
    new C(new Proxy({}, {get(_,key) {
      if(members.includes(key)) reads.push(key); return key in values ? values[key] : undefined;
    }, has() {throw Error('Get required');}}));
    assert(reads.join() === members.join(), C.name + ' lexical order: ' + reads);
    for (const member of members) for (const sentinel of [{}, null, undefined, 7]) {
      reads.length=0;let caught=Symbol();
      try {new C(new Proxy({}, {get(_,key) {
        if(members.includes(key)) reads.push(key);
        if(key === member) throw sentinel;return key in values ? values[key] : undefined;
      }}));} catch(e) {caught=e;}
      assert(caught === sentinel, C.name + '/' + member + ' original exception');
      assert(reads.join() === members.slice(0,members.indexOf(member)+1).join(), C.name + '/' + member + ' stop');
    }
  }
  for (const input of [undefined,null,{}, {type:undefined}]) {
    const value = new RTCSessionDescription(input);
    assert(value.type === null && value.sdp === '' && value.toJSON().type === null, 'legacy optional description type');
  }
  for (const type of [null,'bad',Symbol()]) {
    let caught;try {new RTCSessionDescription({type});} catch(e) {caught=e;}
    assert(caught instanceof TypeError, 'SDP enum validation');
  }
  const raw = '\ud800\0\udc00';
  const description = new RTCSessionDescription({type:'offer',sdp:raw});
  assert(description.sdp === raw && description.toJSON().sdp === raw, 'SDP lossless UTF16');
  const candidate = new RTCIceCandidate({candidate:raw,sdpMid:raw,sdpMLineIndex:1,usernameFragment:raw});
  assert(candidate.candidate === raw && candidate.sdpMid === raw && candidate.usernameFragment === raw, 'ICE lossless UTF16');
  assert(candidate.toJSON().candidate === raw && candidate.toJSON().usernameFragment === raw, 'ICE JSON lossless UTF16');
  for (const [value,expected] of [[-1,65535],[65536,0],[1.9,1],[NaN,0],[Infinity,0]])
    assert(new RTCIceCandidate({sdpMid:'0',sdpMLineIndex:value}).sdpMLineIndex === expected, 'ICE unsigned short conversion');
  for (const value of [undefined,null,{}]) {
    let caught;try {new RTCIceCandidate(value);} catch(e) {caught=e;}
    assert(caught instanceof TypeError, 'ICE needs mid or line after conversion');
  }
  const sentinel={};let later=0,caught;
  try {new RTCSessionDescription({sdp:{toString(){throw sentinel;}},get type(){later++;return 'offer';}});} catch(e){caught=e;}
  assert(caught===sentinel && later===0, 'SDP ToString abrupt completion');
  let stringCalls=0;
  const wrapped={toString(){stringCalls++;return raw;}};
  assert(new RTCSessionDescription({type:'offer',sdp:wrapped}).sdp===raw && stringCalls===1,'SDP coerces once');
  return true;
})()
