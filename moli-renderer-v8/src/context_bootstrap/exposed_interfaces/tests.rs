use std::cell::Cell;
use std::pin::pin;

use crate::context_bootstrap::specs::{ConstructorKind, ConstructorSpec};
use crate::util::v8str;

thread_local! {
    static LAZY_GETTER_CALLS: Cell<u32> = const { Cell::new(0) };
}

fn reset_lazy_getter_calls() {
    LAZY_GETTER_CALLS.set(0);
}

fn lazy_getter_calls() -> u32 {
    LAZY_GETTER_CALLS.get()
}

fn counting_lazy_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let count = LAZY_GETTER_CALLS.get().saturating_add(1);
    LAZY_GETTER_CALLS.set(count);
    let data = args.data();
    if data.is_undefined() {
        rv.set(v8::Integer::new_from_unsigned(scope, count).into());
    } else {
        rv.set(data);
    }
}

#[test]
fn object_lazy_data_property_materializes_once_into_a_data_descriptor() {
    crate::ensure_v8_for_test();
    reset_lazy_getter_calls();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let object = v8::Object::new(scope);
    let sentinel = v8::Object::new(scope);
    let name = v8str(scope, "lazy");

    assert_eq!(
        object.set_lazy_data_property_with_configuration(
            scope,
            name.into(),
            v8::LazyDataPropertyConfiguration::new(counting_lazy_getter)
                .data(sentinel.into())
                .property_attribute(v8::PropertyAttribute::DONT_ENUM),
        ),
        Some(true)
    );
    assert_eq!(lazy_getter_calls(), 0);

    let first = object.get(scope, name.into()).expect("first lazy read");
    let second = object.get(scope, name.into()).expect("second lazy read");
    assert!(first.strict_equals(sentinel.into()));
    assert!(second.strict_equals(sentinel.into()));
    assert_eq!(lazy_getter_calls(), 1);

    let descriptor = object
        .get_own_property_descriptor(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("materialized data descriptor");
    assert!(
        descriptor
            .get(scope, v8str(scope, "value").into())
            .is_some_and(|value| value.strict_equals(sentinel.into()))
    );
    assert!(
        descriptor
            .get(scope, v8str(scope, "get").into())
            .expect("descriptor getter field")
            .is_undefined()
    );
    assert!(
        descriptor
            .get(scope, v8str(scope, "writable").into())
            .expect("descriptor writable field")
            .boolean_value(scope)
    );
    assert!(
        !descriptor
            .get(scope, v8str(scope, "enumerable").into())
            .expect("descriptor enumerable field")
            .boolean_value(scope)
    );
    assert!(
        descriptor
            .get(scope, v8str(scope, "configurable").into())
            .expect("descriptor configurable field")
            .boolean_value(scope)
    );
}

#[test]
fn object_lazy_data_property_assignment_before_read_skips_the_getter() {
    crate::ensure_v8_for_test();
    reset_lazy_getter_calls();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let object = v8::Object::new(scope);
    let name = v8str(scope, "lazy");

    assert_eq!(
        object.set_lazy_data_property(scope, name.into(), counting_lazy_getter),
        Some(true)
    );
    let replacement = v8::Integer::new(scope, -17);
    assert_eq!(
        object.set(scope, name.into(), replacement.into()),
        Some(true)
    );
    assert_eq!(
        object
            .get(scope, name.into())
            .and_then(|value| value.int32_value(scope)),
        Some(-17)
    );
    assert_eq!(lazy_getter_calls(), 0);
}

#[test]
fn object_lazy_data_property_deletion_before_read_skips_the_getter() {
    crate::ensure_v8_for_test();
    reset_lazy_getter_calls();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let object = v8::Object::new(scope);
    let name = v8str(scope, "lazy");

    assert_eq!(
        object.set_lazy_data_property(scope, name.into(), counting_lazy_getter),
        Some(true)
    );
    assert_eq!(object.delete(scope, name.into()), Some(true));
    assert_eq!(object.has_own_property(scope, name.into()), Some(false));
    assert_eq!(lazy_getter_calls(), 0);
}

#[test]
fn insecure_window_filter_rejects_a_failed_interface_deletion() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let specs = vec![ConstructorSpec {
        name: "StorageManager",
        parent: None,
        kind: ConstructorKind::StorageManager,
    }];
    super::template_registry::ExposedInterfaceTemplateRegistry::install(
        scope,
        specs,
        super::metadata::TemplateBuildProfile::Window,
    )
    .expect("interface registry should install");
    let global = v8::Object::new(scope);
    let name = v8str(scope, "StorageManager");
    assert_eq!(
        global.define_own_property(
            scope,
            name.into(),
            v8::Integer::new(scope, 1).into(),
            v8::PropertyAttribute::DONT_DELETE,
        ),
        Some(true)
    );

    let error = super::install::filter_window_exposed_interfaces(scope, global, false)
        .expect_err("failed secure-context filtering must abort bootstrap");

    assert_eq!(
        error.to_string(),
        "failed to remove unexposed window interface `StorageManager`"
    );
    assert_eq!(global.has_own_property(scope, name.into()), Some(true));
}

