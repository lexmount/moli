mod support;
use support::{eval, publish};

use std::{cell::Cell, pin::pin, rc::Rc};

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{throw_type_error, v8str};
use moli_webapi::{
    RealmCallbacks, dom_quad, dom_rect, dom_rect_list, encoded_video_chunk, geometry,
    geometry_clone, image_data, install_realm_callbacks, interfaces, time_ranges,
};

struct CallbackState {
    point_mutations: Cell<usize>,
}

fn reflect_point<'s>(scope: &mut v8::PinScope<'s, '_>, _: v8::Local<'s, v8::Object>) {
    let state = scope
        .get_current_context()
        .get_slot::<CallbackState>()
        .unwrap();
    state.point_mutations.set(state.point_mutations.get() + 1);
}

fn illegal_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor");
}

fn install_values(
    scope: &mut v8::PinScope<'_, '_>,
    realm: geometry::GeometryRealm,
) -> Rc<CallbackState> {
    support::install_errors(scope);
    let point_readonly =
        v8::FunctionTemplate::new(scope, geometry::dom_point_readonly_constructor_callback);
    let point = v8::FunctionTemplate::new(scope, geometry::dom_point_constructor_callback);
    point.inherit(point_readonly);
    let (matrix_readonly, matrix) = match realm {
        geometry::GeometryRealm::Window => (
            v8::FunctionTemplate::new(scope, geometry::dom_matrix_readonly_constructor_callback),
            v8::FunctionTemplate::new(scope, geometry::dom_matrix_constructor_callback),
        ),
        geometry::GeometryRealm::Worker => (
            v8::FunctionTemplate::new(
                scope,
                geometry::dom_matrix_readonly_worker_constructor_callback,
            ),
            v8::FunctionTemplate::new(scope, geometry::dom_matrix_worker_constructor_callback),
        ),
    };
    matrix.inherit(matrix_readonly);
    let rect_readonly = dom_rect::build_dom_rect_readonly_constructor_template(scope);
    let rect = dom_rect::build_dom_rect_constructor_template(scope);
    rect.inherit(rect_readonly);
    let quad = v8::FunctionTemplate::new(scope, dom_quad::dom_quad_constructor_callback);
    let rect_list = v8::FunctionTemplate::new(scope, illegal_constructor);
    let image = v8::FunctionTemplate::new(scope, image_data::image_data_constructor_callback);
    let chunk = v8::FunctionTemplate::new(
        scope,
        encoded_video_chunk::encoded_video_chunk_constructor_callback,
    );
    let ranges = v8::FunctionTemplate::new(scope, illegal_constructor);
    let global = scope.get_current_context().global(scope);
    for (interface, template) in [
        (interfaces::DOMPointReadOnly::DESCRIPTOR, point_readonly),
        (interfaces::DOMPoint::DESCRIPTOR, point),
        (interfaces::DOMMatrixReadOnly::DESCRIPTOR, matrix_readonly),
        (interfaces::DOMMatrix::DESCRIPTOR, matrix),
        (interfaces::DOMRectReadOnly::DESCRIPTOR, rect_readonly),
        (interfaces::DOMRect::DESCRIPTOR, rect),
        (interfaces::DOMQuad::DESCRIPTOR, quad),
        (interfaces::DOMRectList::DESCRIPTOR, rect_list),
        (interfaces::ImageData::DESCRIPTOR, image),
        (interfaces::EncodedVideoChunk::DESCRIPTOR, chunk),
        (interfaces::TimeRanges::DESCRIPTOR, ranges),
    ] {
        interface.register(scope).unwrap();
        template.set_class_name(v8str(scope, interface.name()));
        template.read_only_prototype();
        geometry::install_geometry_template_bindings(scope, template, interface.name(), realm);
        dom_rect::install_dom_rect_template_bindings(scope, template, interface.name());
        dom_quad::install_dom_quad_template_bindings(scope, template, interface.name());
        dom_rect_list::install_dom_rect_list_template_bindings(scope, template, interface.name());
        time_ranges::install_time_ranges_template_bindings(scope, template, interface.name());
        match interface.name() {
            "ImageData" => image_data::install_image_data_template_bindings(scope, template),
            "EncodedVideoChunk" => {
                encoded_video_chunk::install_encoded_video_chunk_template_bindings(scope, template)
            }
            _ => {}
        }
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
    let state = Rc::new(CallbackState {
        point_mutations: Cell::new(0),
    });
    scope.get_current_context().set_slot(state.clone());
    install_realm_callbacks(
        scope,
        RealmCallbacks {
            runtime: None,
            point_mutated: reflect_point,
            media_query_matches: |_, _| None,
        },
    );
    state
}

#[test]
fn geometry_conversions_and_operations_work_without_a_renderer() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_values(scope, geometry::GeometryRealm::Window);
    assert!(eval(scope, r#"(() => {
        const check = (value, label) => { if (!value) throw new Error(label); };
        const reads = [];
        const point = DOMPoint.fromPoint({
            get x() { reads.push('x'); return 2; },
            get y() { reads.push('y'); return 3; },
            get z() { reads.push('z'); return 0; },
            get w() { reads.push('w'); return 1; }
        });
        check(reads.join(',') === 'w,x,y,z', 'dictionary order');
        check(point instanceof DOMPointReadOnly && point instanceof DOMPoint, 'point inheritance');
        const matrix = new DOMMatrix([2, 0, 0, 3, 4, 5]);
        const transformed = point.matrixTransform(matrix);
        check(transformed.x === 8 && transformed.y === 14, 'matrix transform');
        check(matrix instanceof DOMMatrixReadOnly, 'matrix inheritance');
        check(matrix.inverse().transformPoint(transformed).x === 2, 'matrix inverse');
        check(matrix.toFloat64Array().length === 16, 'typed array');
        check(new DOMMatrix('translate(4px, 5px)').e === 4, 'CSS transform parsing');
        const quad = DOMQuad.fromRect({x: 1, y: 2, width: -3, height: 4});
        const bounds = quad.getBounds();
        check(quad.p1 instanceof DOMPoint && quad.p1 === quad.p1, 'quad points');
        check(bounds instanceof DOMRect && bounds.x === -2 && bounds.width === 3 && bounds.height === 4, 'quad bounds');
        let converted = false;
        try {
            Object.getOwnPropertyDescriptor(DOMPoint.prototype, 'x').set.call(
                Object.create(DOMPoint.prototype), {valueOf() { converted = true; return 4; }});
            throw new Error('forged point accepted');
        } catch (error) { check(error instanceof TypeError && !converted, 'native receiver'); }
        return true;
    })()"#).is_true());
}

