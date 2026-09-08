use super::*;

fn gradient_pixels(script: &str, points: &[(u32, u32)], expected: &[[u8; 4]]) {
    for constructor in [
        "document.createElement('canvas')",
        "new OffscreenCanvas(20,20)",
    ] {
        let mut vm = new_storage_test_vm("https://canvas-gradients.test/");
        let result = vm
            .eval(&format!(
                r#"(() => {{
const canvas = {constructor}; canvas.width = canvas.height = 20;
const ctx = canvas.getContext('2d');
{script}
return JSON.stringify({points}.map(([x,y]) => Array.from(ctx.getImageData(x,y,1,1).data)));
}})()"#,
                points = serde_json::to_string(points).unwrap()
            ))
            .expect("gradient should paint");
        let actual: Vec<[u8; 4]> = serde_json::from_str(&result).unwrap();
        assert_eq!(actual.len(), expected.len());
        for ((actual, expected), point) in actual.iter().zip(expected).zip(points) {
            for (actual, expected) in actual.iter().zip(expected) {
                // Chromium's Skia rasterizer dithers gradient channels; allow
                // two 8-bit levels, not differences in geometry or opacity.
                assert!(
                    actual.abs_diff(*expected) <= 2,
                    "{constructor} pixel {point:?}: {actual} != {expected}; {script}"
                );
            }
        }
    }
}

#[test]
fn canvas_gradients_render_linear_and_radial_pixels() {
    gradient_pixels(
        "const g=ctx.createLinearGradient(0,0,10,0);g.addColorStop(0,'black');g.addColorStop(1,'white');ctx.fillStyle=g;ctx.fillRect(0,0,20,20);",
        &[(0, 10), (2, 10), (7, 10), (12, 10)],
        &[
            [12, 12, 12, 255],
            [63, 63, 63, 255],
            [192, 192, 192, 255],
            [255, 255, 255, 255],
        ],
    );
    gradient_pixels(
        "const g=ctx.createRadialGradient(10,10,0,10,10,10);g.addColorStop(0,'red');g.addColorStop(1,'blue');ctx.fillStyle=g;ctx.fillRect(0,0,20,20);",
        &[(0, 10), (7, 10), (12, 10), (19, 10)],
        &[
            [12, 0, 242, 255],
            [190, 0, 65, 255],
            [190, 0, 65, 255],
            [13, 0, 243, 255],
        ],
    );
}

#[test]
fn canvas_gradients_interpolate_straight_alpha_and_apply_global_alpha() {
    gradient_pixels(
        "const g=ctx.createLinearGradient(0,0,20,0);g.addColorStop(0,'red');g.addColorStop(1,'rgba(0,0,255,0)');ctx.fillStyle=g;ctx.fillRect(0,0,20,20);",
        &[(7, 10), (12, 10)],
        &[[160, 0, 96, 159], [96, 0, 157, 96]],
    );
    gradient_pixels(
        "const g=ctx.createLinearGradient(0,0,20,0);g.addColorStop(0,'red');ctx.fillStyle=g;ctx.globalAlpha=0.5;ctx.fillRect(0,0,20,20);",
        &[(7, 10)],
        &[[255, 0, 0, 128]],
    );
}

#[test]
fn canvas_gradients_preserve_duplicate_stop_order_and_empty_geometry() {
    gradient_pixels(
        "const g=ctx.createLinearGradient(0,0,20,0);g.addColorStop(1,'blue');g.addColorStop(0,'red');g.addColorStop(.5,'red');g.addColorStop(.5,'blue');ctx.fillStyle=g;ctx.fillRect(0,0,20,20);",
        &[(7, 10), (12, 10)],
        &[[255, 0, 0, 255], [0, 0, 255, 255]],
    );
    for constructor in [
        "ctx.createLinearGradient(0,0,20,0)",
        "ctx.createRadialGradient(10,10,0,10,10,10)",
    ] {
        gradient_pixels(
            &format!("ctx.fillStyle={constructor};ctx.fillRect(0,0,20,20);"),
            &[(5, 5)],
            &[[0; 4]],
        );
    }
    for constructor in [
        "ctx.createLinearGradient(0,0,0,0)",
        "ctx.createRadialGradient(10,10,5,10,10,5)",
    ] {
        gradient_pixels(
            &format!(
                "const g={constructor};g.addColorStop(0,'red');g.addColorStop(1,'blue');ctx.fillStyle=g;ctx.fillRect(0,0,20,20);"
            ),
            &[(5, 5)],
            &[[0; 4]],
        );
    }
}

#[test]
fn canvas_gradients_use_the_draw_time_transform_not_creation_transform() {
    for (before, after) in [("ctx.scale(2,1);", ""), ("", "ctx.scale(2,1);")] {
        gradient_pixels(
            &format!(
                "{before} const g=ctx.createLinearGradient(0,0,10,0);{after}g.addColorStop(0,'black');g.addColorStop(1,'white');ctx.fillStyle=g;ctx.fillRect(0,0,20,20);"
            ),
            &[(2, 10), (7, 10), (12, 10), (17, 10)],
            &[
                [31, 31, 31, 255],
                [96, 96, 96, 255],
                [159, 159, 159, 255],
                [224, 224, 224, 255],
            ],
        );
    }
    // A path is already in canvas coordinates when the transform changes.
    gradient_pixels(
        "ctx.rect(0,0,20,20);ctx.scale(2,1);const g=ctx.createLinearGradient(0,0,10,0);g.addColorStop(0,'black');g.addColorStop(1,'white');ctx.fillStyle=g;ctx.fill();",
        &[(2, 10), (17, 10)],
        &[[31, 31, 31, 255], [224, 224, 224, 255]],
    );
}

