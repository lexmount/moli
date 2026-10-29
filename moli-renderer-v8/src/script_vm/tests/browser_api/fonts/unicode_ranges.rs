use super::*;
fn font_widths(vm: &mut StandaloneScriptVmHarness) -> [f32; 6] {
    // Use the page API so this warms the same frozen geometry cache as CDP.
    // Building a fresh native pass here would hide missing font invalidation.
    let widths = vm
        .eval(
            r#"JSON.stringify(
        ['rangeA','rangeB','allA','allB','fallbackA','fallbackB'].map(
          id=>document.getElementById(id).getBoundingClientRect().width))"#,
        )
        .unwrap();
    serde_json::from_str(&widths).unwrap()
}

#[test]
fn font_face_unicode_range_limits_layout_and_updates_after_loading() {
    let mut vm = new_parsed_test_vm(
        "https://font-unicode-range.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-layout/tests/fixtures/moli-ahem.woff2"
    )));
    vm.eval(&format!(
        r#"
        const bytes=Uint8Array.from(atob('{font}'),c=>c.charCodeAt(0));
        globalThis.face=new FontFace('RangeFace',bytes,{{unicodeRange:'U+0041'}});
        const all=new FontFace('AllFace',bytes);
        document.fonts.add(face);document.fonts.add(all);
        for(const [id,family,text] of [
          ['rangeA','RangeFace,monospace','AAAA'],['rangeB','RangeFace,monospace','BBBB'],
          ['allA','AllFace,monospace','AAAA'],['allB','AllFace,monospace','BBBB'],
          ['fallbackA','monospace','AAAA'],['fallbackB','monospace','BBBB']
        ]) {{
          const span=document.createElement('span');span.id=id;span.textContent=text;
          span.style.cssText=`display:inline-block;font-size:40px;font-family:${{family}}`;
          document.body.append(span);
        }}
    "#
    ))
    .unwrap();
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    let [range_a, range_b, all_a, all_b, fallback_a, fallback_b] = font_widths(&mut vm);
    assert!(
        (all_a - fallback_a).abs() > 0.1,
        "A control must distinguish the custom font from fallback"
    );
    assert!(
        (all_b - fallback_b).abs() > 0.1,
        "B control must distinguish the custom font from fallback"
    );
    assert_eq!(range_a, all_a);
    assert_eq!(range_b, fallback_b);

    vm.eval("face.unicodeRange='U+0042'").unwrap();
    let [range_a, range_b, ..] = font_widths(&mut vm);
    assert_eq!(range_a, fallback_a);
    assert_eq!(range_b, all_b);

    // Other descriptor writes must not restore the constructor's old range.
    let generation = vm._context_host.borrow().visual_state_generation();
    vm.eval("face.weight='400'").unwrap();
    assert_eq!(
        vm._context_host.borrow().visual_state_generation(),
        generation
    );
    let [range_a, range_b, ..] = font_widths(&mut vm);
    assert_eq!(range_a, fallback_a);
    assert_eq!(range_b, all_b);

    vm.eval("document.fonts.delete(face)").unwrap();
    let [range_a, range_b, ..] = font_widths(&mut vm);
    assert_eq!(range_a, fallback_a);
    assert_eq!(range_b, fallback_b);
    vm.eval("document.fonts.add(face)").unwrap();
    let [range_a, range_b, ..] = font_widths(&mut vm);
    assert_eq!(range_a, fallback_a);
    assert_eq!(range_b, all_b);
}
