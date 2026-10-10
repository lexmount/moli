use super::*;

#[tokio::test(flavor = "current_thread")]
async fn media_track_constraints_public_conversion_rejections_and_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://track-constraints.test/",
        "http://track-constraints.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/media-track-constraints.js"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "constraint fixture must finish through native Page tasks",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(vm.eval("__uiEventResults.total > 100 && new Set(__uiEventResults.checks.map(row=>row.name)).size===__uiEventResults.total").unwrap(), "true");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn media_track_constraints_native_snapshots_commit_in_task_order_and_clone_independently() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://native-constraints.test/",
        &loader,
    );
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    super::media_streams::install_inert_media_tracks(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const other=document.querySelector('iframe').contentWindow;
      globalThis.trace=[];globalThis.failure=null;globalThis.done=false;
      const first={echoCancellation:{ideal:false}};
      const second={width:{max:Infinity,min:-1,exact:3.5,ideal:2.5},
        facingMode:{exact:['user','\ud800']},pointsOfInterest:{ideal:[{x:0.25}]},
        advanced:[{sampleRate:{exact:48000}},{width:12}]};
      // A video-only required width is inapplicable to this audio source.
      const p1=other.MediaStreamTrack.prototype.applyConstraints.call(nativeaudio,first);
      const p2=MediaStreamTrack.prototype.applyConstraints.call(audio,second);
      if(!(p1 instanceof other.Promise)||!(p2 instanceof Promise))throw Error('Promise realms');
      globalThis.before=audio.clone();
      if(Reflect.ownKeys(audio.getConstraints()).length||Reflect.ownKeys(before.getConstraints()).length)throw Error('premature commit');
      first.echoCancellation.ideal=true;second.facingMode.exact[1]='changed';second.pointsOfInterest.ideal[0].x=1;
      p1.then(()=>trace.push(JSON.stringify(audio.getConstraints())));
      p2.then(()=>trace.push(JSON.stringify(audio.getConstraints())));
      Promise.all([p1,p2]).then(async()=>{
        const assert=(ok,message)=>{if(!ok)throw Error(message)};
        assert(trace.length===2&&trace[0]==='{"echoCancellation":{"ideal":false}}','first task checkpoint sees first commit');
        const snapshot=other.MediaStreamTrack.prototype.getConstraints.call(nativeaudio);
        assert(Object.getPrototypeOf(snapshot)===other.Object.prototype&&Object.getPrototypeOf(snapshot.width)===other.Object.prototype,'nested snapshot realm');
        assert(Object.getPrototypeOf(snapshot.advanced)===other.Array.prototype&&Object.getPrototypeOf(snapshot.facingMode.exact)===other.Array.prototype,'sequence realm');
        assert(snapshot.width.max===4294967295&&snapshot.width.min===0&&snapshot.width.exact===4&&snapshot.width.ideal===2,'Clamp ties-to-even and saturation');
        assert(snapshot.facingMode.exact[1]==='\ud800'&&snapshot.pointsOfInterest.ideal[0].x===0.25&&snapshot.pointsOfInterest.ideal[0].y===0,'owned strings and point defaults');
        assert(snapshot.advanced.length===2&&snapshot.advanced[0].sampleRate.exact===48000,'advanced order retained even when sets cannot match');
        snapshot.width.exact=999;snapshot.facingMode.exact.length=0;snapshot.pointsOfInterest.ideal[0].y=999;snapshot.advanced.length=0;
        assert(audio.getConstraints().width.exact===4&&audio.getConstraints().facingMode.exact.length===2&&audio.getConstraints().pointsOfInterest.ideal[0].y===0&&audio.getConstraints().advanced.length===2,'deep snapshot independence');
        const clone=audio.clone();globalThis.clone=clone;
        assert(JSON.stringify(clone.getConstraints())===trace[1]&&Reflect.ownKeys(before.getConstraints()).length===0,'clone copies committed state');
        const prior=JSON.stringify(audio.getConstraints());
        let error;try{await other.MediaStreamTrack.prototype.applyConstraints.call(nativeaudio,{sampleRate:{exact:48000}})}catch(caught){error=caught}
        assert(error instanceof other.OverconstrainedError&&error instanceof other.DOMException&&error.constraint==='sampleRate','native error brand and realm');
        assert(JSON.stringify(audio.getConstraints())===prior,'failure is atomic');
        await clone.applyConstraints({facingMode:{exact:[]},pan:true,echoCancellation:{ideal:'remote-only'}});
        assert(clone.getConstraints().pan===true&&clone.getConstraints().facingMode.exact.length===0&&JSON.stringify(audio.getConstraints())===prior,'clone has independent constraints');
        await audio.applyConstraints();
        assert(Reflect.ownKeys(audio.getConstraints()).length===0&&clone.getConstraints().pan===true,'empty request replaces constraints without changing clone');
        audio.stop();clone.stop();before.stop();
      }).then(()=>done=true,error=>{failure=String(error);done=true});return true;
    })()"#).unwrap(), "true");
    assert_eq!(
        vm.eval("trace.length===0 && Reflect.ownKeys(audio.getConstraints()).length===0")
            .unwrap(),
        "true"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "native constraint commits and rejections",
    )
    .await;
    assert_eq!(vm.eval("String(failure)").unwrap(), "null");
}

