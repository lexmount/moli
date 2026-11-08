use super::*;

const PROBE: &str = include_str!("simple_event_target_handlers.js");

#[test]
fn simple_event_target_listener_storage_is_private_and_separate_for_each_instance() {
    let mut vm = new_parsed_test_vm(
        "https://simple-event-target-storage.test/",
        "<body><iframe id=child></iframe></body>",
    );
    vm.exec(PROBE, None).unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
      for (const w of [window, document.getElementById('child').contentWindow]) {
        const first = new w.XMLHttpRequest(), second = new w.XMLHttpRequest();
        const channel = new w.MessageChannel();
        const targets = [
          new w.EventTarget(), new w.EventTarget(),
          new w.FileReader(), new w.FileReader(),
          first, second, first.upload, second.upload,
          new w.AbortController().signal, new w.AbortController().signal,
          channel.port1, channel.port2,
          new w.VTTCue(0, 1, ''), new w.VTTCue(0, 1, ''),
          w.performance, w.screen.orientation, w.visualViewport,
        ];
        try { checkListenerIsolation(targets, w); }
        finally { channel.port1.close(); channel.port2.close(); }
      }
      return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn indexed_db_handlers_share_synthetic_and_native_listener_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://idb-handler-order.test/");
    vm.exec(PROBE, None).unwrap();
    vm.eval(
        r#"(() => {
          const trace = [];
          globalThis.__idbHandlerOrder = 'pending';
          const open = indexedDB.open('handler-order', 1);
          for (const type of ['success', 'error', 'blocked', 'upgradeneeded']) {
            checkHandlerDispatch(open, type, globalThis, true);
          }
          checkUnregisteredHandler(open);
          open.onupgradeneeded = () => open.result.createObjectStore('values');
          open.addEventListener('success', () => trace.push('open-before'));
          open.onsuccess = () => {
            trace.push('open-handler');
            const db = open.result;
            const tx = db.transaction('values');
            const request = tx.objectStore('values').get('missing');
            checkListenerIsolation([db, tx, request, open]);
            for (const [target, types] of [
              [db, ['abort', 'close', 'error', 'versionchange']],
              [tx, ['abort', 'complete', 'error']],
              [request, ['success', 'error']],
            ]) {
              for (const type of types) checkHandlerDispatch(target, type, globalThis, true);
              checkUnregisteredHandler(target);
            }
            request.addEventListener('success', () => trace.push('request-before'));
            request.onsuccess = () => trace.push('request-handler');
            request.addEventListener('success', () => trace.push('request-after'));
            tx.addEventListener('complete', () => trace.push('tx-before'));
            tx.oncomplete = () => trace.push('tx-handler');
            tx.addEventListener('complete', () => {
              trace.push('tx-after');
              globalThis.__idbHandlerOrder = JSON.stringify(trace);
              db.close();
            });
          };
          open.addEventListener('success', () => trace.push('open-after'));
          return 'scheduled';
        })()"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(__idbHandlerOrder)")
            .unwrap(),
        r#"["open-before","open-handler","open-after","request-before","request-handler","request-after","tx-before","tx-handler","tx-after"]"#
    );
}

#[test]
fn xhr_native_dispatch_uses_registered_handlers_without_property_reads() {
    let mut vm = new_storage_test_vm("https://xhr-handler-order.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
              const xhr = new XMLHttpRequest();
              const trace = [];
              xhr.addEventListener('readystatechange', () => trace.push('before'));
              xhr.onreadystatechange = () => trace.push('handler');
              xhr.addEventListener('readystatechange', () => trace.push('after'));
              Object.defineProperty(xhr, 'onreadystatechange', {
                get() { throw new Error('native dispatch read handler property'); }
              });
              xhr.open('GET', '/resource');
              return JSON.stringify(trace);
            })()"#,
        )
        .unwrap(),
        r#"["before","handler","after"]"#
    );
}

#[test]
fn simple_window_handlers_share_dispatch_and_mutation_semantics() {
    let mut vm = new_storage_test_vm("https://simple-window-handlers.test/");
    vm.exec(PROBE, None).unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              const popups = [open(), open()];
              try {
                for (const target of popups) {
                  checkHandlerDispatch(target, 'message');
                  checkUnregisteredHandler(target);
                }
                checkListenerIsolation(popups);
                return true;
              } finally { for (const popup of popups) popup.close(); }
            })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn simple_event_target_handlers_share_dispatch_and_mutation_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://simple-event-target-handlers.test/",
        "<body><iframe id=child></iframe></body>",
    );
    vm.exec(PROBE, None).unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
              for (const w of [window, document.getElementById('child').contentWindow]) {
                const video = w.document.createElement('video');
                const track = video.addTextTrack('subtitles');
                const reader = new w.FileReader();
                const notification = new w.Notification('handlers');
                const targets = [
                  [w.performance, 'resourcetimingbufferfull'],
                  [w.screen.orientation, 'change'],
                  [new w.VTTCue(0, 1, ''), 'enter'],
                  [new w.VTTCue(0, 1, ''), 'exit'],
                  [track, 'cuechange'],
                  [video.textTracks, 'addtrack'],
                  [video.textTracks, 'removetrack'],
                  [new w.IdleDetector(), 'change'],
                  [new w.OfflineAudioContext(1, 8, 8000), 'complete'],
                  [w.visualViewport, 'resize'],
                  [w.visualViewport, 'scroll'],
                  [w.visualViewport, 'scrollend'],
                  ...['loadstart', 'progress', 'load', 'loadend', 'error', 'abort'].map(type => [reader, type]),
                  ...['click', 'show', 'error', 'close'].map(type => [notification, type]),
                ];
                for (const [target, type] of targets) {
                  checkHandlerDispatch(target, type, w);
                  checkUnregisteredHandler(target, w);
                }
                checkUnregisteredHandler(new w.EventTarget(), w);
              }
              return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
