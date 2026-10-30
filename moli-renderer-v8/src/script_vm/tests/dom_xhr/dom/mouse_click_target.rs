use super::*;

const SCAN_LINK: &str = r#"<!doctype html><body style="margin:0">
  <a id="scan" href="/fingerprint-check"
     style="position:absolute;left:0;top:0;width:240px;height:80px">
    <span id="left" style="position:absolute;left:0;top:0;width:80px;height:80px">Scan</span>
    <span id="right" style="position:absolute;left:80px;top:0;width:80px;height:80px">Now</span>
  </a>
</body>"#;

fn scan_link_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm("https://mouse-click-target.test/", SCAN_LINK);
    vm.eval(
        r#"
        window.events = [];
        window.scans = 0;
        for (const type of ['mousedown', 'mouseup', 'click']) {
          document.addEventListener(type, event => {
            events.push([type, event.target.id, event.isTrusted]);
          });
        }
        document.getElementById('scan').addEventListener('click', event => {
          event.preventDefault();
          scans++;
          history.pushState({}, '', '/fingerprint-check');
        });
        "#,
    )
    .expect("scan link listeners should install");
    vm
}

fn mouse_button(vm: &mut StandaloneScriptVmHarness, x: f64, event: &str) {
    vm.dispatch_mouse_event_at_point(x, 20.0, event, 0, None, 0.0, 0.0)
        .expect("coordinate mouse input should dispatch");
}

#[test]
fn mouse_click_uses_common_ancestor_of_press_and_release_targets() {
    for (down_x, up_x, down_target, up_target, click_target) in [
        (20.0, 20.0, "left", "left", "left"),
        (20.0, 100.0, "left", "right", "scan"),
        (20.0, 200.0, "left", "scan", "scan"),
        (200.0, 20.0, "scan", "left", "scan"),
    ] {
        let mut vm = scan_link_vm();
        mouse_button(&mut vm, down_x, "mousedown");
        mouse_button(&mut vm, up_x, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify([events, scans, location.pathname])")
                .expect("scan result should evaluate"),
            serde_json::json!([
                [
                    ["mousedown", down_target, true],
                    ["mouseup", up_target, true],
                    ["click", click_target, true]
                ],
                1,
                "/fingerprint-check"
            ])
            .to_string(),
            "press {down_target}, release {up_target}"
        );
    }
}

#[test]
fn mouse_click_survives_layout_shift_within_the_same_link() {
    let mut vm = scan_link_vm();
    mouse_button(&mut vm, 20.0, "mousedown");
    vm.eval("document.getElementById('left').style.left = '100px'")
        .expect("the pressed span should move within its link");
    vm.publish_layout_for_test()
        .expect("publish the changed layout before release");
    mouse_button(&mut vm, 20.0, "mouseup");
    assert_eq!(
        vm.eval("JSON.stringify([events, scans, location.pathname])")
            .expect("scan result should evaluate"),
        r#"[[["mousedown","left",true],["mouseup","scan",true],["click","scan",true]],1,"/fingerprint-check"]"#
    );
}

#[test]
fn mouse_click_is_suppressed_when_a_press_or_release_target_is_removed() {
    for remove in [
        "document.getElementById('left').remove()",
        "document.addEventListener('mouseup', () => document.getElementById('left').remove(), {once:true})",
        "document.addEventListener('mouseup', event => event.target.remove(), {once:true})",
        "document.addEventListener('mouseup', () => document.getElementById('right').appendChild(document.getElementById('left')), {once:true})",
        "document.addEventListener('mouseup', () => { const left = document.getElementById('left'); left.remove(); document.getElementById('scan').prepend(left); }, {once:true})",
        "document.addEventListener('mouseup', () => { const scan = document.getElementById('scan'); scan.remove(); document.body.appendChild(scan); }, {once:true})",
        "document.addEventListener('mouseup', () => document.getElementById('right').moveBefore(document.getElementById('left'), null), {once:true})",
    ] {
        let mut vm = scan_link_vm();
        mouse_button(&mut vm, 20.0, "mousedown");
        vm.eval(remove).expect("target removal should install");
        vm.publish_layout_for_test()
            .expect("publish geometry before release");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval(
                "JSON.stringify([events.filter(e => e[0] === 'click'), scans, location.pathname])"
            )
            .expect("scan result should evaluate"),
            r#"[[],0,"/"]"#,
            "removal: {remove}"
        );
    }
}

