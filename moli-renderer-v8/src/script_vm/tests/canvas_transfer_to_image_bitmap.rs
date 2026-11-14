use super::*;

// Expected values were measured in Chrome 157 (headless).

fn run(script: &str) -> String {
    let mut vm = new_storage_test_vm("https://canvas-transfer-to-image-bitmap.test/");
    vm.eval(&format!(
        r#"(() => {{
          const caught = f => {{ try {{ return f(); }} catch (e) {{ return e.name + ': ' + e.message; }} }};
          {script}
        }})()"#
    ))
    .unwrap()
}

#[test]
fn transfer_to_image_bitmap_is_an_enumerable_prototype_method_of_length_zero() {
    let result = run(r#"
      const d = Object.getOwnPropertyDescriptor(OffscreenCanvas.prototype, 'transferToImageBitmap');
      return JSON.stringify([typeof d.value, d.value.length, d.enumerable, d.writable, d.configurable]);
    "#);
    assert_eq!(result, r#"["function",0,true,true,true]"#);
}

#[test]
fn transfer_to_image_bitmap_without_a_context_throws_invalid_state_error() {
    let result = run("return caught(() => new OffscreenCanvas(4, 3).transferToImageBitmap());");
    assert_eq!(
        result,
        "InvalidStateError: Failed to execute 'transferToImageBitmap' on 'OffscreenCanvas': \
         Cannot transfer an ImageBitmap from an OffscreenCanvas with no context"
    );
}

#[test]
fn transfer_to_image_bitmap_returns_an_image_bitmap_with_the_canvas_size() {
    let result = run(r#"
      const canvas = new OffscreenCanvas(4, 3);
      canvas.getContext('2d');
      const bitmap = canvas.transferToImageBitmap(1, 2);
      return JSON.stringify([bitmap instanceof ImageBitmap, bitmap.width, bitmap.height,
        Object.prototype.toString.call(bitmap)]);
    "#);
    assert_eq!(result, r#"[true,4,3,"[object ImageBitmap]"]"#);
}

#[test]
fn transfer_to_image_bitmap_works_for_a_webgl_context() {
    let result = run(r#"
      const canvas = new OffscreenCanvas(5, 4);
      canvas.getContext('webgl');
      const bitmap = canvas.transferToImageBitmap();
      return JSON.stringify([bitmap.width, bitmap.height]);
    "#);
    assert_eq!(result, "[5,4]");
}

#[test]
fn transfer_to_image_bitmap_throws_unknown_error_for_a_zero_sized_canvas() {
    for (width, height) in [(0, 0), (0, 5), (5, 0)] {
        let result = run(&format!(
            "const canvas = new OffscreenCanvas({width}, {height}); canvas.getContext('2d'); \
             return caught(() => canvas.transferToImageBitmap());"
        ));
        assert_eq!(
            result,
            "UnknownError: Failed to execute 'transferToImageBitmap' on 'OffscreenCanvas': \
             ImageBitmap construction failed",
            "{width}x{height}"
        );
    }
}

#[test]
fn transfer_to_image_bitmap_clears_the_bitmap_and_keeps_context_state_and_size() {
    let result = run(r#"
      const canvas = new OffscreenCanvas(4, 4);
      const ctx = canvas.getContext('2d');
      ctx.fillStyle = '#0f0';
      ctx.lineWidth = 7;
      ctx.fillRect(0, 0, 4, 4);
      const before = ctx.getImageData(1, 1, 1, 1).data[3];
      canvas.transferToImageBitmap();
      const after = ctx.getImageData(1, 1, 1, 1).data[3];
      return JSON.stringify([before, after, ctx.fillStyle, ctx.lineWidth,
        canvas.getContext('2d') === ctx, canvas.width, canvas.height]);
    "#);
    assert_eq!(result, r##"[255,0,"#00ff00",7,true,4,4]"##);
}

#[test]
fn transfer_to_image_bitmap_returns_a_new_bitmap_each_time_and_the_canvas_stays_usable() {
    let result = run(r#"
      const canvas = new OffscreenCanvas(4, 4);
      const ctx = canvas.getContext('2d');
      ctx.fillRect(0, 0, 4, 4);
      const first = canvas.transferToImageBitmap();
      const second = canvas.transferToImageBitmap();
      ctx.fillRect(0, 0, 4, 4);
      return JSON.stringify([first !== second, second.width, second.height,
        ctx.getImageData(1, 1, 1, 1).data[3]]);
    "#);
    assert_eq!(result, "[true,4,4,255]");
}

#[test]
fn transferred_image_bitmap_can_be_closed() {
    let result = run(r#"
      const canvas = new OffscreenCanvas(4, 4);
      canvas.getContext('2d');
      const bitmap = canvas.transferToImageBitmap();
      bitmap.close();
      return JSON.stringify([bitmap.width, bitmap.height, canvas.width, canvas.height]);
    "#);
    assert_eq!(result, "[0,0,4,4]");
}
