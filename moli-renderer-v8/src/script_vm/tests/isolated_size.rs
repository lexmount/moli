use super::*;
use moli_page_types::LayoutPolicy;

fn measurement_vm() -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://isolated-size.test/",
        r#"<!doctype html><style>body{margin:0;font:14px sans-serif}</style>
        <main id=background style="width:640px;height:800px">background</main>
        <div id=probe style="position:absolute;top:-9999px;left:-9999px">
          <span id=sample style="font-size:72px">mmMwWLliI0O&amp;1</span>
        </div>"#,
    );
    vm.set_layout_policy(LayoutPolicy::Realtime);
    vm.set_scrollbars_hidden(true);
    vm.eval("globalThis.sample=document.getElementById('sample');globalThis.probe=document.getElementById('probe');sample.offsetWidth").unwrap();
    vm
}

fn isolated_passes(vm: &StandaloneScriptVmHarness) -> u64 {
    vm.context_host_weak_for_test()
        .upgrade()
        .unwrap()
        .borrow()
        .isolated_size_layout_count_for_test()
}

#[test]
fn isolated_size_repeated_text_measurements_retain_background_layout() {
    let mut vm = measurement_vm();
    let builds = vm.layout_pass_observability_for_test().1;
    let result = vm
        .eval(
            r#"
        const measurements=[];
        for (const font of ['monospace','serif','sans-serif']) {
            sample.remove();
            sample.style.fontFamily=font;
            probe.append(sample);
            measurements.push([sample.offsetWidth,sample.offsetHeight]);
        }
        JSON.stringify(measurements);
    "#,
        )
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(isolated_passes(&vm), 3);
    // The full publication remains dirty. Rect consumers publish a fresh tree
    // and must agree with the size-only result rather than reusing stale data.
    let final_pair = serde_json::from_str::<serde_json::Value>(&result).unwrap()[2].clone();
    vm.eval("sample.getBoundingClientRect()").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &vm.eval("JSON.stringify([sample.offsetWidth,sample.offsetHeight])")
                .unwrap()
        )
        .unwrap(),
        final_pair,
    );
}

#[test]
fn isolated_size_matches_full_layout_after_text_padding_and_percentage_changes() {
    let mut vm = measurement_vm();
    for update in [
        "sample.textContent='longer text with spaces';sample.style.fontSize='23px'",
        "sample.style.cssText='display:inline-block;padding:3px 7px;border:2px solid;font:29px monospace'",
        "probe.style.width='50%';sample.style.cssText='display:block;width:70%;padding:4px;box-sizing:border-box'",
    ] {
        let local_before = isolated_passes(&vm);
        vm.eval(update).unwrap();
        let local = vm.eval("JSON.stringify([sample.offsetWidth,sample.offsetHeight,probe.offsetWidth,probe.offsetHeight])").unwrap();
        assert_eq!(isolated_passes(&vm), local_before + 1, "{update}");
        vm.eval("sample.getBoundingClientRect()").unwrap();
        let full = vm.eval("JSON.stringify([sample.offsetWidth,sample.offsetHeight,probe.offsetWidth,probe.offsetHeight])").unwrap();
        assert_eq!(local, full, "{update}");
    }
}

#[test]
fn isolated_size_preserves_full_flush_for_background_changes_and_css_dependencies() {
    for update in [
        "background.style.width='320px'",
        "background.firstChild.data='changed background'",
        "document.body.style.fontSize='30px'",
        "document.head.append(Object.assign(document.createElement('style'),{textContent:'body:has(#sample.wide) #background{width:300px}'}));sample.className='wide'",
    ] {
        let mut vm = measurement_vm();
        let builds = vm.layout_pass_observability_for_test().1;
        vm.eval(update).unwrap();
        vm.eval("sample.style.fontFamily='serif';sample.offsetWidth")
            .unwrap();
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds + 1,
            "{update}"
        );
        assert_eq!(isolated_passes(&vm), 0, "{update}");
    }
}

