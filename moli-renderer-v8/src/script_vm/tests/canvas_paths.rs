use super::*;

fn canvas_pixels(script: &str, points: &[(u32, u32)], expected: &[u8]) {
    for kind in ["html", "offscreen"] {
        let mut vm = new_storage_test_vm("https://canvas-paths.test/");
        let result = vm
            .eval(&format!(
                r#"(() => {{
                  const canvas = {constructor};
                  canvas.width = canvas.height = 96;
                  const ctx = canvas.getContext('2d');
                  ctx.fillStyle = ctx.strokeStyle = 'red';
                  ctx.lineWidth = 2;
                  {script}
                  return JSON.stringify({points}.map(([x,y]) => ctx.getImageData(x,y,1,1).data[3]));
                }})()"#,
                constructor = if kind == "html" {
                    "document.createElement('canvas')"
                } else {
                    "new OffscreenCanvas(96,96)"
                },
                points = serde_json::to_string(points).unwrap(),
            ))
            .expect("canvas script should execute");
        assert_eq!(
            result,
            serde_json::to_string(expected).unwrap(),
            "{kind}: {script}"
        );
    }
}

#[test]
fn canvas_arc_quadrants_match_screen_coordinate_direction() {
    // Pixel probes exclude the antialiased boundary; Chromium 145 gives these
    // same results. Checking only the center misses malformed circular paths.
    let points = [(40, 40), (24, 40), (24, 24), (40, 24)];
    canvas_pixels(
        "ctx.moveTo(32,32);ctx.arc(32,32,20,0,Math.PI/2);ctx.closePath();ctx.fill();",
        &points,
        &[255, 0, 0, 0],
    );
    canvas_pixels(
        "ctx.moveTo(32,32);ctx.arc(32,32,20,0,-Math.PI/2,true);ctx.closePath();ctx.fill();",
        &points,
        &[0, 0, 0, 255],
    );
    canvas_pixels(
        "ctx.moveTo(32,32);ctx.arc(32,32,20,0,Math.PI/2,true);ctx.closePath();ctx.fill();",
        &points,
        &[0, 255, 255, 255],
    );
}

#[test]
fn canvas_full_circle_matches_a_disk_away_from_its_boundary() {
    let mut points = Vec::new();
    let mut expected = Vec::new();
    for y in (5..60).step_by(2) {
        for x in (5..60).step_by(2) {
            let distance = (f64::from(x) + 0.5 - 32.0).hypot(f64::from(y) + 0.5 - 32.0);
            if (distance - 20.0).abs() > 2.0 {
                points.push((x, y));
                expected.push(if distance < 20.0 { 255 } else { 0 });
            }
        }
    }
    for arc in [
        "0,2*Math.PI",
        "0,-2*Math.PI,true",
        "0,4*Math.PI",
        "1,1+2*Math.PI",
    ] {
        canvas_pixels(
            &format!("ctx.arc(32,32,20,{arc});ctx.fill();"),
            &points,
            &expected,
        );
    }
}

#[test]
fn canvas_ellipse_preserves_center_and_rotation() {
    canvas_pixels(
        "ctx.ellipse(32,32,20,10,0,0,2*Math.PI);ctx.fill();",
        &[(48, 32), (32, 39), (32, 45), (32, 32)],
        &[255, 255, 0, 255],
    );
    canvas_pixels(
        "ctx.ellipse(32,32,20,10,Math.PI/2,0,2*Math.PI);ctx.fill();",
        &[(32, 48), (39, 32), (45, 32), (32, 32)],
        &[255, 255, 0, 255],
    );
}

#[test]
fn canvas_arc_to_connects_to_the_first_tangent() {
    canvas_pixels(
        "ctx.moveTo(0,20);ctx.arcTo(40,20,40,60,15);ctx.stroke();",
        &[(10, 20), (25, 20), (39, 34), (48, 20), (39, 50)],
        &[255, 255, 255, 0, 0],
    );
}

#[test]
fn canvas_arc_to_does_not_clamp_radius_to_segment_lengths() {
    canvas_pixels(
        "ctx.moveTo(10,20);ctx.arcTo(40,20,40,30,25);ctx.stroke();",
        &[(14, 20), (39, 44), (39, 30), (39, 25)],
        &[255, 255, 0, 0],
    );
}

#[test]
fn canvas_empty_path_commands_establish_the_required_start_point() {
    for command in [
        "ctx.lineTo(10,10);",
        "ctx.quadraticCurveTo(10,10,10,10);",
        "ctx.bezierCurveTo(10,10,10,10,10,10);",
        "ctx.arcTo(10,10,20,20,5);",
    ] {
        canvas_pixels(
            &format!("{command}ctx.lineTo(30,10);ctx.lineTo(30,30);ctx.closePath();ctx.fill();"),
            &[(25, 15), (15, 25)],
            &[255, 0],
        );
    }
}

