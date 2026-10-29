use super::*;

#[tokio::test]
async fn negative_transition_delay_reports_the_initial_active_time() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/negative-transition-delay",
        "<html><head><style>#box{opacity:0;transition:opacity .08s -.04s}#box.active{opacity:1}</style></head><body><div id=box></div></body></html>",
        &loader,
    );
    vm.eval("globalThis.events=[];globalThis.box=document.getElementById('box');for(const type of ['transitionrun','transitionstart','transitionend'])box.addEventListener(type,e=>events.push([e.type,e.elapsedTime]))").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className='active'").unwrap();
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "3").await;
    let events: Vec<(String, f64)> =
        serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
    assert_eq!(
        events.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
        ["transitionrun", "transitionstart", "transitionend"]
    );
    for (actual, expected) in events.iter().map(|e| e.1).zip([0.04, 0.04, 0.08]) {
        assert!((actual - expected).abs() < 1e-6, "{events:?}");
    }
}

#[tokio::test]
async fn nonpositive_combined_transition_duration_does_not_start() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/nonpositive-combined-duration",
        "<html><head><style>#box{opacity:0;transition:opacity .04s -.08s}#box.active{opacity:1}</style></head><body><div id=box></div></body></html>",
        &loader,
    );
    vm.eval("globalThis.events=[];globalThis.box=document.getElementById('box');for(const type of ['transitionrun','transitionstart','transitionend'])box.addEventListener(type,e=>events.push(e.type))").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className='active'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(vm.eval("events.length").unwrap(), "0");
}

#[tokio::test]
async fn zero_duration_with_positive_delay_runs_then_starts_and_ends_at_the_deadline() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/delayed-zero-duration",
        "<html><head><style>#box{opacity:0;transition:opacity 0s .04s}#box.active{opacity:1}</style></head><body><div id=box></div></body></html>",
        &loader,
    );
    vm.eval("globalThis.events=[];globalThis.box=document.getElementById('box');for(const type of ['transitionrun','transitionstart','transitionend'])box.addEventListener(type,e=>events.push([e.type,e.elapsedTime]))").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className='active'").unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("events.map(e=>e[0]).join('|')").unwrap(),
        "transitionrun"
    );
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "3").await;
    assert_eq!(
        vm.eval("events.map(e=>e[0]).join('|')").unwrap(),
        "transitionrun|transitionstart|transitionend"
    );
    assert_eq!(vm.eval("events.every(e=>e[1]===0)").unwrap(), "true");
}

#[tokio::test]
async fn negative_animation_delay_clamps_start_elapsed_time_to_the_active_duration() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for (delay, start) in [("-.04s", 0.04), ("-.16s", 0.08)] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/negative-animation-delay",
            "<html><head><style>@keyframes move{from{left:0px}to{left:10px}}#box{position:relative}</style></head><body><div id=box></div></body></html>",
            &loader,
        );
        vm.eval("globalThis.events=[];globalThis.box=document.getElementById('box');box.onanimationstart=e=>events.push([e.type,e.elapsedTime]);box.onanimationend=e=>events.push([e.type,e.elapsedTime])").unwrap();
        vm.eval(&format!("box.style.animation='move .08s linear {delay}'"))
            .unwrap();
        wait_for_css_events(&mut vm, &loader, "events.length.toString()", "2").await;
        let events: Vec<(String, f64)> =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        assert_eq!(
            events.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
            ["animationstart", "animationend"]
        );
        assert!(
            (events[0].1 - start).abs() < 1e-6 && (events[1].1 - 0.08).abs() < 1e-6,
            "{delay}: {events:?}"
        );
    }
}
