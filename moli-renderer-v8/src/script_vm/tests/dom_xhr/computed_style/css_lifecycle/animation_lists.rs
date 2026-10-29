use super::*;

#[tokio::test]
async fn mixed_finite_and_infinite_animations_keep_each_items_timing() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for infinite_first in [true, false] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/mixed-animation-timing",
            r#"<html><head><style>
              @keyframes loop { from { left: 0px; } to { left: 10px; } }
              @keyframes finish { from { left: 0px; } to { left: 20px; } }
              #box { position: relative; }
            </style></head><body><div id="box"></div></body></html>"#,
            &loader,
        );
        vm.eval(
            r#"
          globalThis.events = [];
          globalThis.box = document.getElementById('box');
          box.onanimationstart = e => events.push([e.type, e.animationName, e.elapsedTime]);
          box.onanimationend = e => events.push([e.type, e.animationName, e.elapsedTime]);
        "#,
        )
        .unwrap();
        let declaration = if infinite_first {
            "loop 1s infinite, finish 0.04s 1"
        } else {
            "finish 0.04s 1, loop 1s infinite"
        };
        vm.eval(&format!("box.style.animation = '{declaration}'"))
            .unwrap();
        wait_for_css_events(
            &mut vm,
            &loader,
            "events.filter(e => e[0] === 'animationend' && e[1] === 'finish').length.toString()",
            "1",
        )
        .await;
        let events: Vec<(String, String, f64)> =
            serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
        let ends: Vec<_> = events
            .iter()
            .filter(|event| event.0 == "animationend")
            .collect();
        assert_eq!(ends.len(), 1, "infinite_first={infinite_first}: {events:?}");
        assert_eq!(ends[0].1, "finish");
        assert!((ends[0].2 - 0.04).abs() < 1e-6);
    }
}

#[tokio::test]
async fn animation_timing_lists_cycle_without_dropping_none_positions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/cyclic-animation-timing",
        r#"<html><head><style>
          @keyframes loop { from { left: 0px; } to { left: 10px; } }
          @keyframes finish { from { left: 0px; } to { left: 20px; } }
          @keyframes finish2 { from { left: 0px; } to { left: 30px; } }
          #box { position: relative; }
          #box.active {
            animation-name: none, loop, finish, finish2;
            animation-duration: 0.04s, 1s;
            animation-delay: 0s, 0.01s;
            animation-iteration-count: 1, infinite, 2;
          }
        </style></head><body><div id="box"></div></body></html>"#,
        &loader,
    );
    vm.eval(
        r#"
      globalThis.events = [];
      globalThis.box = document.getElementById('box');
      box.onanimationend = e => events.push([e.animationName, e.elapsedTime]);
      box.className = 'active';
    "#,
    )
    .unwrap();
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "2").await;
    let mut ends: Vec<(String, f64)> =
        serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
    ends.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        ends.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
        ["finish", "finish2"]
    );
    assert!((ends[0].1 - 0.08).abs() < 1e-6);
    assert!((ends[1].1 - 1.0).abs() < 1e-6);
}
