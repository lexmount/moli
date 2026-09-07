use super::*;
use base64::Engine;

fn install_font(vm: &mut StandaloneScriptVmHarness, family: &str, bytes: &[u8]) {
    vm.eval("if (!document.documentElement) document.append(document.createElement('html')); if (!document.body) document.documentElement.append(document.createElement('body'));").unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    vm.eval(&format!(r#"globalThis.fixtureFace = new FontFace('{family}', Uint8Array.from(atob('{encoded}'), c => c.charCodeAt(0))); document.fonts.add(fixtureFace);"#)).unwrap();
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moli-layout/tests/fixtures/moli-ahem.ttf"
));
const HEBREW: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../moli-layout/tests/fixtures/moli-hebrew-emoji.ttf"
));

#[test]
fn canvas_text_measures_real_fractional_advances_and_tracks_font_registration() {
    let mut vm = new_storage_test_vm("https://canvas-fonts.test/");
    vm.eval(r#"
globalThis.canvas = document.createElement('canvas'); canvas.width = 100; canvas.height = 40;
globalThis.ctx = canvas.getContext('2d'); ctx.font = '10.5px CanvasFixture';
globalThis.fallbackWidth = ctx.measureText('iiii').width;
ctx.fillText('iiii', 2, 18); globalThis.fallbackImage = canvas.toDataURL(); ctx.clearRect(0, 0, 100, 40);
"#).unwrap();
    install_font(&mut vm, "CanvasFixture", AHEM);
    assert_eq!(
        vm.eval(
            r#"
ctx.fillText('iiii', 2, 18);
JSON.stringify({width: Math.round(ctx.measureText('iiii').width * 1000) / 1000,
  spaces: Math.round(ctx.measureText('A\nA\t').width * 1000) / 1000,
  empty: ctx.measureText('').width, changed: fallbackImage !== canvas.toDataURL()})
"#
        )
        .unwrap(),
        r#"{"width":25.2,"spaces":25.2,"empty":0,"changed":true}"#
    );
    assert_eq!(
        vm.eval(
            "document.fonts.delete(fixtureFace); ctx.measureText('iiii').width === fallbackWidth"
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn canvas_text_respects_non_latin_glyph_widths_for_html_and_offscreen_contexts() {
    let mut vm = new_storage_test_vm("https://canvas-fonts.test/");
    install_font(&mut vm, "CanvasFixture", HEBREW);
    assert_eq!(vm.eval(r#"
JSON.stringify([document.createElement('canvas'), new OffscreenCanvas(100, 40)].map(canvas => {
 const ctx = canvas.getContext('2d'); ctx.font = '16px CanvasFixture';
 const a = ctx.measureText('א').width, b = ctx.measureText('ב').width;
 ctx.font = '16.5px CanvasFixture'; const bigger = ctx.measureText('א').width;
 ctx.fillText('א', 5, 22);
 return a > b && b > 0 && Math.abs(bigger / a - 16.5 / 16) < .001 && ctx.getImageData(0, 0, 80, 40).data.some(v => v !== 0);
}))
"#).unwrap(), "[true,true]");
}

#[test]
fn canvas_font_assignment_validates_shorthand_and_resolves_relative_sizes_once() {
    let mut vm = new_storage_test_vm("https://canvas-fonts.test/");
    install_font(&mut vm, "CanvasFixture", AHEM);
    assert_eq!(vm.eval(r#"
const canvas = document.createElement('canvas'); canvas.style.fontSize = '20px'; document.body.append(canvas);
const ctx = canvas.getContext('2d'); ctx.font = '1.25em CanvasFixture';
const before = ctx.measureText('A').width; canvas.style.fontSize = '40px';
const same = ctx.measureText('A').width === before;
const accepted = ctx.font; ctx.font = 'not a font'; const invalid = ctx.font === accepted;
ctx.font = '12.5pt CanvasFixture';
JSON.stringify([Math.round(before * 1000) / 1000, same, invalid, Math.round(ctx.measureText('A').width * 1000) / 1000])
"#).unwrap(), "[15,true,true,10]");
}

#[test]
fn canvas_text_paint_uses_transform_alpha_max_width_and_real_stroked_outlines() {
    let mut vm = new_storage_test_vm("https://canvas-fonts.test/");
    install_font(&mut vm, "CanvasFixture", AHEM);
    assert_eq!(vm.eval(r#"
function paint(stroke, maxWidth) {
 const canvas = new OffscreenCanvas(100, 60), ctx = canvas.getContext('2d');
 ctx.font = '40px CanvasFixture'; ctx.fillStyle = '#ff0000'; ctx.strokeStyle = '#00ff00';
 ctx.globalAlpha = .5; ctx.translate(15, 0);
 if (stroke) ctx.strokeText('A', 0, 40, maxWidth); else ctx.fillText('A', 0, 40, maxWidth);
 const bytes = ctx.getImageData(0, 0, 100, 60).data;
 let count = 0, maxAlpha = 0, minX = 100, maxX = 0, red = 0, green = 0;
 for (let i = 0; i < bytes.length; i += 4) if (bytes[i + 3]) {
   count++; const x = i / 4 % 100; minX = Math.min(minX, x); maxX = Math.max(maxX, x);
   maxAlpha = Math.max(maxAlpha, bytes[i + 3]); red = Math.max(red, bytes[i]); green = Math.max(green, bytes[i + 1]);
 }
 return {count, maxAlpha, minX, maxX, red, green};
}
const fill = paint(false, 100), compressed = paint(false, 12), stroke = paint(true, 100), empty = paint(false, 0);
JSON.stringify([fill.count > 0, fill.minX >= 15, fill.maxAlpha >= 127 && fill.maxAlpha <= 128,
 fill.red === 255 && fill.green === 0, compressed.maxX < fill.maxX,
 stroke.count > 0 && stroke.count < fill.count, stroke.green === 255 && stroke.red === 0, empty.count === 0])
"#).unwrap(), "[true,true,true,true,true,true,true,true]");
}

#[test]
fn canvas_text_requires_branded_receivers_and_does_not_flush_dom_layout() {
    let mut vm = new_storage_test_vm("https://canvas-fonts.test/");
    install_font(&mut vm, "CanvasFixture", AHEM);
    vm.eval("globalThis.geometryProbe = document.createElement('div'); geometryProbe.style.width = '100px'; document.body.append(geometryProbe);").unwrap();
    refresh_layout_for_test(&mut vm);
    let width_before = vm
        .eval("geometryProbe.getBoundingClientRect().width")
        .unwrap();
    vm.eval("geometryProbe.style.width = '200px'").unwrap();
    let before = vm.layout_pass_observability_for_test().1;
    assert_eq!(vm.eval(r#"
const canvas = document.createElement('canvas'), ctx = canvas.getContext('2d'); ctx.font = '10.5px CanvasFixture';
ctx.measureText('text'); ctx.fillText('text', 0, 20);
JSON.stringify(['measureText', 'fillText', 'strokeText'].map(method => {
 try { ctx[method].call({}, 'A', 0, 20); return false; } catch (e) { return e instanceof TypeError; }
}))
"#).unwrap(), "[true,true,true]");
    assert_eq!(
        vm.layout_pass_observability_for_test().1,
        before,
        "Canvas shaping must not request a DOM layout pass"
    );
    assert_eq!(
        vm.eval("geometryProbe.getBoundingClientRect().width")
            .unwrap(),
        width_before,
        "Canvas drawing must preserve the deliberately stale synchronous layout snapshot"
    );
}
