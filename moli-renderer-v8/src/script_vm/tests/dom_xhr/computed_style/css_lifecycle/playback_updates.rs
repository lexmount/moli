use super::*;

#[tokio::test]
async fn animation_timing_updates_extend_the_existing_playback_clock() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for change in [
        "box.style.animationDuration='0.2s'",
        "box.style.animationIterationCount='5'",
    ] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://css-lifecycle.test/update-animation",
            r#"<html><head><style>
              @keyframes move {from {left:0px} to {left:10px}}
              div {position:relative}
              .active #box {animation:move .04s linear}
              .active #witness {animation:move .12s linear}
            </style></head><body><div id=box></div><div id=witness></div></body></html>"#,
            &loader,
        );
        vm.eval(&format!(r#"
          globalThis.events=[];globalThis.box=document.getElementById('box');globalThis.witnessDone=false;
          for(const type of ['animationstart','animationend','animationcancel']) box.addEventListener(type,e=>events.push([e.type,e.elapsedTime]));
          box.addEventListener('animationstart',()=>{{{change}}},{{once:true}});
          document.getElementById('witness').onanimationend=()=>{{witnessDone=true}};
          document.body.className='active';
        "#)).unwrap();
        wait_for_css_events(&mut vm, &loader, "witnessDone.toString()", "true").await;
        assert_eq!(
            vm.eval("events.filter(e=>e[0]==='animationend').length")
                .unwrap(),
            "0",
            "{change}"
        );
        wait_for_css_events(
            &mut vm,
            &loader,
            "events.filter(e=>e[0]==='animationend').length.toString()",
            "1",
        )
        .await;
        assert_eq!(
            vm.eval("events.map(e=>e[0]).join('|')").unwrap(),
            "animationstart|animationend"
        );
        let elapsed: f64 = vm.eval("events[1][1]").unwrap().parse().unwrap();
        assert!((elapsed - 0.2).abs() < 1e-6, "{change}: {elapsed}");
    }
}

#[tokio::test]
async fn paused_animations_hold_their_clock_until_resumed() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/pause-animation",
        r#"<html><head><style>
          @keyframes move {from {left:0px} to {left:10px}}
          div {position:relative}
          .active #box {animation:move .04s linear}
          .active #witness {animation:move .12s linear}
        </style></head><body><div id=box></div><div id=witness></div></body></html>"#,
        &loader,
    );
    vm.eval(r#"
      globalThis.events=[];globalThis.box=document.getElementById('box');globalThis.witnessDone=false;
      for(const type of ['animationstart','animationend','animationcancel']) box.addEventListener(type,e=>events.push(e.type));
      box.addEventListener('animationstart',()=>{box.style.animationPlayState='paused'},{once:true});
      document.getElementById('witness').onanimationend=()=>{witnessDone=true};
      document.body.className='active';
    "#).unwrap();
    wait_for_css_events(&mut vm, &loader, "witnessDone.toString()", "true").await;
    assert_eq!(vm.eval("events.join('|')").unwrap(), "animationstart");
    vm.eval("box.style.animationPlayState='running'").unwrap();
    wait_for_css_events(&mut vm, &loader, "events.length.toString()", "2").await;
    assert_eq!(
        vm.eval("events.join('|')").unwrap(),
        "animationstart|animationend"
    );
}

#[tokio::test]
async fn duplicate_animation_names_keep_independent_playback_records() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-lifecycle.test/duplicate-animation",
        r#"<html><head><style>
          @keyframes move {from {left:0px} to {left:10px}}
          #box {position:relative}
          #box.active {animation:move .04s linear,move .08s linear}
        </style></head><body><div id=box></div></body></html>"#,
        &loader,
    );
    vm.eval("globalThis.ends=[];const box=document.getElementById('box');box.onanimationend=e=>ends.push(e.elapsedTime);box.className='active'").unwrap();
    wait_for_css_events(&mut vm, &loader, "ends.length.toString()", "2").await;
    let elapsed: Vec<f64> =
        serde_json::from_str(&vm.eval("JSON.stringify(ends)").unwrap()).unwrap();
    assert!((elapsed[0] - 0.04).abs() < 1e-6 && (elapsed[1] - 0.08).abs() < 1e-6);
}
