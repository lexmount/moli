use std::pin::pin;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::v8str;
use moli_webapi::{dom_rect, interfaces};

fn install_rectangles(scope: &mut v8::PinScope<'_, '_>) {
    let readonly = dom_rect::build_dom_rect_readonly_constructor_template(scope);
    let mutable = dom_rect::build_dom_rect_constructor_template(scope);
    mutable.inherit(readonly);
    let global = scope.get_current_context().global(scope);
    for (interface, template) in [
        (interfaces::DOMRectReadOnly::DESCRIPTOR, readonly),
        (interfaces::DOMRect::DESCRIPTOR, mutable),
    ] {
        interface.register(scope).unwrap();
        template.set_class_name(v8str(scope, interface.name()));
        template.read_only_prototype();
        dom_rect::install_dom_rect_template_bindings(scope, template, interface.name());
        let constructor = template.get_function(scope).unwrap();
        let prototype = constructor
            .get(scope, v8str(scope, "prototype").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap();
        moli_webapi_declare::define_to_string_tag(scope, prototype, interface.name());
        global
            .set(
                scope,
                v8str(scope, interface.name()).into(),
                constructor.into(),
            )
            .unwrap();
    }
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).expect("rectangle test source should allocate");
    v8::Script::compile(scope, source, None)
        .and_then(|script| script.run(scope))
        .expect("rectangle test script should execute")
}

#[test]
fn rectangle_declarations_and_callbacks_work_without_a_renderer() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_rectangles(scope);

    assert!(
        eval(
            scope,
            r#"
            (() => {
                function check(value) {
                    if (!value) throw new Error('rectangle check failed');
                }
                const reads = [];
                const readonly = DOMRectReadOnly.fromRect({
                    get x() { reads.push('x'); return 12; },
                    get y() { reads.push('y'); return 34; },
                    get width() { reads.push('width'); return -30; },
                    get height() { reads.push('height'); return -40; }
                });
                check(reads.join(',') === 'height,width,x,y');
                const mutable = DOMRect.fromRect(readonly);
                check(mutable instanceof DOMRect && mutable instanceof DOMRectReadOnly);
                check(Object.getPrototypeOf(DOMRect.prototype) === DOMRectReadOnly.prototype);
                check(JSON.stringify(mutable.toJSON()) ===
                    '{"x":12,"y":34,"width":-30,"height":-40,"top":-6,"right":12,"bottom":34,"left":-18}');
                mutable.x = 55;
                mutable.width = 9;
                check(mutable.right === 64 && mutable.left === 55);
                check(DOMRect.length === 0 && DOMRectReadOnly.length === 0);
                check(Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype, 'x').set === undefined);
                check(Object.getOwnPropertyDescriptor(DOMRect, 'prototype').writable === false);
                check(String(readonly) === '[object DOMRectReadOnly]');
                check(Number.isNaN(new DOMRect(NaN, 0, 1, 1).left));
                check(new DOMRect().width === 0);

                let converted = false;
                try {
                    Object.getOwnPropertyDescriptor(DOMRect.prototype, 'x').set.call(
                        Object.create(DOMRect.prototype),
                        { valueOf() { converted = true; return 7; } }
                    );
                    throw new Error('forged receiver accepted');
                } catch (error) {
                    check(error instanceof TypeError && !converted);
                }
                return true;
            })()
            "#,
        )
        .is_true()
    );
}

#[test]
fn rectangle_clone_helpers_preserve_identity_and_use_the_destination_realm() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let source = v8::Context::new(scope, Default::default());
    let destination = v8::Context::new(scope, Default::default());
    let (mutable, readonly, mutable_data, readonly_data) = {
        let scope = &mut v8::ContextScope::new(scope, source);
        install_rectangles(scope);
        let mutable = dom_rect::build_dom_rect_object(scope, 1.0, 2.0, 3.0, 4.0);
        let readonly = dom_rect::build_dom_rect_readonly_object(scope, 5.0, 6.0, 7.0, 8.0);
        let mutable_data = dom_rect::dom_rect_clone_data(scope, mutable).unwrap();
        let readonly_data = dom_rect::dom_rect_clone_data(scope, readonly).unwrap();
        assert_eq!(mutable_data, (true, [1.0, 2.0, 3.0, 4.0]));
        assert_eq!(readonly_data, (false, [5.0, 6.0, 7.0, 8.0]));
        (mutable, readonly, mutable_data, readonly_data)
    };

    let scope = &mut v8::ContextScope::new(scope, destination);
    install_rectangles(scope);
    assert!(interfaces::DOMRect::is_instance(scope, mutable));
    assert!(interfaces::DOMRectReadOnly::is_instance(scope, mutable));
    assert!(!interfaces::DOMRect::is_instance(scope, readonly));
    assert!(interfaces::DOMRectReadOnly::is_instance(scope, readonly));

    let global = destination.global(scope);
    for (name, data) in [("mutable", mutable_data), ("readonly", readonly_data)] {
        let object = dom_rect::build_dom_rect_clone_object(scope, data.0, data.1);
        global
            .set(scope, v8str(scope, name).into(), object.into())
            .unwrap();
    }
    global
        .set(scope, v8str(scope, "foreign").into(), mutable.into())
        .unwrap();
    let plain = v8::Object::new(scope);
    assert!(dom_rect::dom_rect_clone_data(scope, plain).is_none());

    assert!(
        eval(
            scope,
            r#"
            (() => {
                if (Object.getPrototypeOf(mutable) !== DOMRect.prototype ||
                    Object.getPrototypeOf(readonly) !== DOMRectReadOnly.prototype ||
                    mutable.x !== 1 || readonly.height !== 8 ||
                    Object.getPrototypeOf(foreign) === DOMRect.prototype ||
                    Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype, 'x').get.call(foreign) !== 1) {
                    throw new Error('destination realm or native identity changed');
                }
                const forged = Object.create(DOMRectReadOnly.prototype);
                try {
                    DOMRectReadOnly.prototype.toJSON.call(forged);
                    throw new Error('forged receiver accepted');
                } catch (error) {
                    if (!(error instanceof TypeError)) throw error;
                }
                return true;
            })()
            "#,
        )
        .is_true()
    );
}
