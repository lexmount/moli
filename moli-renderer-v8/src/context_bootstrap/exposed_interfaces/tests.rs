use crate::web_api_interfaces;
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
        interface: web_api_interfaces::StorageManager::DESCRIPTOR,
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
fn uninitialized_dom_string_map_adopts_its_registered_intrinsics() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let registry = super::template_registry::ExposedInterfaceTemplateRegistry::install(
        scope,
        crate::context_bootstrap::specs::constructor_specs(),
        super::metadata::TemplateBuildProfile::Window,
    )
    .expect("template registry");
    let realm = super::realm_registry::IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        super::RealmKind::Window,
    )
    .expect("realm registry");
    let id = registry
        .id_by_name("DOMStringMap")
        .expect("dataset interface");
    let constructor = registry
        .get_or_build_template(scope, id)
        .expect("dataset template")
        .get_function(scope)
        .expect("dataset constructor");
    let prototype = crate::util::constructor_prototype_object(scope, constructor.into())
        .expect("dataset prototype");
    let global = context.global(scope);
    assert!(crate::util::register_intrinsic_interface(
        scope,
        global,
        "DOMStringMap",
        constructor.into(),
        prototype,
    ));
    assert_eq!(
        global.set_lazy_data_property(
            scope,
            v8str(scope, "DOMStringMap").into(),
            counting_lazy_getter,
        ),
        Some(true)
    );
    reset_lazy_getter_calls();

    let recovered = super::ensure_intrinsic_interface_prototype(scope, "DOMStringMap")
        .expect("a complete trusted registration must recover the dataset prototype");

    assert!(recovered.strict_equals(prototype.into()));
    assert_eq!(
        realm.state(id),
        Some(super::realm_registry::RealmInterfaceState::Ready)
    );
    assert!(
        super::ensure_intrinsic_interface_constructor(scope, "DOMStringMap")
            .expect("recovered dataset constructor")
            .strict_equals(constructor.into())
    );
    assert!(
        super::materialize::materialize_interface(scope, id)
            .expect("repeated dataset materialization")
            .strict_equals(constructor.into())
    );
    assert_eq!(registry.build_count(id), 1);
    assert_eq!(
        lazy_getter_calls(),
        0,
        "recovery must not read the public binding"
    );
}

#[test]
fn incomplete_intrinsic_registration_marks_materialization_failed() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let registry = super::template_registry::ExposedInterfaceTemplateRegistry::install(
        scope,
        crate::context_bootstrap::specs::constructor_specs(),
        super::metadata::TemplateBuildProfile::Window,
    )
    .expect("template registry");
    let realm = super::realm_registry::IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        super::RealmKind::Window,
    )
    .expect("realm registry");
    let id = registry
        .id_by_name("DOMStringMap")
        .expect("dataset interface");
    let global = context.global(scope);
    crate::util::initialize_intrinsic_interface_registry(scope, global);
    let prototypes = crate::util::get_private_object(scope, global, "__moliIntrinsicPrototypes")
        .expect("private prototype registry");
    assert_eq!(
        prototypes.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true)
    );
    let constructor = v8::Object::new(scope);
    let prototype = v8::Object::new(scope);
    assert!(!crate::util::register_intrinsic_interface(
        scope,
        global,
        "DOMStringMap",
        constructor,
        prototype,
    ));
    assert!(crate::util::registered_intrinsic_constructor(scope, global, "DOMStringMap").is_some());
    assert!(crate::util::registered_intrinsic_prototype(scope, global, "DOMStringMap").is_none());

    let error = super::ensure_intrinsic_interface_prototype(scope, "DOMStringMap")
        .expect_err("an incomplete pair must not supply a dataset prototype");

    assert!(error.to_string().contains("partial registry state"));
    assert_eq!(
        realm.state(id),
        Some(super::realm_registry::RealmInterfaceState::Failed)
    );
    let repeated_error = super::ensure_intrinsic_interface_prototype(scope, "DOMStringMap")
        .expect_err("a failed materialization must not retry the partial registration");
    assert!(
        repeated_error
            .to_string()
            .contains("previous materialization")
    );
    assert_eq!(registry.build_count(id), 0);
}

