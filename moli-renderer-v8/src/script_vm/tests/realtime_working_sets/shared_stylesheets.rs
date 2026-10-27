use super::*;

const SETUP: &str = r#"
(() => {
  const roots = ['left', 'right'].map(id =>
    document.getElementById(id).attachShadow({mode:'open'}));
  const targets = roots.map(root => {
    const target = document.createElement('div');
    target.className = 'shared-target';
    root.append(target);
    return target;
  });
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.shared-target {display:block;width:100px;height:20px;padding:0;border:0;color:rgb(1,2,3)}');
  roots.forEach(root => { root.adoptedStyleSheets = [sheet]; });
  const held = targets.map(target => getComputedStyle(target));
  globalThis.sharedSheet = sheet;
  globalThis.readShared = () => {
    // Preserve the first answers before any subsequent read could refresh a
    // stale style world or frozen geometry. Each API starts in a fresh VM.
    const first = sharedFirstRead === 'color' ? held.map(style => style.color)
      : sharedFirstRead === 'width' ? held.map(style => style.width)
      : targets.map(target => target.offsetWidth);
    return {
      first,
      colors:held.map(style => style.color),
      widths:held.map(style => style.width),
      boxes:targets.map(target => target.offsetWidth),
      displays:held.map(style => style.display)
    };
  };
  return JSON.stringify(readShared());
})()
"#;

fn expected(first_read: &str, color: &str, width: u32, display: &str) -> serde_json::Value {
    let css_width = format!("{width}px");
    let first = match first_read {
        "color" => serde_json::json!([color, color]),
        "width" => serde_json::json!([css_width, css_width]),
        "offsetWidth" => serde_json::json!([width, width]),
        _ => unreachable!("the fixture has three first-read variants"),
    };
    serde_json::json!({
        "first": first,
        "colors": [color, color],
        "widths": [css_width, css_width],
        "boxes": [width, width],
        "displays": [display, display]
    })
}

fn evaluate_json(vm: &mut StandaloneScriptVmHarness, script: &str) -> serde_json::Value {
    let result = vm
        .eval(script)
        .expect("the shared stylesheet script must succeed");
    serde_json::from_str(&result).expect("the script must return primitive JSON")
}

#[test]
fn realtime_shared_shadow_stylesheets_refresh_first_reads_and_retained_graphs() {
    for first_read in ["color", "width", "offsetWidth"] {
        let mut vm = new_parsed_test_vm(
            "https://realtime-shared-shadow-sheet.test/",
            "<!doctype html><body style='margin:0'><section id=left></section><section id=right></section>",
        );
        vm.set_layout_policy(LayoutPolicy::Realtime);
        let initial = evaluate_json(
            &mut vm,
            &format!("globalThis.sharedFirstRead={first_read:?};{SETUP}"),
        );
        assert_eq!(initial, expected(first_read, "rgb(1, 2, 3)", 100, "block"));
        let cold = main_stats(&vm).expect("the small supported graph must be admitted");
        assert!(cold.retained_boxes > 0, "{first_read}: {cold:?}");
        assert!(
            cold.estimated_construction_bytes > 0,
            "{first_read}: {cold:?}"
        );
        assert_eq!(cold.reused_construction_boxes, 0);
        let builds = vm.layout_pass_observability_for_test().1;

        // Every source change and its first read are in this single eval.
        // No turn-exit drain, screenshot, or new computed-style wrapper can
        // repair the first returned observations.
        let changed = evaluate_json(
            &mut vm,
            r#"(() => {
              sharedSheet.replaceSync('.shared-target {display:block;width:200px;height:20px;padding:0;border:0;color:rgb(4,5,6)}');
              const replaced = readShared();
              const replacedClean = readShared();
              const declaration = sharedSheet.cssRules[0].style;
              declaration.color = 'rgb(7,8,9)';
              declaration.width = '300px';
              const edited = readShared();
              const editedClean = readShared();
              return JSON.stringify([replaced, replacedClean, edited, editedClean]);
            })()"#,
        );
        let replaced = expected(first_read, "rgb(4, 5, 6)", 200, "block");
        let edited = expected(first_read, "rgb(7, 8, 9)", 300, "block");
        assert_eq!(
            changed,
            serde_json::json!([replaced, replaced, edited, edited])
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds + 2);
        let warm = main_stats(&vm).expect("width and color changes keep an admitted graph");
        assert!(warm.reused_construction_boxes > 0, "{first_read}: {warm:?}");
        assert!(warm.reused_box_slots > 0, "{first_read}: {warm:?}");

        // Both shapes are supported, but their principal box kinds differ.
        // This assertion is meaningful only after the actual warm hit above.
        let changed_shape = evaluate_json(
            &mut vm,
            r#"(() => {
              const declaration = sharedSheet.cssRules[0].style;
              declaration.display = 'flow-root';
              declaration.width = '400px';
              declaration.color = 'rgb(10,11,12)';
              const first = readShared();
              const clean = readShared();
              return JSON.stringify([first, clean]);
            })()"#,
        );
        let changed = expected(first_read, "rgb(10, 11, 12)", 400, "flow-root");
        assert_eq!(changed_shape, serde_json::json!([changed, changed]));
        assert_eq!(vm.layout_pass_observability_for_test().1, builds + 3);
        let rebuilt = main_stats(&vm).expect("the new graph must remain admitted");
        assert_eq!(
            rebuilt.reused_construction_boxes, 0,
            "{first_read}: {rebuilt:?}"
        );
        assert_eq!(rebuilt.reused_box_slots, 0, "{first_read}: {rebuilt:?}");
        assert_eq!(
            vm.layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0,
        );
    }
}
