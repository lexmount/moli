(async () => {
  const facts = {complete:false, checks:[], observations:[]};
  globalThis.__uiEventResults = facts;
  const assert = (condition, message) => {if (!condition) throw Error(message)};
  const members = ['aspectRatio','autoGainControl','backgroundBlur','backgroundSegmentationMask','brightness','channelCount',
    'colorTemperature','contrast','cursor','deviceId','displaySurface','echoCancellation','exposureCompensation','exposureMode',
    'exposureTime','eyeGazeCorrection','faceFraming','facingMode','focusDistance','focusMode','frameRate','gestureReactions',
    'groupId','height','humanFaceDetectionMode','iso','latency','logicalSurface','noiseSuppression','pan','pointsOfInterest',
    'resizeMode','restrictOwnAudio','sampleRate','sampleSize','saturation','sharpness','suppressLocalAudioPlayback',
    'tilt','torch','voiceIsolation','whiteBalanceMode','width','zoom'];
  for (const [label, w, other] of [['main', globalThis, document.querySelector('iframe').contentWindow],
      ['iframe', document.querySelector('iframe').contentWindow, globalThis]]) {
    const pc = new w.RTCPeerConnection();
    const audio = pc.addTransceiver('audio').receiver.track;
    const video = pc.addTransceiver('video').receiver.track;
    const apply = w.MediaStreamTrack.prototype.applyConstraints;
    const test = async (name, body) => {
      const row = {name:label+':'+name,passed:false};facts.checks.push(row);
      try {await body();row.passed=true} catch(error) {row.error=String(error)}
    };
    const rejected = async (receiver, request, constructor, method=apply) => {
      let promise, error, caught=false;
      try {promise=method.call(receiver,request)} catch(error) {throw Error('synchronous exception: '+error)}
      assert(promise instanceof (method===apply?w:other).Promise, 'Promise callee realm');
      try {await promise} catch(value) {error=value;caught=true}
      assert(caught, 'expected rejection');
      if(constructor)assert(error instanceof constructor, 'rejection constructor');
      return error;
    };
    try {
      await test('method-descriptor', () => {
        const d=w.Object.getOwnPropertyDescriptor(w.MediaStreamTrack.prototype,'applyConstraints');
        assert(typeof apply==='function'&&apply.length===0&&apply.name==='applyConstraints', 'method identity');
        assert(d.enumerable&&d.configurable&&d.writable&&!w.Object.hasOwn(audio,'applyConstraints'),'prototype descriptor');
      });
      for(const [name,request] of [['omitted',undefined],['null',null],['empty',{}],['unknown',{notAConstraint:true}]]) {
        await test('empty-'+name, async () => {
          const promise=name==='omitted'?apply.call(audio):apply.call(audio,request);
          assert(promise instanceof w.Promise, 'native Promise');
          assert(await promise===undefined&&Reflect.ownKeys(audio.getConstraints()).length===0,'empty constraints');
        });
      }
      await test('unknown-getter-not-read', async () => {
        await apply.call(audio, {get unknown(){throw Error('unknown getter')}});
      });
      for(const [index,receiver] of [null,undefined,{},Object.create(audio),Object.create(w.MediaStreamTrack.prototype),
          new Proxy(audio,{get(){throw Error('author Proxy trap')},getPrototypeOf(){throw Error('prototype trap')}}),
          (()=>{const r=Proxy.revocable(audio,{});r.revoke();return r.proxy})()].entries()) {
        await test('invalid-receiver-'+index, async () => {
          let gets=0;
          const request=new Proxy({},{get(){gets++;throw Error('input getter')}});
          await rejected(receiver,request,w.TypeError);
          assert(gets===0,'receiver check precedes dictionary conversion');
        });
      }
      await test('borrowed-rejection-realm', async () => {
        const method=other.MediaStreamTrack.prototype.applyConstraints;
        const error=await rejected({}, {}, other.TypeError, method);
        assert(!(error instanceof w.TypeError),'callee TypeError');
      });
      for(const [index,request] of [true,42,'constraints',Symbol('constraints')].entries()) {
        await test('invalid-dictionary-'+index, () => rejected(audio,request,w.TypeError));
      }
      await test('merged-dictionary-getter-order', async () => {
        const log=[];
        await apply.call(audio,new Proxy({}, {get(target,key){log.push(String(key));return undefined}}));
        assert(log.join('|')===[...members,'advanced'].join('|'),'merged base members precede derived advanced: '+log);
      });
      await test('inherited-range-getter-order', async () => {
        const log=[], marker={};
        const range={get max(){log.push('max');return 30},get min(){log.push('min');return 1},
          get exact(){log.push('exact');return 24},get ideal(){log.push('ideal');throw marker}};
        const error=await rejected(audio,{frameRate:range});
        assert(error===marker&&log.join('|')==='max|min|exact|ideal','range inheritance and exception identity');
      });
      for(const [index,request] of [{aspectRatio:NaN},{frameRate:Infinity},{latency:-Infinity},
          {width:Symbol('width')},{facingMode:Symbol('mode')},{advanced:null},{advanced:[1]},
          {pointsOfInterest:[{x:Infinity}]},{pointsOfInterest:{ideal:[{y:NaN}]}}].entries()) {
        await test('conversion-rejection-'+index, () => rejected(audio,request,w.TypeError));
      }
      for(const name of ['width','height','frameRate','aspectRatio','facingMode','echoCancellation','advanced']) {
        await test('getter-exception-'+name, async () => {
          const marker={name}, request={};Object.defineProperty(request,name,{get(){throw marker}});
          assert(await rejected(audio,request)===marker,'getter exception identity');
        });
      }
      await test('string-sequence-iterator-once', async () => {
        const log=[],marker={};
        const iterable={get [Symbol.iterator](){log.push('iterator');return function(){return {
          next(){log.push('next');return {done:false,value:{toString(){log.push('string');throw marker}}}},
          get return(){throw Error('IteratorClose')}}}}};
        assert(await rejected(audio,{facingMode:{ideal:iterable}})===marker,'item exception identity');
        assert(log.join('|')==='iterator|next|string','one iterator probe and no IteratorClose');
      });
      await test('advanced-sequence-item-before-next', async () => {
        const log=[],marker={};
        const advanced={[Symbol.iterator](){log.push('iterator');return {next(){log.push('next');return {
          done:false,value:{get width(){log.push('width');throw marker}}}},get return(){throw Error('IteratorClose')}}}};
        assert(await rejected(audio,{advanced})===marker,'advanced conversion exception');
        assert(log.join('|')==='iterator|next|width','advanced converts each dictionary before next');
      });
      await test('point-dictionary-member-order', async () => {
        const log=[],marker={};
        const points=[{get x(){log.push('x');return 1},get y(){log.push('y');throw marker}}];
        assert(await rejected(audio,{pointsOfInterest:points})===marker&&log.join('|')==='x|y','Point2D conversion');
      });
      for(const name of ['width','height','frameRate','aspectRatio']) {
        for(const [form,value] of [['bare',2],['ideal',{ideal:2}],['exact',{exact:2}],['advanced',2]]) {
          await test('remote-video-'+name+'-'+form, async () => {
            const track=pc.addTransceiver('video').receiver.track;
            try {
              const request=form==='advanced'?{advanced:[{[name]:value}]}:{[name]:value};
              const error=await rejected(track,request,w.OverconstrainedError);
              assert(error instanceof w.DOMException&&error.name==='OverconstrainedError'&&typeof error.constraint==='string','remote rejection brand and constraint type');
              // WebRTC specifies rejection here without prescribing which
              // diagnostic constraint string the implementation must choose.
              facts.observations.push({realm:label,constraint:name,form,errorConstraint:error.constraint});
              assert(Reflect.ownKeys(track.getConstraints()).length===0,'failed request retains previous constraints');
            } finally {track.stop()}
          });
        }
      }
      await test('remote-audio-inapplicable-properties', async () => {
        const request={width:{exact:2},facingMode:{exact:['user','\ud800']}};
        await apply.call(audio,request);
        const snapshot=audio.getConstraints();
        assert(snapshot.width.exact===2&&snapshot.facingMode.exact[1]==='\ud800','ignored source properties remain converted constraints');
        assert(Reflect.ownKeys(audio.getCapabilities()).length===0&&Reflect.ownKeys(audio.getSettings()).length===0,'no synthetic capabilities or settings');
        request.width.exact=99;snapshot.facingMode.exact.length=0;
        assert(audio.getConstraints().width.exact===2&&audio.getConstraints().facingMode.exact.length===2,'deep independent snapshots');
      });
      await test('borrowed-success-and-snapshot-realm', async () => {
        await other.MediaStreamTrack.prototype.applyConstraints.call(audio,{});
        const snapshot=other.MediaStreamTrack.prototype.getConstraints.call(audio);
        assert(Object.getPrototypeOf(snapshot)===other.Object.prototype&&Reflect.ownKeys(snapshot).length===0,'dictionary callee realm');
      });
      await test('global-constructor-tampering', async () => {
        const Constructor=w.OverconstrainedError;let calls=0;
        w.OverconstrainedError=function(){calls++;throw Error('author constructor')};
        try {const error=await rejected(video,{width:1},Constructor);assert(calls===0&&error.constraint==='width','intrinsic rejection')}
        finally {w.OverconstrainedError=Constructor}
      });
      await test('ended-track-still-converts', async () => {
        const track=audio.clone(),marker={};track.stop();
        const error=await rejected(track,{get width(){throw marker}});
        assert(error===marker,'conversion before ended short circuit');
      });
      await test('ended-track-does-not-change-constraints', async () => {
        const track=video.clone();track.stop();
        const before=JSON.stringify(track.getConstraints());
        assert(await apply.call(track,{width:{exact:999}})===undefined,'ended resolution');
        assert(JSON.stringify(track.getConstraints())===before,'ended constraints unchanged');
      });
      if(w.isSecureContext) {
        await test('supported-constraints-descriptor', () => {
          const d=w.Object.getOwnPropertyDescriptor(w.MediaDevices.prototype,'getSupportedConstraints');
          assert(d.value.length===0&&d.value.name==='getSupportedConstraints'&&d.enumerable&&d.writable&&d.configurable,'support method descriptor');
        });
        await test('supported-constraints-vocabulary', () => {
          const snapshot=w.navigator.mediaDevices.getSupportedConstraints();
          assert(members.every(name=>snapshot[name]===true)&&Object.values(snapshot).every(value=>value===true),'recognized merged vocabulary');
          assert(!('powerEfficient' in snapshot)&&!('powerEfficientPixelFormat' in snapshot),'read-only settings not constrainable');
          snapshot.width=false;assert(w.navigator.mediaDevices.getSupportedConstraints().width===true,'fresh support snapshot');
        });
        await test('supported-constraints-receiver-and-realm', () => {
          const method=other.MediaDevices.prototype.getSupportedConstraints;
          const snapshot=method.call(w.navigator.mediaDevices);
          assert(Object.getPrototypeOf(snapshot)===other.Object.prototype,'support dictionary callee realm');
          for(const receiver of [{},Object.create(w.navigator.mediaDevices),new Proxy(w.navigator.mediaDevices,{})]) {
            let error;try{method.call(receiver)}catch(caught){error=caught}
            assert(error instanceof other.TypeError,'support brand check');
          }
        });
      }
    } finally {audio.stop();video.stop();pc.close()}
  }
  facts.complete=true;facts.total=facts.checks.length;facts.passed=facts.checks.filter(row=>row.passed).length;
  return facts.passed===facts.total;
})()