#[test]
fn object_template_lazy_property_materializes_once_per_instance() {
    crate::ensure_v8_for_test();
    reset_lazy_getter_calls();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let template = v8::ObjectTemplate::new(scope);
    let name = v8str(scope, "lazy");
    template.set_lazy_data_property(name.into(), counting_lazy_getter);

    let first = template.new_instance(scope).expect("first instance");
    assert_eq!(
        first
            .get(scope, name.into())
            .and_then(|value| value.uint32_value(scope)),
        Some(1)
    );
    assert_eq!(
        first
            .get(scope, name.into())
            .and_then(|value| value.uint32_value(scope)),
        Some(1)
    );

    let second = template.new_instance(scope).expect("second instance");
    assert_eq!(
        second
            .get(scope, name.into())
            .and_then(|value| value.uint32_value(scope)),
        Some(2)
    );
    assert_eq!(
        second
            .get(scope, name.into())
            .and_then(|value| value.uint32_value(scope)),
        Some(2)
    );

    let overwritten = template.new_instance(scope).expect("overwritten instance");
    let replacement = v8::Integer::new(scope, -23);
    assert_eq!(
        overwritten.set(scope, name.into(), replacement.into()),
        Some(true)
    );
    assert_eq!(
        overwritten
            .get(scope, name.into())
            .and_then(|value| value.int32_value(scope)),
        Some(-23)
    );
    assert_eq!(lazy_getter_calls(), 2);
}

#[test]
fn entered_context_template_build_is_isolate_cached_and_realm_neutral() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let specs = crate::context_bootstrap::specs::constructor_specs();

    let first_context = v8::Context::new(scope, Default::default());
    let (registry, first_constructor) = {
        let scope = &mut v8::ContextScope::new(scope, first_context);
        let registry = super::template_registry::ExposedInterfaceTemplateRegistry::install(
            scope,
            specs,
            super::metadata::TemplateBuildProfile::Window,
        )
        .expect("template registry");
        let id = registry.id_by_name("HTMLAreaElement").expect("area id");
        let template = registry
            .get_or_build_template(scope, id)
            .expect("entered-context template build");
        let constructor = template
            .get_function(scope)
            .expect("first realm constructor");
        let instance = template
            .instance_template(scope)
            .new_instance(scope)
            .expect("first realm instance");
        assert!(template.has_instance(instance.into()));
        (registry, v8::Global::new(scope, constructor))
    };

    let second_context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, second_context);
    let id = registry.id_by_name("HTMLAreaElement").expect("area id");
    let template = registry
        .get_or_build_template(scope, id)
        .expect("second realm template lookup");
    let second_constructor = template
        .get_function(scope)
        .expect("second realm constructor");
    let first_constructor = v8::Local::new(scope, &first_constructor);
    assert!(!second_constructor.strict_equals(first_constructor.into()));
    let second_instance = template
        .instance_template(scope)
        .new_instance(scope)
        .expect("second realm instance");
    assert!(template.has_instance(second_instance.into()));
    assert_eq!(registry.build_count(id), 1);
}

#[test]
fn worker_realm_lazy_properties_follow_chromium_exposure_sets() {
    fn own_properties(realm: super::RealmKind) -> Vec<bool> {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
            scope, global, realm, true,
        )
        .expect("worker lazy interfaces should install");
        let registry = super::template_registry::ExposedInterfaceTemplateRegistry::current(scope)
            .expect("worker template registry");
        assert_eq!(
            registry.ready_template_count(),
            0,
            "installing worker lazy properties must not build interface templates"
        );
        let url_id = registry.id_by_name("URL").expect("URL template id");
        assert_eq!(registry.build_count(url_id), 0);
        let first_url = global
            .get(scope, v8str(scope, "URL").into())
            .expect("first worker URL read");
        let second_url = global
            .get(scope, v8str(scope, "URL").into())
            .expect("second worker URL read");
        assert!(first_url.strict_equals(second_url));
        assert_eq!(registry.build_count(url_id), 1);

        [
            "Worker",
            "XMLHttpRequest",
            "FileReaderSync",
            "CSSStyleRule",
            "FileSystemSyncAccessHandle",
            "URL",
            "WorkerLocation",
        ]
        .iter()
        .map(|name| {
            global
                .has_own_property(scope, v8str(scope, name).into())
                .unwrap_or(false)
        })
        .collect()
    }

    assert_eq!(
        own_properties(super::RealmKind::DedicatedWorker),
        vec![true, true, true, false, true, true, false]
    );
    assert_eq!(
        own_properties(super::RealmKind::SharedWorker),
        vec![true, true, true, false, false, true, false]
    );
    assert_eq!(
        own_properties(super::RealmKind::ServiceWorker),
        vec![false, false, false, false, false, true, false]
    );
}