#[test]
fn canvas_gradients_paint_strokes() {
    for draw in [
        "ctx.strokeRect(0,4,20,12);",
        "ctx.moveTo(0,10);ctx.lineTo(20,10);ctx.stroke();",
    ] {
        gradient_pixels(
            &format!(
                "const g=ctx.createLinearGradient(0,0,20,0);g.addColorStop(0,'red');g.addColorStop(1,'blue');ctx.strokeStyle=g;ctx.lineWidth=16;{draw}"
            ),
            &[(2, 10), (17, 10)],
            &[[223, 0, 31, 255], [32, 0, 223, 255]],
        );
    }
}

#[test]
fn canvas_gradients_paint_text_in_canvas_not_glyph_coordinates() {
    let mut vm = new_storage_test_vm("https://canvas-gradient-text.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
for (const method of ['fillText','strokeText']) {
  const ctx = new OffscreenCanvas(96,48).getContext('2d');
  const g = ctx.createLinearGradient(0,0,10,0);
  g.addColorStop(0,'red'); g.addColorStop(1,'blue');
  ctx.fillStyle=ctx.strokeStyle=g; ctx.font='20px sans-serif';
  ctx[method]('M',20,30);
  const data=ctx.getImageData(0,0,96,48).data;
  let count=0;
  for (let i=0;i<data.length;i+=4) {
    if (!data[i+3]) continue;
    count++;
    if (data[i]!==0 || data[i+1]!==0 || data[i+2]!==255) return method+' wrong brush origin';
  }
  if (!count) return method+' missing pixels';
}
return 'ok';
})()"#
        )
        .unwrap(),
        "ok"
    );
}

#[test]
fn canvas_gradients_share_live_stops_without_coercing_gradient_objects() {
    let mut vm = new_storage_test_vm("https://canvas-gradient-state.test/");
    assert_eq!(vm.eval(r#"(() => {
const canvas=document.createElement('canvas'); canvas.width=canvas.height=20;
const a=canvas.getContext('2d'), b=new OffscreenCanvas(20,20).getContext('2d');
const g=a.createLinearGradient(0,0,20,0);
g.toString=()=>{throw Error('must not stringify gradients')};
a.fillStyle=g; b.fillStyle=g;
let hits=0;
Object.defineProperty(Array.prototype,'0',{set(){hits++},configurable:true});
try { g.addColorStop(0,'red'); } finally { delete Array.prototype[0]; }
a.fillRect(0,0,20,20); b.fillRect(0,0,20,20);
const result=[a.fillStyle===g,b.fillStyle===g,hits,...a.getImageData(5,5,1,1).data,...b.getImageData(5,5,1,1).data];
canvas.width=20;
result.push(a.fillStyle,b.fillStyle===g);
return JSON.stringify(result);
})()"#).unwrap(), r##"[true,true,0,255,0,0,255,255,0,0,255,"#000000",true]"##);
}

#[test]
fn canvas_gradients_remain_live_when_only_the_context_retains_them() {
    let mut vm = new_storage_test_vm("https://canvas-gradient-gc.test/");
    vm.eval(
        r#"globalThis.gradientContext = new OffscreenCanvas(20,20).getContext('2d');
(() => {
  const g = gradientContext.createLinearGradient(0,0,20,0);
  g.addColorStop(0,'red'); gradientContext.fillStyle = g;
})()"#,
    )
    .unwrap();
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .expect("gradient storage should survive a collecting GC");
    assert_eq!(
        vm.eval(
            r#"(() => {
const c=gradientContext;
c.fillStyle.addColorStop(1,'blue'); c.fillRect(0,0,20,20);
const left=c.getImageData(1,10,1,1).data, right=c.getImageData(18,10,1,1).data;
return left[0]>left[2] && right[2]>right[0] && left[3]===255 && right[3]===255;
})()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn canvas_gradients_validate_native_receivers_coordinates_radii_and_colors() {
    let mut vm = new_storage_test_vm("https://canvas-gradient-validation.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
const c=new OffscreenCanvas(20,20).getContext('2d');
const g=c.createLinearGradient(0,0,10,0);
const error=fn=>{try{fn();return 'ok'}catch(e){return e.name}};
return [c.createRadialGradient.length,
  error(()=>c.createLinearGradient(0,0,Infinity,0)),
  error(()=>c.createRadialGradient(0,0,0,0,0,Infinity)),
  error(()=>c.createRadialGradient(0,0,-1,0,0,10)),
  error(()=>g.addColorStop(0,'not-a-color')),
  error(()=>g.addColorStop(NaN,'red')),
  error(()=>g.addColorStop(2,'red')),
  error(()=>g.addColorStop.call(Object.create(CanvasGradient.prototype),0,'red')),
  error(()=>g.addColorStop.call(new Proxy(g,{}),0,'red')),
  error(()=>c.createRadialGradient.call({},0,0,0,0,0,10)),
  c.createRadialGradient(0,0,0,0,0,10) instanceof CanvasGradient
].join('|');
})()"#
        )
        .unwrap(),
        "6|TypeError|TypeError|IndexSizeError|SyntaxError|TypeError|IndexSizeError|TypeError|TypeError|TypeError|true"
    );
}
