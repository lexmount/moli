use super::*;

async fn assert_last_transition_match(transition: &str, expected: &[(&str, f64)]) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/last-transition-match",
        &format!(
            r#"<html><head><style>
              #box {{ opacity: 0; width: 50px; transition: {transition}; }}
              #box.active {{ opacity: 1; width: 200px; }}
            </style></head><body><div id="box"></div></body></html>"#
        ),
        &loader,
    );
    vm.eval(
        r#"
        globalThis.events = [];
        globalThis.box = document.getElementById('box');
        for (const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) {
          box.addEventListener(type, e => events.push([e.type, e.propertyName, e.elapsedTime]));
        }
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className = 'active'").unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm.eval("events.filter(e => e[1] === 'opacity').map(e => e[0]).join('|')")
            .unwrap(),
        if expected.iter().any(|(property, _)| *property == "opacity") {
            "transitionrun"
        } else {
            ""
        },
        "{transition}: initial opacity events must follow the last match"
    );
    let expected_ends = expected.len().to_string();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.filter(e => e[0] === 'transitionend').length.toString()",
        &expected_ends,
    )
    .await;
    let events: Vec<(String, String, f64)> =
        serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
    assert_eq!(events.len(), 3 * expected.len(), "{transition}: {events:?}");
    for (property, duration) in expected {
        let events = events
            .iter()
            .filter(|event| event.1 == *property)
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| event.0.as_str())
                .collect::<Vec<_>>(),
            ["transitionrun", "transitionstart", "transitionend"],
            "{transition}: {property} must transition exactly once"
        );
        assert!(
            (events[2].2 - duration).abs() < 1e-6,
            "{transition}: {property} end elapsedTime must be {duration}, got {}",
            events[2].2
        );
    }
}

#[tokio::test]
async fn repeated_transition_properties_use_the_last_match_once() {
    assert_last_transition_match("OPACITY 0.04s, opacity 0.08s 0.02s", &[("opacity", 0.08)]).await;
}

#[tokio::test]
async fn trailing_all_overrides_an_explicit_transition_match() {
    assert_last_transition_match(
        "opacity 0.04s, all 0.08s 0.02s",
        &[("opacity", 0.08), ("width", 0.08)],
    )
    .await;
}

#[tokio::test]
async fn trailing_explicit_transition_overrides_all_for_only_that_property() {
    assert_last_transition_match(
        "all 0.04s, opacity 0.08s 0.02s",
        &[("opacity", 0.08), ("width", 0.04)],
    )
    .await;
}

#[tokio::test]
async fn nonadjacent_repeated_transition_properties_use_the_last_match_once() {
    assert_last_transition_match(
        "opacity 0.04s, width 0.06s, opacity 0.08s 0.02s",
        &[("opacity", 0.08), ("width", 0.06)],
    )
    .await;
}

#[tokio::test]
async fn last_zero_duration_match_disables_earlier_nonzero_transitions() {
    for transition in ["opacity 0.04s, opacity 0s", "opacity 0.04s, all 0s"] {
        assert_last_transition_match(transition, &[]).await;
    }
}

#[tokio::test]
async fn transition_matching_preserves_custom_property_case() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/custom-property-case",
        r#"<html><head><style>
          @property --x { syntax: '<number>'; inherits: false; initial-value: 0; }
          @property --X { syntax: '<number>'; inherits: false; initial-value: 0; }
          #box { --x: 0; --X: 0; transition: --x 0.04s, --X 0.08s; }
          #box.active { --x: 1; --X: 1; }
        </style></head><body><div id="box"></div></body></html>"#,
        &loader,
    );
    vm.eval(
        r#"
        globalThis.events = [];
        globalThis.box = document.getElementById('box');
        for (const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) {
          box.addEventListener(type, e => events.push([e.type, e.propertyName, e.elapsedTime]));
        }
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className = 'active'").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.filter(e => e[0] === 'transitionend').length.toString()",
        "2",
    )
    .await;
    let events: Vec<(String, String, f64)> =
        serde_json::from_str(&vm.eval("JSON.stringify(events)").unwrap()).unwrap();
    assert_eq!(events.len(), 6);
    for (property, duration) in [("--x", 0.04), ("--X", 0.08)] {
        let events = events
            .iter()
            .filter(|event| event.1 == property)
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| event.0.as_str())
                .collect::<Vec<_>>(),
            ["transitionrun", "transitionstart", "transitionend"],
            "{property} must retain its own transition"
        );
        assert!((events[2].2 - duration).abs() < 1e-6);
    }
}