#[test]
fn mouse_click_uses_current_ancestry_when_only_the_release_target_is_reinserted() {
    for (mutation, target) in [
        (
            "const parent = event.target.parentNode; event.target.remove(); parent.appendChild(event.target);",
            "scan",
        ),
        (
            "document.getElementById('left').appendChild(event.target);",
            "left",
        ),
    ] {
        let mut vm = scan_link_vm();
        vm.eval(&format!(
            "document.addEventListener('mouseup', event => {{ {mutation} }}, {{once:true}})"
        ))
        .expect("release target mutation should install");
        mouse_button(&mut vm, 20.0, "mousedown");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify([events, scans, location.pathname])")
                .expect("scan result should evaluate"),
            serde_json::json!([
                [
                    ["mousedown", "left", true],
                    ["mouseup", "right", true],
                    ["click", target, true]
                ],
                1,
                "/fingerprint-check"
            ])
            .to_string(),
            "release mutation: {mutation}"
        );
    }
}

#[test]
fn mouse_click_targets_the_pointerup_capture_target_after_capture_is_released() {
    let mut vm = scan_link_vm();
    vm.eval(
        "document.getElementById('left').addEventListener('pointerdown', event => document.getElementById('right').setPointerCapture(event.pointerId))",
    )
    .expect("capture transfer should install");
    mouse_button(&mut vm, 20.0, "mousedown");
    mouse_button(&mut vm, 20.0, "mouseup");
    assert_eq!(
        vm.eval("JSON.stringify([events, scans, location.pathname, document.getElementById('right').hasPointerCapture(1)])")
            .expect("scan result should evaluate"),
        r#"[[["mousedown","left",true],["mouseup","right",true],["click","right",true]],1,"/fingerprint-check",false]"#
    );
}

const SHADOW_CLICK_HOST: &str = r#"<!doctype html><body style="margin:0">
  <div id="host" style="position:absolute;left:0;top:0;width:240px;height:80px">
    <span id="left" style="position:absolute;left:0;top:0;width:80px;height:80px">Left</span>
    <span id="right" style="position:absolute;left:80px;top:0;width:80px;height:80px">Right</span>
  </div>
</body>"#;

fn shadow_click_vm(setup: &str) -> StandaloneScriptVmHarness {
    shadow_click_vm_with_options(setup, "{mode:'open'}")
}

fn shadow_click_vm_with_options(setup: &str, options: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_rendered_test_vm("https://shadow-click-target.test/", SHADOW_CLICK_HOST);
    vm.eval(&format!(
        r#"
        window.host = document.getElementById('host');
        window.left = document.getElementById('left');
        window.right = document.getElementById('right');
        window.root = host.attachShadow({options});
        {setup}
        window.clicks = [];
        document.addEventListener('click', event => {{
          clicks.push([event.target.id, event.composedPath()[0].id, event.isTrusted]);
        }});
        "#
    ))
    .expect("shadow click fixture should initialize");
    vm.publish_layout_for_test()
        .expect("publish distributed shadow content before input");
    vm
}

