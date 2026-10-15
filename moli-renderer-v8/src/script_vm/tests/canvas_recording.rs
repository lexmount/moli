use super::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn global_canvas<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    let key = crate::util::v8str(scope, "canvas");
    v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap()
}

fn native_surface(
    vm: &mut StandaloneScriptVmHarness,
) -> Rc<RefCell<Option<moli_canvas::CanvasSurface>>> {
    vm.with_default_context_scope(|scope, _| {
        let canvas = global_canvas(scope);
        Ok(crate::context_bootstrap::canvas_surface_for_test(
            scope, canvas,
        ))
    })
    .unwrap()
}

fn published_image(vm: &mut StandaloneScriptVmHarness) -> Option<Arc<moli_image::RgbaImage>> {
    vm.with_default_context_scope(|scope, _| {
        let canvas = global_canvas(scope);
        let (host, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object(scope, canvas).unwrap();
        Ok(unsafe { &*host }.canvas_pixels_for_layout(handle))
    })
    .unwrap()
}

fn publish_canvases(vm: &mut StandaloneScriptVmHarness) {
    vm.with_default_context_scope(|scope, _| {
        crate::context_bootstrap::flush_all_recordings(scope);
        Ok(())
    })
    .unwrap();
}

#[test]
fn canvas_region_readback_flushes_without_publishing_full_surface() {
    for constructor in [
        "document.createElement('canvas')",
        "new OffscreenCanvas(2048,2048)",
    ] {
        let mut vm = new_storage_test_vm("https://canvas-region-readback.test/");
        vm.eval(&format!(
            "globalThis.canvas={constructor}; canvas.width=canvas.height=2048;\
             globalThis.ctx=canvas.getContext('2d');\
             ctx.fillStyle='red'; ctx.fillRect(2047,2047,1,1);"
        ))
        .unwrap();
        let cell = native_surface(&mut vm);
        assert_eq!(
            cell.borrow()
                .as_ref()
                .map_or(0, |surface| surface.flush_count()),
            0,
            "drawing should remain recorded"
        );
        for _ in 0..3 {
            assert_eq!(
                vm.eval("JSON.stringify(Array.from(ctx.getImageData(2047,2047,1,1).data))")
                    .unwrap(),
                "[255,0,0,255]"
            );
            let surface = cell.borrow();
            let surface = surface.as_ref().unwrap();
            assert_eq!(surface.flush_count(), 1);
            assert_eq!(
                surface.snapshot_count(),
                0,
                "a 1x1 read must not convert 16 MiB"
            );
        }
        if constructor.starts_with("document") {
            assert!(published_image(&mut vm).is_none());
            publish_canvases(&mut vm);
            let image =
                published_image(&mut vm).expect("page painting must see the region-read draw");
            assert_eq!((image.width, image.height), (2048, 2048));
            assert_eq!(&image.rgba[image.rgba.len() - 4..], &[255, 0, 0, 255]);
            assert_eq!(cell.borrow().as_ref().unwrap().snapshot_count(), 1);

            let generation = vm.visual_resource_generation_handle_for_test();
            let before = generation.current();
            publish_canvases(&mut vm);
            assert_eq!(
                generation.current(),
                before,
                "clean publication must be reused"
            );
            assert_eq!(cell.borrow().as_ref().unwrap().snapshot_count(), 1);
        }
    }
}

#[test]
fn canvas_flush_and_readback_use_native_dimensions_without_js_reentry() {
    for constructor in [
        "document.createElement('canvas')",
        "new OffscreenCanvas(2,2)",
    ] {
        for override_prototype in [false, true] {
            let mut vm = new_storage_test_vm("https://canvas-native-dimensions.test/");
            vm.eval(&format!(
                r#"globalThis.canvas={constructor}; canvas.width=canvas.height=2;
                globalThis.ctx=canvas.getContext('2d'); globalThis.dimensionReads=0;
                const target={override_prototype}?Object.getPrototypeOf(canvas):canvas;
                for (const name of ['width','height']) Object.defineProperty(target,name,{{
                    configurable:true,
                    get() {{
                        dimensionReads++;
                        if (dimensionReads===1) ctx.fillRect(0,0,1,1);
                        return 1;
                    }}
                }});
                ctx.fillStyle='red'; ctx.fillRect(1,1,1,1);"#
            ))
            .unwrap();
            publish_canvases(&mut vm);
            assert_eq!(vm.eval("dimensionReads").unwrap(), "0");
            assert_eq!(
                vm.eval("JSON.stringify(Array.from(ctx.getImageData(1,1,1,1).data))")
                    .unwrap(),
                "[255,0,0,255]"
            );
            assert_eq!(
                vm.eval(
                    "ctx.fillStyle='blue'; ctx.fillRect(1,1,1,1);\
                     JSON.stringify(Array.from(ctx.getImageData(1,1,1,1).data))"
                )
                .unwrap(),
                "[0,0,255,255]"
            );
            vm.eval(
                "if (canvas instanceof HTMLCanvasElement) canvas.toDataURL();\
                 else canvas.convertToBlob();",
            )
            .unwrap();
            assert_eq!(vm.eval("dimensionReads").unwrap(), "0");
        }
    }
}

#[test]
fn canvas_first_draw_invalidates_before_any_flush_or_readback() {
    let mut vm = new_storage_test_vm("https://canvas-record-time-invalidation.test/");
    vm.eval(
        "globalThis.canvas=document.createElement('canvas');\
         globalThis.ctx=canvas.getContext('2d');",
    )
    .unwrap();
    let generation = vm.visual_resource_generation_handle_for_test();
    let before = generation.current();
    vm.eval("ctx.fillRect(0,0,1,1)").unwrap();
    let after = generation.current();
    assert!(
        after > before,
        "the first recorded draw must invalidate visual state"
    );
    assert!(
        native_surface(&mut vm).borrow().is_none(),
        "the first draw must not flush"
    );
    assert!(
        published_image(&mut vm).is_none(),
        "invalidation must not require a snapshot"
    );

    vm.eval("ctx.beginPath(); ctx.moveTo(0,0); ctx.lineTo(1,1); ctx.rect(2,2,1,1)")
        .unwrap();
    assert_eq!(
        generation.current(),
        after,
        "geometry alone must not invalidate pixels"
    );
}

#[test]
fn canvas_put_image_data_records_clipped_dimensions_and_destination_offsets() {
    for constructor in [
        "document.createElement('canvas')",
        "new OffscreenCanvas(4,4)",
    ] {
        let mut vm = new_storage_test_vm("https://canvas-recording-dirty-rect.test/");
        vm.eval(&format!(
            "globalThis.canvas={constructor}; canvas.width=canvas.height=4;\
             globalThis.ctx=canvas.getContext('2d');\
             globalThis.red=new ImageData(new Uint8ClampedArray([255,0,0,255]),1,1);"
        ))
        .unwrap();
        for dirty_rect in ["0,0,2,1", "0,0,1,2", "-1,-1,2,2", "1,1,-2,-2"] {
            assert_eq!(
                vm.eval(&format!(
                    "ctx.clearRect(0,0,4,4); ctx.putImageData(red,0,0,{dirty_rect});\
                     JSON.stringify(Array.from(ctx.getImageData(0,0,2,1).data))"
                ))
                .unwrap(),
                "[255,0,0,255,0,0,0,0]",
                "{constructor}: {dirty_rect}"
            );
        }
        assert_eq!(
            vm.eval(
                "ctx.clearRect(0,0,4,4);\
                 ctx.putImageData(new ImageData(new Uint8ClampedArray([255,0,0,255,0,255,0,255]),2,1),\
                                 1,2,1,0,10,10);\
                 JSON.stringify(Array.from(ctx.getImageData(1,2,2,1).data))"
            )
            .unwrap(),
            "[0,0,0,0,0,255,0,255]"
        );
    }
}
