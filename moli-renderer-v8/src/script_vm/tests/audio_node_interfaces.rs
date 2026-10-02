use super::*;

#[test]
fn audio_nodes_share_native_inheritance_channels_and_receiver_validation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-node-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__audioNodeDone = value, error => globalThis.__audioNodeDone = String(error));", include_str!("audio_node_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__audioNodeDone").unwrap(), "true");
}

#[test]
fn offline_destination_channels_come_from_native_context_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-node-channels.test/");
    assert_eq!(vm.eval(r#"(() => {
      const context = new OfflineAudioContext(3,128,48000), node = context.destination;
      context.channelCount = 9;
      Object.defineProperty(context,'destination',{value:{}});
      if (node.context !== context || node.channelCount !== 3 || node.maxChannelCount !== 3) throw Error('native context channels');
      let error;try {node.channelCount = 2;} catch(e){error=e;}
      if (error?.name !== 'NotSupportedError' || node.channelCount !== 3) throw Error('offline count cannot change');
      try {node.channelCountMode = 'max';} catch(e){error=e;}
      if (error?.name !== 'InvalidStateError' || node.channelCountMode !== 'explicit') throw Error('offline mode cannot change');
      return true;
    })()"#).unwrap(), "true");
}