#[test]
fn mouse_click_uses_the_flat_tree_across_shadow_roots_and_slots() {
    for (setup, target) in [
        ("root.append(left, right);", "host"),
        ("root.innerHTML = '<slot id=slot></slot>';", "slot"),
        (
            "left.slot = 'left'; right.slot = 'right'; root.innerHTML = '<a id=scan><slot name=left></slot><slot name=right></slot></a>';",
            "scan",
        ),
        (
            "root.innerHTML = '<a id=scan><slot></slot></a>'; root.querySelector('a').append(right);",
            "scan",
        ),
        (
            "root.innerHTML = '<slot id=slot name=fallback></slot>'; root.querySelector('slot').append(left, right);",
            "slot",
        ),
        (
            "const a = document.createElement('div'), b = document.createElement('div'); a.style.cssText = 'position:absolute;left:0;top:0;width:80px;height:80px'; b.style.cssText = 'position:absolute;left:80px;top:0;width:80px;height:80px'; right.style.left = '0'; a.attachShadow({mode:'open'}).append(left); b.attachShadow({mode:'open'}).append(right); root.append(a,b);",
            "host",
        ),
    ] {
        let mut vm = shadow_click_vm(setup);
        mouse_button(&mut vm, 20.0, "mousedown");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify(clicks)")
                .expect("clicks should evaluate"),
            serde_json::json!([["host", target, true]]).to_string(),
            "shadow setup: {setup}"
        );
    }
}

#[test]
fn mouse_click_uses_slot_assignment_at_click_dispatch_time() {
    for manual in [false, true] {
        let setup = if manual {
            "host.shadowRoot.innerHTML = '<slot id=first></slot><slot id=second></slot>'; root.querySelector('#first').assign(left); root.querySelector('#second').assign(right);"
        } else {
            "left.slot = 'first'; right.slot = 'second'; root.innerHTML = '<slot id=first name=first></slot><slot id=second name=second></slot>';"
        };
        let options = if manual {
            "{mode:'open',slotAssignment:'manual'}"
        } else {
            "{mode:'open'}"
        };
        let mut vm = shadow_click_vm_with_options(setup, options);
        vm.eval(if manual {
            "document.addEventListener('mouseup', () => root.querySelector('#second').assign(left,right), {once:true})"
        } else {
            "document.addEventListener('mouseup', () => { left.slot = 'second'; }, {once:true})"
        }).expect("slot reassignment listener should install");
        mouse_button(&mut vm, 20.0, "mousedown");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify(clicks)")
                .expect("clicks should evaluate"),
            r#"[["host","second",true]]"#,
            "manual slot assignment: {manual}"
        );
    }
}

#[test]
fn mouse_click_removal_checks_dom_ancestry_instead_of_slot_distribution() {
    for (mutation, expected) in [
        (
            "const slot = root.querySelector('slot'); slot.remove(); root.append(slot);",
            r#"[["host","slot",true]]"#,
        ),
        ("host.remove(); document.body.append(host);", "[]"),
    ] {
        let mut vm = shadow_click_vm("root.innerHTML = '<slot id=slot></slot>';");
        vm.eval(&format!(
            "document.addEventListener('mouseup', () => {{ {mutation} }}, {{once:true}})"
        ))
        .expect("removal listener should install");
        mouse_button(&mut vm, 20.0, "mousedown");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify(clicks)")
                .expect("clicks should evaluate"),
            expected,
            "removal: {mutation}"
        );
    }
}

#[test]
fn mouse_click_does_not_cross_unassigned_nodes_or_replaced_slot_fallback() {
    for (setup, mutation) in [
        (
            "left.slot = 'first'; right.slot = 'second'; root.innerHTML = '<slot name=first></slot><slot name=second></slot>';",
            "left.slot = 'missing';",
        ),
        (
            "left.slot = 'first'; right.slot = 'second'; root.innerHTML = '<slot name=first></slot><slot name=second></slot>';",
            "right.slot = 'missing';",
        ),
        (
            "root.innerHTML = '<slot name=fallback></slot>'; root.querySelector('slot').append(left,right);",
            "const assigned = document.createElement('span'); assigned.slot = 'fallback'; host.append(assigned);",
        ),
    ] {
        let mut vm = shadow_click_vm(setup);
        vm.eval(&format!(
            "document.addEventListener('mouseup', () => {{ {mutation} }}, {{once:true}})"
        ))
        .expect("distribution change listener should install");
        mouse_button(&mut vm, 20.0, "mousedown");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify(clicks)")
                .expect("clicks should evaluate"),
            "[]",
            "distribution change: {mutation}"
        );
    }
}