#[tokio::test(flavor = "current_thread")]
async fn media_track_constraints_reentrant_conversion_and_ended_tracks_preserve_atomic_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://reentrant-constraints.test/",
        &loader,
    );
    super::media_streams::install_inert_media_tracks(&mut vm);
    vm.eval(r#"globalThis.done=false;globalThis.failure=null;globalThis.order=[];
      const outer={get facingMode(){
        audio.applyConstraints({width:{ideal:11}}).then(()=>order.push(audio.getConstraints().width.ideal));
        return {ideal:['environment']};
      }};
      audio.applyConstraints(outer).then(async()=>{
        if(order.join()!=='11'||audio.getConstraints().facingMode.ideal[0]!=='environment')throw Error('conversion reentrancy admission order');
        const prior=JSON.stringify(audio.getConstraints());audio.stop();
        const marker={};let error;
        try{await audio.applyConstraints({width:{get exact(){throw marker}}})}catch(caught){error=caught}
        if(error!==marker)throw Error('ended conversion exception');
        await audio.applyConstraints({sampleRate:{exact:48000}});
        if(JSON.stringify(audio.getConstraints())!==prior)throw Error('ended mutation');
        const clone=audio.clone();await clone.applyConstraints({});
        if(JSON.stringify(clone.getConstraints())!==prior)throw Error('ended clone state');
      }).then(()=>done=true,error=>{failure=String(error);done=true});'queued'"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(done)",
        "true",
        "reentrant and ended constraint semantics",
    )
    .await;
    assert_eq!(vm.eval("String(failure)").unwrap(), "null");
}

#[tokio::test(flavor = "current_thread")]
async fn media_track_constraints_exact_owner_retirement_does_not_use_the_borrowed_callee() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://constraint-owner.test/",
        &loader,
    );
    vm.eval(r#"document.body.innerHTML='<iframe></iframe>';
      const frame=document.querySelector('iframe'),other=frame.contentWindow;
      const pc=new RTCPeerConnection(),track=pc.addTransceiver('audio').receiver.track;
      const childPC=new other.RTCPeerConnection(),childTrack=childPC.addTransceiver('audio').receiver.track;
      const borrowed=other.MediaStreamTrack.prototype.applyConstraints;
      globalThis.childSettled=false;globalThis.activeSettled=false;
      MediaStreamTrack.prototype.applyConstraints.call(childTrack,{}).then(()=>childSettled=true,()=>childSettled=true);
      frame.remove();
      borrowed.call(track,{}).then(()=>activeSettled=true);
      MediaStreamTrack.prototype.applyConstraints.call(childTrack,{}).then(()=>childSettled=true,()=>childSettled=true);
      'queued'"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(activeSettled)",
        "true",
        "active receiver with discarded callee realm",
    )
    .await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(vm.eval("!childSettled && Reflect.ownKeys(childTrack.getConstraints()).length===0 && activeSettled").unwrap(), "true");
    vm.eval("pc.close();childPC.close()").unwrap();
}