#[tokio::test]
async fn zero_duration_update_preserves_running_and_delayed_transitions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for change_at in ["transitionrun", "transitionstart"] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/change-transition-timing",
            r#"<html><head><style>
              #box { opacity: 0; transition: opacity 0.04s 0.04s; }
              #box.active { opacity: 1; }
            </style></head><body><div id="box"></div></body></html>"#,
            &loader,
        );
        vm.eval(&format!("globalThis.changeAt = '{change_at}'"))
            .unwrap();
        vm.eval(r#"
          globalThis.events = [];
          globalThis.box = document.getElementById('box');
          for (const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) {
            box.addEventListener(type, e => {
              events.push([e.type, e.elapsedTime]);
              if (e.type === changeAt) box.style.transitionDuration = '0s';
            });
          }
          document.body.className = 'baseline';
        "#).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval("box.className = 'active'").unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("events.filter(e => e[0] === 'transitioncancel').length")
                .unwrap(),
            "0"
        );
        wait_for_css_events(
            &mut vm,
            &loader,
            "events.filter(e => e[0] === 'transitionend').length.toString()",
            "1",
        )
        .await;
        assert_eq!(
            vm.eval("events.map(e => e[0]).join('|')").unwrap(),
            "transitionrun|transitionstart|transitionend"
        );
        let elapsed: f64 = vm.eval("events[2][1]").unwrap().parse().unwrap();
        assert!(
            (elapsed - 0.04).abs() < 1e-6,
            "change_at={change_at}: {elapsed}"
        );
    }
}

#[tokio::test]
async fn zero_duration_with_a_new_target_value_still_cancels_the_old_transition() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/zero-duration-new-target",
        r#"<html><head><style>
          #box { opacity: 0; transition: opacity 60s; }
          #box.active { opacity: 1; }
        </style></head><body><div id="box"></div></body></html>"#,
        &loader,
    );
    vm.eval(
        r#"
      globalThis.events = [];
      globalThis.box = document.getElementById('box');
      for (const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) {
        box.addEventListener(type, e => events.push(e.type));
      }
      document.body.className = 'baseline';
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className = 'active'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.style.transitionDuration = '0s'; box.style.opacity = '0.5'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "transitionrun|transitionstart|transitioncancel"
    );
}

#[tokio::test]
async fn transition_batch_captures_all_properties_before_invoking_script() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/transition-batch",
        r#"<html><head><style>
          #box { width: 50px; opacity: 0; transition: width 0.04s, opacity 0.04s; }
          #box.active { width: 200px; opacity: 1; }
        </style></head><body><div id=box></div></body></html>"#,
        &loader,
    );
    vm.eval(r#"
        globalThis.events = [];
        globalThis.box = document.getElementById('box');
        for (const type of ['transitionrun', 'transitionstart', 'transitionend', 'transitioncancel']) {
          box.addEventListener(type, e => events.push([e.type, e.propertyName]));
        }
        box.addEventListener('transitionrun', () => box.style.transitionDuration = '0s', {once: true});
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.eval("box.className = 'active'").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "events.filter(e => e[0] === 'transitionend').length.toString()",
        "2",
    )
    .await;
    for property in ["width", "opacity"] {
        assert_eq!(
            vm.eval(&format!(
                "events.filter(e => e[1] === '{property}').map(e => e[0]).join('|')"
            ))
            .unwrap(),
            "transitionrun|transitionstart|transitionend",
            "{property} must retain the timing captured before the first callback"
        );
    }
}