#[test]
fn mouse_auxclick_uses_the_flat_tree_common_ancestor() {
    let mut vm = shadow_click_vm("root.innerHTML = '<slot id=slot></slot>';");
    vm.eval(
        "document.addEventListener('auxclick', event => { event.preventDefault(); clicks.push([event.target.id,event.composedPath()[0].id,event.isTrusted]); });",
    ).expect("auxclick listener should install");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 1, None, 0.0, 0.0)
        .expect("middle button press should dispatch");
    vm.dispatch_mouse_event_at_point(100.0, 20.0, "mouseup", 1, None, 0.0, 0.0)
        .expect("middle button release should dispatch");
    assert_eq!(
        vm.eval("JSON.stringify(clicks)")
            .expect("clicks should evaluate"),
        r#"[["host","slot",true]]"#,
    );
}

#[test]
fn mouse_release_outside_viewport_clears_the_pending_click() {
    let mut vm = scan_link_vm();
    mouse_button(&mut vm, 20.0, "mousedown");
    vm.dispatch_mouse_event_at_point(2000.0, 2000.0, "mouseup", 0, None, 0.0, 0.0)
        .expect("release outside the viewport should complete");
    mouse_button(&mut vm, 20.0, "mouseup");
    assert_eq!(
        vm.eval("JSON.stringify([events.filter(e => e[0] === 'click'), scans])")
            .expect("clicks should evaluate"),
        r#"[[["click","",true]],0]"#,
    );
}

#[test]
fn mouse_click_in_a_closed_shadow_tree_targets_the_slot_and_retargets_outside() {
    let mut vm = shadow_click_vm_with_options(
        "root.innerHTML = '<slot id=slot></slot>';",
        "{mode:'closed'}",
    );
    vm.eval(
        "root.addEventListener('click', event => clicks.push(['shadow',event.target.id,event.isTrusted]));",
    ).expect("closed shadow listener should install");
    mouse_button(&mut vm, 20.0, "mousedown");
    mouse_button(&mut vm, 100.0, "mouseup");
    assert_eq!(
        vm.eval("JSON.stringify(clicks)")
            .expect("clicks should evaluate"),
        r#"[["shadow","slot",true],["host","host",true]]"#,
    );
}

#[test]
fn mouse_click_capture_overrides_the_flat_tree_for_an_unassigned_target() {
    let mut vm = shadow_click_vm(
        "left.slot = 'shown'; right.slot = 'hidden'; root.innerHTML = '<slot name=shown></slot>'; left.addEventListener('pointerdown', event => right.setPointerCapture(event.pointerId));",
    );
    mouse_button(&mut vm, 20.0, "mousedown");
    mouse_button(&mut vm, 20.0, "mouseup");
    assert_eq!(
        vm.eval("JSON.stringify([clicks,right.hasPointerCapture(1)])")
            .expect("capture result should evaluate"),
        r#"[[["right","right",true]],false]"#,
    );
}

#[test]
fn mouse_release_retargets_when_pointerup_removes_or_reparents_its_target() {
    for mutation in [
        "event.target.remove();",
        "document.getElementById('left').append(event.target);",
    ] {
        let mut vm = scan_link_vm();
        vm.eval(&format!(
            "document.addEventListener('pointerup', event => {{ {mutation} }}, {{once:true}})"
        ))
        .expect("pointerup mutation listener should install");
        mouse_button(&mut vm, 20.0, "mousedown");
        mouse_button(&mut vm, 100.0, "mouseup");
        assert_eq!(
            vm.eval("JSON.stringify([events,scans,location.pathname])")
                .expect("release result should evaluate"),
            r#"[[["mousedown","left",true],["mouseup","scan",true],["click","scan",true]],1,"/fingerprint-check"]"#,
            "pointerup mutation: {mutation}"
        );
    }
}

#[test]
fn mouse_release_retargets_a_removed_slotted_target_to_its_dom_parent() {
    let mut vm = shadow_click_vm("root.innerHTML = '<slot id=slot></slot>';");
    vm.eval(
        "window.releases=[]; document.addEventListener('mouseup', e => releases.push(e.composedPath()[0].id)); document.addEventListener('pointerup', e => e.target.remove(), {once:true});",
    ).expect("slotted removal listeners should install");
    mouse_button(&mut vm, 20.0, "mousedown");
    mouse_button(&mut vm, 100.0, "mouseup");
    assert_eq!(
        vm.eval("JSON.stringify([releases,clicks])")
            .expect("slotted release result should evaluate"),
        r#"[["host"],[["host","host",true]]]"#,
    );
}

