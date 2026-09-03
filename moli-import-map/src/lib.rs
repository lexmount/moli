//! Import map registration and module specifier resolution.
//!
//! This crate stores and merges import maps, tracks already-resolved module
//! specifiers, and resolves module specifiers against document/module base URLs
//! without depending on the V8 module runtime.

mod helpers;
mod import_map;
mod registry;
mod types;

pub use types::ImportMapRegistryState;

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::ImportMapRegistryState;

    #[test]
    fn registry_unifies_registration_and_resolution() {
        let base_url = Url::parse("https://example.test/app/index.html").unwrap();
        let mut registry = ImportMapRegistryState::default();
        registry
            .register_import_map(r#"{"imports":{"fixture":"/mapped.mjs"}}"#, &base_url)
            .expect("initial import map should register");

        let resolved = registry
            .resolve_module_specifier("fixture", &base_url)
            .expect("mapped specifier should resolve");
        assert_eq!(resolved.as_str(), "https://example.test/mapped.mjs");
    }

    #[test]
    fn later_import_maps_add_unresolved_specifiers_after_module_resolution_starts() {
        let base_url = Url::parse("https://example.test/app/index.html").unwrap();
        let mut registry = ImportMapRegistryState::default();
        let first = registry
            .resolve_module_specifier("./already-resolved.mjs", &base_url)
            .expect("module resolution should start before the later import map");
        assert_eq!(
            first.as_str(),
            "https://example.test/app/already-resolved.mjs"
        );

        registry
            .register_import_map(r#"{"imports":{"late":"/late.mjs"}}"#, &base_url)
            .expect("later import map should merge");
        let late = registry
            .resolve_module_specifier("late", &base_url)
            .expect("a previously unresolved specifier should use the later import map");
        assert_eq!(late.as_str(), "https://example.test/late.mjs");
    }

    #[test]
    fn resolved_modules_are_not_remapped_by_late_import_maps() {
        let base_url = Url::parse("https://example.test/app/index.html").unwrap();
        let mut registry = ImportMapRegistryState::default();

        let first = registry
            .resolve_module_specifier("./mod.mjs", &base_url)
            .expect("relative module should resolve without import map");
        assert_eq!(first.as_str(), "https://example.test/app/mod.mjs");

        registry
            .register_import_map(
                r#"{"imports":{"./mod.mjs":"/replacement.mjs","new":"/new.mjs"}}"#,
                &base_url,
            )
            .expect("later import map should merge");

        let second = registry
            .resolve_module_specifier("./mod.mjs", &base_url)
            .expect("previously resolved module should keep original resolution");
        assert_eq!(second.as_str(), "https://example.test/app/mod.mjs");
        assert_eq!(
            registry
                .resolve_module_specifier("new", &base_url)
                .expect("late import map should add unresolved specifiers")
                .as_str(),
            "https://example.test/new.mjs"
        );
    }

    #[test]
    fn registry_uses_import_map_crate_for_scopes_prefixes_and_null_entries() {
        let base_url = Url::parse("https://example.test/app/index.html").unwrap();
        let scoped_referrer = Url::parse("https://example.test/scope/main.mjs").unwrap();
        let other_referrer = Url::parse("https://example.test/other/main.mjs").unwrap();
        let mut registry = ImportMapRegistryState::default();

        registry
            .register_import_map(
                r#"{
                    "imports": {
                        "blocked": null,
                        "pkg/": "/packages/pkg/"
                    },
                    "scopes": {
                        "/scope/": {
                            "dep": "/scope/dep.mjs"
                        }
                    }
                }"#,
                &base_url,
            )
            .expect("import map should register");

        let prefixed = registry
            .resolve_module_specifier("pkg/component.mjs", &base_url)
            .expect("package prefix should resolve");
        assert_eq!(
            prefixed.as_str(),
            "https://example.test/packages/pkg/component.mjs"
        );

        let scoped = registry
            .resolve_module_specifier("dep", &scoped_referrer)
            .expect("scoped bare specifier should resolve");
        assert_eq!(scoped.as_str(), "https://example.test/scope/dep.mjs");

        assert!(
            registry
                .resolve_module_specifier("dep", &other_referrer)
                .is_err()
        );
        assert!(
            registry
                .resolve_module_specifier("blocked", &base_url)
                .expect_err("null import map entries should block resolution")
                .contains("Blocked by null entry")
        );
    }

    #[test]
    fn registry_resolves_import_map_integrity_by_normalized_url() {
        let base_url = Url::parse("https://example.test/app/index.html").unwrap();
        let mut registry = ImportMapRegistryState::default();

        registry
            .register_import_map(
                r#"{
                    "imports": { "entry": "/entry.mjs" },
                    "integrity": {
                        "/entry.mjs": "sha384-entry",
                        "./dep.mjs": "sha384-dep",
                        "bare": "sha384-ignored",
                        "/not-string.mjs": 7
                    }
                }"#,
                &base_url,
            )
            .expect("import map with integrity should register");

        assert_eq!(
            registry
                .resolve_module_integrity(&Url::parse("https://example.test/entry.mjs").unwrap())
                .as_deref(),
            Some("sha384-entry")
        );
        assert_eq!(
            registry
                .resolve_module_integrity(&Url::parse("https://example.test/app/dep.mjs").unwrap())
                .as_deref(),
            Some("sha384-dep")
        );
        assert!(
            registry
                .resolve_module_integrity(&Url::parse("https://example.test/app/bare").unwrap())
                .is_none()
        );
        assert!(
            registry
                .resolve_module_integrity(
                    &Url::parse("https://example.test/not-string.mjs").unwrap()
                )
                .is_none()
        );
    }

    #[test]
    fn registry_keeps_first_import_map_integrity_rule_on_conflict() {
        let base_url = Url::parse("https://example.test/app/index.html").unwrap();
        let mut registry = ImportMapRegistryState::default();

        registry
            .register_import_map(r#"{"integrity":{"/entry.mjs":"sha384-first"}}"#, &base_url)
            .expect("first integrity rule should register");
        registry
            .register_import_map(r#"{"integrity":{"/entry.mjs":"sha384-second"}}"#, &base_url)
            .expect("conflicting integrity rule should not fail registration");

        assert_eq!(
            registry
                .resolve_module_integrity(&Url::parse("https://example.test/entry.mjs").unwrap())
                .as_deref(),
            Some("sha384-first")
        );
    }
}
