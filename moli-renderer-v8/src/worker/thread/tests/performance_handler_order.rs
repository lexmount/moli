use super::*;

#[tokio::test]
async fn worker_simple_event_handlers_share_dispatch_and_mutation_semantics() {
    ensure_v8();
    let probe =
        include_str!("../../../script_vm/tests/browser_api/simple_event_target_handlers.js");
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let source = format!(
            r#"{probe}
            const targets = [
              [self, 'message'], [self, 'online'],
              [performance, 'resourcetimingbufferfull'],
              [new AbortController().signal, 'abort'],
              ...['loadstart', 'progress', 'load', 'loadend', 'error', 'abort'].map(type => [new FileReader(), type]),
              ...['click', 'show', 'error', 'close'].map(type => [new Notification('handlers'), type]),
            ];
            for (const [target, type] of targets) {{
              checkHandlerDispatch(target, type);
              checkUnregisteredHandler(target);
            }}
            checkUnregisteredHandler(new EventTarget());
            const first = new XMLHttpRequest(), second = new XMLHttpRequest();
            const channel = new MessageChannel();
            try {{
              checkListenerIsolation([
                self, performance, new EventTarget(), new EventTarget(),
                new FileReader(), new FileReader(), first, second, first.upload, second.upload,
                new AbortController().signal, new AbortController().signal,
                channel.port1, channel.port2,
              ]);
            }} finally {{ channel.port1.close(); channel.port2.close(); }}
            postMessage(true);
            close();"#
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source,
                "https://simple-event-target-handlers.test/worker.js".to_owned(),
            )
            .with_script_kind(kind),
        );
        assert_eq!(recv_post_json(&mut handle).await, "true");
    }
}

#[tokio::test]
async fn worker_performance_handlers_keep_their_listener_position_across_replacement() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let source = r#"
            const rows = [], target = performance, type = 'resourcetimingbufferfull';
            target.addEventListener(type, () => rows.push('first'));
            target.onresourcetimingbufferfull = () => rows.push('handler-one');
            target.addEventListener(type, () => rows.push('last'));
            const fire = () => target.dispatchEvent(new Event(type));
            fire();
            target.onresourcetimingbufferfull = function(event) {
                rows.push(this === target && event.currentTarget === target ? 'handler-two' : 'bad receiver');
            };
            fire();
            target.onresourcetimingbufferfull = null;
            fire();
            target.onresourcetimingbufferfull = () => rows.push('readded');
            fire();
            postMessage(rows);
            close();
        "#;
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source.to_owned(),
                "https://performance-handler.test/worker.js".to_owned(),
            )
            .with_script_kind(kind),
        );
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"["first","handler-one","last","first","handler-two","last","first","last","first","last","readded"]"#
        );
    }
}