#[test]
fn worker_geometry_keeps_its_constructor_and_prototype_restrictions() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_values(scope, geometry::GeometryRealm::Worker);
    assert!(eval(scope, r#"(() => {
        if (Object.hasOwn(DOMMatrix.prototype, 'setMatrixValue') ||
            Object.hasOwn(DOMMatrixReadOnly.prototype, 'toString')) throw new Error('Window methods leaked');
        let converted = false;
        try {
            new DOMMatrix({toString() { converted = true; return 'matrix(1, 0, 0, 1, 0, 0)'; }});
            throw new Error('worker CSS constructor accepted');
        } catch (error) { if (!(error instanceof TypeError) || !converted) throw error; }
        const matrix = new DOMMatrix([1, 0, 0, 1, 6, 7]);
        const point = matrix.transformPoint({x: 2, y: 3});
        if (!(point instanceof DOMPoint) || point.x !== 8 || point.y !== 10) throw new Error('worker values');
        return true;
    })()"#).is_true());
}

#[test]
fn geometry_clone_helpers_use_the_destination_realm() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let source = v8::Context::new(scope, Default::default());
    let destination = v8::Context::new(scope, Default::default());
    let payloads = {
        let scope = &mut v8::ContextScope::new(scope, source);
        install_values(scope, geometry::GeometryRealm::Window);
        [
            ("point", "new DOMPoint(1, 2, 3, 4)"),
            ("readonlyPoint", "new DOMPointReadOnly(5, 6)"),
            ("matrix", "new DOMMatrix([2, 0, 0, 3, 4, 5])"),
            (
                "readonlyMatrix",
                "new DOMMatrixReadOnly([1, 0, 0, 1, 7, 8])",
            ),
            (
                "quad",
                "DOMQuad.fromRect({x: 1, y: 2, width: 3, height: 4})",
            ),
        ]
        .map(|(name, expression)| {
            let object = v8::Local::<v8::Object>::try_from(eval(scope, expression)).unwrap();
            (
                name,
                object,
                geometry_clone::geometry_clone_payload_from_object(scope, object).unwrap(),
            )
        })
    };
    let scope = &mut v8::ContextScope::new(scope, destination);
    install_values(scope, geometry::GeometryRealm::Window);
    for (name, foreign, payload) in payloads {
        let object = geometry_clone::build_geometry_object_from_clone_payload(scope, payload);
        assert!(!foreign.strict_equals(object.into()));
        publish(scope, name, object);
    }
    assert!(eval(scope, r#"(() => {
        if (Object.getPrototypeOf(point) !== DOMPoint.prototype || point.w !== 4 ||
            Object.getPrototypeOf(readonlyPoint) !== DOMPointReadOnly.prototype || readonlyPoint.x !== 5 ||
            Object.getPrototypeOf(matrix) !== DOMMatrix.prototype || matrix.f !== 5 ||
            Object.getPrototypeOf(readonlyMatrix) !== DOMMatrixReadOnly.prototype || readonlyMatrix.e !== 7 ||
            Object.getPrototypeOf(quad) !== DOMQuad.prototype ||
            Object.getPrototypeOf(quad.p1) !== DOMPoint.prototype || quad.getBounds().height !== 4) {
            throw new Error('clone data or destination prototype changed');
        }
        return true;
    })()"#).is_true());
}

