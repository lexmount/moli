use super::*;

#[test]
fn listener_exception_reporting_preserves_outer_realm_window_event() {
    let mut vm = new_storage_html_test_vm("https://event-listener-nested-window-event.test/");

    vm.eval(
        r#"
        (() => {
          const host = document.body || document.documentElement || document;
          const listenerFrame = document.createElement("iframe");
          listenerFrame.srcdoc = `<script>
            function listener() { throw new Error("listener failure"); }
            function currentEventType() { return window.event?.type; }
          <\/script>`;
          const handlerFrame = document.createElement("iframe");
          host.appendChild(listenerFrame);
          host.appendChild(handlerFrame);
          globalThis.__nestedEventFrames = [listenerFrame, handlerFrame];
          return "ready";
        })()
        "#,
    )
    .expect("nested window.event frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const [listenerFrame, handlerFrame] = globalThis.__nestedEventFrames;
              const listenerWindow = listenerFrame.contentWindow;
              const handlerWindow = handlerFrame.contentWindow;
              listenerWindow.onerror = new handlerWindow.Function(`
                parent.__nestedWindowEventState = [
                  parent.__nestedEventFrames[0].contentWindow.currentEventType(),
                  window.event?.type
                ];
                return true;
              `);

              const target = new EventTarget();
              target.addEventListener("party", listenerWindow.listener);
              target.dispatchEvent(new Event("party"));

              return JSON.stringify({
                during: globalThis.__nestedWindowEventState,
                listenerAfter: listenerWindow.event,
                handlerAfter: handlerWindow.event
              });
            })()
            "#,
        )
        .expect("nested window.event exception reporting should evaluate");

    assert_eq!(result, r#"{"during":["party","error"]}"#,);
}

#[test]
fn cross_realm_window_event_getters_preserve_nested_handler_events() {
    let mut vm = new_storage_html_test_vm("https://nested-handler-window-event.test/");

    vm.eval(
        r#"
        (() => {
          const host = document.body || document.documentElement || document;
          const firstFrame = document.createElement("iframe");
          const secondFrame = document.createElement("iframe");
          host.appendChild(firstFrame);
          host.appendChild(secondFrame);
          globalThis.__handlerEventFrames = [firstFrame, secondFrame];
          return "ready";
        })()
        "#,
    )
    .expect("nested handler frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const [firstFrame, secondFrame] = globalThis.__handlerEventFrames;
              firstFrame.contentWindow.onerror = new secondFrame.contentWindow.Function(`
                top.__secondHandlerEvents = [
                  top.event?.type,
                  top.frames[0].event?.type,
                  top.frames[1].event?.type,
                  top.frames[1].event?.error?.name
                ];
                return true;
              `);
              window.onerror = new firstFrame.contentWindow.Function(`
                top.__firstHandlerEvents = [
                  top.event?.type,
                  top.frames[0].event?.type,
                  top.frames[1].event?.type ?? null
                ];
                missingNestedHandlerName;
              `);
              window.onload = loadEvent => {
                const errorEvent = new ErrorEvent("error", {
                  error: new Error("outer error")
                });
                window.dispatchEvent(errorEvent);
              };

              window.dispatchEvent(new Event("load"));
              return JSON.stringify({
                first: globalThis.__firstHandlerEvents,
                second: globalThis.__secondHandlerEvents,
                after: [window.event, frames[0].event, frames[1].event]
              });
            })()
            "#,
        )
        .expect("nested cross-realm Window event handlers should evaluate");

    assert_eq!(
        result,
        r#"{"first":["load","error",null],"second":["load","error","error","ReferenceError"],"after":[null,null,null]}"#,
    );
}

#[test]
fn exception_reporting_suppresses_recursion_per_window_and_restores_afterwards() {
    let mut vm = new_storage_html_test_vm("https://window-reporting-recursion.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
        const events = [];
        onerror = (_message, _url, _line, _column, error) => {
            events.push([error.message, window.event.type]);
            throw Error('handler failure');
        };
        const target = new EventTarget();
        target.addEventListener('x', () => { throw Error('listener failure'); });
        for (let n = 0; n < 2; n++) target.dispatchEvent(new Event('x'));
        const after = window.event;
        onerror = null;
        return JSON.stringify([events, after]);
    })()"#
        )
        .unwrap(),
        r#"[[["listener failure","error"],["listener failure","error"]],null]"#
    );
}