#[test]
fn failed_eager_intrinsic_capture_does_not_leave_an_uninitialized_interface() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let registry = super::template_registry::ExposedInterfaceTemplateRegistry::install(
        scope,
        crate::context_bootstrap::specs::constructor_specs(),
        super::metadata::TemplateBuildProfile::Window,
    )
    .expect("template registry");
    let realm = super::realm_registry::IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        super::RealmKind::Window,
    )
    .expect("realm registry");
    let global = context.global(scope);
    let error_constructor = crate::util::constructor_object(scope, global, "Error")
        .expect("ECMAScript Error constructor");
    let error_prototype = crate::util::constructor_prototype_object(scope, error_constructor)
        .expect("ECMAScript Error prototype");
    assert!(crate::util::register_intrinsic_interface(
        scope,
        global,
        "Error",
        error_constructor,
        error_prototype,
    ));
    let id = registry
        .id_by_name("Window")
        .expect("eager Window interface");
    let constructor = v8::Object::new(scope);
    let prototype = v8::Object::new(scope);
    assert_eq!(
        constructor.set(scope, v8str(scope, "prototype").into(), prototype.into()),
        Some(true)
    );
    assert_eq!(
        global.set(scope, v8str(scope, "Window").into(), constructor.into()),
        Some(true)
    );
    let prototypes = crate::util::get_private_object(scope, global, "__moliIntrinsicPrototypes")
        .expect("private prototype registry");
    assert_eq!(
        prototypes.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true)
    );

    super::capture_eager_intrinsic_interfaces(scope, global, super::RealmKind::Window)
        .expect_err("a failed private registration must abort eager capture");

    assert_eq!(
        realm.state(id),
        Some(super::realm_registry::RealmInterfaceState::Failed)
    );
    assert!(realm.constructor(scope, id).is_none());
    assert!(realm.prototype(scope, id).is_none());
    assert!(realm.public_interface(scope, id).is_none());
    assert!(super::ensure_intrinsic_interface_prototype(scope, "Window").is_err());
}

#[test]
fn failed_intrinsic_finalization_does_not_expose_registered_objects() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let registry = super::template_registry::ExposedInterfaceTemplateRegistry::install(
        scope,
        crate::context_bootstrap::specs::constructor_specs(),
        super::metadata::TemplateBuildProfile::Window,
    )
    .expect("template registry");
    let realm = super::realm_registry::IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        super::RealmKind::Window,
    )
    .expect("realm registry");
    let id = registry.id_by_name("Crypto").expect("Crypto interface");
    let constructor = registry
        .get_or_build_template(scope, id)
        .expect("Crypto template")
        .get_function(scope)
        .expect("Crypto constructor");
    let prototype = crate::util::constructor_prototype_object(scope, constructor.into())
        .expect("Crypto prototype");
    let global = context.global(scope);
    assert!(crate::util::register_intrinsic_interface(
        scope,
        global,
        "Crypto",
        constructor.into(),
        prototype,
    ));
    let subtle_crypto_available = v8::Boolean::new(scope, true);
    crate::util::set_private_value(
        scope,
        global,
        "__moliWindowCryptoSubtleAvailable",
        subtle_crypto_available.into(),
    );
    assert_eq!(
        prototype.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true)
    );

    super::ensure_intrinsic_interface_prototype(scope, "Crypto")
        .expect_err("a frozen prototype must reject secure-context finalization");

    assert_eq!(
        realm.state(id),
        Some(super::realm_registry::RealmInterfaceState::Failed)
    );
    assert!(realm.constructor(scope, id).is_none());
    assert!(realm.prototype(scope, id).is_none());
    assert!(realm.public_interface(scope, id).is_none());
    assert!(super::ensure_intrinsic_interface_constructor(scope, "Crypto").is_err());
    assert!(super::ensure_intrinsic_interface_prototype(scope, "Crypto").is_err());
    assert!(super::materialized_intrinsic_interface_prototype(scope, "Crypto").is_none());
    assert_eq!(registry.materialization_count(id), 0);
    assert!(
        crate::util::registered_intrinsic_constructor(scope, global, "Crypto")
            .is_some_and(|value| value.strict_equals(constructor.into()))
    );
    assert!(
        crate::util::registered_intrinsic_prototype(scope, global, "Crypto")
            .is_some_and(|value| value.strict_equals(prototype.into()))
    );
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

        let source = v8str(
            scope,
            r#"(() => {
              const Original = URL;
              const parse = Original.parse;
              globalThis.URL = null;
              try {
                const value = parse('https://example.test/?q=value');
                return value instanceof Original &&
                  Object.getPrototypeOf(value) === Original.prototype &&
                  value.searchParams.get('q') === 'value' && globalThis.URL === null;
              } finally {
                globalThis.URL = Original;
              }
            })()"#,
        );
        let script = v8::Script::compile(scope, source, None).expect("worker URL factory script");
        assert!(
            crate::script_execution::execute_compiled_script(scope, script)
                .expect("worker URL factory evaluation")
                .is_true(),
            "worker URL factories must not follow the public constructor binding"
        );
        assert_eq!(registry.build_count(url_id), 1);

        [
            "Worker",
            "XMLHttpRequest",
            "FileReaderSync",
            "CSSStyleRule",
            "FileSystemSyncAccessHandle",
            "URL",
            "WorkerLocation",
            "webkitURL",
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
        vec![true, true, true, false, true, true, false, false]
    );
    assert_eq!(
        own_properties(super::RealmKind::SharedWorker),
        vec![true, true, true, false, false, true, false, false]
    );
    assert_eq!(
        own_properties(super::RealmKind::ServiceWorker),
        vec![false, false, false, false, false, true, false, false]
    );
}