#[test]
fn renderer_callbacks_keep_svg_writes_and_native_exceptions_in_the_callee_realm() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let source = v8::Context::new(scope, Default::default());
    let destination = v8::Context::new(scope, Default::default());
    let (foreign, source_state) = {
        let scope = &mut v8::ContextScope::new(scope, source);
        let state = install_values(scope, geometry::GeometryRealm::Window);
        (
            geometry::build_svg_point_object_with_values(scope, 1.0, 2.0),
            state,
        )
    };
    let scope = &mut v8::ContextScope::new(scope, destination);
    let destination_state = install_values(scope, geometry::GeometryRealm::Window);
    publish(scope, "foreignPoint", foreign);
    assert!(eval(scope, r#"(() => {
        globalThis.nativeDOMException = DOMException;
        globalThis.DOMException = function() { throw new Error('public constructor called'); };
        const setter = Object.getOwnPropertyDescriptor(DOMPoint.prototype, 'x').set;
        setter.call(foreignPoint, 12);
        if (foreignPoint.x !== 12) throw new Error('borrowed setter did not update point');
        try { new DOMMatrix('invalid transform'); throw new Error('invalid transform accepted'); }
        catch (error) { if (error.name !== 'SyntaxError' || !(error instanceof nativeDOMException)) throw error; }
        try { new ImageData(0, 1); throw new Error('invalid image size accepted'); }
        catch (error) { if (error.name !== 'IndexSizeError' || !(error instanceof nativeDOMException)) throw error; }
        return true;
    })()"#).is_true());
    assert_eq!(source_state.point_mutations.get(), 0);
    assert_eq!(destination_state.point_mutations.get(), 1);
    geometry::set_svg_point_read_only(scope, foreign, true);
    assert!(eval(scope, r#"(() => {
        try {
            Object.getOwnPropertyDescriptor(DOMPoint.prototype, 'x').set.call(foreignPoint, 99);
            throw new Error('read-only SVG point accepted');
        } catch (error) {
            if (error.name !== 'NoModificationAllowedError' || !(error instanceof nativeDOMException) || foreignPoint.x !== 12) throw error;
        }
        return true;
    })()"#).is_true());
    assert_eq!(destination_state.point_mutations.get(), 1);
}

