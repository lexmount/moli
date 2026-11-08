use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_peer_methods_convert_complete_dictionaries_before_native_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://rtc-peer-dictionary.test/",
        &loader,
    );
    vm.eval(&format!("globalThis.rtcPeerResult='pending'; ({}).then(value=>rtcPeerResult=value,error=>rtcPeerResult=String(error.stack||error));",include_str!("../../../../tests/fixtures/rtc-peer-dictionaries.js"))).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(rtcPeerResult !== 'pending')",
        "true",
        "dictionary conversion must complete through selected RTC tasks",
    )
    .await;
    assert_eq!(vm.eval("String(rtcPeerResult)").unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_local_description_retains_native_snapshot_without_author_aliases() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://rtc-local-snapshot.test/",
        &loader,
    );
    vm.eval(r#"
      globalThis.snapshotResult='pending';
      (async()=>{
        const peer=new RTCPeerConnection(), offer=await peer.createOffer();
        const input={type:offer.type,sdp:offer.sdp};
        await peer.setLocalDescription(input);
        input.sdp='changed'; input.type='answer';
        const native=peer.localDescription;
        if(!(native instanceof RTCSessionDescription) || native===input || native.sdp!==offer.sdp || native.type!=='offer') throw Error('snapshot');
        const marker={};let caught;
        try{await peer.setLocalDescription({get sdp(){throw marker;}});}catch(e){caught=e;}
        if(caught!==marker || peer.localDescription!==native) throw Error('failed conversion changed state');
        const next=await peer.createOffer({iceRestart:true});
        const original=globalThis.RTCSessionDescription;
        try{globalThis.RTCSessionDescription=function(){throw Error('public constructor');};await peer.setLocalDescription({type:next.type,sdp:next.sdp});}finally{globalThis.RTCSessionDescription=original;}
        if(peer.localDescription.sdp!==next.sdp) throw Error('intrinsic snapshot');
        peer.close();return true;
      })().then(value=>snapshotResult=value,error=>snapshotResult=String(error.stack||error));
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(snapshotResult !== 'pending')",
        "true",
        "local descriptions must settle through selected RTC tasks",
    )
    .await;
    assert_eq!(vm.eval("String(snapshotResult)").unwrap(), "true");
}
