use super::*;

#[tokio::test]
async fn observed_style_seeds_transition_without_listener_or_lifecycle_prescan() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/first-observation",
        r#"<html><head><style>
          #box { opacity: 0; }
          #box.active { opacity: 1; transition: opacity 0.04s; }
        </style></head><body><div id="box"></div></body></html>"#,
        &loader,
    );
    assert_eq!(
        vm.eval(
            r#"
          globalThis.events = [];
          globalThis.box = document.getElementById('box');
          box.ontransitionrun = e => events.push(e.type);
          box.ontransitionstart = e => events.push(e.type);
          box.ontransitionend = e => events.push(e.type);
          globalThis.observed = getComputedStyle(box);
          observed.opacity;
        "#
        )
        .unwrap(),
        "0"
    );
    // No rendering turn, class mutation, or addEventListener seeded a baseline.
    // A live read reconciles the new style against that old baseline.
    // The resulting running effect is sampled, rather than exposing the target immediately.
    let initial: f64 = vm
        .eval("box.className = 'active'; observed.opacity")
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (0.0..=1.0).contains(&initial),
        "the same before-style now starts a sampled transition: {initial}"
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.slice(0, 2).join('|')").unwrap(),
        "transitionrun|transitionstart"
    );
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "3").await;
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "transitionrun|transitionstart|transitionend"
    );
}