#[test]
fn worker_geometry_interfaces_use_worker_specific_matrix_surface() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
        scope,
        global,
        super::RealmKind::DedicatedWorker,
        true,
    )
    .expect("worker geometry interfaces should install");

    let source = v8::String::new(
        scope,
        r#"
        (() => {
          const names = [
            "DOMPointReadOnly", "DOMPoint", "DOMRectReadOnly", "DOMRect",
            "DOMQuad", "DOMMatrixReadOnly", "DOMMatrix"
          ];
          const errorName = callback => {
            try {
              callback();
              return "missing";
            } catch (error) {
              return error.name;
            }
          };
          const operationSets = [
            [DOMPointReadOnly, ["fromPoint"]],
            [DOMPointReadOnly.prototype, ["matrixTransform"]],
            [DOMPoint, ["fromPoint"]],
            [DOMRectReadOnly, ["fromRect"]],
            [DOMRect, ["fromRect"]],
            [DOMQuad, ["fromRect", "fromQuad"]],
            [DOMMatrixReadOnly, [
              "fromMatrix", "fromFloat32Array", "fromFloat64Array"
            ]],
            [DOMMatrixReadOnly.prototype, [
              "translate", "scale", "scaleNonUniform", "scale3d", "rotate",
              "rotateFromVector", "rotateAxisAngle", "skewX", "skewY",
              "multiply", "flipX", "flipY", "inverse", "transformPoint",
              "toFloat32Array", "toFloat64Array"
            ]],
            [DOMMatrix, [
              "fromMatrix", "fromFloat32Array", "fromFloat64Array"
            ]],
            [DOMMatrix.prototype, [
              "multiplySelf", "preMultiplySelf", "translateSelf", "scaleSelf",
              "scale3dSelf", "rotateSelf", "rotateFromVectorSelf",
              "rotateAxisAngleSelf", "skewXSelf", "skewYSelf", "invertSelf"
            ]]
          ];
          const hasWebIdlOperationDescriptor = (owner, name) => {
            const descriptor = Object.getOwnPropertyDescriptor(owner, name);
            return descriptor !== undefined &&
              typeof descriptor.value === "function" &&
              descriptor.enumerable && descriptor.writable && descriptor.configurable;
          };
          return JSON.stringify({
            constructors: names.map(name => typeof globalThis[name]).join(","),
            strings: [
              errorName(() => new DOMMatrix("matrix(1,0,0,1,0,0)")),
              errorName(() => new DOMMatrixReadOnly(""))
            ].join(","),
            tags: [
              String(new DOMMatrix()),
              String(DOMMatrixReadOnly.fromMatrix({is2D: false})),
              String(new DOMMatrix([1, 0, 0, NaN, Infinity, -Infinity]))
            ].join(","),
            windowOnly: [
              Object.hasOwn(DOMMatrixReadOnly.prototype, "toString"),
              "setMatrixValue" in DOMMatrix.prototype,
              "WebKitCSSMatrix" in globalThis,
              "SVGPoint" in globalThis,
              "SVGRect" in globalThis,
              "SVGMatrix" in globalThis
            ].join(","),
            operations: operationSets.every(([owner, operationNames]) =>
              operationNames.every(name => hasWebIdlOperationDescriptor(owner, name))
            ),
            lengths: [DOMMatrix.length, DOMMatrixReadOnly.length].join(",")
          });
        })()
        "#,
    )
    .expect("worker geometry test source");
    let script = v8::Script::compile(scope, source, None).expect("worker geometry test compile");
    let result = script
        .run(scope)
        .expect("worker geometry test evaluation")
        .to_rust_string_lossy(scope);

    assert_eq!(
        result,
        r#"{"constructors":"function,function,function,function,function,function,function","strings":"TypeError,TypeError","tags":"[object DOMMatrix],[object DOMMatrixReadOnly],[object DOMMatrix]","windowOnly":"false,false,false,false,false,false","operations":true,"lengths":"0,0"}"#
    );
}