#[test]
fn mouse_click_keeps_the_capture_target_when_release_listeners_move_or_remove_it() {
    for event_type in ["pointerup", "mouseup"] {
        for reinsert in [false, true] {
            let mut vm = scan_link_vm();
            let mutation = if reinsert {
                "document.getElementById('left').append(event.target);"
            } else {
                "event.target.remove();"
            };
            vm.eval(&format!(
                r#"
                window.right = document.getElementById('right');
                window.nodeClicks = [];
                right.addEventListener('click', e => nodeClicks.push([e.target.id,e.isTrusted]));
                document.getElementById('left').addEventListener('pointerdown', e => right.setPointerCapture(e.pointerId));
                document.addEventListener('{event_type}', event => {{ {mutation} }}, {{once:true}});
                "#
            )).expect("captured release listeners should install");
            mouse_button(&mut vm, 20.0, "mousedown");
            mouse_button(&mut vm, 20.0, "mouseup");
            assert_eq!(
                vm.eval("JSON.stringify([events.filter(e=>e[0]==='mouseup'),events.filter(e=>e[0]==='click'),nodeClicks,scans,right.hasPointerCapture(1)])")
                    .expect("captured release result should evaluate"),
                serde_json::json!([
                    [["mouseup", if event_type == "pointerup" { "scan" } else { "right" }, true]],
                    if reinsert { serde_json::json!([["click", "right", true]]) } else { serde_json::json!([]) },
                    [["right", true]],
                    if reinsert { 1 } else { 0 },
                    false,
                ]).to_string(),
                "release event: {event_type}, reinsert: {reinsert}",
            );
        }
    }
}

#[test]
fn removing_one_chorded_press_preserves_the_other_buttons_activation() {
    for (removed_button, removed_mask, kept_button, kept_mask) in
        [(0, 1, 1, 4), (1, 4, 0, 1), (2, 2, 0, 1), (0, 1, 2, 2)]
    {
        let mut vm = scan_link_vm();
        vm.eval(
            "window.activations = []; for (const type of ['click', 'auxclick']) document.addEventListener(type, event => { activations.push([event.type, event.target.id, event.button, event.buttons, event.isTrusted]); event.preventDefault(); }); document.addEventListener('contextmenu', event => event.preventDefault());",
        ).expect("record all chorded activations");
        let dispatch = |vm: &mut StandaloneScriptVmHarness, x, event, button, buttons| {
            vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                x,
                20.0,
                event,
                button,
                Some(buttons),
                1,
                0.0,
                0.0,
                crate::runtime::RendererPointerEventProperties {
                    pointer_id: 17,
                    pointer_type: "mouse".to_owned(),
                    ..Default::default()
                },
                0,
                false,
            )
            .expect("chorded coordinate input should dispatch");
        };
        dispatch(&mut vm, 20.0, "mousedown", removed_button, removed_mask);
        dispatch(
            &mut vm,
            100.0,
            "mousedown",
            kept_button,
            removed_mask | kept_mask,
        );
        vm.eval("document.getElementById('left').remove()")
            .expect("remove only one pressed subtree");
        vm.publish_layout_for_test()
            .expect("publish the surviving release target");
        dispatch(&mut vm, 100.0, "mouseup", removed_button, kept_mask);
        dispatch(&mut vm, 100.0, "mouseup", kept_button, 0);
        dispatch(&mut vm, 100.0, "mouseup", kept_button, 0);
        let actual = vm
            .eval("JSON.stringify(activations)")
            .expect("read chorded activations");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            serde_json::json!([[
                if kept_button == 0 {
                    "click"
                } else {
                    "auxclick"
                },
                "right",
                kept_button,
                0,
                true,
            ]]),
            "removed button {removed_button}, surviving button {kept_button}"
        );
    }
}
