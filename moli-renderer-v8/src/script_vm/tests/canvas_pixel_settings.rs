use super::*;

#[test]
fn canvas_pixel_settings_preserve_formats_colors_and_webidl_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-pixel-settings.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(
        r#"(() => {{
        const probe = {};
        const realms = [globalThis, document.getElementById('child').contentWindow];
        const checks = [];
        for(let owner=0;owner<2;owner++) for(let callee=0;callee<2;callee++)
            for(const kind of ['element','offscreen'])
                checks.push(...probe(realms[owner],realms[callee],kind,owner+'-'+callee+'-'+kind));
        globalThis.pixelSettingsChecks = checks;
    }})()"#,
        include_str!("canvas_pixel_settings.js")
    ))
    .unwrap();
    assert_eq!(vm.eval("pixelSettingsChecks.length").unwrap(), "464");
    assert_eq!(
        vm.eval("JSON.stringify(pixelSettingsChecks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