#[test]
fn canvas_zero_radius_arc_keeps_its_connecting_line() {
    canvas_pixels(
        "ctx.moveTo(5,20);ctx.arc(30,20,0,0,1);ctx.stroke();",
        &[(15, 20), (35, 20)],
        &[255, 0],
    );
}

#[test]
fn canvas_arc_to_degenerate_segments_remain_lines() {
    for arguments in ["30,20,50,20,8", "30,20,30,20,8", "30,20,30,40,0"] {
        canvas_pixels(
            &format!("ctx.moveTo(5,20);ctx.arcTo({arguments});ctx.stroke();"),
            &[(15, 20), (35, 20)],
            &[255, 0],
        );
    }
}

#[test]
fn canvas_resize_discards_the_current_path_even_when_size_is_unchanged() {
    for assignment in [
        "canvas.width=canvas.width",
        "canvas.height=canvas.height",
        "canvas.width=64",
        "canvas.height=0;canvas.height=96",
    ] {
        canvas_pixels(
            &format!("ctx.rect(0,0,10,10);{assignment};ctx.fill();"),
            &[(5, 5)],
            &[0],
        );
    }
}

#[test]
fn canvas_dimension_attribute_mutation_resets_path_and_transform() {
    let mut vm = new_storage_test_vm("https://canvas-reset.test/");
    let result = vm.eval(r#"(() => {
      const canvas = document.createElement('canvas');
      canvas.width=96;canvas.height=96;
      const ctx=canvas.getContext('2d');
      ctx.translate(30,0);ctx.rect(0,0,10,10);
      canvas.setAttribute('width','96');
      ctx.fill();
      const old=ctx.getImageData(35,5,1,1).data[3];
      ctx.rect(0,0,10,10);ctx.fill();
      return JSON.stringify([old,ctx.getImageData(5,5,1,1).data[3],ctx.getImageData(35,5,1,1).data[3]]);
    })()"#).unwrap();
    assert_eq!(result, "[0,255,0]");
}

#[test]
fn canvas_resize_resets_all_implemented_drawing_state_without_replacing_context() {
    for constructor in [
        "document.createElement('canvas')",
        "new OffscreenCanvas(96,96)",
    ] {
        let mut vm = new_storage_test_vm("https://canvas-state.test/");
        let result=vm.eval(&format!(r#"(() => {{
          const canvas={constructor},ctx=canvas.getContext('2d');
          ctx.fillStyle='red';ctx.strokeStyle='blue';ctx.font='30px serif';
          ctx.lineWidth=8;ctx.miterLimit=4;ctx.lineCap='round';ctx.lineJoin='bevel';
          ctx.lineDashOffset=3;ctx.setLineDash([2,3]);ctx.globalAlpha=.5;
          ctx.globalCompositeOperation='copy';ctx.imageSmoothingEnabled=false;
          ctx.imageSmoothingQuality='high';ctx.translate(20,0);
          canvas.height=canvas.height;
          return JSON.stringify([ctx===canvas.getContext('2d'),ctx.fillStyle,ctx.strokeStyle,ctx.font,
            ctx.lineWidth,ctx.miterLimit,ctx.lineCap,ctx.lineJoin,ctx.lineDashOffset,ctx.getLineDash(),
            ctx.globalAlpha,ctx.globalCompositeOperation,ctx.imageSmoothingEnabled,ctx.imageSmoothingQuality]);
        }})()"#)).unwrap();
        assert_eq!(
            result,
            r##"[true,"#000000","#000000","10px sans-serif",1,10,"butt","miter",0,[],1,"source-over",true,"low"]"##
        );
    }
}

#[test]
fn canvas_path_state_is_independent_between_contexts_and_after_reset() {
    let mut vm = new_storage_test_vm("https://canvas-independent.test/");
    assert_eq!(vm.eval(r#"(() => {
      const a=new OffscreenCanvas(20,20), b=new OffscreenCanvas(20,20);
      const x=a.getContext('2d'),y=b.getContext('2d');
      x.rect(0,0,10,10);y.rect(10,10,10,10);a.width=20;
      x.fill();y.fill();
      return JSON.stringify([x.getImageData(5,5,1,1).data[3],y.getImageData(15,15,1,1).data[3],y.getImageData(5,5,1,1).data[3]]);
    })()"#).unwrap(),"[0,255,0]");
}
