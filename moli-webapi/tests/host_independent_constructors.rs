mod support;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{get_private_value, set_private_value};
use moli_webapi::{
    animation, canvas_context_values as canvas, constructors, dom_parser_values,
    event_target::{self, EventTargetRuntime},
    idle_detector_values as idle, interfaces, media_cue_values as cues,
};
use std::pin::pin;
use support::{eval, finish_template, install_errors, publish};

fn initialize_target(scope: &mut v8::PinScope<'_, '_>, object: v8::Local<'_, v8::Object>) {
    let registry = moli_v8_util::new_null_prototype_object(scope);
    set_private_value(
        scope,
        object,
        event_target::SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
        registry.into(),
    );
}

fn set_handler(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    _: &str,
    slot: &str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, object, slot, value);
}

fn with_values(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_errors(scope);
    event_target::install_runtime(
        scope,
        EventTargetRuntime {
            initialize: initialize_target,
            set_handler,
        },
    );
    macro_rules! install {
        ($interface:ident, $callback:path) => {{
            let template = v8::FunctionTemplate::new(
                scope,
                moli_webapi_declare::web_api_constructor!(interfaces::$interface, $callback),
            );
            animation::install_value_template_bindings(scope, template, stringify!($interface));
            finish_template(scope, interfaces::$interface::DESCRIPTOR, template);
        }};
    }
    install!(Animation, animation::animation_constructor_callback);
    install!(
        KeyframeEffect,
        animation::keyframe_effect_constructor_callback
    );
    install!(
        DOMParser,
        dom_parser_values::dom_parser_constructor_callback
    );
    install!(
        XMLSerializer,
        constructors::xml_serializer_constructor_callback
    );
    install!(
        FileReaderSync,
        constructors::file_reader_sync_constructor_callback
    );
    install!(IdleDetector, idle::idle_detector_constructor_callback);
    install!(VTTCue, cues::vtt_cue_constructor_callback);
    install!(
        OffscreenCanvas,
        canvas::offscreen_canvas_constructor_callback
    );
    install!(
        CanvasRenderingContext2D,
        canvas::canvas_rendering_context_2d_constructor_callback
    );
    install!(
        OffscreenCanvasRenderingContext2D,
        canvas::offscreen_canvas_rendering_context_2d_constructor_callback
    );
    test(scope);
}

#[test]
fn animation_values_construct_with_intrinsic_promises_without_dom_or_dispatch() {
    with_values(|scope| {
        assert!(
            eval(
                scope,
                r#"(() => {
          const check = ok => { if (!ok) throw new Error('animation value'); };
          const target = {}, keyframes = [{opacity:0}, {opacity:1}], timeline = {};
          const effect = new KeyframeEffect(target, keyframes);
          const animation = new Animation(effect, timeline);
          check(animation.id === '' && animation.playState === 'idle' && !animation.pending);
          check(animation.ready instanceof Promise && animation.finished instanceof Promise);
          check(animation.timeline === timeline);
          animation.id = 'retained'; animation.timeline = null;
          check(animation.id === 'retained' && animation.timeline === null);
          const handler = () => {};
          animation.onfinish = handler; check(animation.onfinish === handler);
          effect.setKeyframes([{opacity:0.5}]);
          globalThis.savedAnimation = animation;
          return true;
        })()"#
            )
            .is_true()
        );
        let animation = v8::Local::<v8::Object>::try_from(eval(scope, "savedAnimation")).unwrap();
        let effect = get_private_value(scope, animation, animation::ANIMATION_EFFECT_SLOT).unwrap();
        publish(
            scope,
            "savedEffect",
            v8::Local::<v8::Object>::try_from(effect).unwrap(),
        );
        assert!(eval(scope, "savedEffect instanceof KeyframeEffect").is_true());
    });
}

#[test]
fn independent_constructor_values_keep_native_identity_and_creation_realm() {
    with_values(|scope| {
        let parser = v8::Local::<v8::Object>::try_from(eval(scope, "new DOMParser()")).unwrap();
        let witness = get_private_value(
            scope,
            parser,
            dom_parser_values::DOM_PARSER_CONSTRUCTION_REALM_SLOT,
        )
        .unwrap();
        let witness = v8::Local::<v8::Object>::try_from(witness).unwrap();
        assert!(witness.get_creation_context(scope).unwrap() == scope.get_current_context());
        assert!(eval(scope, "Object.prototype.toString.call(new XMLSerializer()) === '[object XMLSerializer]' && Object.prototype.toString.call(new FileReaderSync()) === '[object FileReaderSync]'").is_true());
        let detector =
            v8::Local::<v8::Object>::try_from(eval(scope, "new IdleDetector()")).unwrap();
        assert!(
            get_private_value(scope, detector, idle::IDLE_DETECTOR_STARTED_SLOT)
                .unwrap()
                .is_false()
        );
        let cue =
            v8::Local::<v8::Object>::try_from(eval(scope, "new VTTCue(1, 3, 'text')")).unwrap();
        assert_eq!(
            get_private_value(scope, cue, cues::TEXT_TRACK_CUE_START_TIME_SLOT)
                .unwrap()
                .number_value(scope),
            Some(1.0)
        );
        assert_eq!(
            get_private_value(scope, cue, cues::TEXT_TRACK_CUE_TEXT_SLOT)
                .unwrap()
                .to_rust_string_lossy(scope),
            "text"
        );
    });
}

#[test]
fn canvas_construction_converts_dimensions_once_and_allocates_initial_pixels_without_dom() {
    with_values(|scope| {
        let object = v8::Local::<v8::Object>::try_from(eval(scope, r#"(() => {
          const order = [];
          const canvas = new OffscreenCanvas({valueOf(){order.push('width');return 2;}}, {valueOf(){order.push('height');return 3;}});
          if (order.join() !== 'width,height') throw new Error('conversion order');
          return canvas;
        })()"#)).unwrap();
        assert_eq!(
            get_private_value(scope, object, canvas::OFFSCREEN_CANVAS_WIDTH_SLOT)
                .unwrap()
                .number_value(scope),
            Some(2.0)
        );
        let pixels = v8::Local::<v8::Uint8ClampedArray>::try_from(
            get_private_value(scope, object, canvas::CANVAS_BACKING_STORE_SLOT).unwrap(),
        )
        .unwrap();
        assert_eq!(pixels.byte_length(), 24);
        let mut bytes = [1; 24];
        pixels.copy_contents(&mut bytes);
        assert_eq!(bytes, [0; 24]);
        for source in [
            "new CanvasRenderingContext2D()",
            "new OffscreenCanvasRenderingContext2D()",
        ] {
            let context = v8::Local::<v8::Object>::try_from(eval(scope, source)).unwrap();
            assert_eq!(
                get_private_value(scope, context, canvas::CANVAS_CONTEXT_FILL_STYLE_SLOT)
                    .unwrap()
                    .to_rust_string_lossy(scope),
                moli_canvas::DEFAULT_FILL_STYLE
            );
        }
    });
}
