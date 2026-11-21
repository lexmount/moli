use super::*;

async fn assert_document_replacement_stops_transition_batch(boundary: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let delay = if boundary == "transitionstart" {
        "0.04s"
    } else {
        "0s"
    };
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/transition-reentrancy",
        &format!(
            r#"<html><head><style>
          #box {{ width: 50px; opacity: 0; transition: width 0.04s {delay}, opacity 0.04s {delay}; }}
          #box.active {{ width: 200px; opacity: 1; }}
        </style></head><body><div id="box"></div></body></html>"#
        ),
        &loader,
    );
    let old_owner = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .unwrap();
    vm.eval(&format!("globalThis.boundary = '{boundary}'"))
        .unwrap();
    vm.eval(
        r#"
      globalThis.events = [];
      globalThis.oldBox = document.getElementById('box');
      oldBox.addEventListener(boundary, e => {
        events.push(e.propertyName);
        oldBox.remove();
        document.open();
        document.write('<html><body>replacement</body></html>');
        document.close();
      });
      document.body.className = 'baseline';
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("oldBox.className = 'active'").unwrap();
    wait_for_css_events(&mut vm, &loader, "(events.length > 0).toString()", "true").await;
    // Expanded longhands use canonical property order; Chromium also reports
    // opacity first at these simultaneous delayed-start/end boundaries.
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "opacity",
        "boundary={boundary}"
    );
    assert_ne!(
        vm._context_host.borrow().current_main_document_task_owner(),
        Some(old_owner)
    );
}

#[tokio::test]
async fn delayed_transition_start_batch_stops_after_document_replacement() {
    assert_document_replacement_stops_transition_batch("transitionstart").await;
}

#[tokio::test]
async fn transition_end_batch_stops_after_document_replacement() {
    assert_document_replacement_stops_transition_batch("transitionend").await;
}
