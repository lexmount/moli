use std::pin::pin;

use super::super::metadata::{RealmKind, TemplateBuildProfile};
use super::super::template_registry::ExposedInterfaceTemplateRegistry;
use crate::util::v8str;

#[test]
fn url_object_url_statics_follow_realm_exposure() {
    crate::ensure_v8_for_test();
    for realm in [
        RealmKind::Window,
        RealmKind::DedicatedWorker,
        RealmKind::SharedWorker,
        RealmKind::ServiceWorker,
    ] {
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let global_template = v8::ObjectTemplate::new(scope);
        if realm == RealmKind::Window {
            let registry = ExposedInterfaceTemplateRegistry::install(
                scope,
                crate::context_bootstrap::specs::constructor_specs(),
                TemplateBuildProfile::Window,
            )
            .expect("window template registry");
            super::super::install::install_window_exposed_interfaces(
                scope,
                global_template,
                &registry,
            )
            .expect("window lazy interface properties");
        }
        let context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(global_template),
                ..Default::default()
            },
        );
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        if realm == RealmKind::Window {
            super::super::install::initialize_realm_interface_registry(scope, realm)
                .expect("window realm registry");
        } else {
            crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
                scope, global, realm, true,
            )
            .expect("worker lazy interface properties");
        }

        let source = v8str(
            scope,
            r#"JSON.stringify((() => {
              const constructor = URL;
              const parsed = constructor.parse('/child?q=one&q=two', 'https://example.test/base');
              return {
                stable: URL === constructor,
                common: [
                  parsed.href,
                  parsed.searchParams.getAll('q'),
                  constructor.canParse('/child', 'https://example.test/base'),
                  constructor.parse('http://[') === null,
                  Object.getPrototypeOf(parsed) === constructor.prototype,
                  Object.prototype.toString.call(parsed),
                ],
                methods: ['parse', 'canParse', 'createObjectURL', 'revokeObjectURL'].map(name => {
                  const descriptor = Object.getOwnPropertyDescriptor(constructor, name);
                  return [name, name in constructor, Object.hasOwn(constructor, name),
                    typeof constructor[name], typeof constructor.prototype[name],
                    descriptor ? [descriptor.value.name, descriptor.value.length,
                      descriptor.writable, descriptor.enumerable, descriptor.configurable] : null];
                }),
              };
            })())"#,
        );
        let script = v8::Script::compile(scope, source, None).expect("URL exposure script");
        let actual = crate::script_execution::execute_compiled_script(scope, script)
            .expect("URL exposure evaluation")
            .to_rust_string_lossy(scope);
        let actual: serde_json::Value = serde_json::from_str(&actual).expect("URL exposure JSON");
        let methods: Vec<_> = ["parse", "canParse", "createObjectURL", "revokeObjectURL"]
            .into_iter()
            .map(|name| {
                if realm == RealmKind::ServiceWorker
                    && matches!(name, "createObjectURL" | "revokeObjectURL")
                {
                    serde_json::json!([name, false, false, "undefined", "undefined", null])
                } else {
                    serde_json::json!([
                        name,
                        true,
                        true,
                        "function",
                        "undefined",
                        [name, 1, true, true, true]
                    ])
                }
            })
            .collect();
        assert_eq!(
            actual,
            serde_json::json!({
                "stable": true,
                "common": [
                    "https://example.test/child?q=one&q=two",
                    ["one", "two"],
                    true,
                    true,
                    true,
                    "[object URL]",
                ],
                "methods": methods,
            }),
            "{realm:?} URL statics must follow their exposure set"
        );
    }
}
