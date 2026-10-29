use super::*;

#[test]
fn rtc_peer_methods_convert_complete_dictionaries_before_native_state() {
    let mut vm = new_storage_html_test_vm("https://rtc-peer-dictionary.test/");
    vm.eval(&format!("globalThis.rtcPeerResult='pending'; ({}).then(value=>rtcPeerResult=value,error=>rtcPeerResult=String(error.stack||error));",include_str!("../../../../tests/fixtures/rtc-peer-dictionaries.js"))).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("String(rtcPeerResult)").unwrap(), "true");
}

#[test]
fn rtc_local_description_retains_native_snapshot_without_author_aliases() {
    let mut vm = new_storage_html_test_vm("https://rtc-local-snapshot.test/");
    vm.eval(r#"
      globalThis.snapshotResult='pending';
      (async()=>{
        const peer=new RTCPeerConnection(), input={type:'offer',sdp:'native SDP\ud800'};
        await peer.setLocalDescription(input);
        input.sdp='changed'; input.type='answer';
        const native=peer.localDescription;
        if(!(native instanceof RTCSessionDescription) || native===input || native.sdp!=='native SDP\ud800' || native.type!=='offer') throw Error('snapshot');
        const marker={};let caught;
        try{await peer.setLocalDescription({get sdp(){throw marker;}});}catch(e){caught=e;}
        if(caught!==marker || peer.localDescription!==native) throw Error('failed conversion changed state');
        const original=globalThis.RTCSessionDescription;
        try{globalThis.RTCSessionDescription=function(){throw Error('public constructor');};await peer.setLocalDescription({type:'offer',sdp:'second'});}finally{globalThis.RTCSessionDescription=original;}
        if(peer.localDescription.sdp!=='second') throw Error('intrinsic snapshot');
        peer.close();return true;
      })().then(value=>snapshotResult=value,error=>snapshotResult=String(error.stack||error));
    "#).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("String(snapshotResult)").unwrap(), "true");
}
