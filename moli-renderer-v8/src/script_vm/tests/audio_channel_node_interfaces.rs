use super::*;

#[test]
fn audio_graph_derived_arguments_preserve_ports_and_late_conversion_errors() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-port-conversion.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, message) => { if (!ok) throw Error(message); };
      for (const realm of [window, document.querySelector('iframe').contentWindow]) {
        const context = new realm.OfflineAudioContext(2, 16, 48000);
        const source = context.createChannelSplitter(2);
        const target = context.createChannelMerger(2);
        source.connect(target, 0, 0);
        source.connect(target, 1, 1);
        const sentinel = {}, order = [];
        let error;
        try {
          source.disconnect(target,
            {valueOf() { order.push('output'); return 99; }},
            {valueOf() { order.push('input'); throw sentinel; }});
        } catch (caught) { error = caught; }
        assert(error === sentinel && order.join() === 'output,input', 'late conversion precedes port checks');
        source.disconnect(target, undefined, undefined);
        try { source.disconnect(target, 0, 0); } catch (caught) { error = caught; }
        assert(error instanceof realm.DOMException && error.name === 'InvalidAccessError', 'explicit undefined selects port zero');
        source.disconnect(target, 1, 1);
        error = undefined;
        try { source.disconnect(target); } catch (caught) { error = caught; }
        assert(error instanceof realm.DOMException && error.name === 'InvalidAccessError', 'conversion failure preserved the other route');
        source.connect(target, 0, 1);
        source.connect(target, 1, 0);
        source.disconnect(target);
        for (const [output, input] of [[0, 1], [1, 0]]) {
          error = undefined;
          try { source.disconnect(target, output, input); } catch (caught) { error = caught; }
          assert(error instanceof realm.DOMException && error.name === 'InvalidAccessError', 'omitted ports remove all routes');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn audio_channel_nodes_use_native_interfaces_and_port_constraints() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-channel-nodes.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__channelNodesDone = value, error => globalThis.__channelNodesDone = String(error));", include_str!("audio_channel_node_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__channelNodesDone").unwrap(), "true");
}

#[test]
fn audio_channel_nodes_preserve_port_connections_and_reject_unsupported_processing() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-channel-node-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__channelNodesDone = value, error => globalThis.__channelNodesDone = String(error));", include_str!("audio_channel_node_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__channelNodesDone").unwrap(), "true");
}