#[test]
fn isolated_size_rejects_nonindependent_formatting_contexts() {
    for update in [
        "probe.style.position='relative'",
        "probe.style.left='auto';probe.style.right='auto'",
        "probe.style.transform='scale(2)'",
        "probe.style.zoom='2'",
        "probe.style.direction='rtl'",
        "probe.style.display='flex'",
        "probe.style.contain='size'",
        "probe.style.setProperty('content-visibility','hidden')",
        "sample.style.display='list-item'",
        "document.body.style.position='relative'",
        "probe.append(Object.assign(document.createElement('input'),{value:'form'}))",
        "sample.textContent='x'.repeat(8193)",
    ] {
        let mut vm = measurement_vm();
        let builds = vm.layout_pass_observability_for_test().1;
        vm.eval(update).unwrap();
        vm.eval("sample.offsetWidth").unwrap();
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds + 1,
            "{update}"
        );
        assert_eq!(isolated_passes(&vm), 0, "{update}");
    }
}

#[test]
fn isolated_size_tracks_css_only_font_changes_and_used_size_reads() {
    let mut vm = measurement_vm();
    vm.eval("document.head.append(Object.assign(document.createElement('style'),{textContent:'#sample.small{font-size:12px!important}'}));sample.getBoundingClientRect()").unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    let local = isolated_passes(&vm);
    vm.eval(
        "sample.className='small';globalThis.measured=[sample.offsetWidth,sample.offsetHeight]",
    )
    .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(isolated_passes(&vm), local + 1);
    assert_eq!(
        vm.eval("getComputedStyle(sample).fontSize").unwrap(),
        "12px"
    );
    // A used-size CSSOM read keeps its normal complete freshness barrier.
    vm.eval("getComputedStyle(probe).width").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(
        vm.eval(
            "JSON.stringify(measured)===JSON.stringify([sample.offsetWidth,sample.offsetHeight])"
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn isolated_size_css_dependencies_outside_region_force_full_layout() {
    let mut vm = measurement_vm();
    vm.eval("document.head.append(Object.assign(document.createElement('style'),{textContent:'body:has(#sample.wide) #background{width:300px!important}'}));sample.getBoundingClientRect()").unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    let local = isolated_passes(&vm);
    vm.eval("sample.className='wide';sample.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(isolated_passes(&vm), local);
    assert_eq!(
        vm.eval("getComputedStyle(background).width").unwrap(),
        "300px"
    );
}

#[test]
fn isolated_size_preserves_previously_resolved_damage_outside_region() {
    let mut vm = measurement_vm();
    vm.eval("document.head.append(Object.assign(document.createElement('style'),{textContent:'body:has(#sample.wide) #background{width:300px!important}'}));sample.getBoundingClientRect()").unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    let local = isolated_passes(&vm);
    vm.eval("sample.className='wide';getComputedStyle(background).color;sample.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(isolated_passes(&vm), local);
    assert_eq!(vm.eval("String(background.offsetWidth)").unwrap(), "300");
}

#[test]
fn isolated_size_retires_cache_after_removal_and_document_replacement() {
    let mut vm = measurement_vm();
    vm.eval("sample.style.fontSize='24px';sample.offsetWidth")
        .unwrap();
    let local = isolated_passes(&vm);
    let builds = vm.layout_pass_observability_for_test().1;
    assert_eq!(
        vm.eval("probe.remove();String(sample.offsetWidth)")
            .unwrap(),
        "0"
    );
    vm.eval("document.body.append(probe);sample.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
    assert_eq!(isolated_passes(&vm), local);
    vm.eval("document.open();document.write('<!doctype html><div id=newTarget style=width:55px></div>');document.close()").unwrap();
    assert_eq!(
        vm.eval("String(document.getElementById('newTarget').offsetWidth)")
            .unwrap(),
        "55"
    );
    assert_eq!(vm.eval("String(sample.offsetWidth)").unwrap(), "0");
}

#[test]
fn isolated_size_positioned_height_rounding_matches_full_layout() {
    for position in ["absolute", "fixed"] {
        for (inset, expected_local_passes) in
            [("", 0), ("top:0", 1), ("bottom:0", 1), ("top:0.3px", 1)]
        {
            let mut vm = new_parsed_test_vm(
                "https://isolated-size.test/",
                &format!(
                    r#"<!doctype html><style>
                    body {{ margin:0 }}
                    #background {{ height:0.3px }}
                    #probe {{ position:{position};left:0;width:100px;height:10.49px;{inset} }}
                    </style><div id=background></div><div id=probe></div>"#
                ),
            );
            vm.set_layout_policy(LayoutPolicy::Realtime);
            vm.set_scrollbars_hidden(true);
            let result = vm
                .eval(
                    r#"
                const probe=document.getElementById('probe');
                probe.getBoundingClientRect();
                const before=probe.offsetHeight;
                probe.style.width='101px';
                const measured=probe.offsetHeight;
                probe.getBoundingClientRect();
                JSON.stringify([before,measured,probe.offsetHeight]);
            "#,
                )
                .unwrap();
            let heights: Vec<u32> = serde_json::from_str(&result).unwrap();
            assert_eq!(heights[1], heights[0], "{position};{inset}: {result}");
            assert_eq!(heights[1], heights[2], "{position};{inset}: {result}");
            assert_eq!(
                isolated_passes(&vm),
                expected_local_passes,
                "{position};{inset}"
            );
        }
    }
}

#[test]
fn isolated_size_ignores_unchanged_ancestor_counters() {
    let mut vm = measurement_vm();
    vm.eval("document.body.style.counterReset='equation 0';probe.style.cssText='';sample.style.cssText='position:absolute;top:-9999px;left:-9999px;font-size:72px';sample.getBoundingClientRect()").unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    let local = isolated_passes(&vm);
    let measured = vm.eval("sample.style.fontFamily='monospace';JSON.stringify([sample.offsetWidth,sample.offsetHeight])").unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(isolated_passes(&vm), local + 1);
    vm.eval("sample.getBoundingClientRect()").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([sample.offsetWidth,sample.offsetHeight])")
            .unwrap(),
        measured
    );
    // Counter changes inside the measured region can affect generated text
    // elsewhere, so they keep the ordinary full-publication path.
    let builds = vm.layout_pass_observability_for_test().1;
    vm.eval("sample.style.counterIncrement='equation';sample.offsetWidth")
        .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[test]
fn isolated_size_reuses_published_sources_with_form_controls_and_shadow_background() {
    let mut vm = measurement_vm();
    vm.eval(r#"
        background.innerHTML='<div style="display:flex"><textarea>default text</textarea><select><option>one</option></select></div><div id=shadowHost></div>';
        shadowHost.attachShadow({mode:'open'}).innerHTML='<style>span{display:block;width:44px}</style><span>shadow text</span>';
        sample.getBoundingClientRect();
    "#).unwrap();
    let builds = vm.layout_pass_observability_for_test().1;
    let local = isolated_passes(&vm);
    vm.eval(
        "sample.remove();sample.style.fontFamily='serif';probe.append(sample);sample.offsetWidth",
    )
    .unwrap();
    assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    assert_eq!(isolated_passes(&vm), local + 1);
    assert_eq!(
        vm.eval("shadowHost.shadowRoot.querySelector('span').offsetWidth")
            .unwrap(),
        "44"
    );
    assert_eq!(vm.layout_pass_observability_for_test().1, builds + 1);
}

#[test]
fn isolated_size_preserves_placeholder_and_marker_invalidation_outside_region() {
    for (markup, selector, pseudo) in [
        (
            "<textarea placeholder=hint></textarea>",
            "textarea",
            "placeholder",
        ),
        ("<ul><li>item</li></ul>", "li", "marker"),
    ] {
        let mut vm = measurement_vm();
        vm.eval(&format!(r#"
            background.innerHTML={markup:?};
            document.head.append(Object.assign(document.createElement('style'),{{textContent:
                '#background {selector}::{pseudo}{{font-size:12px}}body:has(#sample.wide) #background {selector}::{pseudo}{{font-size:24px}}'
            }}));
            sample.getBoundingClientRect();
        "#)).unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        vm.eval("sample.className='wide';sample.offsetWidth")
            .unwrap();
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds + 1,
            "{pseudo}"
        );
        assert_eq!(
            vm.eval(&format!(
                "getComputedStyle(background.querySelector('{selector}'),'::{pseudo}').fontSize"
            ))
            .unwrap(),
            "24px",
            "{pseudo}"
        );
    }
}
