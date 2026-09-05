use super::{canvas_paths::canvas_pixels, *};

fn check(script: &str, expected: &str) {
    for constructor in [
        "document.createElement('canvas')",
        "new OffscreenCanvas(96,96)",
    ] {
        let mut vm = new_storage_test_vm("https://canvas-arguments.test/");
        let result = vm
            .eval(&format!(
                r#"(() => {{
          const canvas={constructor};canvas.width=canvas.height=96;
          const ctx=canvas.getContext('2d');
          const caught=f=>{{try{{f();return 'ok';}}catch(e){{return e.name;}}}};
          {script}
        }})()"#
            ))
            .unwrap();
        assert_eq!(result, expected, "{constructor}: {script}");
    }
}

#[test]
fn canvas_line_width_and_miter_limit_ignore_nonpositive_or_nonfinite_values() {
    check(
        r#"
      ctx.lineWidth=8;ctx.miterLimit=7;
      for(const n of [0,-0,-1,NaN,Infinity,-Infinity]) {
        ctx.lineWidth=n;ctx.miterLimit=n;
        if(ctx.lineWidth!==8||ctx.miterLimit!==7)throw Error('invalid setter changed state');
      }
      ctx.lineWidth={valueOf(){return 0;}};ctx.miterLimit='0';
      return JSON.stringify([ctx.lineWidth,ctx.miterLimit]);
    "#,
        "[8,7]",
    );
}

#[test]
fn canvas_transform_dictionary_and_dommatrix_overloads_record_the_same_geometry() {
    for init in [
        "{e:20}",
        "{m41:20}",
        "{a:1,m11:1,e:20,m41:20}",
        "new DOMMatrix([1,0,0,1,20,0])",
    ] {
        canvas_pixels(
            &format!("ctx.setTransform({init});ctx.rect(0,0,10,10);ctx.fill();"),
            &[(5, 5), (25, 5)],
            &[0, 255],
        );
    }
}

#[test]
fn canvas_set_transform_default_dictionary_resets_to_identity() {
    for init in ["", "undefined", "null", "{}"] {
        canvas_pixels(
            &format!(
                "ctx.translate(20,0);ctx.setTransform({init});ctx.rect(0,0,10,10);ctx.fill();"
            ),
            &[(5, 5), (25, 5)],
            &[255, 0],
        );
    }
    check(
        "return JSON.stringify([ctx.setTransform.length,ctx.transform.length]);",
        "[0,6]",
    );
}

#[test]
fn canvas_transform_dictionary_rejects_each_inconsistent_alias_atomically() {
    check(
        r#"
      ctx.translate(20,0);
      const results=[];
      for(const [a,b] of [['a','m11'],['b','m12'],['c','m21'],['d','m22'],['e','m41'],['f','m42']]) {
        results.push(caught(()=>ctx.setTransform({[a]:1,[b]:2})));
      }
      ctx.rect(0,0,10,10);ctx.fill();
      results.push(ctx.getImageData(25,5,1,1).data[3],ctx.getImageData(5,5,1,1).data[3]);
      return JSON.stringify(results);
    "#,
        r#"["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError",255,0]"#,
    );
}

#[test]
fn canvas_transform_dictionary_reads_members_in_webidl_order() {
    check(
        r#"
      const names=['a','b','c','d','e','f','m11','m12','m21','m22','m41','m42'];
      const order=[],init={};
      for(const name of names)Object.defineProperty(init,name,{get(){order.push(name);return undefined;}});
      ctx.setTransform(init);
      return JSON.stringify(order);
    "#,
        r#"["a","b","c","d","e","f","m11","m12","m21","m22","m41","m42"]"#,
    );
}

#[test]
fn canvas_transform_dictionary_getter_exception_preserves_current_transform() {
    check(
        r#"
      ctx.translate(20,0);
      let message;
      try{ctx.setTransform({a:2,get e(){throw Error('dictionary failed');}});}catch(e){message=e.message;}
      ctx.rect(0,0,10,10);ctx.fill();
      return JSON.stringify([message,ctx.getImageData(25,5,1,1).data[3],ctx.getImageData(5,5,1,1).data[3]]);
    "#,
        r#"["dictionary failed",255,0]"#,
    );
}

#[test]
fn canvas_transform_nonfinite_values_are_ignored_after_alias_validation() {
    check(
        r#"
      ctx.translate(20,0);
      const results=[caught(()=>ctx.setTransform({a:NaN,m11:NaN})),caught(()=>ctx.setTransform({b:0,m12:-0}))];
      ctx.translate(20,0);
      ctx.setTransform(Infinity,0,0,1,0,0);ctx.transform(1,0,0,NaN,0,0);
      ctx.rotate(NaN);ctx.scale(Infinity,1);ctx.translate(0,Infinity);
      ctx.rect(0,0,10,10);ctx.fill();
      results.push(ctx.getImageData(25,5,1,1).data[3],ctx.getImageData(5,5,1,1).data[3]);
      return JSON.stringify(results);
    "#,
        r#"["ok","ok",255,0]"#,
    );
}

#[test]
fn canvas_transform_overloads_reject_invalid_arity_and_primitives() {
    check(
        r#"
      return JSON.stringify([
        caught(()=>ctx.setTransform(1)),caught(()=>ctx.setTransform('matrix(1,0,0,1,0,0)')),
        caught(()=>ctx.setTransform(1,0)),caught(()=>ctx.setTransform(1,0,0,1,0)),
        caught(()=>ctx.transform()),caught(()=>ctx.setTransform(1,0,0,1,0,0))
      ]);
    "#,
        r#"["TypeError","TypeError","TypeError","TypeError","TypeError","ok"]"#,
    );
}

#[test]
fn canvas_arc_argument_validation_ignores_nonfinite_before_negative_radius() {
    check(
        r#"
      return JSON.stringify([
        caught(()=>ctx.arc(NaN,0,-1,0,1)),caught(()=>ctx.arc(0,0,-1,0,1)),
        caught(()=>ctx.ellipse(0,0,-1,2,Infinity,0,1)),caught(()=>ctx.ellipse(0,0,-1,2,0,0,1)),
        caught(()=>ctx.arcTo(0,0,NaN,1,-1)),caught(()=>ctx.arcTo(0,0,1,1,-1))
      ]);
    "#,
        r#"["ok","IndexSizeError","ok","IndexSizeError","ok","IndexSizeError"]"#,
    );
}

#[test]
fn canvas_hex_alpha_is_accepted_and_preserved_for_fill_and_stroke() {
    for color in ["#0f08", "#00ff0088", "#0F08", "rgba(0,255,0,0.533)"] {
        check(
            &format!(
                r#"
          ctx.fillStyle='{color}';ctx.strokeStyle='{color}';
          ctx.rect(0,0,10,10);ctx.fill();ctx.beginPath();
          ctx.lineWidth=2;ctx.moveTo(20,20);ctx.lineTo(40,20);ctx.stroke();
          return JSON.stringify([Array.from(ctx.getImageData(5,5,1,1).data),Array.from(ctx.getImageData(25,20,1,1).data)]);
        "#
            ),
            "[[0,255,0,136],[0,255,0,136]]",
        );
    }
}
