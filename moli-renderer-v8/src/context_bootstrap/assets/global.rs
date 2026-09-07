use super::super::shared::define_global_template_value;
use super::super::{
    exposed_interfaces::{
        ExposedInterfaceTemplateRegistry, TemplateBuildProfile, constructor_spec_is_eager,
        install_window_exposed_interfaces,
    },
    specs::constructor_specs,
    window_template::install_window_own_template_bindings,
};
use anyhow::{Result, anyhow};

pub(crate) struct ContextBootstrapAssets {
    global_template: v8::Global<v8::ObjectTemplate>,
    cross_origin_window_global_template: v8::Global<v8::ObjectTemplate>,
}

impl ContextBootstrapAssets {
    pub(crate) fn build(scope: &mut v8::PinScope<'_, '_, ()>) -> Result<Self> {
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let total_start = timing_enabled.then(std::time::Instant::now);

        let specs_start = timing_enabled.then(std::time::Instant::now);
        let constructor_specs = constructor_specs();
        if let Some(specs_start) = specs_start {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "constructor_specs_count",
                elapsed_ms = specs_start.elapsed().as_secs_f64() * 1000.0,
                count = constructor_specs.len(),
                "constructor_specs() returned"
            );
        }

        let registry = ExposedInterfaceTemplateRegistry::install(
            scope,
            constructor_specs.clone(),
            TemplateBuildProfile::Window,
        )?;

        let global_template_start = timing_enabled.then(std::time::Instant::now);
        let window_id = registry
            .id_by_name("Window")
            .ok_or_else(|| anyhow!("missing constructor template metadata `Window`"))?;
        let window_template = registry.get_or_build_template(scope, window_id)?;
        // A cross-origin WindowProxy shell must retain Window's V8 wrapper
        // identity so that detaching and reusing it for the committed child
        // realm discards the facade's temporary own properties. It must not,
        // however, inherit the same-origin [Global] members installed below:
        // non-configurable Window.location would collide with the restricted
        // cross-origin Location surface before the child realm exists.
        let cross_origin_window_global_template =
            v8::ObjectTemplate::new_from_template(scope, window_template);
        crate::native_bridge::install_child_window_proxy_access_check_handlers(
            cross_origin_window_global_template,
        );
        let global_template = v8::ObjectTemplate::new_from_template(scope, window_template);
        crate::native_bridge::install_child_window_proxy_access_check_handlers(global_template);
        install_window_own_template_bindings(scope, global_template);

        install_window_exposed_interfaces(scope, global_template, &registry)?;
        for spec in &constructor_specs {
            if !constructor_spec_is_eager(*spec) {
                continue;
            }
            let id = registry
                .id_by_name(spec.name)
                .ok_or_else(|| anyhow!("missing constructor template metadata `{}`", spec.name))?;
            let template = registry.get_or_build_template(scope, id)?;
            define_global_template_value(scope, global_template, spec.name, template.into())?;
        }

        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "define_global_template",
                elapsed_ms = global_template_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "Define global template with lazy constructor metadata"
            );
            tracing::info!(
                target: "moli_cdp_nav_timing",
                stage = "context_bootstrap_assets_build_total",
                elapsed_ms = total_start.unwrap().elapsed().as_secs_f64() * 1000.0,
                "ContextBootstrapAssets::build total"
            );
        }

        Ok(Self {
            global_template: v8::Global::new(scope, global_template),
            cross_origin_window_global_template: v8::Global::new(
                scope,
                cross_origin_window_global_template,
            ),
        })
    }

    pub(crate) fn global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        v8::Local::new(scope, &self.global_template)
    }

    pub(crate) fn cross_origin_window_global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        v8::Local::new(scope, &self.cross_origin_window_global_template)
    }
}