#[test]
fn image_data_and_encoded_chunk_clones_keep_bytes_and_destination_brands() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let source = v8::Context::new(scope, Default::default());
    let destination = v8::Context::new(scope, Default::default());
    let (image_payload, chunk_payload) = {
        let scope = &mut v8::ContextScope::new(scope, source);
        install_values(scope, geometry::GeometryRealm::Window);
        let image = v8::Local::<v8::Object>::try_from(eval(scope,
            "new ImageData(new Uint8ClampedArray([1, 2, 3, 4, 5, 6, 7, 8]), 2, 1, {colorSpace: 'display-p3'})")).unwrap();
        let chunk = v8::Local::<v8::Object>::try_from(eval(scope, r#"(() => {
            const data = new Uint8Array([9, 8, 7, 6]);
            const chunk = new EncodedVideoChunk({type: 'key', timestamp: -3, duration: 8, data, transfer: [data.buffer]});
            if (data.byteLength !== 0) throw new Error('input was not transferred');
            return chunk;
        })()"#)).unwrap();
        (
            image_data::image_data_clone_payload_from_object(scope, image).unwrap(),
            encoded_video_chunk::encoded_video_chunk_clone_payload_from_object(scope, chunk)
                .unwrap(),
        )
    };
    let scope = &mut v8::ContextScope::new(scope, destination);
    assert!(
        encoded_video_chunk::build_encoded_video_chunk_from_clone_payload(
            scope,
            chunk_payload.clone()
        )
        .is_none()
    );
    install_values(scope, geometry::GeometryRealm::Worker);
    let image =
        image_data::build_image_data_object_from_clone_payload(scope, image_payload).unwrap();
    let chunk =
        encoded_video_chunk::build_encoded_video_chunk_from_clone_payload(scope, chunk_payload)
            .unwrap();
    publish(scope, "image", image);
    publish(scope, "chunk", chunk);
    assert!(eval(scope, r#"(() => {
        if (Object.getPrototypeOf(image) !== ImageData.prototype || image.width !== 2 || image.height !== 1 ||
            image.colorSpace !== 'display-p3' || image.data.join(',') !== '1,2,3,4,5,6,7,8') throw new Error('ImageData clone');
        if (Object.getPrototypeOf(chunk) !== EncodedVideoChunk.prototype || chunk.timestamp !== -3 ||
            chunk.type !== 'key' || chunk.duration !== 8 || chunk.byteLength !== 4) throw new Error('chunk metadata');
        const destination = new Uint8Array(6);
        chunk.copyTo(destination.subarray(1, 5));
        if (destination.join(',') !== '0,9,8,7,6,0') throw new Error('chunk bytes');
        try { chunk.copyTo(new Uint8Array(1)); throw new Error('undersized buffer accepted'); }
        catch (error) { if (!(error instanceof TypeError)) throw error; }
        return true;
    })()"#).is_true());
}

#[test]
fn rect_lists_and_time_ranges_bind_native_snapshots_without_layout_or_media_hosts() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_values(scope, geometry::GeometryRealm::Window);
    let rect = dom_rect::build_dom_rect_object(scope, 1.0, 2.0, 3.0, 4.0);
    let list = dom_rect_list::build_dom_rect_list_object(scope, &[rect]);
    let ranges = time_ranges::new_time_ranges_value(scope, &[(1.25, 2.5), (5.0, 8.0)]);
    publish(scope, "list", list);
    publish(scope, "ranges", ranges);
    assert!(eval(scope, r#"(() => {
        if (!(list instanceof DOMRectList) || list.length !== 1 || list.item(0) !== list[0] ||
            list.item(100) !== null || [...list][0].width !== 3) throw new Error('rectangle list');
        if (Object.getOwnPropertyDescriptor(list, '0').writable !== false) throw new Error('writable list index');
        if (!(ranges instanceof TimeRanges) || ranges.length !== 2 || ranges.start(0) !== 1.25 ||
            ranges.end(1) !== 8) throw new Error('time range values');
        for (const index of [2, -1, 4294967295]) {
            try { ranges.start(index); throw new Error('out-of-range index accepted'); }
            catch (error) { if (error.name !== 'IndexSizeError' || !(error instanceof DOMException)) throw error; }
        }
        try { TimeRanges.prototype.start.call({}, 0); throw new Error('forged range accepted'); }
        catch (error) { if (!(error instanceof TypeError)) throw error; }
        return true;
    })()"#).is_true());
}
