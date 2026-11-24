mod support;
use moli_v8_test_util::ensure_v8;
use moli_webapi::{canvas_gradient, image_bitmap, interfaces, text_metrics, webgl};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};
fn with_canvas_values(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_errors(scope);
    for interface in [
        interfaces::WebGLRenderingContext::DESCRIPTOR,
        interfaces::WebGL2RenderingContext::DESCRIPTOR,
        interfaces::WEBGLDebugRendererInfo::DESCRIPTOR,
        interfaces::WEBGLLoseContext::DESCRIPTOR,
        interfaces::WebGLShaderPrecisionFormat::DESCRIPTOR,
        interfaces::ImageBitmap::DESCRIPTOR,
        interfaces::TextMetrics::DESCRIPTOR,
        interfaces::CanvasGradient::DESCRIPTOR,
    ] {
        let template = match interface.name() {
            "WebGLRenderingContext" => v8::FunctionTemplate::new(
                scope,
                webgl::webgl_rendering_context_constructor_callback,
            ),
            "WEBGL_debug_renderer_info" => v8::FunctionTemplate::new(
                scope,
                webgl::webgl_debug_renderer_info_constructor_callback,
            ),
            "WEBGL_lose_context" => {
                v8::FunctionTemplate::new(scope, webgl::webgl_lose_context_constructor_callback)
            }
            _ => v8::FunctionTemplate::new(scope, illegal_constructor),
        };
        webgl::install_template_bindings(scope, template, interface.name());
        match interface.name() {
            "ImageBitmap" => image_bitmap::install_image_bitmap_template_bindings(scope, template),
            "TextMetrics" => text_metrics::install_text_metrics_template_bindings(scope, template),
            "CanvasGradient" => canvas_gradient::install(scope, template),
            _ => {}
        }
        finish_template(scope, interface, template);
    }
    test(scope);
}
#[test]
fn standalone_webgl_state_uses_supplied_dimensions_and_protected_native_factories() {
    with_canvas_values(|scope| {
        let gl = webgl::build_webgl_context_object(scope).unwrap();
        webgl::initialize_viewport(scope, Some((20000, 42)), gl);
        publish(scope, "gl", gl);
        eval(
            scope,
            r#"
        if([...gl.getParameter(0x0BA2)].join()!=='0,0,8192,42') throw 'initial viewport';
        gl.viewport(2,3,4,5); const viewport=gl.getParameter(0x0BA2);viewport[2]=99;
        if([...gl.getParameter(0x0BA2)].join()!=='2,3,4,5') throw 'snapshot';
        gl.viewport(0,0,-1,1);if(gl.getError()!==0x0501 || gl.getError()!==0) throw 'error state';
        const nativeExtension=WEBGL_debug_renderer_info;
        globalThis.WEBGL_debug_renderer_info=function(){throw 'author extension'};
        const extension=gl.getExtension('WEBGL_debug_renderer_info');
        if(!(extension instanceof nativeExtension) || extension.UNMASKED_VENDOR_WEBGL!==0x9245) throw 'extension';
        const precision=gl.getShaderPrecisionFormat(0,0);if(precision.precision!==23 || precision.rangeMin!==127) throw 'precision';
        const buffer=gl.createBuffer();if(Object.prototype.toString.call(buffer)!=='[object WebGLBuffer]') throw 'resource brand';
        "#,
        );
        let gl2 = webgl::build_webgl2_context_object(scope).unwrap();
        publish(scope, "gl2", gl2);
        eval(
            scope,
            "if(gl2.drawingBufferColorSpace!=='srgb') throw 'default color'; gl2.drawingBufferColorSpace='display-p3';if(gl2.drawingBufferColorSpace!=='display-p3') throw 'color state';",
        );
    });
}
#[test]
fn standalone_bitmap_metrics_and_gradient_bindings_need_no_pixel_host() {
    with_canvas_values(|scope| {
        let bitmap = image_bitmap::build_image_bitmap_object(scope, 3, 4).unwrap();
        publish(scope, "bitmap", bitmap);
        let metrics = text_metrics::CanvasTextMetricsDeclaration {
            width: 12.5,
            actual_bounding_box_ascent: 8.0,
            actual_bounding_box_descent: 2.0,
        }
        .bind(scope)
        .unwrap();
        publish(scope, "metrics", metrics);
        let gradient = canvas_gradient::build(scope);
        publish(scope, "gradient", gradient);
        eval(
            scope,
            "if(bitmap.width!==3 || bitmap.height!==4 || metrics.width!==12.5 || metrics.actualBoundingBoxAscent!==8 || metrics.actualBoundingBoxDescent!==2) throw 'value'; bitmap.close();bitmap.close();if(bitmap.width!==0 || bitmap.height!==0) throw 'close'; gradient.addColorStop(0.5,'red'); try{gradient.addColorStop(2,'red');throw 'offset accepted'}catch(e){if(e.name!=='IndexSizeError')throw e;}",
        );
    });
}
